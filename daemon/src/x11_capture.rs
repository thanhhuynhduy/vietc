use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::daemon::Daemon;
use crate::display;
use crate::focus::SharedFocusSnapshot;
use crate::inject::{create_injector, execute_commands};
use crate::log::log_info;
use crate::signal::SIGNAL_EXIT;

#[cfg(feature = "x11")]
pub fn run_with_x11_keymap(
    daemon: &mut Daemon,
    shared_focus: SharedFocusSnapshot,
    config_changed: Arc<std::sync::atomic::AtomicBool>,
    status_changed: Arc<std::sync::atomic::AtomicBool>,
    _engine_enabled: Arc<std::sync::atomic::AtomicBool>,
    display: display::DisplayServer,
) -> Result<(), Box<dyn std::error::Error>> {
    use vietc_protocol::x11_inject::set_keymap_suppress;
    use vietc_protocol::x11_inject::X11KeymapCapture;

    let mut capture = X11KeymapCapture::new()?;
    let injector = create_injector(display)?;
    let mut last_focus_generation = shared_focus.lock().unwrap().generation;
    let mut last_window_class = String::new();
    let mut key_state: HashSet<u32> = HashSet::new();

    // In keymap mode the original keystrokes stay on screen, so screen
    // accounting must use the raw on-screen length. This flag also drives the
    // feedback suppression while we inject our own keystrokes.
    daemon.keymap_mode = true;

    log_info("[vietc] X11 keymap capture active");
    loop {
        if SIGNAL_EXIT.load(Ordering::SeqCst) {
            log_info("[vietc] Exiting on signal");
            return Ok(());
        }

        if status_changed.load(Ordering::SeqCst) {
            daemon.sync_status_file();
            status_changed.store(false, Ordering::SeqCst);
        }
        if config_changed.load(Ordering::SeqCst) {
            daemon.reload_config();
            config_changed.store(false, Ordering::SeqCst);
        }

        {
            let generation = shared_focus.lock().unwrap().generation;
            if generation != last_focus_generation {
                last_focus_generation = generation;
                daemon.reset_composition_state();
            }
        }
        if daemon.config.app_state.enabled {
            let class = shared_focus
                .lock()
                .unwrap()
                .class
                .clone()
                .unwrap_or_default();
            if !class.is_empty() && class != last_window_class {
                last_window_class = class.clone();
                injector.set_active_window(&class);
                daemon.check_app_change_with(class.clone());
            }
        }

        let events = capture.poll();
        if events.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }

        for (keycode, pressed) in &events {
            if *pressed {
                key_state.insert(*keycode);
            } else {
                key_state.remove(keycode);
            }
        }

        for (keycode, pressed) in &events {
            if !*pressed {
                continue;
            }
            let keycode = *keycode;

            let shift_pressed = key_state.contains(&42) || key_state.contains(&54);
            let ctrl_pressed = key_state.contains(&29) || key_state.contains(&97);
            let alt_pressed = key_state.contains(&56) || key_state.contains(&100);
            let caps_state = key_state.contains(&58);

            let mut mod_state = 0i32;
            if shift_pressed {
                mod_state |= 1;
            }
            if caps_state {
                mod_state |= 2;
            }
            if ctrl_pressed {
                mod_state |= 4;
            }
            if alt_pressed {
                mod_state |= 8;
            }

            if ctrl_pressed && keycode == 57 {
                daemon.toggle_method();
                continue;
            }

            let is_mod = ctrl_pressed || alt_pressed || key_state.contains(&125);

            if is_mod {
                continue;
            }

            if daemon.config.app_state.enabled {
                let is_pw = daemon.app_state.check_password_field();
                let currently_enabled = daemon.engine.is_enabled();
                if is_pw && currently_enabled {
                    daemon.engine.set_enabled(false);
                    daemon.engine.reset();
                    daemon.replay_reset();
                    daemon.write_status();
                } else if !is_pw && !currently_enabled && daemon.config.start_enabled {
                    let default_state = daemon.app_state.get_default_state();
                    if default_state {
                        daemon.engine.set_enabled(true);
                        daemon.engine.reset();
                        daemon.replay_reset();
                        daemon.write_status();
                    }
                }
            }

            if !daemon.engine.is_enabled() {
                continue;
            }

            // Backspace (evdev 14) is not returned by lookup_keycode, so handle
            // it explicitly: record it in the engine and correct the screen.
            if *pressed && keycode == 14 {
                let commands = daemon.replay_backspace();
                if !commands.is_empty() {
                    set_keymap_suppress(true);
                    if !matches!(
                        execute_commands(&*injector, &commands, &mut daemon.visible_revision),
                        crate::commands::TransactionOutcome::Applied { .. }
                    ) {
                        log_info("[vietc] X11 keymap backspace injection failed");
                    }
                    // Keep suppressing while our injected keystrokes play out,
                    // resyncing the keymap baseline each poll so they are never
                    // re-captured. 200ms comfortably covers a short correction.
                    for _ in 0..40 {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        capture.poll();
                    }
                    set_keymap_suppress(false);
                }
                continue;
            }

            if let Some(ch) = capture.lookup_keycode(keycode, mod_state) {
                let commands = daemon.replay_and_inject(ch);

                if !commands.is_empty() {
                    // Inject our output, but suppress re-capturing the keystrokes
                    // we generate (backspace + typed result), which would feed
                    // back into the engine and corrupt the text.
                    set_keymap_suppress(true);
                    if !matches!(
                        execute_commands(&*injector, &commands, &mut daemon.visible_revision),
                        crate::commands::TransactionOutcome::Applied { .. }
                    ) {
                        log_info("[vietc] X11 keymap injection failed");
                    }
                    // Resync the keymap baseline so the injected keystrokes are
                    // discarded rather than re-reported as new events.
                    // Keep suppressing while our injected keystrokes play out,
                    // resyncing the keymap baseline each poll so they are never
                    // re-captured. 200ms comfortably covers a short correction.
                    for _ in 0..40 {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        capture.poll();
                    }
                    set_keymap_suppress(false);
                }
            }
        }
    }
}
