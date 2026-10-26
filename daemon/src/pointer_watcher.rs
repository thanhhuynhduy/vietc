// SPDX-License-Identifier: MIT
//! Read-only pointer button watcher.
//!
//! A mouse/touchpad click can move the caret or change window focus without
//! any keyboard event, leaving a half-typed word in the engine that no longer
//! matches what is on screen (the next key would then delete/replace text the
//! user never typed). Like other IMEs, we treat any pointer button press as a
//! composition boundary. Devices are only opened for reading, never grabbed.

use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
const POLL_TIMEOUT_MS: i32 = 200;

fn is_pointer_button(code: u16) -> bool {
    matches!(
        evdev::Key(code),
        evdev::Key::BTN_LEFT | evdev::Key::BTN_RIGHT | evdev::Key::BTN_MIDDLE
    )
}

fn is_pointer_device(device: &evdev::Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(evdev::Key::BTN_LEFT))
}

fn scan_new(open_paths: &mut HashSet<std::path::PathBuf>) -> Vec<(std::path::PathBuf, evdev::Device)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if !is_event || open_paths.contains(&path) {
            continue;
        }
        if let Ok(device) = evdev::Device::open(&path) {
            if is_pointer_device(&device) {
                open_paths.insert(path.clone());
                found.push((path, device));
            }
        }
    }
    found
}

/// Spawn the watcher. The returned counter increments on every pointer button
/// press; consumers compare it with the last value they saw.
pub fn spawn_pointer_watcher() -> Arc<AtomicU64> {
    let counter = Arc::new(AtomicU64::new(0));
    let shared = counter.clone();
    std::thread::spawn(move || {
        let mut open_paths: HashSet<std::path::PathBuf> = HashSet::new();
        let mut devices: Vec<(std::path::PathBuf, evdev::Device)> = Vec::new();
        let mut last_scan = Instant::now() - RESCAN_INTERVAL;
        loop {
            if last_scan.elapsed() >= RESCAN_INTERVAL {
                last_scan = Instant::now();
                devices.extend(scan_new(&mut open_paths));
            }
            if devices.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            let mut pfds: Vec<libc::pollfd> = devices
                .iter()
                .map(|(_, device)| libc::pollfd {
                    fd: device.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            let ready =
                unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, POLL_TIMEOUT_MS) };
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
                match devices[index].1.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if event.event_type() == evdev::EventType::KEY
                                && event.value() == 1
                                && is_pointer_button(event.code())
                            {
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
                let (path, _) = devices.remove(index);
                open_paths.remove(&path);
            }
        }
    });
    counter
}

#[cfg(test)]
mod tests {
    use super::is_pointer_button;

    #[test]
    fn only_real_pointer_buttons_count() {
        assert!(is_pointer_button(evdev::Key::BTN_LEFT.code()));
        assert!(is_pointer_button(evdev::Key::BTN_RIGHT.code()));
        assert!(is_pointer_button(evdev::Key::BTN_MIDDLE.code()));
        assert!(!is_pointer_button(evdev::Key::KEY_A.code()));
        assert!(!is_pointer_button(evdev::Key::BTN_TOUCH.code()));
    }
}
