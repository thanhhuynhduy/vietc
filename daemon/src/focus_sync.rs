// SPDX-License-Identifier: MIT
//! Production focus, per-app state, and password-field synchronization on key press.
//!
//! Extracted from `evdev_loop.rs` so both the real evdev event loop and
//! multi-environment stress tests execute the exact same state transitions.

use std::time::Instant;

use dboard_protocol::KeyInjector;

use crate::daemon::Daemon;
use crate::focus::{FocusSnapshot, SharedFocusSnapshot, TargetIdentity};
use crate::log::log_info;

pub struct FocusSyncState {
    pub last_focus_generation: u64,
    pub last_focus_target: Option<TargetIdentity>,
    pub last_window_class: String,
    last_injector_window: String,
    last_injector_terminal: Option<bool>,
    last_injector_bypass: Option<bool>,
    last_key_time: Instant,
}

impl FocusSyncState {
    pub fn from_initial(initial: FocusSnapshot) -> Self {
        Self {
            last_focus_generation: initial.generation,
            last_focus_target: initial.target,
            last_window_class: String::new(),
            last_injector_window: String::new(),
            last_injector_terminal: None,
            last_injector_bypass: None,
            last_key_time: Instant::now(),
        }
    }

    fn sync_injector_if_changed(
        &mut self,
        injector: &dyn KeyInjector,
        window_class: Option<&str>,
        is_terminal: bool,
        is_bypass: bool,
    ) {
        if let Some(class) = window_class {
            if self.last_injector_window != class {
                injector.set_active_window(class);
                self.last_injector_window.clear();
                self.last_injector_window.push_str(class);
            }
        }
        if self.last_injector_terminal != Some(is_terminal) {
            injector.set_terminal_app(is_terminal);
            self.last_injector_terminal = Some(is_terminal);
        }
        if self.last_injector_bypass != Some(is_bypass) {
            injector.set_bypass_mode(is_bypass);
            self.last_injector_bypass = Some(is_bypass);
        }
    }

    /// Observe focus generation changes, per-app class changes, and password
    /// field transitions on `value == 1` (KeyDown) BEFORE engine/Backspace logic.
    ///
    /// Invariants:
    /// - Locks `shared_focus` at most once per keystroke.
    /// - Only updates `injector.set_active_window` / `set_terminal_app` /
    ///   `set_bypass_mode` when the values actually change.
    /// - On `FocusLost` (Polkit/launcher overlay), clears `daemon.app_state.current_app`
    ///   so bypass/terminal/english overrides from the previous window are never inherited.
    pub fn sync_on_key_press(
        &mut self,
        daemon: &mut Daemon,
        injector: &dyn KeyInjector,
        shared_focus: &SharedFocusSnapshot,
    ) {
        self.sync_inner(daemon, injector, shared_focus, true);
    }

    /// Synchronizes focus, `FocusLost` (Polkit/launcher), password protection, and
    /// injector state during idle `poll` ticks so `last_focus_generation` stays fresh
    /// for mid-sentence caret re-selection (`C5`) and idle overlays hide the popup.
    pub fn sync_on_idle(
        &mut self,
        daemon: &mut Daemon,
        injector: &dyn KeyInjector,
        shared_focus: &SharedFocusSnapshot,
    ) {
        self.sync_inner(daemon, injector, shared_focus, false);
    }

    fn sync_inner(
        &mut self,
        daemon: &mut Daemon,
        injector: &dyn KeyInjector,
        shared_focus: &SharedFocusSnapshot,
        update_key_time: bool,
    ) {
        let gap = self.last_key_time.elapsed();
        if update_key_time {
            self.last_key_time = Instant::now();
        }

        let focus = shared_focus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let active_window_class = focus.class.unwrap_or_default();
        let focus_changed = focus.generation != self.last_focus_generation;
        let class_changed =
            !active_window_class.is_empty() && active_window_class != self.last_window_class;

        if focus_changed {
            log_info(&format!(
                "[dboard] Focus generation changed: {} -> {} identity={:?} (gap={:?})",
                self.last_focus_generation, focus.generation, focus.identity, gap
            ));
            let previous_generation = self.last_focus_generation;
            let new_target = focus.target.clone();
            self.last_focus_generation = focus.generation;
            injector.cancel_pending_replacement(self.last_focus_generation, "target-changed");
            log_info(&format!(
                "[dboard] composition reset: focus_generation={} old_target_generation={} old_target={:?} new_target={:?} reason=target-changed",
                self.last_focus_generation,
                previous_generation,
                self.last_focus_target,
                new_target,
            ));
            self.last_focus_target = new_target;
            if class_changed {
                crate::password_detector::clear_atspi_password_flag_if_non_atspi(
                    focus.identity.as_deref(),
                );
                self.last_window_class = active_window_class.clone();
            }
            daemon.reset_composition_state();

            if daemon.config.app_state.enabled && !active_window_class.is_empty() {
                daemon.check_app_change_with(active_window_class.clone());
                self.sync_injector_if_changed(
                    injector,
                    Some(&active_window_class),
                    daemon.app_state.is_terminal_app(),
                    daemon.is_current_app_bypassed(),
                );
            } else if active_window_class.is_empty() {
                // FocusLost (layer-shell launcher / Polkit): never inherit previous app's
                // bypass, terminal, method, or english overrides. Clear stale AT-SPI2
                // password/app flags outside the 150ms grace window so freshly focused
                // Polkit/pinentry/cosmic-osd prompts remain protected while launchers don't
                // inherit stale password state.
                crate::password_detector::clear_atspi_password_flag_if_non_atspi(
                    focus.identity.as_deref(),
                );
                self.last_window_class.clear();
                daemon.reset_app_focus_to_global();
                self.sync_injector_if_changed(injector, Some(""), false, false);
            }
        } else if daemon.config.app_state.enabled && !active_window_class.is_empty() {
            if class_changed {
                crate::password_detector::clear_atspi_password_flag_if_non_atspi(
                    focus.identity.as_deref(),
                );
                self.last_window_class = active_window_class.clone();
                daemon.check_app_change_with(active_window_class.clone());
            }
            self.sync_injector_if_changed(
                injector,
                Some(&active_window_class),
                daemon.app_state.is_terminal_app(),
                daemon.is_current_app_bypassed(),
            );
        }

        if daemon.config.password_detection.enabled {
            let is_pw = daemon.app_state.check_password_field();
            let currently_enabled = daemon.engine.is_enabled();
            if is_pw && currently_enabled {
                daemon.engine.set_enabled(false);
                daemon.reset_composition_state();
                daemon.write_status();
            } else if !is_pw && !currently_enabled && daemon.app_state.get_default_state() {
                daemon.engine.set_enabled(true);
                daemon.reset_composition_state();
                daemon.write_status();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct CountingInjector {
        set_window_calls: AtomicUsize,
        bypass: AtomicBool,
    }

    impl CountingInjector {
        fn new() -> Self {
            Self {
                set_window_calls: AtomicUsize::new(0),
                bypass: AtomicBool::new(false),
            }
        }
    }

    impl KeyInjector for CountingInjector {
        fn send_char(&self, _ch: char) -> dboard_protocol::inject::InjectResult {
            dboard_protocol::inject::InjectResult::Success
        }
        fn send_backspace(&self) -> dboard_protocol::inject::InjectResult {
            dboard_protocol::inject::InjectResult::Success
        }
        fn send_string(&self, _text: &str) -> dboard_protocol::inject::InjectResult {
            dboard_protocol::inject::InjectResult::Success
        }
        fn flush(&self) -> dboard_protocol::inject::InjectResult {
            dboard_protocol::inject::InjectResult::Success
        }
        fn set_active_window(&self, _class: &str) -> dboard_protocol::inject::InjectResult {
            self.set_window_calls.fetch_add(1, Ordering::SeqCst);
            dboard_protocol::inject::InjectResult::Success
        }
        fn set_bypass_mode(&self, bypass: bool) -> dboard_protocol::inject::InjectResult {
            self.bypass.store(bypass, Ordering::SeqCst);
            dboard_protocol::inject::InjectResult::Success
        }
    }

    #[test]
    fn focus_lost_clears_bypass_app_and_same_window_avoids_redundant_injector_calls() {
        let mut cfg = crate::config::Config::default();
        cfg.app_state.enabled = true;
        cfg.app_state.bypass_apps = vec!["steam".into()];
        let enabled = Arc::new(AtomicBool::new(true));
        let mut daemon = Daemon::new(cfg, std::path::PathBuf::from("test.toml"), enabled);
        let shared = Arc::new(Mutex::new(FocusSnapshot::default()));
        let mut sync = FocusSyncState::from_initial(FocusSnapshot::default());
        let inj = CountingInjector::new();

        // Focus steam (bypassed app)
        shared
            .lock()
            .unwrap()
            .observe(Some("toplevel:1".into()), Some("steam".into()));
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert!(daemon.is_current_app_bypassed());
        assert!(inj.bypass.load(Ordering::SeqCst));
        assert_eq!(inj.set_window_calls.load(Ordering::SeqCst), 1);

        // Subsequent keystrokes in the same window must NOT call set_active_window again
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert_eq!(inj.set_window_calls.load(Ordering::SeqCst), 1);

        // FocusLost (Polkit / launcher overlay): must clear current_app so bypass is NOT inherited
        shared.lock().unwrap().observe_focus_lost();
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert!(
            !daemon.is_current_app_bypassed(),
            "FocusLost must clear daemon.app_state.current_app so Polkit/launcher never inherits bypass_apps"
        );
        assert!(!inj.bypass.load(Ordering::SeqCst));
    }

    #[test]
    fn focus_lost_restores_global_method_and_enabled_from_terminal_or_english_app() {
        let _lock = crate::password_detector::atspi_test_lock();
        crate::password_detector::reset_atspi_focus_state();

        let mut cfg = crate::config::Config {
            input_method: "telex".into(),
            ..Default::default()
        };
        cfg.app_state.enabled = true;
        cfg.app_state.terminal_apps = vec!["kitty".into()];
        cfg.app_state.terminal_input_method = "vni".into();
        cfg.app_state.english_apps = vec!["kitty".into()];
        let enabled = Arc::new(AtomicBool::new(true));
        let mut daemon = Daemon::new(cfg, std::path::PathBuf::from("test.toml"), enabled);
        let shared = Arc::new(Mutex::new(FocusSnapshot::default()));
        let mut sync = FocusSyncState::from_initial(FocusSnapshot::default());
        let inj = CountingInjector::new();

        shared
            .lock()
            .unwrap()
            .observe(Some("toplevel:term".into()), Some("kitty".into()));
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert_eq!(daemon.app_state.effective_method(), "vni");
        assert!(!daemon.engine.is_enabled());

        // Focus moves to a layer-shell launcher (FocusLost): must restore global Telex + enabled.
        shared.lock().unwrap().observe_focus_lost();
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert_eq!(daemon.app_state.effective_method(), "telex");
        assert!(daemon.engine.is_enabled());
        // Verify engine itself switched back to Telex (not stuck in VNI from terminal).
        let _ = daemon.engine.process_key('a');
        let _ = daemon.engine.process_key('s');
        assert_eq!(daemon.engine.buffer(), "á");
    }

    #[test]
    fn idle_sync_handles_focus_lost_and_updates_generation() {
        let mut cfg = crate::config::Config::default();
        cfg.app_state.enabled = true;
        cfg.app_state.bypass_apps = vec!["cs2".into()];
        let enabled = Arc::new(AtomicBool::new(true));
        let mut daemon = Daemon::new(cfg, std::path::PathBuf::from("test.toml"), enabled);
        let shared = Arc::new(Mutex::new(FocusSnapshot::default()));
        let mut sync = FocusSyncState::from_initial(FocusSnapshot::default());
        let inj = CountingInjector::new();

        shared
            .lock()
            .unwrap()
            .observe(Some("toplevel:game".into()), Some("cs2".into()));
        sync.sync_on_idle(&mut daemon, &inj, &shared);
        assert!(inj.bypass.load(Ordering::SeqCst));
        let gen_after_game = sync.last_focus_generation;

        shared.lock().unwrap().observe_focus_lost();
        sync.sync_on_idle(&mut daemon, &inj, &shared);
        assert!(
            !inj.bypass.load(Ordering::SeqCst),
            "idle FocusLost must immediately clear game bypass_mode"
        );
        assert!(
            sync.last_focus_generation > gen_after_game,
            "idle FocusLost must advance last_focus_generation"
        );
    }

    #[test]
    fn focus_lost_preserves_layer2_password_protection_for_polkit_and_cosmic_osd() {
        let _lock = crate::password_detector::atspi_test_lock();
        crate::password_detector::reset_atspi_focus_state();

        let mut cfg = crate::config::Config::default();
        cfg.app_state.enabled = true;
        cfg.password_detection.enabled = true;
        let enabled = Arc::new(AtomicBool::new(true));
        let mut daemon = Daemon::new(cfg, std::path::PathBuf::from("test.toml"), enabled);
        let shared = Arc::new(Mutex::new(FocusSnapshot::default()));
        let mut sync = FocusSyncState::from_initial(FocusSnapshot::default());
        let inj = CountingInjector::new();

        // Start in a normal browser window (VN enabled).
        shared
            .lock()
            .unwrap()
            .observe(Some("toplevel:browser".into()), Some("firefox".into()));
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert!(daemon.engine.is_enabled());
        assert!(!daemon.app_state.is_password_active());

        // Case A: AT-SPI2 reports `cosmic-osd` (without ROLE_PASSWORD_TEXT) just before
        // compositor emits `FocusLost` for the layer-shell password dialog.
        crate::password_detector::record_atspi_focus_state(false, Some("cosmic-osd".to_string()));
        shared.lock().unwrap().observe_focus_lost();
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert!(
            daemon.app_state.is_password_active(),
            "Layer 2 password_apps must catch cosmic-osd even during FocusLost"
        );
        assert!(
            !daemon.engine.is_enabled(),
            "Engine must be disabled for layer-shell cosmic-osd password prompt"
        );

        // Case B: Returning to a normal window clears password mode cleanly.
        crate::password_detector::reset_atspi_focus_state();
        shared
            .lock()
            .unwrap()
            .observe(Some("toplevel:browser2".into()), Some("firefox".into()));
        sync.sync_on_key_press(&mut daemon, &inj, &shared);
        assert!(!daemon.app_state.is_password_active());
        assert!(daemon.engine.is_enabled());
    }
}
