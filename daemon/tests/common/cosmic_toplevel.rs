// SPDX-License-Identifier: MIT
//! Native COSMIC toplevel activation for controlled runtime tests.

use cosmic_protocols::{
    toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1},
    toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
};
use std::{
    os::fd::{AsFd, AsRawFd},
    time::{Duration, Instant},
};
use wayland_client::{
    event_created_child,
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};

#[derive(Default)]
struct ActivationState {
    info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
    list: Option<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1>,
    manager: Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    toplevels: Vec<Toplevel>,
}

struct Toplevel {
    foreign_handle: ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    cosmic_handle: zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
    title: Option<String>,
    app_id: Option<String>,
    activated: bool,
    geometry: Option<(i32, i32, i32, i32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for ActivationState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
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
                "zcosmic_toplevel_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ActivationState {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        eprintln!("cosmic-toplevel: seat id={:?} event={event:?}", seat.id());
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for ActivationState {
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
            let cosmic_handle = info.get_cosmic_toplevel(&toplevel, qh, ());
            state.toplevels.push(Toplevel {
                foreign_handle: toplevel,
                cosmic_handle,
                title: None,
                app_id: None,
                activated: false,
                geometry: None,
            });
        }
    }

    event_created_child!(
        ActivationState,
        ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (
                ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
                ()
            ),
        ]
    );
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for ActivationState {
    fn event(
        state: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(toplevel) = state
            .toplevels
            .iter_mut()
            .find(|candidate| &candidate.foreign_handle == handle)
        else {
            return;
        };

        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                toplevel.title = Some(title);
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                toplevel.app_id = Some(app_id);
            }
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for ActivationState {
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

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for ActivationState {
    fn event(
        state: &mut Self,
        handle: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(toplevel) = state
            .toplevels
            .iter_mut()
            .find(|candidate| &candidate.cosmic_handle == handle)
        else {
            return;
        };

        match event {
            zcosmic_toplevel_handle_v1::Event::State { state } => {
                let states = state
                    .chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()))
                    .collect::<Vec<_>>();
                toplevel.activated = states.contains(&2);
                eprintln!(
                    "cosmic-toplevel: state title={:?} states={states:?} activated={}",
                    toplevel.title, toplevel.activated
                );
            }
            zcosmic_toplevel_handle_v1::Event::Geometry {
                output: _,
                x,
                y,
                width,
                height,
            } => {
                toplevel.geometry = Some((x, y, width, height));
            }
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, ()> for ActivationState {
    fn event(
        _: &mut Self,
        _: &zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1,
        _: zcosmic_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn matches_controlled_toplevel(
    title: Option<&str>,
    app_id: Option<&str>,
    title_fragment: &str,
    accepted_app_ids: &[&str],
) -> bool {
    title.is_some_and(|title| title.contains(title_fragment))
        && app_id.is_some_and(|app_id| {
            let app_id = app_id.to_ascii_lowercase();
            accepted_app_ids.iter().any(|accepted| {
                let accepted = accepted.to_ascii_lowercase();
                app_id == accepted || accepted == "chrome-" && app_id.starts_with("chrome-")
            })
        })
}

fn dispatch_with_deadline(
    connection: &Connection,
    queue: &mut EventQueue<ActivationState>,
    state: &mut ActivationState,
    deadline: Instant,
) -> Result<(), String> {
    queue
        .dispatch_pending(state)
        .map_err(|error| error.to_string())?;
    connection.flush().map_err(|error| error.to_string())?;

    let Some(guard) = queue.prepare_read() else {
        return Ok(());
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(());
    }

    let timeout_ms = remaining
        .min(Duration::from_millis(25))
        .as_millis()
        .min(i32::MAX as u128) as i32;
    let mut descriptor = libc::pollfd {
        fd: connection.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if result < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if result > 0 {
        guard.read().map_err(|error| error.to_string())?;
        queue
            .dispatch_pending(state)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn activate_controlled_toplevel(
    title_fragment: &str,
    accepted_app_ids: &[&str],
    require_geometry: bool,
    timeout: Duration,
) -> Result<(i32, i32), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let display = connection.display();
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = ActivationState::default();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        dispatch_with_deadline(&connection, &mut queue, &mut state, deadline)?;

        let matching_handle = state
            .toplevels
            .iter()
            .find(|toplevel| {
                matches_controlled_toplevel(
                    toplevel.title.as_deref(),
                    toplevel.app_id.as_deref(),
                    title_fragment,
                    accepted_app_ids,
                )
            })
            .map(|toplevel| {
                eprintln!(
                    "cosmic-toplevel: activating title={:?} app_id={:?}",
                    toplevel.title, toplevel.app_id
                );
                toplevel.cosmic_handle.clone()
            });

        if matching_handle.is_none() {
            let discovered = state
                .toplevels
                .iter()
                .map(|toplevel| {
                    format!(
                        "title={:?} app_id={:?} activated={} geometry={:?}",
                        toplevel.title, toplevel.app_id, toplevel.activated, toplevel.geometry
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "cosmic-toplevel: no match for title={title_fragment:?} app_ids={accepted_app_ids:?}; discovered={discovered:?}"
            );
        }

        if let (Some(manager), Some(seat), Some(handle)) =
            (state.manager.clone(), state.seat.clone(), matching_handle)
        {
            manager.activate(&handle, &seat);
            connection.flush().map_err(|error| error.to_string())?;

            while Instant::now() < deadline {
                manager.activate(&handle, &seat);
                connection.flush().map_err(|error| error.to_string())?;
                dispatch_with_deadline(&connection, &mut queue, &mut state, deadline)?;
                if let Some(toplevel) = state
                    .toplevels
                    .iter()
                    .find(|toplevel| toplevel.cosmic_handle == handle && toplevel.activated)
                {
                    if let Some((x, y, width, height)) = toplevel.geometry {
                        return Ok((x + width / 2, y + height / 2));
                    }
                    if !require_geometry {
                        return Ok((0, 0));
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }

            return Err(
                "COSMIC compositor did not confirm controlled toplevel activation".to_string(),
            );
        }

        std::thread::sleep(Duration::from_millis(25));
    }

    Err(format!(
        "timed out locating controlled toplevel title={title_fragment:?} app_ids={accepted_app_ids:?}"
    ))
}

pub fn activate_cosmic_edit(title_fragment: &str, timeout: Duration) -> Result<(i32, i32), String> {
    activate_controlled_toplevel(
        title_fragment,
        &[
            "com.system76.cosmicedit",
            "com.system76.cosmic-edit",
            "cosmic-edit",
        ],
        true,
        timeout,
    )
}

pub fn activate_chromium(title_fragment: &str, timeout: Duration) -> Result<(i32, i32), String> {
    activate_controlled_toplevel(
        title_fragment,
        &[
            "chromium-browser",
            "chromium",
            "google-chrome",
            "microsoft-edge",
            "microsoft-edge-stable",
            "microsoft-edge-beta",
            "microsoft-edge-dev",
            "chrome-",
        ],
        false,
        timeout,
    )
}

pub fn activate_firefox(title_fragment: &str, timeout: Duration) -> Result<(i32, i32), String> {
    activate_controlled_toplevel(
        title_fragment,
        &["firefox", "org.mozilla.firefox"],
        false,
        timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::matches_controlled_toplevel;

    #[test]
    fn matches_only_controlled_cosmic_edit_title_and_app_id() {
        let app_ids = [
            "com.system76.cosmicedit",
            "com.system76.cosmic-edit",
            "cosmic-edit",
        ];
        assert!(matches_controlled_toplevel(
            Some("controlled-target.txt"),
            Some("com.system76.CosmicEdit"),
            "controlled-target.txt",
            &app_ids,
        ));
        assert!(!matches_controlled_toplevel(
            Some("controlled-target.txt"),
            Some("org.mozilla.firefox"),
            "controlled-target.txt",
            &app_ids,
        ));
        assert!(!matches_controlled_toplevel(
            Some("unrelated.txt"),
            Some("com.system76.CosmicEdit"),
            "controlled-target.txt",
            &app_ids,
        ));
        assert!(!matches_controlled_toplevel(
            Some("controlled-target.txt"),
            None,
            "controlled-target.txt",
            &app_ids,
        ));
        assert!(matches_controlled_toplevel(
            Some("Vietc controlled runtime focus-switch-a"),
            Some("chrome-__fixture_controlled-target.html-Default"),
            "Vietc controlled runtime focus-switch-a",
            &["chrome-"],
        ));
    }
}
