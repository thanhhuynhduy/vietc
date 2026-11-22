use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static CAPS_SHIFT_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const CAPS_SHIFT_TRACE_LIMIT: usize = 96;

use crate::daemon::Daemon;
use crate::device::discover_new_keyboards;
use crate::display;
use crate::event::{
    consume_released_or_repeated_key, is_caps_lock_on, is_flush_char, is_modifier_held_shift,
    is_modifier_pressed, is_navigation_or_control_key, is_num_lock_on, is_toggle_combination_state,
    is_toggle_key, is_vn_control_key, key_to_char_with_case, update_keyboard_state,
};
use crate::focus::SharedFocusSnapshot;
use crate::inject::{create_injector_with_locks, execute_commands};
use crate::log::{log_debug, log_info, log_trace};
use crate::signal::SIGNAL_EXIT;

pub struct EvdevSharedState {
    pub focus: SharedFocusSnapshot,
    pub config_changed: Arc<std::sync::atomic::AtomicBool>,
    pub status_changed: Arc<std::sync::atomic::AtomicBool>,
    /// Incremented by the pointer watcher on every mouse/touchpad button press.
    pub pointer_activity: Arc<std::sync::atomic::AtomicU64>,
}

fn clear_omnibox_shield(
    injector: &dyn dboard_protocol::inject::KeyInjector,
    shared_focus: &SharedFocusSnapshot,
) {
    crate::focus::clear_omnibox_inline_selection(shared_focus);
    injector.set_omnibox_inline_selection(false);
}

/// Apply the focused app's class to per-app state (enabled/bypass/method) and
/// to the injector (terminal paste & bypass mode). Must run before any per-app
/// decision for a key so the first key typed in a newly focused window already
/// obeys that window's rules.
fn sync_app_class(
    daemon: &mut Daemon,
    injector: &dyn dboard_protocol::inject::KeyInjector,
    shared_focus: &SharedFocusSnapshot,
    last_window_class: &mut String,
) {
    if !daemon.config.app_state.enabled {
        injector.set_bypass_mode(false);
        return;
    }
    let (class, identity) = {
        let focus = shared_focus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            focus.class.clone().unwrap_or_default(),
            focus.identity.clone(),
        )
    };
    if class.is_empty() || class == *last_window_class {
        return;
    }
    crate::password_detector::clear_atspi_password_flag_if_non_atspi(identity.as_deref());
    *last_window_class = class.clone();
    injector.set_active_window(&class);
    daemon.check_app_change_with(class);
    injector.set_terminal_app(daemon.app_state.is_terminal_app());
    let bypassed = daemon.is_current_app_bypassed();
    injector.set_bypass_mode(bypassed);
    if bypassed {
        daemon.reset_composition_state();
    }
}

fn finish_forwarded_stroke(
    injector: &dyn dboard_protocol::KeyInjector,
    keycode: u16,
    value: i32,
    forwarded_keys: &mut HashSet<u16>,
) -> bool {
    if (value == 0 || value == 2) && forwarded_keys.contains(&keycode) {
        injector.send_key_event(keycode, value);
        if value == 0 {
            forwarded_keys.remove(&keycode);
        }
        return true;
    }
    false
}

fn bypass_uncertain_edit(
    injector: &dyn dboard_protocol::KeyInjector,
    grabbed: bool,
    keycode: u16,
    value: i32,
    consumed_keys: &mut HashSet<u16>,
    forwarded_keys: &mut HashSet<u16>,
) -> bool {
    if finish_forwarded_stroke(injector, keycode, value, forwarded_keys) {
        return true;
    }
    if !injector.has_uncertain_edit() {
        return false;
    }
    // The triggering transform's release/repeat must remain consumed, but new
    // physical keys (including Backspace and shortcuts) must not be swallowed.
    if value == 1 {
        consumed_keys.remove(&keycode);
    }
    if grabbed && !((value == 0 || value == 2) && consumed_keys.contains(&keycode)) {
        injector.send_key_event(keycode, value);
        if value == 1 {
            forwarded_keys.insert(keycode);
        }
    }
    if value == 0 {
        consumed_keys.remove(&keycode);
    }
    true
}

fn execute_with_shift_released(
    injector: &dyn dboard_protocol::KeyInjector,
    commands: &[crate::commands::OutputCommand],
    visible_revision: &mut u64,
    focus_generation: u64,
    key_state: &evdev::AttributeSet<evdev::KeyCode>,
) -> crate::commands::TransactionOutcome {
    if injector.uses_virtual_keyboard() {
        return execute_commands(injector, commands, visible_revision, focus_generation);
    }
    use dboard_protocol::inject::InjectResult;
    let shifts = [
        evdev::KeyCode::KEY_LEFTSHIFT,
        evdev::KeyCode::KEY_RIGHTSHIFT,
    ];
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
        log_info("[dboard] Failed to restore held Shift after replacement");
    }
    outcome
}

struct KeyboardState {
    keys: evdev::AttributeSet<evdev::KeyCode>,
    grabbed: bool,
    consumed_keys: HashSet<u16>,
    forwarded_keys: HashSet<u16>,
}

impl KeyboardState {
    fn new(_caps_lock: bool, grabbed: bool) -> Self {
        Self {
            keys: evdev::AttributeSet::new(),
            grabbed,
            consumed_keys: HashSet::new(),
            forwarded_keys: HashSet::new(),
        }
    }
}

fn release_disconnected_device_keys(
    injector: &dyn dboard_protocol::KeyInjector,
    state: &KeyboardState,
) {
    let mut held: HashSet<u16> = state.forwarded_keys.iter().copied().collect();
    for key in state.keys.iter() {
        let code = key.code();
        if !state.consumed_keys.contains(&code) {
            held.insert(code);
        }
    }
    for code in held {
        injector.send_key_event(code, 0);
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
    // Read hardware CapsLock/NumLock LEDs BEFORE connecting Wayland virtual
    // keyboard so its initial `vk.modifiers(...)` preserves NumLock instead of
    // clearing locked_mods to 0 on startup.
    let mut global_caps_lock = devices.iter().any(|(device, _)| is_caps_lock_on(device));
    let mut global_num_lock = devices.iter().any(|(device, _)| is_num_lock_on(device));
    let injector = create_injector_with_locks(display, global_caps_lock, global_num_lock)?;
    let focus_generation_signal = shared_focus
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .generation_signal
        .clone();
    injector.bind_focus_generation(focus_generation_signal.clone());
    injector.set_caps_lock(global_caps_lock);
    injector.set_num_lock(global_num_lock);
    let mut device_states: Vec<KeyboardState> = devices
        .iter()
        .map(|(device, _)| KeyboardState::new(is_caps_lock_on(device), false))
        .collect();
    if daemon.grab_enabled && !devices.is_empty() {
        for (idx, (dev, _name)) in devices.iter_mut().enumerate() {
            crate::device::wait_until_keys_released(dev, 400);
            match dev.grab() {
                Ok(()) => {
                    device_states[idx].grabbed = true;
                    log_info(&format!("[dboard] Grabbed keyboard device #{}", idx));
                }
                Err(e) => {
                    log_info(&format!(
                        "[dboard] Could not grab keyboard #{}: {} (check uaccess/udev rules in /etc/udev/rules.d/70-dboard.rules)",
                        idx, e
                    ));
                }
            }
        }
        if device_states.iter().any(|state| state.grabbed) {
            log_info("[dboard] Keyboard grab state recorded per device");
        } else {
            log_info("[dboard] Falling back to non-grabbing mode (may have race)");
        }
    } else if !daemon.grab_enabled {
        log_info("[dboard] Keyboard grab disabled (config grab = false)");
        log_info("[dboard] Set grab = true in dboard.toml to enable");
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

    if !devices.is_empty()
        && !device_states.iter().any(|state| state.grabbed)
        && unsafe { libc::geteuid() } != 0
    {
        log_info("[dboard] WARNING: keyboard grab unavailable — check uaccess ACLs (/etc/udev/rules.d/70-dboard.rules)");
        log_info("[dboard] WARNING: non-grabbed mode has race conditions with fast typing");
        log_info("[dboard] WARNING: do NOT run dboard with sudo on Wayland (breaks virtual keyboard & clipboard)");
    }

    struct GrabReleaseGuard<'a> {
        devices: &'a mut Vec<(evdev::Device, String)>,
    }
    impl<'a> Drop for GrabReleaseGuard<'a> {
        fn drop(&mut self) {
            for (dev, _name) in self.devices.iter_mut() {
                let _ = dev.ungrab();
            }
        }
    }
    let grab_guard = GrabReleaseGuard { devices };
    let devices = &mut *grab_guard.devices;

    let mut dedup = crate::key_dedup::DedupState::new();
    let initial_focus = shared_focus
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut focus_sync = crate::focus_sync::FocusSyncState::from_initial(initial_focus);
    let mut last_pointer_activity = pointer_activity.load(Ordering::SeqCst);
    let mut hotplug_poll_ticks: u32 = 0;

    log_info("[dboard] Event loop started");

    // Cache debug flag to avoid reloading config every keystroke
    let mut debug_logging = crate::log::is_debug_active(daemon.config.debug);

    loop {
        if SIGNAL_EXIT.load(Ordering::SeqCst) {
            if !devices.is_empty() {
                for ((dev, _name), state) in devices.iter_mut().zip(&device_states) {
                    if state.grabbed {
                        let _ = dev.ungrab();
                    }
                }
                log_info("[dboard] Signal received — keyboard grab released");
            }
            log_info("[dboard] Exiting on signal");
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

        let pfds_ptr = if pfds.is_empty() {
            std::ptr::null_mut()
        } else {
            pfds.as_mut_ptr()
        };
        let poll_ret = unsafe { libc::poll(pfds_ptr, pfds.len() as libc::nfds_t, 100) };
        if poll_ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            log_info(&format!(
                "[dboard] poll error on evdev fd: {:?} — exiting",
                err
            ));
            return Err(err.into());
        }
        // Check for status changes instantly (including during idle poll timeouts)
        if status_changed.swap(false, Ordering::SeqCst) {
            daemon.sync_status_file();
        }

        // Check for config reload instantly (including during idle poll timeouts)
        if config_changed.swap(false, Ordering::SeqCst) {
            daemon.reload_config();
            debug_logging = crate::log::is_debug_active(daemon.config.debug);
            injector.set_terminal_app(daemon.app_state.is_terminal_app());
            injector.set_bypass_mode(daemon.is_current_app_bypassed());
        }

        if poll_ret == 0 {
            sync_app_class(
                daemon,
                &*injector,
                &shared_focus,
                &mut focus_sync.last_window_class,
            );
            // Hotplug: scan /dev/input every 2 seconds (20 * 100ms idle ticks) instead of 10 Hz
            hotplug_poll_ticks += 1;
            if hotplug_poll_ticks >= 20 {
                hotplug_poll_ticks = 0;
                let new_devs = discover_new_keyboards(&known_paths, &mut known_phys_ids);
                for (mut dev, name) in new_devs {
                    if daemon.grab_enabled && crate::device::has_held_keys(&dev) {
                        if let Some(pid) = crate::device::get_device_phys_id(std::path::Path::new(
                            &dev_path_of(&name),
                        )) {
                            known_phys_ids.remove(&pid);
                        }
                        continue;
                    }
                    known_paths.insert(dev_path_of(&name));
                    let grabbed = daemon.grab_enabled && dev.grab().is_ok();
                    if grabbed {
                        log_info(&format!("[dboard] Hotplug grabbed keyboard: {}", name));
                    }
                    let caps = is_caps_lock_on(&dev);
                    if (devices.is_empty() || caps) && caps != global_caps_lock {
                        global_caps_lock = caps;
                        injector.set_caps_lock(global_caps_lock);
                    }
                    let num = is_num_lock_on(&dev);
                    if (devices.is_empty() || num) && num != global_num_lock {
                        global_num_lock = num;
                        injector.set_num_lock(global_num_lock);
                    }
                    log_info(&format!("[dboard] Hotplug connected keyboard: {}", name));
                    devices.push((dev, name.clone()));
                    device_states.push(KeyboardState::new(caps, grabbed));
                }
            }
            continue;
        }

        // Process events from whichever device(s) have data ready
        let mut dead: Vec<usize> = Vec::new();
        for (i, pfd) in pfds.iter().enumerate() {
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                log_info(&format!(
                    "[dboard] evdev device {} disconnected (poll revents={:#x}) — removing device",
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
                        "[dboard] fetch_events error on device {}: {:?} — removing device",
                        i, e
                    ));
                    dead.push(i);
                    continue;
                }
            };

            let grabbed = device_states[i].grabbed;
            let mut key_state = std::mem::take(&mut device_states[i].keys);
            let mut consumed_keys = std::mem::take(&mut device_states[i].consumed_keys);
            let mut forwarded_keys = std::mem::take(&mut device_states[i].forwarded_keys);
            for event in event_list {
                if event.event_type() != evdev::EventType::KEY {
                    continue;
                }
                let keycode = event.code();
                let value = event.value();
                let key = evdev::KeyCode(keycode);

                // Update modifier and shared lock state before translating this event.
                update_keyboard_state(
                    &mut key_state,
                    &mut global_caps_lock,
                    &mut global_num_lock,
                    key,
                    value,
                );
                injector.set_caps_lock(global_caps_lock);
                injector.set_num_lock(global_num_lock);
                // Complete a stroke already forwarded during uncertainty before
                // focus recovery, policy, dedup or normal Backspace handling.
                if finish_forwarded_stroke(&*injector, keycode, value, &mut forwarded_keys) {
                    continue;
                }

                // A mouse click may have moved the caret or switched windows
                // without any keyboard event: the half-typed word no longer
                // matches the screen, so it must not be replayed or replaced.
                if value == 1 {
                    let pointer_now = pointer_activity.load(Ordering::SeqCst);
                    if pointer_now != last_pointer_activity {
                        last_pointer_activity = pointer_now;
                        daemon.reset_composition_state();
                        clear_omnibox_shield(&*injector, &shared_focus);
                    }
                }

                if matches!(
                    key,
                    evdev::KeyCode::KEY_CAPSLOCK
                        | evdev::KeyCode::KEY_LEFTSHIFT
                        | evdev::KeyCode::KEY_RIGHTSHIFT
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
                            "[dboard] keyboard-state trace={trace_index} device={i} key={key:?} action={action} value={value} caps_led={} left_shift={} right_shift={}",
                            global_caps_lock,
                            key_state.contains(evdev::KeyCode::KEY_LEFTSHIFT),
                            key_state.contains(evdev::KeyCode::KEY_RIGHTSHIFT),
                        ));
                    }
                }

                if value == 1 {
                    sync_app_class(
                        daemon,
                        &*injector,
                        &shared_focus,
                        &mut focus_sync.last_window_class,
                    );
                }

                // Completely bypass all IME processing for remote desktops, VM viewers and games
                if daemon.is_current_app_bypassed() {
                    injector.set_bypass_mode(true);
                    if consume_released_or_repeated_key(&mut consumed_keys, keycode, value) {
                        continue;
                    }
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
                    clear_omnibox_shield(&*injector, &shared_focus);
                    daemon.toggle_method();
                    continue;
                }

                if value == 1 && daemon.config.deduplicate_keys && !injector.has_uncertain_edit() {
                    let is_nav_or_mod = matches!(
                        keycode,
                        1 | 14 | 15 | 28 | 29 | 42 | 54 | 56 | 58 | 96 | 97 | 100 | 102
                            ..=111 | 125 | 126
                    );
                    let dedupable =
                        keycode < 256 && !is_nav_or_mod && !is_modifier_pressed(&key_state);
                    if dedup.observe(
                        keycode as u32,
                        dedupable,
                        keycode == 57,
                        daemon.config.deduplicate_two_back,
                        daemon.config.deduplicate_window_ms,
                        std::time::Instant::now(),
                    ) {
                        if debug_logging {
                            log_debug(&format!(
                                "[dboard] dedup dropped keycode {} (window {}ms)",
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
                // 3) Entering a terminal sudo/ssh/passwd prompt disables the engine on the very 1st keystroke
                if value == 1 {
                    focus_sync.sync_on_key_press(daemon, &*injector, &shared_focus);
                }

                if bypass_uncertain_edit(
                    &*injector,
                    grabbed,
                    keycode,
                    value,
                    &mut consumed_keys,
                    &mut forwarded_keys,
                ) {
                    daemon.reset_composition_state();
                    clear_omnibox_shield(&*injector, &shared_focus);
                    continue;
                }

                if !grabbed {
                    if value != 1 {
                        continue;
                    }
                    if is_modifier_pressed(&key_state) {
                        daemon.reset_composition_state();
                        clear_omnibox_shield(&*injector, &shared_focus);
                        continue;
                    }
                    if !daemon.engine.is_enabled() {
                        continue;
                    }
                    if let Some(ch) = key_to_char_with_case(
                        key,
                        is_modifier_held_shift(&key_state),
                        global_caps_lock,
                    ) {
                        let commands = daemon.process_forwarded_key(ch);
                        if debug_logging && !commands.is_empty() {
                            log_debug(&format!(
                                "[dboard] non-grabbed inject: count={}",
                                commands.len()
                            ));
                        }
                        let outcome = execute_commands(
                            &*injector,
                            &commands,
                            &mut daemon.visible_revision,
                            focus_sync.last_focus_generation,
                        );
                        if !daemon.handle_transaction_outcome(outcome) {
                            log_info("[dboard] non-grabbed injection failed");
                        }
                    } else if is_navigation_or_control_key(key) {
                        daemon.reset_composition_state();
                        clear_omnibox_shield(&*injector, &shared_focus);
                    }
                } else {
                    if !(value == 2
                        && key == evdev::KeyCode::KEY_BACKSPACE
                        && !is_modifier_pressed(&key_state))
                        && consume_released_or_repeated_key(&mut consumed_keys, keycode, value)
                    {
                        if value == 2
                            && keycode == 57
                            && daemon.engine.is_enabled()
                            && !is_modifier_pressed(&key_state)
                        {
                            let _ = injector.send_char(' ');
                        }
                        continue;
                    }
                    if is_modifier_pressed(&key_state) {
                        if value == 1
                            && !matches!(
                                key,
                                evdev::KeyCode::KEY_LEFTCTRL
                                    | evdev::KeyCode::KEY_RIGHTCTRL
                                    | evdev::KeyCode::KEY_LEFTALT
                                    | evdev::KeyCode::KEY_RIGHTALT
                                    | evdev::KeyCode::KEY_LEFTMETA
                                    | evdev::KeyCode::KEY_RIGHTMETA
                            )
                        {
                            daemon.reset_composition_state();
                            crate::focus::clear_surrounding_word(&shared_focus);
                            clear_omnibox_shield(&*injector, &shared_focus);
                        }
                        injector.send_key_event(keycode, value);
                        continue;
                    }

                    // Engine disabled in grabbed mode: forward keys directly
                    if !daemon.engine.is_enabled() {
                        if value == 1 {
                            clear_omnibox_shield(&*injector, &shared_focus);
                        }
                        injector.send_key_event(keycode, value);
                        continue;
                    }

                    if key == evdev::KeyCode::KEY_BACKSPACE {
                        if value == 1 || value == 2 {
                            crate::focus::clear_surrounding_word(&shared_focus);
                            let buf_before = daemon.engine.buffer().chars().count();
                            let commands = daemon.process_key('\x08');
                            if commands.is_empty() {
                                clear_omnibox_shield(&*injector, &shared_focus);
                                injector.send_key_event(14, 1);
                                injector.send_key_event(14, 0);
                            } else {
                                if buf_before > 0
                                    && crate::focus::take_omnibox_inline_selection(&shared_focus)
                                {
                                    injector.set_omnibox_inline_selection(true);
                                } else {
                                    clear_omnibox_shield(&*injector, &shared_focus);
                                }
                                let outcome = execute_with_shift_released(
                                    &*injector,
                                    &commands,
                                    &mut daemon.visible_revision,
                                    focus_sync.last_focus_generation,
                                    &key_state,
                                );
                                daemon.handle_transaction_outcome(outcome);
                            }
                        }
                        if value == 0 {
                            if !consumed_keys.remove(&keycode) {
                                // KeyDown was forwarded directly (e.g. while disabled/bypassed);
                                // release it on the virtual device to prevent stuck Backspace.
                                injector.send_key_event(keycode, 0);
                            }
                        } else {
                            consumed_keys.insert(keycode);
                        }
                        continue;
                    }

                    if value == 1 {
                        if debug_logging {
                            log_debug(&format!(
                                "[dboard] grabbed key: buf_len={} enabled={}",
                                daemon.engine.buffer().chars().count(),
                                daemon.engine.is_enabled(),
                            ));
                        }
                        if consumed_keys.contains(&keycode) {
                            consumed_keys.remove(&keycode);
                        }
                        if let Some(ch) = key_to_char_with_case(
                            key,
                            is_modifier_held_shift(&key_state),
                            global_caps_lock,
                        ) {
                            crate::surrounding_context::maybe_seed_surrounding_word(
                                daemon,
                                &shared_focus,
                                focus_sync.last_focus_generation,
                                ch,
                            );
                            let buf_before = daemon.engine.buffer().chars().count();
                            let commands = daemon.process_key(ch);
                            if !commands.is_empty() {
                                if debug_logging {
                                    log_debug(&format!(
                                        "[dboard] grabbed inject: cmd_count={}",
                                        commands.len(),
                                    ));
                                }
                                consumed_keys.insert(keycode);

                                if buf_before > 0
                                    && crate::focus::take_omnibox_inline_selection(&shared_focus)
                                {
                                    injector.set_omnibox_inline_selection(true);
                                } else {
                                    clear_omnibox_shield(&*injector, &shared_focus);
                                }
                                let outcome = execute_with_shift_released(
                                    &*injector,
                                    &commands,
                                    &mut daemon.visible_revision,
                                    focus_sync.last_focus_generation,
                                    &key_state,
                                );
                                if !daemon.handle_transaction_outcome(outcome) {
                                    log_info("[dboard] grabbed injection failed");
                                }

                                if key == evdev::KeyCode::KEY_SPACE
                                    && !is_modifier_held_shift(&key_state)
                                    && daemon.engine.is_enabled()
                                {
                                    clear_omnibox_shield(&*injector, &shared_focus);
                                    let _ = injector.send_char(' ');
                                    consumed_keys.insert(keycode);
                                } else if is_flush_char(ch) && daemon.engine.is_enabled() {
                                    clear_omnibox_shield(&*injector, &shared_focus);
                                    injector.send_key_event(keycode, 1);
                                    injector.send_key_event(keycode, 0);
                                    consumed_keys.insert(keycode);
                                }
                            } else if daemon.engine.is_enabled()
                                && is_vn_control_key(daemon.app_state.effective_method(), ch)
                                && daemon.engine.buffer().chars().count() <= buf_before
                                && !daemon.engine.buffer().ends_with(ch)
                            {
                                if debug_logging {
                                    log_debug(
                                        "[dboard] grabbed consumed: control key (no screen change)",
                                    );
                                }
                                consumed_keys.insert(keycode);
                            } else {
                                if debug_logging {
                                    log_debug(
                                        "[dboard] grabbed forward: printable key (no commands)",
                                    );
                                }
                                // Any forwarded printable key overwrites prior inline autocomplete
                                // unless the browser emits a fresh TextSelectionChanged for it.
                                clear_omnibox_shield(&*injector, &shared_focus);
                                if key == evdev::KeyCode::KEY_SPACE
                                    && !is_modifier_held_shift(&key_state)
                                    && daemon.engine.is_enabled()
                                {
                                    let _ = injector.send_char(' ');
                                    consumed_keys.insert(keycode);
                                } else if is_flush_char(ch) && daemon.engine.is_enabled() {
                                    // Invariant 3: dispatch all flush delimiters (Shift+Space,
                                    // punctuation, Enter) atomically while preserving Shift.
                                    injector.send_key_event(keycode, 1);
                                    injector.send_key_event(keycode, 0);
                                    consumed_keys.insert(keycode);
                                } else {
                                    injector.send_key_event(keycode, 1);
                                }
                            }
                        } else {
                            if is_navigation_or_control_key(key) {
                                daemon.reset_composition_state();
                                crate::focus::clear_surrounding_word(&shared_focus);
                            }
                            clear_omnibox_shield(&*injector, &shared_focus);
                            injector.send_key_event(keycode, 1);
                        }
                    } else if value == 2 {
                        if !daemon.engine.buffer().is_empty() {
                            daemon.reset_composition_state();
                        }
                        clear_omnibox_shield(&*injector, &shared_focus);
                        injector.send_key_event(keycode, 2);
                    } else if value == 0 {
                        if debug_logging {
                            log_trace(&format!("[dboard] release forward: code={}", keycode));
                        }
                        injector.send_key_event(keycode, 0);
                    }
                }
            }

            // Save updated key and lock state back.
            device_states[i].keys = key_state;
            device_states[i].consumed_keys = consumed_keys;
            device_states[i].forwarded_keys = forwarded_keys;
        }

        // Remove any devices that errored out (e.g. a uinput/Bluetooth/USB keyboard
        // disconnected mid-chord) and release all held keys so modifiers never stick.
        if !dead.is_empty() {
            dead.sort_unstable();
            dead.dedup();
            daemon.reset_composition_state();
            clear_omnibox_shield(&*injector, &shared_focus);
            for &d in dead.iter().rev() {
                let name = devices[d].1.clone();
                devices.remove(d);
                release_disconnected_device_keys(&*injector, &device_states[d]);
                device_states.remove(d);
                let path = dev_path_of(&name);
                known_paths.remove(&path);
                known_phys_ids.clear();
                for p in known_paths.iter() {
                    if let Some(pid) = crate::device::get_device_phys_id(std::path::Path::new(p)) {
                        known_phys_ids.insert(pid);
                    }
                }
                log_info(&format!("[dboard] Device disconnected/removed: {}", name));
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

    struct UncertainInjector {
        events: std::cell::RefCell<Vec<(u16, i32)>>,
        uncertain: std::cell::Cell<bool>,
    }

    impl Default for UncertainInjector {
        fn default() -> Self {
            Self {
                events: Default::default(),
                uncertain: std::cell::Cell::new(true),
            }
        }
    }

    impl dboard_protocol::KeyInjector for UncertainInjector {
        fn send_backspace(&self) -> dboard_protocol::inject::InjectResult {
            panic!("uncertain edits must not perform automatic deletion")
        }
        fn send_char(&self, _: char) -> dboard_protocol::inject::InjectResult {
            panic!("uncertain edits must bypass composition")
        }
        fn send_string(&self, _: &str) -> dboard_protocol::inject::InjectResult {
            panic!("uncertain edits must not retry insertion")
        }
        fn send_key_event(&self, code: u16, value: i32) -> dboard_protocol::inject::InjectResult {
            self.events.borrow_mut().push((code, value));
            dboard_protocol::inject::InjectResult::Success
        }
        fn has_uncertain_edit(&self) -> bool {
            self.uncertain.get()
        }
        fn flush(&self) -> dboard_protocol::inject::InjectResult {
            dboard_protocol::inject::InjectResult::Success
        }
    }

    #[test]
    fn uncertain_paste_forwards_new_keys_but_keeps_original_trigger_release_consumed() {
        let injector = UncertainInjector::default();
        // Include the stale Backspace marker produced by older daemon builds.
        let mut consumed = HashSet::from([31, 14]);
        let mut forwarded = HashSet::new();
        for (code, value) in [
            (31, 2),
            (31, 0),
            (14, 1),
            (14, 0),
            (29, 1),
            (46, 1),
            (46, 0),
            (29, 0),
        ] {
            assert!(bypass_uncertain_edit(
                &injector,
                true,
                code,
                value,
                &mut consumed,
                &mut forwarded,
            ));
        }
        assert!(consumed.is_empty());
        assert_eq!(
            &*injector.events.borrow(),
            &[(14, 1), (14, 0), (29, 1), (46, 1), (46, 0), (29, 0)]
        );
    }

    #[test]
    fn uncertain_ungrabbed_input_is_not_forwarded_twice() {
        let injector = UncertainInjector::default();
        let mut consumed = HashSet::new();
        let mut forwarded = HashSet::new();
        assert!(bypass_uncertain_edit(
            &injector,
            false,
            30,
            1,
            &mut consumed,
            &mut forwarded,
        ));
        assert!(injector.events.borrow().is_empty());
    }

    #[test]
    fn uncertain_backspace_release_survives_focus_recovery_mid_stroke() {
        let injector = UncertainInjector::default();
        let mut consumed = HashSet::new();
        let mut forwarded = HashSet::new();
        assert!(bypass_uncertain_edit(
            &injector,
            true,
            14,
            1,
            &mut consumed,
            &mut forwarded
        ));
        // Another physical key observes a new field and clears the latch.
        injector.uncertain.set(false);
        assert!(finish_forwarded_stroke(&injector, 14, 2, &mut forwarded));
        assert!(finish_forwarded_stroke(&injector, 14, 0, &mut forwarded));
        assert!(forwarded.is_empty());
        assert_eq!(&*injector.events.borrow(), &[(14, 1), (14, 2), (14, 0)]);
    }

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
            std::path::PathBuf::from("/tmp/dboard-grabbed-stream-test.toml"),
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
        key_state: &evdev::AttributeSet<evdev::KeyCode>,
        caps_lock: bool,
        key: evdev::KeyCode,
    ) -> char {
        key_to_char_with_case(key, is_modifier_held_shift(key_state), caps_lock)
            .expect("test key must translate")
    }

    #[test]
    fn backspace_replacement_releases_and_restores_held_shift() {
        use dboard_protocol::inject::InjectResult;
        use std::cell::RefCell;
        struct Recorder(RefCell<Vec<(u16, i32)>>, bool);
        impl dboard_protocol::KeyInjector for Recorder {
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
        let config = crate::config::Config {
            start_enabled: true,
            input_method: "telex".into(),
            ..Default::default()
        };
        let mut daemon = Daemon::new(
            config,
            std::path::PathBuf::from("/tmp/dboard-shift-test.toml"),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        daemon.process_key('a');
        daemon.process_key('b');
        let commands = daemon.process_key('\x08');
        let mut keys = evdev::AttributeSet::new();
        keys.insert(evdev::KeyCode::KEY_LEFTSHIFT);
        keys.insert(evdev::KeyCode::KEY_RIGHTSHIFT);
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
        let mut num_lock = false;

        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            1,
        );
        assert!(caps_lock);
        assert!(!num_lock);
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
            'A'
        );

        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_NUMLOCK,
            1,
        );
        assert!(num_lock);
        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_NUMLOCK,
            0,
        );
        assert!(num_lock);

        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_LEFTSHIFT,
            1,
        );
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
            'a'
        );
        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_LEFTSHIFT,
            0,
        );

        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            0,
        );
        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            1,
        );
        assert!(!caps_lock);
        assert!(num_lock);
        assert_eq!(
            translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
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
        let mut num_lock = false;
        let mut translated = String::new();

        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            1,
        );
        for key in [
            evdev::KeyCode::KEY_T,
            evdev::KeyCode::KEY_I,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_N,
            evdev::KeyCode::KEY_G,
            evdev::KeyCode::KEY_S,
        ] {
            translated.push(translated_key(&key_state, caps_lock, key));
        }
        translated.push(' ');
        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            0,
        );
        update_keyboard_state(
            &mut key_state,
            &mut caps_lock,
            &mut num_lock,
            evdev::KeyCode::KEY_CAPSLOCK,
            1,
        );
        for key in [
            evdev::KeyCode::KEY_N,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_S,
            evdev::KeyCode::KEY_U,
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
        assert_eq!(simulate_grabbed_telex(&physical_stream("hoaf")), "hòa");
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
    #[test]
    fn disconnect_cleanup_releases_all_unconsumed_held_and_forwarded_keys() {
        let injector = UncertainInjector::default();
        let mut state = KeyboardState::new(false, true);
        state.keys.insert(evdev::KeyCode::KEY_LEFTCTRL);
        state.keys.insert(evdev::KeyCode::KEY_LEFTSHIFT);
        state.keys.insert(evdev::KeyCode::KEY_D);
        state.consumed_keys.insert(evdev::KeyCode::KEY_D.code());
        state.forwarded_keys.insert(evdev::KeyCode::KEY_C.code());
        release_disconnected_device_keys(&injector, &state);
        let mut released: Vec<u16> = injector
            .events
            .borrow()
            .iter()
            .map(|&(c, v)| {
                assert_eq!(v, 0);
                c
            })
            .collect();
        released.sort_unstable();
        assert_eq!(
            released,
            vec![
                evdev::KeyCode::KEY_LEFTCTRL.code(),
                evdev::KeyCode::KEY_LEFTSHIFT.code(),
                evdev::KeyCode::KEY_C.code(),
            ]
        );
    }
}
