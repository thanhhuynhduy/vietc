use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static CAPS_SHIFT_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const CAPS_SHIFT_TRACE_LIMIT: usize = 96;

use crate::daemon::Daemon;
use crate::device::discover_new_keyboards;
use crate::display;
#[cfg(test)]
use crate::event::key_to_char_with_case;
use crate::event::{
    consume_released_or_repeated_key, is_caps_lock_on, is_flush_char, is_modifier_held_shift,
    is_modifier_pressed, is_navigation_or_control_key, is_num_lock_on, is_toggle_combination_state,
    is_toggle_key, is_vn_control_key, key_to_char_with_layout, update_keyboard_state,
};
use crate::focus::SharedFocusSnapshot;
use crate::inject::{create_injector_with_locks_and_layout, execute_commands};
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
    remaining_states: &[KeyboardState],
) {
    let mut held: HashSet<u16> = state.forwarded_keys.iter().copied().collect();
    for key in state.keys.iter() {
        let code = key.code();
        if !state.consumed_keys.contains(&code) {
            held.insert(code);
        }
    }
    for code in held {
        if !crate::hotplug::is_modifier_held_elsewhere(
            code,
            remaining_states.iter().map(|s| &s.keys),
        ) {
            injector.send_key_event(code, 0);
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
    // Read hardware CapsLock/NumLock LEDs BEFORE connecting Wayland virtual
    // keyboard so its initial `vk.modifiers(...)` preserves NumLock instead of
    // clearing locked_mods to 0 on startup.
    let mut global_caps_lock = devices.iter().any(|(device, _)| is_caps_lock_on(device));
    let mut global_num_lock = devices.iter().any(|(device, _)| is_num_lock_on(device));
    let active_layout =
        dboard_protocol::layout::KeyboardLayout::from_str_name(&daemon.config.keyboard_layout);
    let injector = create_injector_with_locks_and_layout(
        display,
        global_caps_lock,
        global_num_lock,
        active_layout,
    )?;
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
    let key_activity_seq = initial_focus.key_activity_signal();
    let mut focus_sync = crate::focus_sync::FocusSyncState::from_initial(initial_focus);
    let mut last_terminal_enter_at: Option<std::time::Instant> = None;
    let mut last_pty_refresh_at: Option<std::time::Instant> = None;
    let mut last_pointer_activity = pointer_activity.load(Ordering::SeqCst);
    let mut last_idle_suggestion_seq = key_activity_seq.load(Ordering::Acquire);
    let mut hotplug_state = crate::hotplug::HotplugState::new();

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

        // Poll ALL keyboard devices (+ optional non-blocking popup click-select socket)
        let mut pfds: Vec<libc::pollfd> = devices
            .iter()
            .map(|(d, _)| libc::pollfd {
                fd: d.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        let dev_fd_count = pfds.len();
        if let Some(sel_fd) = daemon.candidate_ipc.select_rx_fd() {
            pfds.push(libc::pollfd {
                fd: sel_fd,
                events: libc::POLLIN,
                revents: 0,
            });
        }

        let pfds_ptr = if pfds.is_empty() {
            std::ptr::null_mut()
        } else {
            pfds.as_mut_ptr()
        };
        let poll_ret = unsafe { libc::poll(pfds_ptr, pfds.len() as libc::nfds_t, 50) };
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

        // Wall-clock hotplug scan every 2s (immune to continuous-typing poll starvation)
        if hotplug_state.due_for_scan(std::time::Instant::now()) {
            let new_devs = discover_new_keyboards(&known_paths, &mut known_phys_ids);
            for (mut dev, name) in new_devs {
                let path = dev_path_of(&name);
                if daemon.grab_enabled && crate::device::has_held_keys(&dev) {
                    if let Some(pid) =
                        crate::device::get_device_phys_id(std::path::Path::new(&path))
                    {
                        known_phys_ids.remove(&pid);
                    }
                    continue;
                }
                let grabbed = daemon.grab_enabled && dev.grab().is_ok();
                if daemon.grab_enabled && !grabbed && hotplug_state.should_retry_grab(&path) {
                    if let Some(pid) =
                        crate::device::get_device_phys_id(std::path::Path::new(&path))
                    {
                        known_phys_ids.remove(&pid);
                    }
                    continue;
                }
                hotplug_state.clear_path(&path);
                known_paths.insert(path);
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

        if poll_ret == 0 {
            focus_sync.sync_on_idle(daemon, &*injector, &shared_focus);
            if daemon.config.candidate_bar && daemon.engine.is_enabled() {
                if daemon.engine.buffer().is_empty() {
                    let surrounding = crate::focus::peek_surrounding_context(
                        &shared_focus,
                        focus_sync.last_focus_generation,
                    );
                    let word = surrounding
                        .as_ref()
                        .and_then(|s| s.word_at_caret.as_deref());
                    let prev = surrounding.as_ref().and_then(|s| s.prev_word.as_deref());
                    daemon.sync_caret_reselection(word, prev);
                } else {
                    let cur_seq = key_activity_seq.load(Ordering::Acquire);
                    if cur_seq != last_idle_suggestion_seq {
                        last_idle_suggestion_seq = cur_seq;
                        daemon.sync_candidate_bar(true);
                    }
                }
            }
            continue;
        }

        // Handle non-blocking mouse-click candidate selection from `dboard-popup`
        if pfds
            .get(dev_fd_count)
            .is_some_and(|p| (p.revents & libc::POLLIN) != 0)
        {
            if let Some(idx) = daemon.candidate_ipc.poll_clicked_index() {
                focus_sync.sync_on_idle(daemon, &*injector, &shared_focus);
                if !daemon.app_state.is_password_active()
                    && !daemon.is_current_app_bypassed()
                    && daemon.engine.is_enabled()
                {
                    let commands = daemon.select_candidate(idx);
                    crate::focus::clear_surrounding_word(&shared_focus);
                    // Absorb the popup's own BTN_LEFT click so the next keypress does not
                    // reset `ring_buffer` and break chained next-word prediction.
                    last_pointer_activity = pointer_activity.load(Ordering::SeqCst);
                    if !commands.is_empty() {
                        let mut combined_keys = evdev::AttributeSet::<evdev::KeyCode>::new();
                        for ds in &device_states {
                            for k in ds.keys.iter() {
                                combined_keys.insert(k);
                            }
                        }
                        let outcome = execute_with_shift_released(
                            &*injector,
                            &commands,
                            &mut daemon.visible_revision,
                            focus_sync.last_focus_generation,
                            &combined_keys,
                        );
                        let _ = daemon.handle_transaction_outcome(outcome);
                    }
                }
            }
        }

        // Process events from whichever device(s) have data ready
        let mut dead: Vec<usize> = Vec::new();
        for (i, pfd) in pfds[..dev_fd_count].iter().enumerate() {
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
            let mut dev_key_state = std::mem::take(&mut device_states[i].keys);
            let mut consumed_keys = std::mem::take(&mut device_states[i].consumed_keys);
            let mut forwarded_keys = std::mem::take(&mut device_states[i].forwarded_keys);
            for event in event_list {
                if event.event_type() != evdev::EventType::KEY {
                    continue;
                }
                let keycode = event.code();
                let value = event.value();
                let key = evdev::KeyCode(keycode);

                // Update per-device modifier and shared lock state, then OR-union
                // modifiers across all connected keyboards (split keyboards / numpads).
                let prev_caps = global_caps_lock;
                let prev_num = global_num_lock;
                update_keyboard_state(
                    &mut dev_key_state,
                    &mut global_caps_lock,
                    &mut global_num_lock,
                    key,
                    value,
                );
                let key_state = if device_states.len() > 1 {
                    let combined = crate::hotplug::combine_cross_device_modifiers(
                        &dev_key_state,
                        device_states
                            .iter()
                            .enumerate()
                            .filter(|(idx, _)| *idx != i)
                            .map(|(_, st)| &st.keys),
                    );
                    if value == 1
                        && is_modifier_pressed(&combined)
                        && matches!(
                            key,
                            evdev::KeyCode::KEY_CAPSLOCK | evdev::KeyCode::KEY_NUMLOCK
                        )
                    {
                        global_caps_lock = prev_caps;
                        global_num_lock = prev_num;
                    }
                    combined
                } else {
                    dev_key_state.clone()
                };
                if global_caps_lock != prev_caps {
                    injector.set_caps_lock(global_caps_lock);
                }
                if global_num_lock != prev_num {
                    injector.set_num_lock(global_num_lock);
                }
                // Complete a stroke already forwarded during uncertainty before
                // focus recovery, policy, dedup or normal Backspace handling.
                if finish_forwarded_stroke(&*injector, keycode, value, &mut forwarded_keys) {
                    continue;
                }

                // A mouse click may have moved the caret or switched windows
                // without any keyboard event: the half-typed word no longer
                // matches the screen, so it must not be replayed or replaced.
                if value == 1 {
                    key_activity_seq.fetch_add(1, Ordering::AcqRel);
                    let pointer_now = pointer_activity.load(Ordering::SeqCst);
                    if pointer_now != last_pointer_activity {
                        last_pointer_activity = pointer_now;
                        daemon.reset_composition_state();
                        clear_omnibox_shield(&*injector, &shared_focus);
                    }
                }

                // Only the primary ungrabbed keyboard is composed; the others
                // already deliver native input. A grabbed device always needs
                // forwarding, regardless of the grab results of other devices.
                if !grabbed && i != 0 {
                    continue;
                }

                // Observe focus changes, per-app state, FocusLost resets, and
                // password field transitions ONCE per keystroke (single lock on
                // `shared_focus`) BEFORE bypass, toggle, engine, or Backspace logic.
                if value == 1 {
                    if daemon.config.password_detection.enabled
                        && daemon.app_state.is_terminal_app()
                    {
                        if matches!(key, evdev::KeyCode::KEY_ENTER | evdev::KeyCode::KEY_KPENTER) {
                            last_terminal_enter_at = Some(std::time::Instant::now());
                            last_pty_refresh_at = None;
                        } else if !crate::password_detector::cached_pty_no_echo()
                            && last_terminal_enter_at.is_some_and(|at| {
                                at.elapsed() < std::time::Duration::from_millis(2000)
                            })
                            && last_pty_refresh_at.is_none_or(|at| {
                                at.elapsed() >= std::time::Duration::from_millis(100)
                            })
                        {
                            last_pty_refresh_at = Some(std::time::Instant::now());
                            crate::password_detector::refresh_pty_no_echo_now();
                        }
                    }
                    focus_sync.sync_on_key_press(daemon, &*injector, &shared_focus);
                }

                if !daemon.app_state.is_password_active()
                    && matches!(
                        key,
                        evdev::KeyCode::KEY_CAPSLOCK
                            | evdev::KeyCode::KEY_LEFTSHIFT
                            | evdev::KeyCode::KEY_RIGHTSHIFT
                    )
                {
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

                // Completely bypass all IME processing for remote desktops, VM viewers and games
                if daemon.is_current_app_bypassed() {
                    if consume_released_or_repeated_key(&mut consumed_keys, keycode, value) {
                        continue;
                    }
                    if grabbed {
                        injector.send_key_event(keycode, value);
                    }
                    continue;
                }

                if value == 1
                    && !daemon.app_state.is_password_active()
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
                        1 | 14 | 15 | 28 | 29 | 42 | 54 | 56 | 58 | 69 | 96 | 97 | 100 | 102
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
                        if debug_logging && !daemon.app_state.is_password_active() {
                            log_debug(&format!(
                                "[dboard] dedup dropped keycode {} (window {}ms)",
                                keycode, daemon.config.deduplicate_window_ms
                            ));
                        }
                        consumed_keys.insert(keycode);
                        continue;
                    }
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
                    if let Some(ch) = key_to_char_with_layout(
                        active_layout,
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
                            && daemon.engine.is_enabled()
                            && !is_modifier_pressed(&key_state)
                        {
                            let shift = is_modifier_held_shift(&key_state);
                            if keycode == 57 && !shift {
                                let _ = daemon.process_key(' ');
                                let _ = injector.send_char(' ');
                            } else if let Some(ch) =
                                key_to_char_with_layout(active_layout, key, shift, global_caps_lock)
                                    .filter(|&c| is_flush_char(c))
                            {
                                let _ = daemon.process_key(ch);
                                injector.send_key_event(keycode, 1);
                                injector.send_key_event(keycode, 0);
                            }
                        }
                        continue;
                    }
                    if is_modifier_pressed(&key_state) {
                        if value == 1
                            && daemon.config.candidate_bar
                            && daemon.engine.is_enabled()
                            && (!daemon.engine.buffer().is_empty()
                                || daemon.caret_reselection.is_some())
                            && !is_modifier_held_shift(&key_state)
                            && (key_state.contains(evdev::KeyCode::KEY_LEFTALT)
                                || key_state.contains(evdev::KeyCode::KEY_RIGHTALT))
                            && !key_state.contains(evdev::KeyCode::KEY_LEFTCTRL)
                            && !key_state.contains(evdev::KeyCode::KEY_RIGHTCTRL)
                            && !key_state.contains(evdev::KeyCode::KEY_LEFTMETA)
                            && !key_state.contains(evdev::KeyCode::KEY_RIGHTMETA)
                            && matches!(
                                key,
                                evdev::KeyCode::KEY_1
                                    | evdev::KeyCode::KEY_2
                                    | evdev::KeyCode::KEY_3
                            )
                        {
                            let idx = match key {
                                evdev::KeyCode::KEY_1 => 0,
                                evdev::KeyCode::KEY_2 => 1,
                                _ => 2,
                            };
                            if let Some(commands) = daemon.try_select_candidate(idx, false) {
                                crate::focus::clear_surrounding_word(&shared_focus);
                                consumed_keys.insert(keycode);
                                if !commands.is_empty() {
                                    // Release physically held Alt before injecting Backspace+Type so
                                    // target apps never receive Alt+Backspace or Alt+<char> mnemonics.
                                    for alt in
                                        [evdev::KeyCode::KEY_LEFTALT, evdev::KeyCode::KEY_RIGHTALT]
                                    {
                                        if key_state.contains(alt) {
                                            injector.send_key_event(alt.code(), 0);
                                            if dev_key_state.contains(alt) {
                                                consumed_keys.insert(alt.code());
                                            }
                                            for (idx, st) in device_states.iter_mut().enumerate() {
                                                if idx != i && st.keys.contains(alt) {
                                                    st.consumed_keys.insert(alt.code());
                                                }
                                            }
                                        }
                                    }
                                    let outcome = execute_commands(
                                        &*injector,
                                        &commands,
                                        &mut daemon.visible_revision,
                                        focus_sync.last_focus_generation,
                                    );
                                    let _ = daemon.handle_transaction_outcome(outcome);
                                }
                                continue;
                            }
                        }
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
                        if let Some(ch) = key_to_char_with_layout(
                            active_layout,
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
                        if matches!(
                            key,
                            evdev::KeyCode::KEY_LEFTSHIFT | evdev::KeyCode::KEY_RIGHTSHIFT
                        ) {
                            continue;
                        }
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

            // Save updated per-device key state back.
            device_states[i].keys = dev_key_state;
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
                let removed = device_states.remove(d);
                release_disconnected_device_keys(&*injector, &removed, &device_states);
                let path = dev_path_of(&name);
                hotplug_state.clear_path(&path);
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
#[path = "evdev_loop_tests.rs"]
mod grabbed_event_stream_tests;
