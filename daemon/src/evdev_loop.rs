use std::collections::HashSet;
use std::fs;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static CAPS_SHIFT_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const CAPS_SHIFT_TRACE_LIMIT: usize = 96;

use crate::daemon::Daemon;
use crate::display;
#[cfg(feature = "x11")]
use crate::display::DisplayServer;
use crate::event::{
    is_caps_lock_on, is_flush_char, is_modifier_held_shift, is_modifier_pressed,
    is_toggle_combination_state, is_toggle_key, is_vn_control_key, key_to_char_with_case,
};
use crate::focus::SharedFocusSnapshot;
use crate::inject::{create_injector, execute_commands};
use crate::log::log_info;
use crate::signal::SIGNAL_EXIT;

pub struct EvdevSharedState {
    pub focus: SharedFocusSnapshot,
    pub config_changed: Arc<std::sync::atomic::AtomicBool>,
    pub status_changed: Arc<std::sync::atomic::AtomicBool>,
    /// Incremented by the pointer watcher on every mouse/touchpad button press.
    pub pointer_activity: Arc<std::sync::atomic::AtomicU64>,
}

/// Apply the focused app's class to per-app state (enabled/bypass/method) and
/// to the injector (terminal paste mode). Must run before any per-app decision
/// for a key so the first key typed in a newly focused window already obeys
/// that window's rules.
fn sync_app_class(
    daemon: &mut Daemon,
    injector: &dyn vietc_protocol::inject::KeyInjector,
    shared_focus: &SharedFocusSnapshot,
    last_window_class: &mut String,
) {
    if !daemon.config.app_state.enabled {
        return;
    }
    let class = shared_focus
        .lock()
        .unwrap()
        .class
        .clone()
        .unwrap_or_default();
    if class.is_empty() || class == *last_window_class {
        return;
    }
    *last_window_class = class.clone();
    injector.set_active_window(&class);
    daemon.check_app_change_with(class);
    injector.set_terminal_app(daemon.app_state.is_terminal_app());
    if daemon.is_current_app_bypassed() {
        daemon.reset_composition_state();
    }
}

fn is_navigation_or_control_key(key: evdev::Key) -> bool {
    matches!(
        key,
        evdev::Key::KEY_LEFT
            | evdev::Key::KEY_RIGHT
            | evdev::Key::KEY_UP
            | evdev::Key::KEY_DOWN
            | evdev::Key::KEY_HOME
            | evdev::Key::KEY_END
            | evdev::Key::KEY_PAGEUP
            | evdev::Key::KEY_PAGEDOWN
            | evdev::Key::KEY_DELETE
            | evdev::Key::KEY_INSERT
            | evdev::Key::KEY_ESC
            | evdev::Key::KEY_TAB
    )
}

fn update_keyboard_state(
    key_state: &mut evdev::AttributeSet<evdev::Key>,
    caps_lock: &mut bool,
    key: evdev::Key,
    value: i32,
) {
    if value == 1 {
        key_state.insert(key);
    } else if value == 0 {
        key_state.remove(key);
    }

    if key == evdev::Key::KEY_CAPSLOCK && value == 1 {
        *caps_lock = !*caps_lock;
    }
}

fn execute_with_shift_released(
    injector: &dyn vietc_protocol::KeyInjector,
    commands: &[crate::commands::OutputCommand],
    visible_revision: &mut u64,
    focus_generation: u64,
    key_state: &evdev::AttributeSet<evdev::Key>,
) -> crate::commands::TransactionOutcome {
    use vietc_protocol::inject::InjectResult;
    let shifts = [evdev::Key::KEY_LEFTSHIFT, evdev::Key::KEY_RIGHTSHIFT];
    let mut released = true;
    for key in shifts {
        if key_state.contains(key) {
            released &= injector.send_key_event(key.code(), 0) == InjectResult::Success;
        }
    }
    let outcome = if released {
        execute_commands(injector, commands, visible_revision, focus_generation)
    } else {
        crate::commands::TransactionOutcome::Failed
    };
    let mut restored = true;
    for key in shifts {
        if key_state.contains(key) {
            restored &= injector.send_key_event(key.code(), 1) == InjectResult::Success;
        }
    }
    if !restored {
        // Modifier recovery is separate from delivery: an applied replacement
        // must not roll back the engine's already-advanced visible revision.
        log_info("[vietc] Failed to restore held Shift after replacement");
    }
    outcome
}

struct KeyboardState {
    keys: evdev::AttributeSet<evdev::Key>,
    caps_lock: bool,
    grabbed: bool,
    consumed_keys: HashSet<u16>,
}

impl KeyboardState {
    fn new(caps_lock: bool, grabbed: bool) -> Self {
        Self {
            keys: evdev::AttributeSet::new(),
            caps_lock,
            grabbed,
            consumed_keys: HashSet::new(),
        }
    }
}

pub fn run_with_evdev(
    devices: &mut Vec<(evdev::Device, String)>,
    daemon: &mut Daemon,
    shared: EvdevSharedState,
    display: display::DisplayServer,
) -> Result<(), Box<dyn std::error::Error>> {
    let EvdevSharedState {
        focus: shared_focus,
        config_changed,
        status_changed,
        pointer_activity,
    } = shared;
    let mut injector = create_injector(display)?;
    let focus_generation_signal = shared_focus.lock().unwrap().generation_signal.clone();
    injector.bind_focus_generation(focus_generation_signal);

    let mut device_states: Vec<KeyboardState> = devices
        .iter()
        .map(|(device, _)| KeyboardState::new(is_caps_lock_on(device), false))
        .collect();
    if daemon.grab_enabled && !devices.is_empty() {
        for (idx, (dev, _name)) in devices.iter_mut().enumerate() {
            match dev.grab() {
                Ok(()) => {
                    device_states[idx].grabbed = true;
                    log_info(&format!("[vietc] Grabbed keyboard device #{}", idx));
                }
                Err(e) => {
                    log_info(&format!(
                        "[vietc] Could not grab keyboard #{}: {} (run as root for grab)",
                        idx, e
                    ));
                }
            }
        }
        if device_states.iter().any(|state| state.grabbed) {
            log_info("[vietc] Keyboard grab state recorded per device");
        } else {
            log_info("[vietc] Falling back to non-grabbing mode (may have race)");
        }
    } else if !daemon.grab_enabled {
        log_info("[vietc] Keyboard grab disabled (config grab = false)");
        log_info("[vietc] Set grab = true in vietc.toml to enable (needs root)");
    }

    // Track currently opened device paths and physical IDs so newly connected keyboards can be hotplugged without duplicates
    let mut known_paths: HashSet<String> = HashSet::new();
    let mut known_phys_ids: HashSet<String> = HashSet::new();
    for (_, n) in devices.iter() {
        let p = dev_path_of(n);
        known_paths.insert(p.clone());
        if let Some(phys) = crate::device::get_device_phys_id(std::path::Path::new(&p)) {
            known_phys_ids.insert(phys);
        }
    }

    if !device_states.iter().any(|state| state.grabbed) {
        if unsafe { libc::geteuid() } != 0 {
            log_info("[vietc] WARNING: not running as root — keyboard grab unavailable");
            log_info("[vietc] WARNING: non-grabbed mode has race conditions with fast typing");
            log_info("[vietc] WARNING: run with sudo, or setcap cap_sys_admin+ep on the binary");
        }
        #[cfg(feature = "x11")]
        if display != DisplayServer::Wayland {
            if let Ok(x11_inj) = vietc_protocol::x11_inject::X11Injector::new() {
                injector = Box::new(x11_inj);
                log_info("[vietc] Non-grabbed: using X11 injection (faster than uinput)");
            }
        }
    }

    let mut dedup = crate::key_dedup::DedupState::new();
    let initial_focus = shared_focus.lock().unwrap().clone();
    let mut last_focus_generation = initial_focus.generation;
    let mut last_focus_target = initial_focus.target;
    let mut last_window_class = String::new();
    let mut last_pointer_activity = pointer_activity.load(Ordering::SeqCst);
    let mut password_check_counter: u32 = 0;
    let mut last_key_time = std::time::Instant::now();

    log_info("[vietc] Event loop started");

    // Cache debug flag to avoid reloading config every keystroke
    let mut debug_logging = daemon.config.debug;

    loop {
        if SIGNAL_EXIT.load(Ordering::SeqCst) {
            if !devices.is_empty() {
                for ((dev, _name), state) in devices.iter_mut().zip(&device_states) {
                    if state.grabbed {
                        let _ = dev.ungrab();
                    }
                }
                log_info("[vietc] Signal received — keyboard grab released");
            }
            log_info("[vietc] Exiting on signal");
            return Ok(());
        }

        // Poll ALL devices simultaneously
        let mut pfds: Vec<libc::pollfd> = devices
            .iter()
            .map(|(d, _)| libc::pollfd {
                fd: d.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();

        let poll_ret = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 100) };
        if poll_ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            log_info(&format!(
                "[vietc] poll error on evdev fd: {:?} — exiting",
                err
            ));
            return Err(err.into());
        }
        // Check for status changes instantly (including during idle poll timeouts)
        if status_changed.load(Ordering::SeqCst) {
            daemon.sync_status_file();
            status_changed.store(false, Ordering::SeqCst);
        }

        // Check for config reload instantly (including during idle poll timeouts)
        if config_changed.load(Ordering::SeqCst) {
            daemon.reload_config();
            config_changed.store(false, Ordering::SeqCst);
            debug_logging = daemon.config.debug;
        }

        if poll_ret == 0 {
            sync_app_class(daemon, &*injector, &shared_focus, &mut last_window_class);
            // Hotplug: dynamically attach keyboards connected after startup
            let new_devs = discover_new_keyboards(&known_paths, &mut known_phys_ids);
            for (mut dev, name) in new_devs {
                known_paths.insert(dev_path_of(&name));
                let grabbed = daemon.grab_enabled && dev.grab().is_ok();
                if grabbed {
                    log_info(&format!("[vietc] Hotplug grabbed keyboard: {}", name));
                }
                let caps = is_caps_lock_on(&dev);
                log_info(&format!("[vietc] Hotplug connected keyboard: {}", name));
                devices.push((dev, name.clone()));
                device_states.push(KeyboardState::new(caps, grabbed));
            }
            continue;
        }

        // Process events from whichever device(s) have data ready
        let mut dead: Vec<usize> = Vec::new();
        for (i, pfd) in pfds.iter().enumerate() {
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                log_info(&format!(
                    "[vietc] evdev device {} disconnected (poll revents={:#x}) — removing device",
                    i, pfd.revents
                ));
                dead.push(i);
                continue;
            }
            if (pfd.revents & libc::POLLIN) == 0 {
                continue;
            }

            let (ref mut device, ref _name) = devices[i];

            let event_list = match device.fetch_events() {
                Ok(events) => events.collect::<Vec<_>>(),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    log_info(&format!(
                        "[vietc] fetch_events error on device {}: {:?} — removing device",
                        i, e
                    ));
                    dead.push(i);
                    continue;
                }
            };

            let grabbed = device_states[i].grabbed;
            let mut caps = device_states[i].caps_lock;
            let mut key_state = std::mem::take(&mut device_states[i].keys);
            let mut consumed_keys = std::mem::take(&mut device_states[i].consumed_keys);
            for event in event_list {
                if event.event_type() != evdev::EventType::KEY {
                    continue;
                }
                let keycode = event.code();
                let value = event.value();
                let key = evdev::Key(keycode);

                // Update modifier and lock state before translating this event.
                update_keyboard_state(&mut key_state, &mut caps, key, value);
                injector.set_caps_lock(caps);

                // A mouse click may have moved the caret or switched windows
                // without any keyboard event: the half-typed word no longer
                // matches the screen, so it must not be replayed or replaced.
                if value == 1 {
                    let pointer_now = pointer_activity.load(Ordering::SeqCst);
                    if pointer_now != last_pointer_activity {
                        last_pointer_activity = pointer_now;
                        daemon.reset_composition_state();
                    }
                }

                if matches!(
                    key,
                    evdev::Key::KEY_CAPSLOCK
                        | evdev::Key::KEY_LEFTSHIFT
                        | evdev::Key::KEY_RIGHTSHIFT
                ) {
                    let trace_index = CAPS_SHIFT_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
                    if trace_index < CAPS_SHIFT_TRACE_LIMIT {
                        let action = match value {
                            0 => "release",
                            1 => "press",
                            2 => "repeat",
                            _ => "unknown",
                        };
                        log_info(&format!(
                            "[vietc] keyboard-state trace={trace_index} device={i} key={key:?} action={action} value={value} caps_led={} left_shift={} right_shift={}",
                            caps,
                            key_state.contains(evdev::Key::KEY_LEFTSHIFT),
                            key_state.contains(evdev::Key::KEY_RIGHTSHIFT),
                        ));
                    }
                }

                if value == 1 {
                    sync_app_class(daemon, &*injector, &shared_focus, &mut last_window_class);
                }

                // Completely bypass all IME processing for remote desktops, VM viewers and games
                if daemon.is_current_app_bypassed() {
                    if grabbed {
                        injector.send_key_event(keycode, value);
                    }
                    continue;
                }

                if value == 1
                    && is_toggle_key(key, &daemon.config.toggle_key)
                    && is_toggle_combination_state(&key_state, &daemon.config.toggle_key)
                {
                    consumed_keys.insert(keycode);
                    daemon.toggle_method();
                    continue;
                }

                if value == 1 && daemon.config.deduplicate_keys {
                    let dedupable = keycode < 256 && !is_modifier_pressed(&key_state);
                    if dedup.observe(
                        keycode as u32,
                        dedupable,
                        keycode == 57,
                        daemon.config.deduplicate_two_back,
                        daemon.config.deduplicate_window_ms,
                        std::time::Instant::now(),
                    ) {
                        if debug_logging {
                            log_info(&format!(
                                "[vietc] dedup dropped keycode {} (window {}ms)",
                                keycode, daemon.config.deduplicate_window_ms
                            ));
                        }
                        consumed_keys.insert(keycode);
                        continue;
                    }
                }

                // Only the primary ungrabbed keyboard is composed; the others
                // already deliver native input. A grabbed device always needs
                // forwarding, regardless of the grab results of other devices.
                if !grabbed && i != 0 {
                    continue;
                }

                // Observe focus changes and password field transitions on key press
                // BEFORE checking !daemon.engine.is_enabled() or Backspace, so that:
                // 1) Backspace after a focus switch does not delete characters in the new field (A14)
                // 2) Leaving a password field in the same window re-enables the engine (A07)
                if value == 1 {
                    let gap = last_key_time.elapsed();
                    last_key_time = std::time::Instant::now();

                    let focus = shared_focus.lock().unwrap().clone();
                    let active_window_class = focus.class.unwrap_or_default();
                    let focus_changed = focus.generation != last_focus_generation;

                    if focus_changed {
                        log_info(&format!(
                            "[vietc] Focus generation changed: {} -> {} identity={:?} (gap={:?})",
                            last_focus_generation,
                            focus.generation,
                            focus.identity,
                            gap
                        ));
                        let previous_generation = last_focus_generation;
                        let new_target = focus.target.clone();
                        last_focus_generation = focus.generation;
                        injector.cancel_pending_replacement(
                            last_focus_generation,
                            "target-changed",
                        );
                        log_info(&format!(
                            "[vietc] composition reset: focus_generation={} old_target_generation={} old_target={:?} new_target={:?} reason=target-changed",
                            last_focus_generation,
                            previous_generation,
                            last_focus_target,
                            new_target,
                        ));
                        last_focus_target = new_target;
                        if !active_window_class.is_empty() {
                            last_window_class = active_window_class.clone();
                        }
                        daemon.reset_composition_state();

                        if daemon.config.app_state.enabled {
                            let class = shared_focus
                                .lock()
                                .unwrap()
                                .class
                                .clone()
                                .unwrap_or_default();
                            let class = if class.is_empty() {
                                crate::app_state::get_focused_window_class()
                                    .unwrap_or_default()
                            } else {
                                class
                            };
                            injector.set_active_window(&class);
                            daemon.check_app_change_with(class);
                            injector.set_terminal_app(daemon.app_state.is_terminal_app());
                        }

                        if daemon.config.password_detection.enabled {
                            let is_pw = daemon.app_state.check_password_field();
                            let currently_enabled = daemon.engine.is_enabled();
                            if is_pw && currently_enabled {
                                daemon.engine.set_enabled(false);
                                daemon.engine.reset();
                                daemon.replay_reset();
                                daemon.write_status();
                            } else if !is_pw
                                && !currently_enabled
                                && daemon.app_state.get_default_state()
                            {
                                daemon.engine.set_enabled(true);
                                daemon.engine.reset();
                                daemon.replay_reset();
                                daemon.write_status();
                            }
                        }
                    } else if daemon.config.app_state.enabled {
                        let class = shared_focus
                            .lock()
                            .unwrap()
                            .class
                            .clone()
                            .unwrap_or_default();
                        if !class.is_empty() {
                            injector.set_active_window(&class);
                            if class != last_window_class {
                                last_window_class = class.clone();
                                daemon.check_app_change_with(class);
                            }
                            injector.set_terminal_app(daemon.app_state.is_terminal_app());
                        }
                    }

                    if daemon.config.password_detection.enabled {
                        password_check_counter += 1;
                        if password_check_counter >= 30 {
                            password_check_counter = 0;
                            let is_pw = daemon.app_state.check_password_field();
                            let currently_enabled = daemon.engine.is_enabled();
                            if is_pw && currently_enabled {
                                daemon.engine.set_enabled(false);
                                daemon.engine.reset();
                                daemon.replay_reset();
                                daemon.write_status();
                                log_info("[vietc] Password field detected (periodic) — engine disabled");
                            } else if !is_pw
                                && !currently_enabled
                                && daemon.app_state.get_default_state()
                            {
                                daemon.engine.set_enabled(true);
                                daemon.engine.reset();
                                daemon.replay_reset();
                                daemon.write_status();
                            }
                        }
                    }
                }

                if !grabbed {
                    if value != 1 {
                        continue;
                    }
                    if is_modifier_pressed(&key_state) {
                        daemon.reset_composition_state();
                        continue;
                    }
                    if !daemon.engine.is_enabled() {
                        continue;
                    }
                    if let Some(ch) =
                        key_to_char_with_case(key, is_modifier_held_shift(&key_state), caps)
                    {
                        let commands = daemon.process_forwarded_key(ch);
                        if !commands.is_empty() {
                            log_info(&format!(
                                "[vietc] non-grabbed inject: ch='{}' cmds={:?}",
                                ch.escape_default(),
                                commands
                            ));
                        }
                        let outcome = execute_commands(
                            &*injector,
                            &commands,
                            &mut daemon.visible_revision,
                            last_focus_generation,
                        );
                        if !daemon.handle_transaction_outcome(outcome) {
                            log_info("[vietc] non-grabbed injection failed");
                        }
                    } else if is_navigation_or_control_key(key) {
                        daemon.reset_composition_state();
                    }
                } else {
                    if is_modifier_pressed(&key_state) {
                        if value == 1
                            && !matches!(
                                key,
                                evdev::Key::KEY_LEFTCTRL
                                    | evdev::Key::KEY_RIGHTCTRL
                                    | evdev::Key::KEY_LEFTALT
                                    | evdev::Key::KEY_RIGHTALT
                                    | evdev::Key::KEY_LEFTMETA
                                    | evdev::Key::KEY_RIGHTMETA
                            )
                        {
                            daemon.reset_composition_state();
                        }
                        injector.send_key_event(keycode, value);
                        continue;
                    }

                    // Engine disabled in grabbed mode: forward keys directly
                    if !daemon.engine.is_enabled() {
                        injector.send_key_event(keycode, value);
                        continue;
                    }

                    if key == evdev::Key::KEY_BACKSPACE {
                        if value == 1 || value == 2 {
                            let commands = daemon.process_key('\x08');
                            if commands.is_empty() {
                                injector.send_key_event(14, 1);
                                injector.send_key_event(14, 0);
                            } else {
                                let outcome = execute_with_shift_released(
                                    &*injector,
                                    &commands,
                                    &mut daemon.visible_revision,
                                    last_focus_generation,
                                    &key_state,
                                );
                                daemon.handle_transaction_outcome(outcome);
                            }
                        }
                        consumed_keys.insert(keycode);
                        continue;
                    }

                    if value == 1 {
                        if debug_logging {
                            log_info(&format!(
                                "[vietc] grabbed key: code={} ch='{}' buf='{}' enabled={}",
                                keycode,
                                key_to_char_with_case(key, is_modifier_held_shift(&key_state), caps)
                                    .map(|c| c.escape_default().to_string())
                                    .unwrap_or_default(),
                                daemon.engine.buffer().escape_default(),
                                daemon.engine.is_enabled(),
                            ));
                        }
                        if consumed_keys.contains(&keycode) {
                            consumed_keys.remove(&keycode);
                        }
                        if let Some(ch) =
                            key_to_char_with_case(key, is_modifier_held_shift(&key_state), caps)
                        {

                            let buf_before = daemon.engine.buffer().chars().count();
                            let commands = daemon.process_key(ch);
                            if !commands.is_empty() {
                                if debug_logging {
                                    log_info(&format!(
                                        "[vietc] grabbed inject: ch='{}' cmds={:?}",
                                        ch.escape_default(),
                                        commands,
                                    ));
                                }
                                consumed_keys.insert(keycode);

                                let outcome = execute_with_shift_released(
                                    &*injector,
                                    &commands,
                                    &mut daemon.visible_revision,
                                    last_focus_generation,
                                    &key_state,
                                );
                                if !daemon.handle_transaction_outcome(outcome) {
                                    log_info("[vietc] grabbed injection failed");
                                }

                                if is_flush_char(ch) && daemon.engine.is_enabled() {
                                    injector.send_key_event(keycode, 1);
                                    injector.send_key_event(keycode, 0);
                                }
                            } else if daemon.engine.is_enabled()
                                && is_vn_control_key(daemon.app_state.effective_method(), ch)
                                && daemon.engine.buffer().chars().count() <= buf_before
                            {
                                if debug_logging {
                                    log_info(&format!(
                                        "[vietc] grabbed consumed: ch='{}' (control key, no screen change)",
                                        ch.escape_default(),
                                    ));
                                }
                                consumed_keys.insert(keycode);
                            } else {
                                if debug_logging {
                                    log_info(&format!(
                                        "[vietc] grabbed forward: ch='{}' (no commands)",
                                        ch.escape_default(),
                                    ));
                                }
                                injector.send_key_event(keycode, 1);
                            }
                        } else {
                            if is_navigation_or_control_key(key) {
                                daemon.reset_composition_state();
                            }
                            injector.send_key_event(keycode, 1);
                        }
                    } else if value == 2 {
                        if consumed_keys.contains(&keycode) {
                            continue;
                        }
                        injector.send_key_event(keycode, 2);
                    } else if value == 0 {
                        if consumed_keys.remove(&keycode) {
                            continue;
                        }
                        injector.send_key_event(keycode, 0);
                    }
                }
            }

            // Save updated key and lock state back.
            device_states[i].keys = key_state;
            device_states[i].caps_lock = caps;
            device_states[i].consumed_keys = consumed_keys;
        }

        // Remove any devices that errored out (e.g. a uinput virtual keyboard
        // like vietc-vk was closed) so we don't crash the whole daemon.
        if !dead.is_empty() {
            dead.sort_unstable();
            dead.dedup();
            for &d in dead.iter().rev() {
                let name = devices[d].1.clone();
                devices.remove(d);
                device_states.remove(d);
                let path = dev_path_of(&name);
                known_paths.remove(&path);
                // Rebuild phys set from remaining paths — virtual uinput
                // devices disappear from sysfs on close, so recomputing phys_id
                // for the dead path would return None and leave a stale entry
                // that blocks the next virtual keyboard with the same input
                // number (e.g. re-used /dev/input/event31 → same
                // sysfs:/sys/devices/virtual/input/input31).
                known_phys_ids.clear();
                for p in known_paths.iter() {
                    if let Some(pid) = crate::device::get_device_phys_id(std::path::Path::new(p)) {
                        known_phys_ids.insert(pid);
                    }
                }
                log_info(&format!("[vietc] Device disconnected/removed: {}", name));
            }
        }
    }
}

/// Extract the device path (the part before " (") from a device label of the
/// form `"<path> (<name>)"` produced by `open_keyboard_devices`.
fn dev_path_of(label: &str) -> String {
    label.split(" (").next().unwrap_or(label).to_string()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod grabbed_event_stream_tests {
    use super::*;
    use crate::commands::OutputCommand;

    #[derive(Clone, Copy)]
    enum TestEvent {
        Press(char),
        Repeat(char),
        Release(char),
    }

    fn keycode(ch: char) -> u16 {
        ch.to_ascii_lowercase() as u16
    }

    fn apply(screen: &mut String, commands: &[OutputCommand]) {
        for command in commands {
            match command {
                OutputCommand::Backspace(count) => {
                    let keep = screen.chars().count().saturating_sub(*count);
                    *screen = screen.chars().take(keep).collect();
                }
                OutputCommand::Type(text) => screen.push_str(text),
                OutputCommand::Replace(replacement) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(replacement.delete_before_cursor);
                    *screen = screen.chars().take(keep).collect();
                    screen.push_str(&replacement.insert);
                }
            }
        }
    }

    fn simulate_grabbed_telex(events: &[TestEvent]) -> String {
        let config = crate::config::Config {
            input_method: "telex".to_string(),
            start_enabled: true,
            ..Default::default()
        };
        let mut daemon = Daemon::new(
            config,
            std::path::PathBuf::from("/tmp/vietc-grabbed-stream-test.toml"),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );

        let mut screen = String::new();
        let mut consumed_keys = HashSet::new();

        for event in events {
            match *event {
                TestEvent::Press(ch) => {
                    let code = keycode(ch);
                    let commands = daemon.process_key(ch);
                    if commands.is_empty() {
                        screen.push(ch);
                    } else {
                        consumed_keys.insert(code);
                        apply(&mut screen, &commands);
                    }
                }
                TestEvent::Repeat(ch) => {
                    if !consumed_keys.contains(&keycode(ch)) {
                        screen.push(ch);
                    }
                }
                TestEvent::Release(ch) => {
                    consumed_keys.remove(&keycode(ch));
                }
            }
        }

        screen
    }

    fn physical_stream(text: &str) -> Vec<TestEvent> {
        text.chars()
            .flat_map(|ch| [TestEvent::Press(ch), TestEvent::Release(ch)])
            .collect()
    }

    fn translated_key(
        key_state: &evdev::AttributeSet<evdev::Key>,
        caps_lock: bool,
        key: evdev::Key,
    ) -> char {
        key_to_char_with_case(key, is_modifier_held_shift(key_state), caps_lock)
            .expect("test key must translate")
    }

    #[test]
    fn backspace_replacement_releases_and_restores_held_shift() {
        use std::cell::RefCell;
        use vietc_protocol::inject::InjectResult;
        struct Recorder(RefCell<Vec<(u16, i32)>>, bool);
        impl vietc_protocol::KeyInjector for Recorder {
            fn send_key_event(&self, code: u16, value: i32) -> InjectResult {
                self.0.borrow_mut().push((code, value));
                if value == 1 && self.1 {
                    InjectResult::Failed
                } else {
                    InjectResult::Success
                }
            }
            fn send_char(&self, _: char) -> InjectResult {
                InjectResult::NotSupported
            }
            fn send_string(&self, _: &str) -> InjectResult {
                InjectResult::NotSupported
            }
            fn send_backspace(&self) -> InjectResult {
                InjectResult::NotSupported
            }
            fn flush(&self) -> InjectResult {
                InjectResult::Success
            }
            fn inject_replacement(&self, _: usize, _: &str) -> InjectResult {
                assert_eq!(*self.0.borrow(), vec![(42, 0), (54, 0)]);
                InjectResult::Success
            }
        }
        let mut config = crate::config::Config::default();
        config.start_enabled = true;
        config.input_method = "telex".into();
        let mut daemon = Daemon::new(
            config,
            std::path::PathBuf::from("/tmp/vietc-shift-test.toml"),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        daemon.process_key('a');
        daemon.process_key('b');
        let commands = daemon.process_key('\x08');
        let mut keys = evdev::AttributeSet::new();
        keys.insert(evdev::Key::KEY_LEFTSHIFT);
        keys.insert(evdev::Key::KEY_RIGHTSHIFT);
        for restore_fails in [false, true] {
            let recorder = Recorder(RefCell::new(Vec::new()), restore_fails);
            let mut revision = 0;
            let outcome =
                execute_with_shift_released(&recorder, &commands, &mut revision, 0, &keys);
            assert!(matches!(
                outcome,
                crate::commands::TransactionOutcome::Applied {
                    visible_revision: 1
                }
            ));
            assert_eq!(revision, 1);
            assert_eq!(
                *recorder.0.borrow(),
                vec![(42, 0), (54, 0), (42, 1), (54, 1)]
            );
        }
    }

    #[test]
    fn mixed_grab_results_and_hotplug_keep_per_keyboard_state() {
        let mut states = vec![
            KeyboardState::new(false, true),
            KeyboardState::new(false, false),
        ];
        states[0].consumed_keys.insert(30);
        assert!(states[0].grabbed);
        assert!(!states[1].grabbed);
        assert!(!states[1].consumed_keys.contains(&30));
        states.push(KeyboardState::new(false, true));
        states.remove(0);
        assert!(!states[0].grabbed);
        assert!(states[1].grabbed);
    }

    #[test]
    fn production_evdev_state_tracks_caps_shift_and_lock_toggles() {
        let mut key_state = evdev::AttributeSet::new();
        let mut caps_lock = false;

        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 1);
        assert!(caps_lock);
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::Key::KEY_A),
            'A'
        );

        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_LEFTSHIFT, 1);
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::Key::KEY_A),
            'a'
        );
        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_LEFTSHIFT, 0);

        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 0);
        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 1);
        assert!(!caps_lock);
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::Key::KEY_A),
            'a'
        );
    }

    #[test]
    fn production_evdev_casing_composes_multiple_words_without_mixed_case() {
        let cases = [
            ("TIEENGS", "TIẾNG"),
            ("DDUOWNGF", "ĐƯỜNG"),
            ("DDUOJWC", "ĐƯỢC"),
            ("NEESU", "NẾU"),
            ("TIEENGS DDUOWNGF DDUOJWC NEESU", "TIẾNG ĐƯỜNG ĐƯỢC NẾU"),
        ];

        for (raw, expected) in cases {
            assert_eq!(
                simulate_grabbed_telex(&physical_stream(raw)),
                expected,
                "raw={raw}"
            );
        }
    }

    #[test]
    fn production_evdev_caps_toggle_between_words_does_not_mix_case() {
        let mut key_state = evdev::AttributeSet::new();
        let mut caps_lock = false;
        let mut translated = String::new();

        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 1);
        for key in [
            evdev::Key::KEY_T,
            evdev::Key::KEY_I,
            evdev::Key::KEY_E,
            evdev::Key::KEY_E,
            evdev::Key::KEY_N,
            evdev::Key::KEY_G,
            evdev::Key::KEY_S,
        ] {
            translated.push(translated_key(&key_state, caps_lock, key));
        }
        translated.push(' ');
        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 0);
        update_keyboard_state(&mut key_state, &mut caps_lock, evdev::Key::KEY_CAPSLOCK, 1);
        for key in [
            evdev::Key::KEY_N,
            evdev::Key::KEY_E,
            evdev::Key::KEY_E,
            evdev::Key::KEY_S,
            evdev::Key::KEY_U,
        ] {
            translated.push(translated_key(&key_state, caps_lock, key));
        }

        assert_eq!(translated, "TIEENGS neesu");
        assert_eq!(
            simulate_grabbed_telex(&physical_stream(&translated)),
            "TIẾNG nếu"
        );
    }

    #[test]
    fn grabbed_stream_composes_telex_without_duplicate_prefixes() {
        assert_eq!(simulate_grabbed_telex(&physical_stream("hieenj")), "hiện");
        assert_eq!(simulate_grabbed_telex(&physical_stream("dd")), "đ");
        assert_eq!(simulate_grabbed_telex(&physical_stream("hoaf")), "hoà");
    }

    #[test]
    fn grabbed_stream_second_tone_key_removes_the_tone() {
        assert_eq!(simulate_grabbed_telex(&physical_stream("hoaff")), "hoaf");
    }

    #[test]
    fn grabbed_stream_preserves_word_boundaries_and_replacement_lengths() {
        let events = physical_stream("hieenj taij tao ddang gox tieengs Vieetj");
        assert_eq!(
            simulate_grabbed_telex(&events),
            "hiện tại tao đang gõ tiếng Việt"
        );
    }

    #[test]
    fn consumed_press_suppresses_repeat_and_release_feedback() {
        let events = [
            TestEvent::Press('d'),
            TestEvent::Release('d'),
            TestEvent::Press('d'),
            TestEvent::Repeat('d'),
            TestEvent::Release('d'),
        ];

        assert_eq!(simulate_grabbed_telex(&events), "đ");
    }
}

/// Discover keyboard devices present in /dev/input that are not already known.
/// Used for hotplug so newly connected keyboards (USB/wireless/virtual) are attached dynamically.
fn discover_new_keyboards(
    existing_paths: &HashSet<String>,
    existing_phys: &mut HashSet<String>,
) -> Vec<(evdev::Device, String)> {
    let mut out = Vec::new();
    let mut added_paths = HashSet::new();

    // 1. Scan /dev/input/by-path/*-event-kbd
    let by_path = std::path::Path::new("/dev/input/by-path");
    if by_path.exists() {
        if let Ok(rd) = fs::read_dir(by_path) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with("-event-kbd") {
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        if existing_paths.contains(&p) || !added_paths.insert(p.clone()) {
                            continue;
                        }
                        let phys_opt = crate::device::get_device_phys_id(&real_path);
                        if let Some(phys_id) = &phys_opt {
                            if existing_phys.contains(phys_id) {
                                continue;
                            }
                        }
                        if let Ok(device) = evdev::Device::open(&real_path) {
                            let dev_name = device.name().unwrap_or("unknown").to_string();
                            if crate::device::is_valid_keyboard(&device) {
                                if let Some(pid) = phys_opt {
                                    existing_phys.insert(pid);
                                }
                                out.push((
                                    device,
                                    format!("{} ({})", real_path.display(), dev_name),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Scan /dev/input/by-id/*-event-kbd
    let by_id = std::path::Path::new("/dev/input/by-id");
    if by_id.exists() {
        if let Ok(rd) = fs::read_dir(by_id) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with("-event-kbd") {
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        if existing_paths.contains(&p) || !added_paths.insert(p.clone()) {
                            continue;
                        }
                        let phys_opt = crate::device::get_device_phys_id(&real_path);
                        if let Some(phys_id) = &phys_opt {
                            if existing_phys.contains(phys_id) {
                                continue;
                            }
                        }
                        if let Ok(device) = evdev::Device::open(&real_path) {
                            let dev_name = device.name().unwrap_or("unknown").to_string();
                            if crate::device::is_valid_keyboard(&device) {
                                if let Some(pid) = phys_opt {
                                    existing_phys.insert(pid);
                                }
                                out.push((
                                    device,
                                    format!("{} ({})", real_path.display(), dev_name),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Scan /dev/input/event* for Bluetooth & direct devices without by-path/by-id symlinks
    let dir = std::path::Path::new("/dev/input");
    if dir.exists() {
        if let Ok(rd) = fs::read_dir(dir) {
            for entry in rd.flatten() {
                let name_str = entry.file_name().to_string_lossy().to_string();
                if name_str.starts_with("event") {
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        if existing_paths.contains(&p) || !added_paths.insert(p.clone()) {
                            continue;
                        }
                        let phys_opt = crate::device::get_device_phys_id(&real_path);
                        if let Some(phys_id) = &phys_opt {
                            if existing_phys.contains(phys_id) {
                                continue;
                            }
                        }
                        if let Ok(device) = evdev::Device::open(&real_path) {
                            let dev_name = device.name().unwrap_or("unknown").to_string();
                            if crate::device::is_valid_keyboard(&device) {
                                if let Some(pid) = phys_opt {
                                    existing_phys.insert(pid);
                                }
                                out.push((
                                    device,
                                    format!("{} ({})", real_path.display(), dev_name),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    out
}
