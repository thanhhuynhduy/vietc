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
    let unsupported_wayland_de = is_unsupported_wayland_compositor(&compositor);
    report(
        display == DisplayServer::Wayland && !unsupported_wayland_de,
        "Display / Compositor",
        &if unsupported_wayland_de {
            format!(
                "{display:?} ({compositor} — unsupported: lacks zwp_virtual_keyboard_v1; requires COSMIC/Hyprland/Sway/wlroots)"
            )
        } else {
            format!("{display:?} ({compositor})")
        },
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
    let cfg = match Config::load() {
        Ok(c) => {
            report(
                cfg_exists,
                "Configuration",
                &format!(
                    "{} (method={}, toggle={}, modes={:?}, autocorrect={}, auto_cap={})",
                    cfg_path.display(),
                    c.input_method,
                    crate::config::format_toggle_key(&c.toggle_key),
                    c.toggle_modes,
                    c.autocorrect,
                    c.auto_capitalize
                ),
            );
            c
        }
        Err(err) => {
            report(
                false,
                "Configuration",
                &format!("{} (TOML parse error: {err})", cfg_path.display()),
            );
            Config::default()
        }
    };

    let mod_conflicts = detect_mod3_mod5_conflicts();
    report(
        mod_conflicts.is_empty(),
        "XKB Mod3/Mod5 keybinds",
        if mod_conflicts.is_empty() {
            "no compositor Mod3/Mod5 conflicts detected".into()
        } else {
            format!(
                "potential Mod3/Mod5 conflict in: {} (may interfere with VK Vietnamese levels)",
                mod_conflicts.join(", ")
            )
        }
        .as_str(),
    );

    let layout_known = is_known_keyboard_layout(&cfg.keyboard_layout);
    let active_layout =
        dboard_protocol::layout::KeyboardLayout::from_str_name(&cfg.keyboard_layout);
    report(
        layout_known,
        "Keyboard layout",
        &if layout_known {
            active_layout.display_name().to_string()
        } else {
            format!(
                "unknown layout '{}' (falling back to {}); expected us | dvorak | colemak",
                cfg.keyboard_layout,
                active_layout.display_name()
            )
        },
    );
    let (cb_ok, cb_detail) = if cfg.candidate_bar {
        let has_runtime = std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|v| !v.is_empty());
        let popup_found = ["/usr/bin/dboard-popup", "/usr/local/bin/dboard-popup"]
            .iter()
            .any(|p| Path::new(p).exists())
            || std::process::Command::new("which")
                .arg("dboard-popup")
                .output()
                .is_ok_and(|o| o.status.success());
        if has_runtime && popup_found {
            (
                true,
                "enabled (dboard-popup found, non-blocking Unix datagram IPC)",
            )
        } else if !has_runtime {
            (
                false,
                "enabled, but XDG_RUNTIME_DIR is unset (popup socket unavailable)",
            )
        } else {
            (
                false,
                "enabled, but dboard-popup binary not found in PATH or /usr/bin",
            )
        }
    } else {
        (true, "disabled (zero-underline default)")
    };
    report(cb_ok, "Candidate bar", cb_detail);

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

fn detect_mod3_mod5_conflicts() -> Vec<String> {
    let mut hits = Vec::new();
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return hits;
    };
    let candidates = [
        ("Hyprland", home.join(".config/hypr/hyprland.conf")),
        ("Sway", home.join(".config/sway/config")),
        (
            "COSMIC",
            home.join(".config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"),
        ),
    ];
    for (label, path) in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let has_conflict = content.lines().any(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.starts_with("//") {
                return false;
            }
            let lower = trimmed.to_ascii_lowercase();
            lower.contains("mod3") || lower.contains("mod5") || lower.contains("iso_level3_shift")
        });
        if has_conflict {
            hits.push(format!("{label} ({})", path.display()));
        }
    }
    hits
}

fn is_unsupported_wayland_compositor(compositor: &str) -> bool {
    let lower = compositor.to_ascii_lowercase();
    lower.contains("gnome")
        || lower.contains("mutter")
        || lower.contains("kde")
        || lower.contains("plasma")
        || lower.contains("kwin")
}

fn is_known_keyboard_layout(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "" | "us" | "qwerty" | "us-qwerty" | "dvorak" | "us-dvorak" | "colemak" | "us-colemak"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tag_formats_pass_and_warn() {
        assert_eq!(status_tag(true), "[OK]  ");
        assert_eq!(status_tag(false), "[WARN]");
    }

    #[test]
    fn doctor_validates_supported_compositors_and_keyboard_layouts() {
        assert!(is_unsupported_wayland_compositor("gnome"));
        assert!(is_unsupported_wayland_compositor("KDE Plasma"));
        assert!(!is_unsupported_wayland_compositor("cosmic"));
        assert!(!is_unsupported_wayland_compositor("hyprland"));
        assert!(!is_unsupported_wayland_compositor("sway"));

        assert!(is_known_keyboard_layout("us"));
        assert!(is_known_keyboard_layout("dvorak"));
        assert!(is_known_keyboard_layout("colemak"));
        assert!(!is_known_keyboard_layout("dvroak"));
    }
}
