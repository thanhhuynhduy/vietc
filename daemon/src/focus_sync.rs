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
    last_key_time: Instant,
}

impl FocusSyncState {
    pub fn from_initial(initial: FocusSnapshot) -> Self {
        Self {
            last_focus_generation: initial.generation,
            last_focus_target: initial.target,
            last_window_class: String::new(),
            last_key_time: Instant::now(),
        }
    }

    /// Observe focus generation changes, per-app class changes, and password
    /// field transitions on `value == 1` (KeyDown) BEFORE engine/Backspace logic.
    ///
    /// Invariant: when neither `focus.generation` nor `active_window_class`
    /// changed (normal typing in the same window), `daemon.check_app_change_with`
    /// and `daemon.reset_composition_state` MUST NOT be called.
    pub fn sync_on_key_press(
        &mut self,
        daemon: &mut Daemon,
        injector: &dyn KeyInjector,
        shared_focus: &SharedFocusSnapshot,
    ) {
        let gap = self.last_key_time.elapsed();
        self.last_key_time = Instant::now();

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
                injector.set_active_window(&active_window_class);
                daemon.check_app_change_with(active_window_class);
                injector.set_terminal_app(daemon.app_state.is_terminal_app());
                injector.set_bypass_mode(daemon.is_current_app_bypassed());
            }
        } else if daemon.config.app_state.enabled && !active_window_class.is_empty() {
            injector.set_active_window(&active_window_class);
            if class_changed {
                crate::password_detector::clear_atspi_password_flag_if_non_atspi(
                    focus.identity.as_deref(),
                );
                self.last_window_class = active_window_class.clone();
                daemon.check_app_change_with(active_window_class);
            }
            injector.set_terminal_app(daemon.app_state.is_terminal_app());
            injector.set_bypass_mode(daemon.is_current_app_bypassed());
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
