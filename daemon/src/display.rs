// SPDX-License-Identifier: MIT
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

/// Detect whether we're running on Wayland or X11
pub fn detect_display_server() -> DisplayServer {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        return DisplayServer::Wayland;
    }

    if let Ok(session_type) = std::env::var("XDG_SESSION_TYPE") {
        if session_type.contains("wayland") {
            return DisplayServer::Wayland;
        }
    }

    if let Ok(xdg_runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if crate::env::find_wayland_socket_in_dir(Path::new(&xdg_runtime)).is_some() {
            return DisplayServer::Wayland;
        }
    }

    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        return DisplayServer::X11;
    }

    if let Ok(output) = Command::new("loginctl")
        .args(["show-session", &get_session_id(), "-p", "Type"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("wayland") {
            return DisplayServer::Wayland;
        }
        if stdout.contains("x11") {
            return DisplayServer::X11;
        }
    }

    DisplayServer::Unknown
}

fn get_session_id() -> String {
    std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "self".into())
}

/// Check if a specific compositor is running
pub fn detect_compositor() -> Option<String> {
    let compositor_vars = [
        ("HYPRLAND_INSTANCE_SIGNATURE", "Hyprland"),
        ("SWAYSOCK", "Sway"),
        ("I3SOCK", "i3"),
        ("KWIN_SESSION", "KWin"),
    ];

    for (var, name) in &compositor_vars {
        if std::env::var_os(var).is_some_and(|v| !v.is_empty()) {
            return Some((*name).to_string());
        }
    }

    for (proc_name, label) in [
        ("cosmic-comp", "COSMIC"),
        ("hyprland", "Hyprland"),
        ("sway", "Sway"),
        ("gnome-shell", "GNOME (Mutter)"),
    ] {
        if let Ok(output) = Command::new("pgrep").arg("-x").arg(proc_name).output() {
            if output.status.success() {
                return Some(label.into());
            }
        }
    }

    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
        let lower = desktop.to_lowercase();
        if lower.contains("cosmic") {
            return Some("COSMIC".into());
        }
        if lower.contains("gnome") {
            return Some("GNOME".into());
        }
    }

    None
}
