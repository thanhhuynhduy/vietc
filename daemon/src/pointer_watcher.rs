// SPDX-License-Identifier: MIT
//! Read-only pointer button and touchpad tap-to-click watcher.
//!
//! A mouse click or touchpad tap-to-click can move the caret or change window
//! focus without any keyboard event, leaving a half-typed word in the engine
//! that no longer matches what is on screen. Devices are only opened for
//! reading, never grabbed.

use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
const POLL_TIMEOUT_MS: i32 = 200;
const TAP_MAX_DURATION: Duration = Duration::from_millis(200);
const TAP_MAX_TRAVEL: i32 = 30;

fn is_pointer_button(code: u16) -> bool {
    matches!(
        evdev::KeyCode(code),
        evdev::KeyCode::BTN_LEFT | evdev::KeyCode::BTN_RIGHT | evdev::KeyCode::BTN_MIDDLE
    )
}

fn is_multitouch_tool(code: u16) -> bool {
    matches!(
        evdev::KeyCode(code),
        evdev::KeyCode::BTN_TOOL_DOUBLETAP
            | evdev::KeyCode::BTN_TOOL_TRIPLETAP
            | evdev::KeyCode::BTN_TOOL_QUADTAP
            | evdev::KeyCode::BTN_TOOL_QUINTTAP
    )
}

fn is_pointer_device(device: &evdev::Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(evdev::KeyCode::BTN_LEFT) || keys.contains(evdev::KeyCode::BTN_TOUCH)
    })
}

/// Per-device tap-to-click state machine distinguishing short single-finger
/// taps (`BTN_TOUCH` press -> release in `< 200ms` with minimal travel) from
/// palm brushes, drags, two-finger scrolls, and physical clickpad presses.
#[derive(Debug, Default)]
struct TapTracker {
    touch_down_at: Option<SystemTime>,
    moved_or_multitouch: bool,
    clicked_during_touch: bool,
    start_x: Option<i32>,
    start_y: Option<i32>,
}

impl TapTracker {
    fn observe_event(
        &mut self,
        event_type: evdev::EventType,
        code: u16,
        value: i32,
        now: SystemTime,
    ) -> bool {
        match event_type {
            evdev::EventType::KEY => {
                if value == 1 && is_pointer_button(code) {
                    self.clicked_during_touch = true;
                    return true;
                }
                if value == 1 && is_multitouch_tool(code) {
                    self.moved_or_multitouch = true;
                    return false;
                }
                if evdev::KeyCode(code) == evdev::KeyCode::BTN_TOUCH {
                    if value == 1 {
                        self.touch_down_at = Some(now);
                        self.moved_or_multitouch = false;
                        self.clicked_during_touch = false;
                        self.start_x = None;
                        self.start_y = None;
                    } else if value == 0 {
                        let short_tap = self.touch_down_at.take().is_some_and(|down| {
                            now.duration_since(down)
                                .ok()
                                .is_some_and(|d| d < TAP_MAX_DURATION)
                        });
                        let fire =
                            short_tap && !self.moved_or_multitouch && !self.clicked_during_touch;
                        self.moved_or_multitouch = false;
                        self.clicked_during_touch = false;
                        self.start_x = None;
                        self.start_y = None;
                        return fire;
                    }
                }
                false
            }
            evdev::EventType::ABSOLUTE if self.touch_down_at.is_some() => {
                let axis = evdev::AbsoluteAxisCode(code);
                if axis == evdev::AbsoluteAxisCode::ABS_X
                    || axis == evdev::AbsoluteAxisCode::ABS_MT_POSITION_X
                {
                    let origin = *self.start_x.get_or_insert(value);
                    if (value - origin).abs() > TAP_MAX_TRAVEL {
                        self.moved_or_multitouch = true;
                    }
                } else if axis == evdev::AbsoluteAxisCode::ABS_Y
                    || axis == evdev::AbsoluteAxisCode::ABS_MT_POSITION_Y
                {
                    let origin = *self.start_y.get_or_insert(value);
                    if (value - origin).abs() > TAP_MAX_TRAVEL {
                        self.moved_or_multitouch = true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

fn scan_new(
    open_paths: &mut HashSet<std::path::PathBuf>,
    rejected_cache: &mut std::collections::HashMap<std::path::PathBuf, (u64, i64)>,
) -> Vec<(std::path::PathBuf, evdev::Device, TapTracker)> {
    use std::os::unix::fs::MetadataExt;

    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return found;
    };
    let mut seen_paths = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if !is_event {
            continue;
        }
        seen_paths.insert(path.clone());
        if open_paths.contains(&path) {
            continue;
        }
        let stamp = std::fs::metadata(&path).ok().map(|m| (m.ino(), m.ctime()));
        if let Some(st) = stamp {
            if rejected_cache.get(&path).copied() == Some(st) {
                continue;
            }
        }
        if let Ok(device) = evdev::Device::open(&path) {
            if is_pointer_device(&device) {
                rejected_cache.remove(&path);
                open_paths.insert(path.clone());
                found.push((path, device, TapTracker::default()));
            } else if let Some(st) = stamp {
                rejected_cache.insert(path, st);
            }
        }
    }
    rejected_cache.retain(|p, _| seen_paths.contains(p));
    found
}

/// Spawn the watcher. The returned counter increments on every pointer button
/// press or touchpad tap-to-click; consumers compare it with the last value
/// they saw. Pre-click surrounding context is cleared immediately at click time
/// so fresh post-click AT-SPI `TextCaretMoved` context survives until the next keypress.
pub fn spawn_pointer_watcher(shared_focus: crate::focus::SharedFocusSnapshot) -> Arc<AtomicU64> {
    let counter = Arc::new(AtomicU64::new(0));
    let shared = counter.clone();
    std::thread::spawn(move || {
        let mut open_paths: HashSet<std::path::PathBuf> = HashSet::new();
        let mut rejected_cache: std::collections::HashMap<std::path::PathBuf, (u64, i64)> =
            std::collections::HashMap::new();
        let mut devices: Vec<(std::path::PathBuf, evdev::Device, TapTracker)> = Vec::new();
        let mut last_scan: Option<Instant> = None;
        loop {
            if crate::signal::SIGNAL_EXIT.load(Ordering::SeqCst) {
                break;
            }
            if last_scan.is_none_or(|t| t.elapsed() >= RESCAN_INTERVAL) {
                last_scan = Some(Instant::now());
                devices.extend(scan_new(&mut open_paths, &mut rejected_cache));
            }
            if devices.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            let mut pfds: Vec<libc::pollfd> = devices
                .iter()
                .map(|(_, device, _)| libc::pollfd {
                    fd: device.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            let ready = unsafe {
                libc::poll(
                    pfds.as_mut_ptr(),
                    pfds.len() as libc::nfds_t,
                    POLL_TIMEOUT_MS,
                )
            };
            if ready <= 0 {
                continue;
            }
            let mut dead = Vec::new();
            for (index, pfd) in pfds.iter().enumerate() {
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    dead.push(index);
                    continue;
                }
                if pfd.revents & libc::POLLIN == 0 {
                    continue;
                }
                let (_, ref mut dev, ref mut tracker) = devices[index];
                match dev.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if tracker.observe_event(
                                event.event_type(),
                                event.code(),
                                event.value(),
                                event.timestamp(),
                            ) {
                                crate::focus::record_pointer_click(&shared_focus);
                                shared.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => dead.push(index),
                }
            }
            dead.sort_unstable();
            dead.dedup();
            for index in dead.into_iter().rev() {
                let (path, _, _) = devices.remove(index);
                open_paths.remove(&path);
            }
        }
    });
    counter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_pointer_buttons_count() {
        assert!(is_pointer_button(evdev::KeyCode::BTN_LEFT.code()));
        assert!(is_pointer_button(evdev::KeyCode::BTN_RIGHT.code()));
        assert!(is_pointer_button(evdev::KeyCode::BTN_MIDDLE.code()));
        assert!(!is_pointer_button(evdev::KeyCode::KEY_A.code()));
        assert!(!is_pointer_button(evdev::KeyCode::BTN_TOUCH.code()));
    }

    #[test]
    fn touchpad_tap_tracker_distinguishes_short_tap_from_drag_scroll_and_clickpad() {
        let mut tracker = TapTracker::default();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

        // 1. Short single-finger tap (< 200ms, minimal movement) -> fires on release
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            1,
            t0,
        ));
        assert!(tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            0,
            t0 + Duration::from_millis(80),
        ));

        // 2. Long touch (>= 200ms) -> does NOT fire
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            1,
            t0,
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            0,
            t0 + Duration::from_millis(250),
        ));

        // 3. Large finger travel (scroll/swipe/palm) -> does NOT fire
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            1,
            t0,
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::ABSOLUTE,
            evdev::AbsoluteAxisCode::ABS_MT_POSITION_X.0,
            100,
            t0 + Duration::from_millis(10),
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::ABSOLUTE,
            evdev::AbsoluteAxisCode::ABS_MT_POSITION_X.0,
            180,
            t0 + Duration::from_millis(20),
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            0,
            t0 + Duration::from_millis(60),
        ));

        // 4. Two-finger tap/scroll -> does NOT fire
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            1,
            t0,
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOOL_DOUBLETAP.code(),
            1,
            t0 + Duration::from_millis(5),
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            0,
            t0 + Duration::from_millis(50),
        ));

        // 5. Clickpad (`BTN_TOUCH` + `BTN_LEFT`) -> fires ONCE on `BTN_LEFT`, not twice
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            1,
            t0,
        ));
        assert!(tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_LEFT.code(),
            1,
            t0 + Duration::from_millis(30),
        ));
        assert!(!tracker.observe_event(
            evdev::EventType::KEY,
            evdev::KeyCode::BTN_TOUCH.code(),
            0,
            t0 + Duration::from_millis(70),
        ));
    }
}
