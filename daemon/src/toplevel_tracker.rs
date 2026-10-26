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

use crate::focus::SharedFocusSnapshot;
use crate::log::log_info;

/// `zcosmic_toplevel_handle_v1::State::Activated`.
const STATE_ACTIVATED: u32 = 2;
const RETRY_DELAY: Duration = Duration::from_secs(3);

struct Toplevel {
    serial: u64,
    foreign: ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    cosmic: zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
    app_id: Option<String>,
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
    NewWindow { serial: u64, app_id: Option<String> },
    /// The active window's app_id arrived/changed after activation.
    ClassOnly(Option<String>),
}

/// Pure decision: compare the currently active toplevel with what was
/// published last. Kept free of Wayland types so it can be unit-tested.
fn plan_publish(
    last: &Published,
    active: Option<(u64, Option<String>)>,
) -> (Publish, Published) {
    let Some((serial, app_id)) = active else {
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
    info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
    list: Option<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1>,
    toplevels: Vec<Toplevel>,
    next_serial: u64,
    published: Published,
    focus: SharedFocusSnapshot,
}

impl Tracker {
    fn new(focus: SharedFocusSnapshot) -> Self {
        Self {
            info: None,
            list: None,
            toplevels: Vec::new(),
            next_serial: 1,
            published: Published::default(),
            focus,
        }
    }

    fn publish(&mut self) {
        let active = self
            .toplevels
            .iter()
            .find(|toplevel| toplevel.activated)
            .map(|toplevel| (toplevel.serial, toplevel.app_id.clone()));
        let (action, next) = plan_publish(&self.published, active);
        self.published = next;
        match action {
            Publish::Nothing => {}
            Publish::NewWindow { serial, app_id } => {
                log_info(&format!(
                    "[vietc] toplevel activated: serial={serial} app_id={app_id:?}"
                ));
                let mut focus = self.focus.lock().unwrap();
                focus.observe(Some(format!("toplevel:{serial}")), app_id);
                focus.mark_class_from_compositor();
            }
            Publish::ClassOnly(app_id) => {
                self.focus.lock().unwrap().set_class(app_id);
            }
        }
    }
}

impl Dispatch<WlRegistry, ()> for Tracker {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zcosmic_toplevel_info_v1" => {
                    state.info = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "ext_foreign_toplevel_list_v1" => {
                    state.list = Some(registry.bind(name, version.min(1), qh, ()));
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
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(index) = state.toplevels.iter().position(|t| &t.foreign == handle) {
                    let closed = state.toplevels.remove(index);
                    closed.cosmic.destroy();
                    closed.foreign.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for Tracker {
    fn event(
        _: &mut Self,
        _: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        _: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
        if let zcosmic_toplevel_handle_v1::Event::State { state: raw } = event {
            if let Some(toplevel) = state.toplevels.iter_mut().find(|t| &t.cosmic == handle) {
                toplevel.activated = raw
                    .chunks_exact(4)
                    .any(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()) == STATE_ACTIVATED);
            }
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
    let _registry = connection.display().get_registry(&qh, ());
    let mut tracker = Tracker::new(focus);

    queue
        .roundtrip(&mut tracker)
        .map_err(|error| RunError::Transient(error.to_string()))?;
    if tracker.info.is_none() || tracker.list.is_none() {
        return Err(RunError::Unsupported);
    }
    queue
        .roundtrip(&mut tracker)
        .map_err(|error| RunError::Transient(error.to_string()))?;
    tracker.publish();
    log_info("[vietc] Compositor toplevel focus tracker active");

    loop {
        queue
            .blocking_dispatch(&mut tracker)
            .map_err(|error| RunError::Transient(error.to_string()))?;
        tracker.publish();
    }
}

/// Spawn the tracker thread. Silently ends when the compositor does not offer
/// the toplevel protocols (AT-SPI and pointer reset still apply there).
pub fn spawn_toplevel_focus_tracker(focus: SharedFocusSnapshot) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    std::thread::spawn(move || loop {
        match run_once(focus.clone()) {
            Ok(()) => return,
            Err(RunError::Unsupported) => {
                log_info("[vietc] Compositor has no toplevel-info protocol; using AT-SPI focus only");
                return;
            }
            Err(RunError::Transient(error)) => {
                log_info(&format!(
                    "[vietc] toplevel tracker disconnected: {error}; retrying"
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
    fn no_active_toplevel_keeps_previous_state() {
        let last = Published {
            serial: Some(4),
            app_id: Some("edge".into()),
        };
        let (action, next) = plan_publish(&last, None);
        assert!(matches!(action, Publish::Nothing));
        assert_eq!(next, last);
    }
}
