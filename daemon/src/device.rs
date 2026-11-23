use std::fs;

use crate::log::log_info;

fn is_dboard_virtual_device_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("dboard")
}

pub fn is_valid_keyboard(device: &evdev::Device) -> bool {
    let dev_name = device.name().unwrap_or("unknown").to_string();
    let dev_name_lower = dev_name.to_lowercase();
    if is_dboard_virtual_device_name(&dev_name) {
        return false;
    }
    if dev_name_lower.contains("touchpad") || dev_name_lower.contains("trackpoint") {
        return false;
    }
    // Reject pointers/mice based on relative axes (REL_X / REL_Y)
    if let Some(rel) = device.supported_relative_axes() {
        if rel.contains(evdev::RelativeAxisCode::REL_X)
            || rel.contains(evdev::RelativeAxisCode::REL_Y)
        {
            return false;
        }
    }
    if let Some(keys) = device.supported_keys() {
        keys.contains(evdev::KeyCode::KEY_A)
            && keys.contains(evdev::KeyCode::KEY_Z)
            && keys.contains(evdev::KeyCode::KEY_SPACE)
            && keys.contains(evdev::KeyCode::KEY_ENTER)
    } else {
        false
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn dboard_uinput_device_names_are_never_capture_candidates() {
        assert!(is_dboard_virtual_device_name("dboard"));
        assert!(is_dboard_virtual_device_name("Dboard"));
        assert!(!is_dboard_virtual_device_name("dboard-test-vni-word"));
        assert!(!is_dboard_virtual_device_name("dboard-vk"));
        assert!(!is_dboard_virtual_device_name(
            "AT Translated Set 2 keyboard"
        ));
    }
}

pub fn get_device_phys_id(path: &std::path::Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    if !file_name.starts_with("event") {
        return None;
    }
    let ev_num = file_name.strip_prefix("event")?;
    let dev_dir = format!("/sys/class/input/event{}/device", ev_num);
    let dev_path = std::path::Path::new(&dev_dir);
    if !dev_path.exists() {
        return None;
    }

    // 1. Check uniq + phys together: composite USB/Bluetooth keyboards expose
    // multiple keyboard interfaces sharing the same `uniq` serial/MAC but
    // distinct `phys` paths (e.g. input0 vs input2). Combining both prevents
    // skipping the real typing interface while still deduplicating the same
    // logical device across reconnects.
    let uniq = fs::read_to_string(dev_path.join("uniq"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let phys = fs::read_to_string(dev_path.join("phys"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match (uniq, phys) {
        (Some(u), Some(ph)) => return Some(format!("uniq:{}|phys:{}", u, ph)),
        (Some(u), None) => return Some(format!("uniq:{}", u)),
        (None, Some(ph)) => return Some(format!("phys:{}", ph)),
        (None, None) => {}
    }

    // 3. Fallback to parent sysfs path — use full canonical path (unique per event)
    // Previous code used parent.parent() which collapsed all virtual uinput
    // devices (e.g. /sys/devices/virtual/input/inputXX) to the same
    // "sysfs:/sys/devices/virtual/input", causing only the first virtual
    // keyboard to be grabbed and subsequent ones (e.g. dboard-vk, tests) to be
    // ignored → raw input, no conversion, and apparent double/loss.
    if let Ok(canonical) = fs::canonicalize(dev_path) {
        let canon_str = canonical.display().to_string();
        // Virtual uinput devices all live under /sys/devices/virtual/input/
        // and reuse input numbers (input31, input32, ...) quickly. Using the
        // canonical path as phys_id would still collide when the kernel
        // reuses the same inputXX for a new device before the old stale entry
        // is cleared. For virtual devices, return None so they are deduped
        // only by their /dev/input/eventXX path, not by phys.
        if canon_str.contains("/virtual/") {
            return None;
        }
        return Some(format!("sysfs:{}", canon_str));
    }

    Some(format!("path:{}", path.display()))
}

pub fn open_keyboard_devices() -> Result<Vec<(evdev::Device, String)>, Box<dyn std::error::Error>> {
    let mut devices: Vec<(evdev::Device, String)> = Vec::new();
    let mut permission_denied_count = 0u32;
    let mut total_event_count = 0u32;
    let mut seen_paths = std::collections::HashSet::new();
    let mut seen_phys_ids = std::collections::HashSet::new();

    // Strategy 1: Open primary keyboard devices via /dev/input/by-path/*-event-kbd
    let by_path = std::path::Path::new("/dev/input/by-path");
    if by_path.exists() {
        if let Ok(rd) = fs::read_dir(by_path) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with("-event-kbd") {
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        let phys_opt = get_device_phys_id(&real_path);
                        if phys_opt
                            .as_ref()
                            .is_some_and(|pid| seen_phys_ids.contains(pid))
                        {
                            continue;
                        }
                        if !seen_paths.insert(p) {
                            continue;
                        }
                        match evdev::Device::open(&real_path) {
                            Ok(device) => {
                                let dev_name = device.name().unwrap_or("unknown").to_string();
                                if is_valid_keyboard(&device) {
                                    if let Some(pid) = phys_opt {
                                        seen_phys_ids.insert(pid);
                                    }
                                    log_info(&format!(
                                        "[dboard] Found keyboard (by-path): {} ({})",
                                        real_path.display(),
                                        dev_name
                                    ));
                                    devices.push((
                                        device,
                                        format!("{} ({})", real_path.display(), dev_name),
                                    ));
                                }
                            }
                            Err(e) => {
                                if e.raw_os_error() == Some(libc::EACCES) {
                                    permission_denied_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Strategy 2: Open keyboard devices via /dev/input/by-id/*-event-kbd
    let by_id = std::path::Path::new("/dev/input/by-id");
    if by_id.exists() {
        if let Ok(rd) = fs::read_dir(by_id) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with("-event-kbd") {
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        let phys_opt = get_device_phys_id(&real_path);
                        if phys_opt
                            .as_ref()
                            .is_some_and(|pid| seen_phys_ids.contains(pid))
                        {
                            continue;
                        }
                        if !seen_paths.insert(p) {
                            continue;
                        }
                        match evdev::Device::open(&real_path) {
                            Ok(device) => {
                                let dev_name = device.name().unwrap_or("unknown").to_string();
                                if is_valid_keyboard(&device) {
                                    if let Some(pid) = phys_opt {
                                        seen_phys_ids.insert(pid);
                                    }
                                    log_info(&format!(
                                        "[dboard] Found keyboard (by-id): {} ({})",
                                        real_path.display(),
                                        dev_name
                                    ));
                                    devices.push((
                                        device,
                                        format!("{} ({})", real_path.display(), dev_name),
                                    ));
                                }
                            }
                            Err(e) => {
                                if e.raw_os_error() == Some(libc::EACCES) {
                                    permission_denied_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Strategy 3: Scan /dev/input/event* for Bluetooth & direct devices without by-path/by-id symlinks
    let dir = std::path::Path::new("/dev/input");
    if dir.exists() {
        if let Ok(rd) = fs::read_dir(dir) {
            for entry in rd.flatten() {
                let name_str = entry.file_name().to_string_lossy().to_string();
                if name_str.starts_with("event") {
                    total_event_count += 1;
                    if let Ok(real_path) = fs::canonicalize(entry.path()) {
                        let p = real_path.to_string_lossy().to_string();
                        let phys_opt = get_device_phys_id(&real_path);
                        if phys_opt
                            .as_ref()
                            .is_some_and(|pid| seen_phys_ids.contains(pid))
                        {
                            continue;
                        }
                        if !seen_paths.insert(p) {
                            continue;
                        }
                        match evdev::Device::open(&real_path) {
                            Ok(device) => {
                                let dev_name = device.name().unwrap_or("unknown").to_string();
                                if is_valid_keyboard(&device) {
                                    if let Some(pid) = phys_opt {
                                        seen_phys_ids.insert(pid);
                                    }
                                    log_info(&format!(
                                        "[dboard] Found keyboard (bluetooth/direct): {} ({})",
                                        real_path.display(),
                                        dev_name
                                    ));
                                    devices.push((
                                        device,
                                        format!("{} ({})", real_path.display(), dev_name),
                                    ));
                                }
                            }
                            Err(e) => {
                                if e.raw_os_error() == Some(libc::EACCES) {
                                    permission_denied_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !devices.is_empty() {
        log_info(&format!(
            "[dboard] Opened {} keyboard device(s)",
            devices.len()
        ));
        return Ok(devices);
    }

    if permission_denied_count > 0 {
        let username = std::env::var("USER").unwrap_or_else(|_| {
            std::process::Command::new("id")
                .arg("-un")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        });
        let in_group_db = if !username.is_empty() {
            std::process::Command::new("id")
                .args(["-nG", &username])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("input"))
                .unwrap_or(false)
        } else {
            false
        };

        if in_group_db {
            Err(format!(
                "Permission denied on {}/{} devices. Your user IS in the 'input' group, \
                 but your current session hasn't picked it up yet. \
                 Please LOG OUT and LOG BACK IN to activate group permissions.",
                permission_denied_count, total_event_count
            )
            .into())
        } else {
            Err(format!(
                "Permission denied on {}/{} devices. Add your user to the 'input' group: \
                 sudo usermod -aG input $USER, \
                 then log out and log back in.",
                permission_denied_count, total_event_count
            )
            .into())
        }
    } else if dir.exists() {
        log_info("[dboard] No keyboard device found at startup — waiting for hotplug");
        Ok(Vec::new())
    } else {
        Err("No keyboard device found".into())
    }
}

fn scan_keyboard_dir(
    dir_path: &str,
    match_name: fn(&str) -> bool,
    existing_paths: &std::collections::HashSet<String>,
    existing_phys: &mut std::collections::HashSet<String>,
    added_paths: &mut std::collections::HashSet<String>,
    out: &mut Vec<(evdev::Device, String)>,
) {
    let dir = std::path::Path::new(dir_path);
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !match_name(&name) {
            continue;
        }
        let Ok(real_path) = fs::canonicalize(entry.path()) else {
            continue;
        };
        let p = real_path.to_string_lossy().to_string();
        if existing_paths.contains(&p) || !added_paths.insert(p) {
            continue;
        }
        let phys_opt = get_device_phys_id(&real_path);
        if phys_opt
            .as_ref()
            .is_some_and(|id| existing_phys.contains(id))
        {
            continue;
        }
        if let Ok(device) = evdev::Device::open(&real_path) {
            let dev_name = device.name().unwrap_or("unknown").to_string();
            if is_valid_keyboard(&device) {
                if let Some(pid) = phys_opt {
                    existing_phys.insert(pid);
                }
                out.push((device, format!("{} ({})", real_path.display(), dev_name)));
            }
        }
    }
}

/// Discover keyboard devices present in `/dev/input` that are not already known.
/// Used for hotplug so newly connected keyboards (USB/wireless/virtual) are attached dynamically.
pub fn discover_new_keyboards(
    existing_paths: &std::collections::HashSet<String>,
    existing_phys: &mut std::collections::HashSet<String>,
) -> Vec<(evdev::Device, String)> {
    let mut out = Vec::new();
    let mut added_paths = std::collections::HashSet::new();
    scan_keyboard_dir(
        "/dev/input/by-path",
        |n| n.ends_with("-event-kbd"),
        existing_paths,
        existing_phys,
        &mut added_paths,
        &mut out,
    );
    scan_keyboard_dir(
        "/dev/input/by-id",
        |n| n.ends_with("-event-kbd"),
        existing_paths,
        existing_phys,
        &mut added_paths,
        &mut out,
    );
    scan_keyboard_dir(
        "/dev/input",
        |n| n.starts_with("event"),
        existing_paths,
        existing_phys,
        &mut added_paths,
        &mut out,
    );
    out
}

/// Returns true if the physical device currently reports any key held down.
pub fn has_held_keys(dev: &evdev::Device) -> bool {
    dev.get_key_state()
        .is_ok_and(|keys| keys.iter().next().is_some())
}

/// Bounded startup wait so physically held keys (e.g. Enter on `systemctl restart`)
/// release to the compositor before `EVIOCGRAB` takes ownership of the device.
pub fn wait_until_keys_released(dev: &evdev::Device, max_wait_ms: u32) {
    let steps = max_wait_ms / 10;
    for _ in 0..steps {
        if !has_held_keys(dev) {
            return;
        }
        unsafe {
            libc::poll(std::ptr::null_mut(), 0, 10);
        }
    }
}
