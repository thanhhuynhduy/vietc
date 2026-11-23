// SPDX-License-Identifier: MIT
//! Keyboard hotplug retry scheduling and cross-device modifier aggregation.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const HOTPLUG_SCAN_INTERVAL: Duration = Duration::from_secs(2);
pub const MAX_GRAB_RETRIES: u8 = 5;

pub const MODIFIER_KEYS: [evdev::KeyCode; 8] = [
    evdev::KeyCode::KEY_LEFTSHIFT,
    evdev::KeyCode::KEY_RIGHTSHIFT,
    evdev::KeyCode::KEY_LEFTCTRL,
    evdev::KeyCode::KEY_RIGHTCTRL,
    evdev::KeyCode::KEY_LEFTALT,
    evdev::KeyCode::KEY_RIGHTALT,
    evdev::KeyCode::KEY_LEFTMETA,
    evdev::KeyCode::KEY_RIGHTMETA,
];

/// Tracks wall-clock hotplug scan cadence (immune to continuous-typing poll
/// starvation) and bounded retries for transient `EVIOCGRAB` failures while
/// udev is still applying `uaccess` ACLs.
pub struct HotplugState {
    last_scan: Instant,
    grab_retries: HashMap<String, u8>,
}

impl HotplugState {
    pub fn new() -> Self {
        Self {
            last_scan: Instant::now(),
            grab_retries: HashMap::new(),
        }
    }

    pub fn due_for_scan(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last_scan) >= HOTPLUG_SCAN_INTERVAL {
            self.last_scan = now;
            true
        } else {
            false
        }
    }

    /// Record a transient `dev.grab()` failure on `path`.
    /// Returns `true` while retries remain (`< MAX_GRAB_RETRIES`) so the caller
    /// defers adding `path` to `known_paths` and removes its `phys_id` for retry.
    pub fn should_retry_grab(&mut self, path: &str) -> bool {
        let count = self.grab_retries.entry(path.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        *count < MAX_GRAB_RETRIES
    }

    pub fn clear_path(&mut self, path: &str) {
        self.grab_retries.remove(path);
    }
}

/// Merge modifier keys (`Shift`/`Ctrl`/`Alt`/`Super`) held on any other
/// connected keyboard into `local` so split keyboards and separate numpads
/// share modifier state seamlessly.
pub fn combine_cross_device_modifiers<'a>(
    local: &evdev::AttributeSet<evdev::KeyCode>,
    others: impl IntoIterator<Item = &'a evdev::AttributeSet<evdev::KeyCode>>,
) -> evdev::AttributeSet<evdev::KeyCode> {
    let mut combined = local.clone();
    for other in others {
        for &m in &MODIFIER_KEYS {
            if other.contains(m) {
                combined.insert(m);
            }
        }
    }
    combined
}

/// Returns `true` if `keycode` is a modifier still held on at least one other
/// connected keyboard (so disconnecting a secondary keyboard must not release it).
pub fn is_modifier_held_elsewhere<'a>(
    keycode: u16,
    others: impl IntoIterator<Item = &'a evdev::AttributeSet<evdev::KeyCode>>,
) -> bool {
    let key = evdev::KeyCode(keycode);
    if !MODIFIER_KEYS.contains(&key) {
        return false;
    }
    others.into_iter().any(|keys| keys.contains(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotplug_grab_retry_allows_up_to_five_attempts_and_resets_on_clear() {
        let mut state = HotplugState::new();
        for _ in 0..4 {
            assert!(state.should_retry_grab("/dev/input/event9"));
        }
        // 5th failure exhausts retry budget
        assert!(!state.should_retry_grab("/dev/input/event9"));
        state.clear_path("/dev/input/event9");
        assert!(state.should_retry_grab("/dev/input/event9"));
    }

    #[test]
    fn hotplug_scan_uses_wall_clock_interval_without_poll_starvation() {
        let mut state = HotplugState::new();
        let t0 = state.last_scan;
        assert!(!state.due_for_scan(t0 + Duration::from_millis(1900)));
        assert!(state.due_for_scan(t0 + Duration::from_secs(2)));
        assert!(!state.due_for_scan(t0 + Duration::from_millis(3500)));
    }

    #[test]
    fn cross_device_modifiers_or_union_and_preserve_other_keyboard_on_disconnect() {
        let mut kb_left = evdev::AttributeSet::new();
        kb_left.insert(evdev::KeyCode::KEY_LEFTSHIFT);
        let mut kb_right = evdev::AttributeSet::new();
        kb_right.insert(evdev::KeyCode::KEY_A);

        let combined = combine_cross_device_modifiers(&kb_right, [&kb_left]);
        assert!(combined.contains(evdev::KeyCode::KEY_LEFTSHIFT));
        assert!(combined.contains(evdev::KeyCode::KEY_A));

        assert!(is_modifier_held_elsewhere(
            evdev::KeyCode::KEY_LEFTSHIFT.code(),
            [&kb_left]
        ));
        assert!(!is_modifier_held_elsewhere(
            evdev::KeyCode::KEY_A.code(),
            [&kb_right]
        ));
    }
}
