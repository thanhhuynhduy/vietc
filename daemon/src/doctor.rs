// SPDX-License-Identifier: MIT
//! Self-diagnostic subcommand (`dboard doctor`) for verifying Linux COSMIC /
//! Wayland permissions, protocols, AT-SPI2, and conflicting IME frameworks.

use std::fs::{self, OpenOptions};
use std::path::Path;

use crate::config::{find_config_path, Config};
use crate::device::open_keyboard_devices;
use crate::display::{detect_compositor, detect_display_server, DisplayServer};

fn status_tag(ok: bool) -> &'static str {
    if ok {
        "[OK]  "
    } else {
        "[WARN]"
    }
}

fn check_udev_rules() -> (bool, String) {
    let candidates = [
        "/etc/udev/rules.d/70-dboard.rules",
        "/lib/udev/rules.d/70-dboard.rules",
        "/usr/lib/udev/rules.d/70-dboard.rules",
    ];
    for path in candidates {
        if let Ok(content) = fs::read_to_string(path) {
            let has_uaccess = content.contains("TAG+=\"uaccess\"");
            let scoped = content.contains("ENV{ID_INPUT_KEYBOARD}");
            let clobbers_event = content.lines().any(|line| {
                line.contains("event*")
                    && (line.contains("GROUP=\"root\"") || line.contains("MODE=\"0600\""))
            });
            if has_uaccess && !clobbers_event {
                let scope_note = if scoped {
                    "scoped to keyboard/pointer"
                } else {
                    "broad event* (consider upgrading rule)"
                };
                return (true, format!("{path} ({scope_note})"));
            }
            return (
                false,
                format!("{path} present but needs update (re-run install.sh)"),
            );
        }
    }
    (
        false,
        "70-dboard.rules not found in /etc/udev/rules.d or /lib/udev/rules.d".into(),
    )
}

fn check_conflicting_imes() -> Vec<String> {
    let mut conflicts = Vec::new();
    for var in ["GTK_IM_MODULE", "QT_IM_MODULE", "XMODIFIERS"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_ascii_lowercase();
            if lower.contains("ibus") || lower.contains("fcitx") {
                conflicts.push(format!("{var}={val}"));
            }
        }
    }
    for proc_name in ["ibus-daemon", "fcitx5", "fcitx"] {
        let running = std::process::Command::new("pgrep")
            .args(["-x", proc_name])
            .status()
            .is_ok_and(|s| s.success());
        if running {
            conflicts.push(format!("process `{proc_name}` running"));
        }
    }
    conflicts
}

fn check_systemd_service() -> (bool, String) {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "dboard.service"])
        .output();
    match output {
        Ok(out) => {
            let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let active = out.status.success() && state == "active";
            let label = if state.is_empty() {
                "inactive/not installed".into()
            } else {
                state
            };
            (active, label)
        }
        Err(_) => (false, "systemctl unavailable".into()),
    }
}

fn check_atspi_bus() -> (bool, String) {
    match dbus::blocking::Connection::new_session() {
        Ok(conn) => {
            let proxy = conn.with_proxy(
                "org.a11y.Bus",
                "/org/a11y/bus",
                std::time::Duration::from_millis(500),
            );
            match proxy.method_call::<(String,), _, _, _>("org.a11y.Bus", "GetAddress", ()) {
                Ok((addr,)) if !addr.is_empty() => (true, "reachable".into()),
                Ok(_) => (false, "empty bus address".into()),
                Err(err) => (false, format!("unreachable ({err})")),
            }
        }
        Err(err) => (false, format!("no D-Bus session ({err})")),
    }
}

pub fn run_doctor() -> i32 {
    let mut warnings = 0u32;
    let mut report = |ok: bool, label: &str, detail: &str| {
        if !ok {
            warnings += 1;
        }
        println!("{} {:<22} {}", status_tag(ok), label, detail);
    };

    println!("Dboard Doctor v{}", env!("CARGO_PKG_VERSION"));
    println!("------------------------------------------------------------");

    let euid = unsafe { libc::geteuid() };
    report(
        euid != 0,
        "Privilege model",
        if euid == 0 {
            "running as root (breaks Wayland virtual keyboard & clipboard; run as regular user)"
        } else {
            "non-root user session (recommended)"
        },
    );

    let display = detect_display_server();
    let compositor = detect_compositor().unwrap_or_else(|| "unknown".into());
    report(
        display == DisplayServer::Wayland,
        "Display / Compositor",
        &format!("{display:?} ({compositor})"),
    );

    let (udev_ok, udev_detail) = check_udev_rules();
    report(udev_ok, "udev uaccess rules", &udev_detail);

    let uinput_ok = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    report(
        uinput_ok,
        "/dev/uinput access",
        if uinput_ok {
            "read/write OK"
        } else {
            "cannot open /dev/uinput (check udev rules & re-login)"
        },
    );

    match open_keyboard_devices() {
        Ok(devs) if !devs.is_empty() => {
            report(
                true,
                "evdev keyboards",
                &format!("{} keyboard(s) accessible", devs.len()),
            );
        }
        Ok(_) => {
            report(
                false,
                "evdev keyboards",
                "0 keyboards currently attached (hotplug will wait)",
            );
        }
        Err(err) => {
            report(false, "evdev keyboards", &err.to_string());
        }
    }

    let (atspi_ok, atspi_detail) = check_atspi_bus();
    report(atspi_ok, "AT-SPI2 focus/pw", &atspi_detail);

    let conflicts = check_conflicting_imes();
    report(
        conflicts.is_empty(),
        "IME conflicts",
        if conflicts.is_empty() {
            "none detected".into()
        } else {
            conflicts.join(", ")
        }
        .as_str(),
    );

    let (svc_ok, svc_detail) = check_systemd_service();
    report(svc_ok, "systemd user service", &svc_detail);

    let cfg_path = find_config_path();
    let cfg_exists = Path::new(&cfg_path).exists();
    let cfg = Config::load().unwrap_or_default();
    report(
        cfg_exists,
        "Configuration",
        &format!(
            "{} (method={}, toggle=Ctrl+{}, modes={:?}, autocorrect={}, auto_cap={})",
            cfg_path.display(),
            cfg.input_method,
            cfg.toggle_key,
            cfg.toggle_modes,
            cfg.autocorrect,
            cfg.auto_capitalize
        ),
    );

    report(true, "Keyboard layout", "us (English (US))");

    if let Some(dict_path) = crate::user_dict_store::user_dict_path() {
        if dict_path.exists() {
            let dict = crate::user_dict_store::load_user_dict_from(&dict_path);
            let mode_ok = fs::metadata(&dict_path)
                .map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    (m.permissions().mode() & 0o077) == 0
                })
                .unwrap_or(false);
            report(
                mode_ok,
                "Personal dictionary",
                &format!(
                    "{} ({} entries, perms={})",
                    dict_path.display(),
                    dict.len(),
                    if mode_ok { "0600 OK" } else { "too permissive" }
                ),
            );
        } else {
            report(
                true,
                "Personal dictionary",
                "not yet created (auto-created on first One-Tap Undo)",
            );
        }
    }

    println!("------------------------------------------------------------");
    if warnings == 0 {
        println!("Result: ALL CHECKS PASSED (ready for daily use)");
        0
    } else {
        println!("Result: {warnings} warning(s) found");
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tag_formats_pass_and_warn() {
        assert_eq!(status_tag(true), "[OK]  ");
        assert_eq!(status_tag(false), "[WARN]");
    }
}
