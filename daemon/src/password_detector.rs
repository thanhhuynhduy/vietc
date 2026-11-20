// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const ROLE_PASSWORD_TEXT: i32 = 62;

static ATSPI_OBSERVED_FOCUS: AtomicBool = AtomicBool::new(false);
static ATSPI_FOCUSED_IS_PASSWORD: AtomicBool = AtomicBool::new(false);
static BG_OBSERVED_WINDOW: AtomicBool = AtomicBool::new(false);
static BG_FOCUSED_IS_SUDO: AtomicBool = AtomicBool::new(false);

fn atspi_app_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn bg_title_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Update cached window title and terminal sudo-descendant status from the
/// 250ms background polling thread so the real-time `evdev_loop` thread never
/// blocks on external CLI processes (`gdbus`, `xdotool`, `xprop`, `hyprctl`, `swaymsg`).
pub fn record_background_window_security(title: Option<String>, is_sudo: bool) {
    if let Ok(mut guard) = bg_title_cache().lock() {
        *guard = title;
    }
    BG_FOCUSED_IS_SUDO.store(is_sudo, Ordering::Release);
    BG_OBSERVED_WINDOW.store(true, Ordering::Release);
}

pub fn cached_window_title() -> Option<(bool, Option<String>)> {
    if !BG_OBSERVED_WINDOW.load(Ordering::Acquire) {
        return None;
    }
    let title = bg_title_cache().lock().ok().and_then(|g| g.clone());
    Some((true, title))
}

pub fn cached_window_is_sudo() -> Option<bool> {
    if !BG_OBSERVED_WINDOW.load(Ordering::Acquire) {
        return None;
    }
    Some(BG_FOCUSED_IS_SUDO.load(Ordering::Acquire))
}

/// Record the focused accessible node's role and owning application name from
/// the background AT-SPI2 event watcher (`O(1)` direct query on `(sender, path)`
/// with zero tree traversal and zero typing-thread latency, working at any DOM depth).
pub fn record_atspi_focus_state(is_password: bool, app_name: Option<String>) {
    ATSPI_FOCUSED_IS_PASSWORD.store(is_password, Ordering::Release);
    ATSPI_OBSERVED_FOCUS.store(true, Ordering::Release);
    if let Some(name) =
        app_name.filter(|n| !n.is_empty() && n != "gnome-shell" && n != "gnome-shell.desktop")
    {
        if let Ok(mut guard) = atspi_app_cache().lock() {
            *guard = Some(name);
        }
    }
}

/// Clear cached AT-SPI2 focus state if the accessibility bus disconnects.
pub fn reset_atspi_focus_state() {
    ATSPI_OBSERVED_FOCUS.store(false, Ordering::Release);
    ATSPI_FOCUSED_IS_PASSWORD.store(false, Ordering::Release);
    if let Ok(mut guard) = atspi_app_cache().lock() {
        *guard = None;
    }
}

pub struct PasswordDetector {
    cached: Option<bool>,
}

impl PasswordDetector {
    pub fn new() -> Self {
        Self { cached: None }
    }

    /// Non-blocking `O(1)` check of the event-driven AT-SPI2 password state.
    pub fn check(&mut self) -> Option<bool> {
        if !ATSPI_OBSERVED_FOCUS.load(Ordering::Acquire) {
            return None;
        }
        let is_pw = ATSPI_FOCUSED_IS_PASSWORD.load(Ordering::Acquire);
        self.cached = Some(is_pw);
        Some(is_pw)
    }
}

pub fn get_focused_window_class_atspi() -> Option<String> {
    atspi_app_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_driven_password_detector_tracks_focus_transitions_without_blocking() {
        reset_atspi_focus_state();
        let mut detector = PasswordDetector::new();
        assert_eq!(detector.check(), None);

        record_atspi_focus_state(true, Some("microsoft-edge".to_string()));
        assert_eq!(detector.check(), Some(true));
        assert_eq!(
            get_focused_window_class_atspi().as_deref(),
            Some("microsoft-edge")
        );

        record_atspi_focus_state(false, Some("gnome-shell".to_string()));
        assert_eq!(detector.check(), Some(false));
        // gnome-shell must not overwrite the last real focused application name
        assert_eq!(
            get_focused_window_class_atspi().as_deref(),
            Some("microsoft-edge")
        );

        reset_atspi_focus_state();
        assert_eq!(detector.check(), None);
    }
}
