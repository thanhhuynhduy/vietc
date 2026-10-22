// SPDX-License-Identifier: MIT
//! Native COSMIC toplevel activation for controlled runtime tests.

use cosmic_protocols::{
    toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1},
    toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
};
use std::time::{Duration, Instant};
use wayland_client::{
    event_created_child,
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
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
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
                toplevel.activated = state
                    .chunks_exact(4)
                    .any(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()) == 2);
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

fn matches_cosmic_edit_toplevel(
    title: Option<&str>,
    app_id: Option<&str>,
    title_fragment: &str,
) -> bool {
    title.is_some_and(|title| title.contains(title_fragment))
        && app_id.is_some_and(|app_id| {
            matches!(
                app_id.to_ascii_lowercase().as_str(),
                "com.system76.cosmicedit" | "com.system76.cosmic-edit" | "cosmic-edit"
            )
        })
}

pub fn activate_cosmic_edit(title_fragment: &str, timeout: Duration) -> Result<(i32, i32), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let display = connection.display();
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = ActivationState::default();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        queue
            .roundtrip(&mut state)
            .map_err(|error| error.to_string())?;

        let matching_handle = state
            .toplevels
            .iter()
            .find(|toplevel| {
                matches_cosmic_edit_toplevel(
                    toplevel.title.as_deref(),
                    toplevel.app_id.as_deref(),
                    title_fragment,
                )
            })
            .map(|toplevel| {
                eprintln!(
                    "cosmic-toplevel: activating title={:?} app_id={:?}",
                    toplevel.title, toplevel.app_id
                );
                toplevel.cosmic_handle.clone()
            });

        if let (Some(manager), Some(seat), Some(handle)) =
            (&state.manager, &state.seat, matching_handle)
        {
            manager.activate(&handle, seat);
            connection.flush().map_err(|error| error.to_string())?;

            while Instant::now() < deadline {
                queue
                    .roundtrip(&mut state)
                    .map_err(|error| error.to_string())?;
                if let Some((x, y, width, height)) = state
                    .toplevels
                    .iter()
                    .find(|toplevel| toplevel.cosmic_handle == handle && toplevel.activated)
                    .and_then(|toplevel| toplevel.geometry)
                {
                    return Ok((x + width / 2, y + height / 2));
                }
                std::thread::sleep(Duration::from_millis(25));
            }

            return Err(
                "COSMIC compositor did not confirm controlled toplevel activation".to_string(),
            );
        }

        std::thread::sleep(Duration::from_millis(25));
    }

    Err("timed out locating the controlled COSMIC Edit toplevel".to_string())
}

#[cfg(test)]
mod tests {
    use super::matches_cosmic_edit_toplevel;

    #[test]
    fn matches_only_controlled_cosmic_edit_title_and_app_id() {
        assert!(matches_cosmic_edit_toplevel(
            Some("controlled-target.txt"),
            Some("com.system76.CosmicEdit"),
            "controlled-target.txt",
        ));
        assert!(!matches_cosmic_edit_toplevel(
            Some("controlled-target.txt"),
            Some("org.mozilla.firefox"),
            "controlled-target.txt",
        ));
        assert!(!matches_cosmic_edit_toplevel(
            Some("unrelated.txt"),
            Some("com.system76.CosmicEdit"),
            "controlled-target.txt",
        ));
        assert!(!matches_cosmic_edit_toplevel(
            Some("controlled-target.txt"),
            None,
            "controlled-target.txt",
        ));
    }
}
