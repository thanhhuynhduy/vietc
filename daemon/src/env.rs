use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use crate::log::log_info;

/// Discover an active Wayland compositor socket (`wayland-0`..`wayland-9` or `wayland-*`)
/// inside `runtime_dir` (ignoring `.lock` files and non-sockets).
pub fn find_wayland_socket_in_dir(runtime_dir: &Path) -> Option<String> {
    for idx in 0..10 {
        let name = format!("wayland-{idx}");
        if fs::symlink_metadata(runtime_dir.join(&name))
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false)
        {
            return Some(name);
        }
    }
    let mut candidates: Vec<String> = fs::read_dir(runtime_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_socket() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            (name.starts_with("wayland-") && !name.ends_with(".lock")).then_some(name)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Recover the user's display environment (`DISPLAY` / `XAUTHORITY` /
/// `WAYLAND_DISPLAY` / `XDG_RUNTIME_DIR` / `XDG_CURRENT_DESKTOP` /
/// `XDG_SESSION_TYPE` / `HYPRLAND_INSTANCE_SIGNATURE` / `SWAYSOCK`) into the daemon's own environment.
pub fn recover_display_env() {
    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
        "HYPRLAND_INSTANCE_SIGNATURE",
        "SWAYSOCK",
    ] {
        if std::env::var_os(key).is_some_and(|v| v.is_empty()) {
            std::env::remove_var(key);
        }
    }

    let target_uid: u32 = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });

    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        let candidate = format!("/run/user/{target_uid}");
        if Path::new(&candidate).is_dir() {
            std::env::set_var("XDG_RUNTIME_DIR", &candidate);
        }
    }

    let mut found_display: Option<String> = std::env::var("DISPLAY").ok().filter(|v| !v.is_empty());
    let mut found_xauth: Option<String> =
        std::env::var("XAUTHORITY").ok().filter(|v| !v.is_empty());
    let mut found_wayland: Option<String> = std::env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|v| !v.is_empty());
    let mut found_xdg: Option<String> = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|v| !v.is_empty());
    let mut found_desktop: Option<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|v| !v.is_empty());
    let mut found_session_type: Option<String> = std::env::var("XDG_SESSION_TYPE")
        .ok()
        .filter(|v| !v.is_empty());
    let mut found_hyprland: Option<String> = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|v| !v.is_empty());
    let mut found_swaysock: Option<String> =
        std::env::var("SWAYSOCK").ok().filter(|v| !v.is_empty());

    if (found_display.is_none() || found_wayland.is_none() || found_desktop.is_none())
        && Path::new("/proc").is_dir()
    {
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name_s = entry.file_name().to_string_lossy().into_owned();
                if !name_s.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                #[cfg(target_os = "linux")]
                {
                    use std::os::linux::fs::MetadataExt;
                    if let Ok(meta) = entry.metadata() {
                        if meta.st_uid() != target_uid {
                            continue;
                        }
                    }
                }
                let Ok(content) = fs::read(entry.path().join("environ")) else {
                    continue;
                };
                for chunk in content.split(|&b| b == 0) {
                    let Ok(s) = std::str::from_utf8(chunk) else {
                        continue;
                    };
                    if found_display.is_none() {
                        if let Some(v) = s.strip_prefix("DISPLAY=") {
                            if !v.is_empty() {
                                found_display = Some(v.to_string());
                            }
                        }
                    }
                    if found_xauth.is_none() {
                        if let Some(v) = s.strip_prefix("XAUTHORITY=") {
                            if !v.is_empty() {
                                found_xauth = Some(v.to_string());
                            }
                        }
                    }
                    if found_wayland.is_none() {
                        if let Some(v) = s.strip_prefix("WAYLAND_DISPLAY=") {
                            if !v.is_empty() {
                                found_wayland = Some(v.to_string());
                            }
                        }
                    }
                    if found_xdg.is_none() {
                        if let Some(v) = s.strip_prefix("XDG_RUNTIME_DIR=") {
                            if !v.is_empty() {
                                found_xdg = Some(v.to_string());
                            }
                        }
                    }
                    if found_desktop.is_none() {
                        if let Some(v) = s.strip_prefix("XDG_CURRENT_DESKTOP=") {
                            if !v.is_empty() {
                                found_desktop = Some(v.to_string());
                            }
                        }
                    }
                    if found_session_type.is_none() {
                        if let Some(v) = s.strip_prefix("XDG_SESSION_TYPE=") {
                            if !v.is_empty() {
                                found_session_type = Some(v.to_string());
                            }
                        }
                    }
                    if found_hyprland.is_none() {
                        if let Some(v) = s.strip_prefix("HYPRLAND_INSTANCE_SIGNATURE=") {
                            if !v.is_empty() {
                                found_hyprland = Some(v.to_string());
                            }
                        }
                    }
                    if found_swaysock.is_none() {
                        if let Some(v) = s.strip_prefix("SWAYSOCK=") {
                            if !v.is_empty() {
                                found_swaysock = Some(v.to_string());
                            }
                        }
                    }
                }
                if found_display.is_some()
                    && found_wayland.is_some()
                    && found_xdg.is_some()
                    && found_desktop.is_some()
                {
                    break;
                }
            }
        }
    }

    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        if let Some(ref r) = found_xdg {
            std::env::set_var("XDG_RUNTIME_DIR", r);
            log_info(&format!(
                "[dboard] Recovered XDG_RUNTIME_DIR={} from /proc",
                r
            ));
        }
    }
    if std::env::var_os("DISPLAY").is_none() {
        if let Some(ref d) = found_display {
            std::env::set_var("DISPLAY", d);
            log_info(&format!("[dboard] Recovered DISPLAY={} from /proc", d));
        }
    }
    if std::env::var_os("XAUTHORITY").is_none() {
        if let Some(ref x) = found_xauth {
            std::env::set_var("XAUTHORITY", x);
        }
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Some(ref w) = found_wayland {
            std::env::set_var("WAYLAND_DISPLAY", w);
            log_info(&format!(
                "[dboard] Recovered WAYLAND_DISPLAY={} from /proc",
                w
            ));
        }
    }
    if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
        if let Some(ref d) = found_desktop {
            std::env::set_var("XDG_CURRENT_DESKTOP", d);
        }
    }
    if std::env::var_os("XDG_SESSION_TYPE").is_none() {
        if let Some(ref s) = found_session_type {
            std::env::set_var("XDG_SESSION_TYPE", s);
        }
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        if let Some(ref h) = found_hyprland {
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", h);
        }
    }
    if std::env::var_os("SWAYSOCK").is_none() {
        if let Some(ref s) = found_swaysock {
            std::env::set_var("SWAYSOCK", s);
        }
    }

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Ok(xdg_dir) = std::env::var("XDG_RUNTIME_DIR") {
            if let Some(sock_name) = find_wayland_socket_in_dir(Path::new(&xdg_dir)) {
                std::env::set_var("WAYLAND_DISPLAY", &sock_name);
                log_info(&format!(
                    "[dboard] Recovered WAYLAND_DISPLAY={} from XDG_RUNTIME_DIR",
                    sock_name
                ));
            }
        }
    }

    recover_dbus_env();
}

pub fn recover_dbus_env() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty()) {
        return;
    }
    let target_uid: u32 = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });

    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name_s = entry.file_name().to_string_lossy().into_owned();
            if !name_s.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            #[cfg(target_os = "linux")]
            {
                use std::os::linux::fs::MetadataExt;
                if let Ok(meta) = entry.metadata() {
                    if meta.st_uid() != target_uid {
                        continue;
                    }
                }
            }
            let environ_path = entry.path().join("environ");
            if let Ok(content) = fs::read(&environ_path) {
                if let Ok(dbus_env) = std::str::from_utf8(&content) {
                    for var in dbus_env.split('\0') {
                        if let Some(val) = var.strip_prefix("DBUS_SESSION_BUS_ADDRESS=") {
                            if !val.is_empty() {
                                std::env::set_var("DBUS_SESSION_BUS_ADDRESS", val);
                                log_info("[dboard] Recovered DBUS_SESSION_BUS_ADDRESS");
                                return;
                            }
                        }
                        if let Some(val) = var.strip_prefix("XDG_RUNTIME_DIR=") {
                            if !val.is_empty() && std::env::var_os("XDG_RUNTIME_DIR").is_none() {
                                std::env::set_var("XDG_RUNTIME_DIR", val);
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(xdg_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let bus_path = Path::new(&xdg_dir).join("bus");
        if bus_path.exists() {
            let addr = format!("unix:path={}", bus_path.display());
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &addr);
            log_info("[dboard] Set DBUS_SESSION_BUS_ADDRESS from XDG_RUNTIME_DIR/bus");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::find_wayland_socket_in_dir;
    use std::os::unix::net::UnixListener;

    #[test]
    fn discovers_wayland_1_socket_and_ignores_lock_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("wayland-0"), b"not-a-socket").unwrap();
        std::fs::write(dir.path().join("wayland-1.lock"), b"").unwrap();
        let _listener = UnixListener::bind(dir.path().join("wayland-1")).unwrap();

        assert_eq!(
            find_wayland_socket_in_dir(dir.path()).as_deref(),
            Some("wayland-1")
        );
    }
}
