// SPDX-License-Identifier: MIT
//! Compositor-level active toplevel tracking.
//!
//! AT-SPI focus events are only emitted by applications that implement
//! accessibility (terminals such as cosmic-term and many Electron builds do
//! not), so relying on them alone lets a half-typed word leak into whichever
//! window receives focus next. Wayland compositors that expose the toplevel
//! protocols tell us *authoritatively* which window is activated and what its
//! `app_id` is; every change bumps the focus generation (which cancels pending
//! replacements and resets the composition) and publishes the app class used
//! for terminal / English-app detection.

use std::time::Duration;

use cosmic_protocols::toplevel_info::v1::client::{
    zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1,
};
use wayland_client::{
    event_created_child,
    protocol::wl_registry::{self, WlRegistry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};

use crate::focus::SharedFocusSnapshot;
use crate::log::log_info;

/// `State::Activated` (`2`) in both `zcosmic_toplevel_handle_v1` and
/// `zwlr_foreign_toplevel_handle_v1`.
const STATE_ACTIVATED: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToplevelBackend {
    Cosmic,
    Wlr,
}

fn select_backend(
    has_cosmic_info: bool,
    has_foreign_list: bool,
    has_wlr_manager: bool,
) -> Option<ToplevelBackend> {
    if has_cosmic_info && has_foreign_list {
        Some(ToplevelBackend::Cosmic)
    } else if has_wlr_manager {
        Some(ToplevelBackend::Wlr)
    } else {
        None
    }
}

struct Toplevel {
    serial: u64,
    foreign: ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    cosmic: zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
    app_id: Option<String>,
    title: Option<String>,
    activated: bool,
}

struct WlrToplevel {
    serial: u64,
    handle: zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    app_id: Option<String>,
    title: Option<String>,
    activated: bool,
}

/// What the tracker last published, so unchanged states never bump the focus
/// generation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Published {
    serial: Option<u64>,
    app_id: Option<String>,
}

enum Publish {
    Nothing,
    /// A different window became active.
    NewWindow {
        serial: u64,
        app_id: Option<String>,
    },
    /// The active window's app_id arrived/changed after activation.
    ClassOnly(Option<String>),
    /// All toplevels lost `State::Activated` (e.g., layer-shell launcher or Polkit dialog).
    FocusLost,
}

/// Pure decision: compare the currently active toplevel with what was
/// published last. Kept free of Wayland types so it can be unit-tested.
fn plan_publish(last: &Published, active: Option<(u64, Option<String>)>) -> (Publish, Published) {
    let Some((serial, app_id)) = active else {
        if last.serial.is_some() {
            return (Publish::FocusLost, Published::default());
        }
        return (Publish::Nothing, last.clone());
    };
    let next = Published {
        serial: Some(serial),
        app_id: app_id.clone(),
    };
    if last.serial != Some(serial) {
        (Publish::NewWindow { serial, app_id }, next)
    } else if last.app_id != app_id {
        (Publish::ClassOnly(app_id), next)
    } else {
        (Publish::Nothing, last.clone())
    }
}

struct Tracker {
    cosmic_info_global: Option<(u32, u32)>,
    foreign_list_global: Option<(u32, u32)>,
    wlr_manager_global: Option<(u32, u32)>,
    info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
    list: Option<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1>,
    wlr_manager: Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
    toplevels: Vec<Toplevel>,
    wlr_toplevels: Vec<WlrToplevel>,
    next_serial: u64,
    published: Published,
    focus: SharedFocusSnapshot,
    saw_done: bool,
}

impl Tracker {
    fn new(focus: SharedFocusSnapshot) -> Self {
        Self {
            cosmic_info_global: None,
            foreign_list_global: None,
            wlr_manager_global: None,
            info: None,
            list: None,
            wlr_manager: None,
            toplevels: Vec::new(),
            wlr_toplevels: Vec::new(),
            next_serial: 1,
            published: Published::default(),
            focus,
            saw_done: false,
        }
    }

    fn publish(&mut self) {
        let active_full = self
            .toplevels
            .iter()
            .find(|toplevel| toplevel.activated)
            .map(|toplevel| {
                (
                    toplevel.serial,
                    toplevel.app_id.clone(),
                    toplevel.title.clone(),
                )
            })
            .or_else(|| {
                self.wlr_toplevels
                    .iter()
                    .find(|toplevel| toplevel.activated)
                    .map(|toplevel| {
                        (
                            toplevel.serial,
                            toplevel.app_id.clone(),
                            toplevel.title.clone(),
                        )
                    })
            });
        let active_title = active_full.as_ref().and_then(|(_, _, t)| t.clone());
        crate::password_detector::record_compositor_window_title(active_title);
        let active = active_full.map(|(serial, app_id, _)| (serial, app_id));
        let (action, next) = plan_publish(&self.published, active);
        self.published = next;
        match action {
            Publish::Nothing => {}
            Publish::NewWindow { serial, app_id } => {
                crate::password_detector::clear_atspi_password_flag_on_window_switch();
                crate::candidate_ipc::record_caret_rect(None);
                log_info(&format!(
                    "[dboard] toplevel activated: serial={serial} app_id={app_id:?}"
                ));
                let mut focus = self.focus.lock().unwrap_or_else(|p| p.into_inner());
                focus.observe(Some(format!("toplevel:{serial}")), app_id);
                focus.mark_class_from_compositor();
            }
            Publish::ClassOnly(app_id) => {
                self.focus
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set_class(app_id);
            }
            Publish::FocusLost => {
                crate::password_detector::clear_atspi_password_flag_on_window_switch();
                crate::candidate_ipc::record_caret_rect(None);
                log_info("[dboard] toplevel focus lost (layer-shell / overlay active)");
                self.focus
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .observe_focus_lost();
            }
        }
    }
}

impl Dispatch<WlRegistry, ()> for Tracker {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zcosmic_toplevel_info_v1" => {
                    state.cosmic_info_global = Some((name, version));
                }
                "ext_foreign_toplevel_list_v1" => {
                    state.foreign_list_global = Some((name, version));
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    state.wlr_manager_global = Some((name, version));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        _: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            let Some(info) = state.info.as_ref() else {
                return;
            };
            let cosmic = info.get_cosmic_toplevel(&toplevel, qh, ());
            let serial = state.next_serial;
            state.next_serial += 1;
            state.toplevels.push(Toplevel {
                serial,
                foreign: toplevel,
                cosmic,
                app_id: None,
                title: None,
                activated: false,
            });
        }
    }

    event_created_child!(
        Tracker,
        ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (
                ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
                ()
            ),
        ]
    );
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(toplevel) = state.toplevels.iter_mut().find(|t| &t.foreign == handle) {
                    toplevel.app_id = Some(app_id).filter(|value| !value.is_empty());
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(toplevel) = state.toplevels.iter_mut().find(|t| &t.foreign == handle) {
                    toplevel.title = Some(title).filter(|value| !value.is_empty());
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                state.saw_done = true;
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(index) = state.toplevels.iter().position(|t| &t.foreign == handle) {
                    let closed = state.toplevels.remove(index);
                    closed.cosmic.destroy();
                    closed.foreign.destroy();
                }
                state.saw_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        _: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zcosmic_toplevel_info_v1::Event::Done) {
            state.saw_done = true;
        }
    }
}

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        handle: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zcosmic_toplevel_handle_v1::Event::State { state: raw } => {
                if let Some(toplevel) = state.toplevels.iter_mut().find(|t| &t.cosmic == handle) {
                    toplevel.activated = raw
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|&chunk| u32::from_ne_bytes(chunk) == STATE_ACTIVATED);
                }
            }
            zcosmic_toplevel_handle_v1::Event::Closed => {
                state.saw_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            let serial = state.next_serial;
            state.next_serial += 1;
            state.wlr_toplevels.push(WlrToplevel {
                serial,
                handle: toplevel,
                app_id: None,
                title: None,
                activated: false,
            });
        }
    }

    event_created_child!(
        Tracker,
        zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        [
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (
                zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
                ()
            ),
        ]
    );
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for Tracker {
    fn event(
        state: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(toplevel) = state.wlr_toplevels.iter_mut().find(|t| &t.handle == handle)
                {
                    toplevel.app_id = Some(app_id).filter(|value| !value.is_empty());
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(toplevel) = state.wlr_toplevels.iter_mut().find(|t| &t.handle == handle)
                {
                    toplevel.title = Some(title).filter(|value| !value.is_empty());
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: raw } => {
                if let Some(toplevel) = state.wlr_toplevels.iter_mut().find(|t| &t.handle == handle)
                {
                    toplevel.activated = raw
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|&chunk| u32::from_ne_bytes(chunk) == STATE_ACTIVATED);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.saw_done = true;
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(index) = state.wlr_toplevels.iter().position(|t| &t.handle == handle) {
                    let closed = state.wlr_toplevels.remove(index);
                    closed.handle.destroy();
                }
                state.saw_done = true;
            }
            _ => {}
        }
    }
}

enum RunError {
    /// Compositor lacks the toplevel protocols; retrying is pointless.
    Unsupported,
    Transient(String),
}

fn run_once(focus: SharedFocusSnapshot) -> Result<(), RunError> {
    let connection =
        Connection::connect_to_env().map_err(|error| RunError::Transient(error.to_string()))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    let mut tracker = Tracker::new(focus);

    queue
        .roundtrip(&mut tracker)
        .map_err(|error| RunError::Transient(error.to_string()))?;
    let backend = select_backend(
        tracker.cosmic_info_global.is_some(),
        tracker.foreign_list_global.is_some(),
        tracker.wlr_manager_global.is_some(),
    )
    .ok_or(RunError::Unsupported)?;

    match backend {
        ToplevelBackend::Cosmic => {
            let (info_name, info_ver) = tracker.cosmic_info_global.unwrap();
            let (list_name, list_ver) = tracker.foreign_list_global.unwrap();
            tracker.info = Some(registry.bind(info_name, info_ver.min(2), &qh, ()));
            tracker.list = Some(registry.bind(list_name, list_ver.min(1), &qh, ()));
        }
        ToplevelBackend::Wlr => {
            let (mgr_name, mgr_ver) = tracker.wlr_manager_global.unwrap();
            tracker.wlr_manager = Some(registry.bind(mgr_name, mgr_ver.min(3), &qh, ()));
        }
    }

    queue
        .roundtrip(&mut tracker)
        .map_err(|error| RunError::Transient(error.to_string()))?;
    if backend == ToplevelBackend::Cosmic {
        // COSMIC creates zcosmic_toplevel_handle_v1 objects inside the second
        // roundtrip; one extra roundtrip ensures initial State/AppId arrive
        // before the first publish().
        queue
            .roundtrip(&mut tracker)
            .map_err(|error| RunError::Transient(error.to_string()))?;
    }
    tracker.saw_done = false;
    tracker.publish();
    log_info(&format!(
        "[dboard] Compositor toplevel focus tracker active ({backend:?})"
    ));

    loop {
        queue
            .blocking_dispatch(&mut tracker)
            .map_err(|error| RunError::Transient(error.to_string()))?;
        if std::mem::take(&mut tracker.saw_done) {
            tracker.publish();
        }
    }
}

/// Spawn the tracker thread. Silently ends when the compositor does not offer
/// the toplevel protocols (AT-SPI and pointer reset still apply there).
pub fn spawn_toplevel_focus_tracker(focus: SharedFocusSnapshot) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|v| v.is_empty()) {
        return;
    }
    std::thread::spawn(move || loop {
        match run_once(focus.clone()) {
            Ok(()) => return,
            Err(RunError::Unsupported) => {
                log_info(
                    "[dboard] Compositor has no toplevel-info protocol; using AT-SPI focus only",
                );
                return;
            }
            Err(RunError::Transient(error)) => {
                log_info(&format!(
                    "[dboard] toplevel tracker disconnected: {error}; retrying"
                ));
                std::thread::sleep(RETRY_DELAY);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{plan_publish, Publish, Published};

    fn active(serial: u64, app: &str) -> Option<(u64, Option<String>)> {
        Some((serial, Some(app.to_string())))
    }

    #[test]
    fn first_activation_publishes_new_window() {
        let (action, next) = plan_publish(&Published::default(), active(1, "term"));
        assert!(matches!(action, Publish::NewWindow { serial: 1, .. }));
        assert_eq!(next.serial, Some(1));
    }

    #[test]
    fn unchanged_active_window_never_republishes() {
        let last = Published {
            serial: Some(1),
            app_id: Some("term".into()),
        };
        let (action, _) = plan_publish(&last, active(1, "term"));
        assert!(matches!(action, Publish::Nothing));
    }

    #[test]
    fn switching_windows_publishes_even_when_app_id_is_identical() {
        let last = Published {
            serial: Some(1),
            app_id: Some("edge".into()),
        };
        let (action, next) = plan_publish(&last, active(2, "edge"));
        assert!(matches!(action, Publish::NewWindow { serial: 2, .. }));
        assert_eq!(next.serial, Some(2));
    }

    #[test]
    fn late_app_id_updates_class_without_new_generation() {
        let last = Published {
            serial: Some(3),
            app_id: None,
        };
        let (action, _) = plan_publish(&last, active(3, "term"));
        assert!(matches!(action, Publish::ClassOnly(Some(ref id)) if id == "term"));
    }

    #[test]
    fn losing_active_toplevel_publishes_focus_lost_once() {
        let last = Published {
            serial: Some(4),
            app_id: Some("edge".into()),
        };
        let (action, next) = plan_publish(&last, None);
        assert!(matches!(action, Publish::FocusLost));
        assert_eq!(next, Published::default());

        // Subsequent None -> None does not re-emit FocusLost
        let (action2, next2) = plan_publish(&next, None);
        assert!(matches!(action2, Publish::Nothing));
        assert_eq!(next2, Published::default());
    }

    #[test]
    fn backend_selection_prefers_cosmic_and_falls_back_to_wlr() {
        use super::{select_backend, ToplevelBackend};
        assert_eq!(
            select_backend(true, true, true),
            Some(ToplevelBackend::Cosmic)
        );
        assert_eq!(
            select_backend(true, true, false),
            Some(ToplevelBackend::Cosmic)
        );
        assert_eq!(
            select_backend(false, true, true),
            Some(ToplevelBackend::Wlr)
        );
        assert_eq!(
            select_backend(false, false, true),
            Some(ToplevelBackend::Wlr)
        );
        assert_eq!(select_backend(false, false, false), None);
    }
}
