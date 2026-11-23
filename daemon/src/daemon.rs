use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dboard_engine::{Engine, EngineEvent, EventStore, InputMethod};

use crate::app_state::AppStateManager;
use crate::commands::OutputCommand;
use crate::config::Config;
use crate::event::is_flush_char;
use crate::log::log_info;

fn show_osd(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = std::process::Command::new("notify-send")
            .args([
                "-t",
                "1000",
                "-h",
                "string:x-canonical-private-synchronous:dboard-osd",
                "-a",
                "Dboard",
                &summary,
                &body,
            ])
            .spawn()
        {
            let _ = child.wait();
        }
    });
}

fn next_toggle_state(
    toggle_modes: &[String],
    current_enabled: bool,
    current_method: &str,
) -> (bool, String) {
    let base_vn_method = if current_method.eq_ignore_ascii_case("vni") {
        "vni"
    } else {
        "telex"
    };

    let mut targets: Vec<(bool, String)> = Vec::new();
    for raw in toggle_modes {
        let target = match raw.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Some((false, base_vn_method.to_string())),
            "vn" | "vietnamese" => Some((true, base_vn_method.to_string())),
            "telex" => Some((true, "telex".to_string())),
            "vni" => Some((true, "vni".to_string())),
            _ => None,
        };
        if let Some(t) = target {
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
    }

    if targets.len() < 2 {
        targets = vec![
            (true, base_vn_method.to_string()),
            (false, base_vn_method.to_string()),
        ];
    }

    let current = (current_enabled, base_vn_method.to_string());
    if let Some(idx) = targets.iter().position(|t| *t == current) {
        targets[(idx + 1) % targets.len()].clone()
    } else {
        targets[0].clone()
    }
}

pub struct Daemon {
    pub engine: Engine,
    pub config: Config,
    pub config_path: PathBuf,
    pub config_modified: std::time::SystemTime,
    pub app_state: AppStateManager,
    pub engine_enabled: Arc<AtomicBool>,
    pub grab_enabled: bool,
    pub event_store: EventStore,
    pub screen_output: String,
    pub visible_revision: u64,
    pub candidate_ipc: crate::candidate_ipc::CandidateBroadcaster,
    pub caret_reselection: Option<(String, Vec<String>)>,
}

impl Daemon {
    pub fn new(config: Config, config_path: PathBuf, engine_enabled: Arc<AtomicBool>) -> Self {
        let method = match config.input_method.as_str() {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        let mut engine = Engine::new(method);
        engine.set_enabled(config.start_enabled);
        engine.set_auto_restore(config.auto_restore.enabled);
        engine.set_autocorrect(config.autocorrect);
        engine.set_keyboard_layout_name(&config.keyboard_layout);
        engine.set_auto_capitalize(config.auto_capitalize);
        if let Some(dict_path) = crate::user_dict_store::user_dict_path() {
            engine.set_user_dict(crate::user_dict_store::load_user_dict_from(&dict_path));
        }
        engine_enabled.store(config.start_enabled, Ordering::SeqCst);

        for (shortcut, expansion) in &config.macros {
            engine.add_macro(shortcut.clone(), expansion.clone());
        }

        let mut app_state = AppStateManager::new(
            config.app_state.english_apps.clone(),
            config.app_state.vietnamese_apps.clone(),
            config.app_state.bypass_apps.clone(),
            config.app_state.terminal_apps.clone(),
            config.app_state.terminal_input_method.clone(),
            config.input_method.clone(),
            config.start_enabled,
        );
        app_state.load_overrides();
        app_state.set_password_config(
            config.password_detection.enabled,
            config.password_detection.check_atspi2,
            config.password_detection.check_window_title,
            config.password_detection.title_keywords.clone(),
            config.password_detection.password_apps.clone(),
        );

        let config_modified = fs::metadata(&config_path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::now());
        let candidate_ipc = crate::candidate_ipc::CandidateBroadcaster::new(config.candidate_bar);

        Self {
            grab_enabled: config.grab,
            engine,
            config,
            config_path,
            config_modified,
            app_state,
            engine_enabled,
            event_store: EventStore::new(),
            screen_output: String::new(),
            visible_revision: 0,
            candidate_ipc,
            caret_reselection: None,
        }
    }

    fn status_file_path(&self) -> PathBuf {
        crate::config::user_ipc_file_path("status")
    }

    fn method_file_path(&self) -> PathBuf {
        crate::config::user_ipc_file_path("method")
    }

    fn write_ipc_file(path: &std::path::Path, content: &str) {
        crate::user_dict_store::enqueue_atomic_file_write(
            path.to_path_buf(),
            content.to_string(),
            false,
        );
    }

    pub fn write_status(&self) {
        let enabled = self.engine.is_enabled();
        self.engine_enabled.store(enabled, Ordering::SeqCst);
        let status_str = if enabled { "vn" } else { "en" };
        Self::write_ipc_file(&self.status_file_path(), status_str);
    }

    pub fn write_method_status(&self) {
        Self::write_ipc_file(&self.method_file_path(), &self.config.input_method);
    }

    pub fn toggle_method(&mut self) {
        let is_enabled = self.engine.is_enabled();
        let current_method = self.config.input_method.as_str();

        let (new_enabled, new_method) =
            next_toggle_state(&self.config.toggle_modes, is_enabled, current_method);

        self.engine.set_enabled(new_enabled);
        self.engine_enabled.store(new_enabled, Ordering::SeqCst);
        self.config.input_method = new_method.clone();
        self.app_state.set_global_enabled(new_enabled);
        self.app_state.set_global_method(&new_method);
        if self.config.app_state.enabled {
            self.app_state.set_current_app_override(new_enabled);
        }
        let effective = self.app_state.effective_method().to_string();
        let engine_method = match effective.as_str() {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        self.engine.set_method(engine_method);
        self.engine
            .set_auto_capitalize(self.config.auto_capitalize && !self.app_state.is_terminal_app());
        self.reset_composition_state();
        self.write_status();
        self.write_method_status();
        let label = if !new_enabled {
            "ENGLISH (EN)"
        } else if new_method == "vni" {
            "VNI (Vietnamese)"
        } else {
            "TELEX (Vietnamese)"
        };
        show_osd("Dboard", &format!("Mode: {}", label));
        log_info(&format!(
            "[dboard] Mode rotated: enabled={}, method={}, effective={}",
            new_enabled, new_method, effective
        ));
    }

    pub fn sync_status_file(&mut self) {
        let mut changed = false;
        let mut status_enabled_override = None;
        let status_path = self.status_file_path();
        if let Ok(content) = fs::read_to_string(&status_path) {
            let s = content.trim();
            if s == "vn" || s == "en" {
                let expect_enabled = s == "vn";
                if self.engine.is_enabled() != expect_enabled {
                    self.engine.set_enabled(expect_enabled);
                    self.engine_enabled.store(expect_enabled, Ordering::SeqCst);
                    self.app_state.set_global_enabled(expect_enabled);
                    status_enabled_override = Some(expect_enabled);
                    changed = true;
                }
            }
        }
        let method_path = self.method_file_path();
        if let Ok(content) = fs::read_to_string(&method_path) {
            let expect_method = content.trim().to_ascii_lowercase();
            if (expect_method == "vni" || expect_method == "telex")
                && !self
                    .config
                    .input_method
                    .eq_ignore_ascii_case(&expect_method)
            {
                self.config.input_method = expect_method.clone();
                self.app_state.set_global_method(&expect_method);
                changed = true;
            }
        }
        if changed {
            if let Some(enabled) = status_enabled_override {
                if self.config.app_state.enabled {
                    self.app_state.set_current_app_override(enabled);
                }
            }
            let effective = self.app_state.effective_method();
            let engine_method = if effective.eq_ignore_ascii_case("vni") {
                InputMethod::Vni
            } else {
                InputMethod::Telex
            };
            self.engine.set_method(engine_method);
            self.reset_composition_state();
        }
    }

    pub fn reload_config(&mut self) -> bool {
        // Promote ~/.config/dboard/config.toml if the user created/saved it after
        // the daemon initially booted from /etc/dboard/config.toml (.deb install).
        if let Some(user_cfg) = crate::config::user_config_path() {
            if user_cfg != self.config_path && user_cfg.exists() {
                self.config_path = user_cfg;
                self.config_modified = std::time::UNIX_EPOCH;
            }
        }
        let modified = fs::metadata(&self.config_path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::now());

        if modified <= self.config_modified {
            return false;
        }

        match Config::load_from(&self.config_path) {
            Ok(new_config) => {
                self.engine
                    .set_auto_restore(new_config.auto_restore.enabled);
                self.engine.set_autocorrect(new_config.autocorrect);
                self.engine
                    .set_keyboard_layout_name(&new_config.keyboard_layout);
                if !new_config
                    .keyboard_layout
                    .eq_ignore_ascii_case(&self.config.keyboard_layout)
                {
                    log_info(&format!(
                        "[dboard] keyboard_layout changed '{}' -> '{}': autocorrect updated immediately; restart dboard.service to reload Wayland XKB keymap",
                        self.config.keyboard_layout, new_config.keyboard_layout
                    ));
                }
                self.engine.set_auto_capitalize(
                    new_config.auto_capitalize && !self.app_state.is_terminal_app(),
                );

                self.engine.clear_macros();
                for (shortcut, expansion) in &new_config.macros {
                    self.engine.add_macro(shortcut.clone(), expansion.clone());
                }

                self.app_state.set_global_method(&new_config.input_method);
                self.app_state.update_lists(
                    new_config.app_state.english_apps.clone(),
                    new_config.app_state.vietnamese_apps.clone(),
                    new_config.app_state.bypass_apps.clone(),
                    new_config.app_state.terminal_apps.clone(),
                    new_config.app_state.terminal_input_method.clone(),
                );

                let effective = self.app_state.effective_method();
                let engine_method = match effective {
                    "vni" => InputMethod::Vni,
                    _ => InputMethod::Telex,
                };
                self.engine.set_method(engine_method);

                self.app_state.set_password_config(
                    new_config.password_detection.enabled,
                    new_config.password_detection.check_atspi2,
                    new_config.password_detection.check_window_title,
                    new_config.password_detection.title_keywords.clone(),
                    new_config.password_detection.password_apps.clone(),
                );

                self.grab_enabled = new_config.grab;
                if new_config.candidate_bar != self.config.candidate_bar {
                    self.candidate_ipc.hide();
                    self.candidate_ipc =
                        crate::candidate_ipc::CandidateBroadcaster::new(new_config.candidate_bar);
                }
                self.config = new_config;
                self.config_modified = modified;
                self.write_method_status();
                true
            }
            Err(err) => {
                crate::log::log_info(&format!(
                    "[dboard] WARN: failed to reload config {}: {err}",
                    self.config_path.display()
                ));
                false
            }
        }
    }

    pub fn sync_candidate_bar(&mut self) {
        if !self.candidate_ipc.is_enabled() {
            return;
        }
        let sensitive_or_bypass = !self.engine.is_enabled()
            || self.is_current_app_bypassed()
            || self.app_state.is_password_active();
        if sensitive_or_bypass {
            self.candidate_ipc.hide();
            return;
        }
        let visible = self.engine.buffer();
        if visible.is_empty() {
            let preds = self.engine.predict_next_words(3);
            if preds.is_empty() {
                self.candidate_ipc.hide();
            } else {
                let refs: Vec<&str> = preds.iter().map(String::as_str).collect();
                self.candidate_ipc.publish_state(false, &refs);
            }
            return;
        }
        let raw = self.engine.raw_buffer().to_string();
        let suggestion = self.engine.candidate_suggestion();
        self.candidate_ipc
            .sync_from_engine(false, &visible, &raw, suggestion.as_deref());
    }

    /// Synchronize mid-sentence re-selection candidates when the user clicks or
    /// navigates the caret to the end of an already-committed word (`tét <-> test`).
    pub fn sync_caret_reselection(&mut self, word_at_caret: Option<&str>, prev_word: Option<&str>) {
        if !self.candidate_ipc.is_enabled() {
            self.caret_reselection = None;
            return;
        }
        let sensitive_or_bypass = !self.engine.is_enabled()
            || self.is_current_app_bypassed()
            || self.app_state.is_terminal_app()
            || self.app_state.is_password_active();
        if sensitive_or_bypass {
            self.caret_reselection = None;
            self.candidate_ipc.hide();
            return;
        }
        let Some(word) = word_at_caret else {
            if self.caret_reselection.take().is_some() {
                self.candidate_ipc.hide();
            }
            return;
        };
        if self
            .caret_reselection
            .as_ref()
            .is_some_and(|(cur, _)| cur == word)
        {
            return;
        }
        let alts = self.engine.alternatives_for_word(word, prev_word);
        if alts.is_empty() {
            if self.caret_reselection.take().is_some() {
                self.candidate_ipc.hide();
            }
            return;
        }
        let refs: Vec<&str> = alts.iter().map(String::as_str).collect();
        self.candidate_ipc.publish_state(false, &refs);
        self.caret_reselection = Some((word.to_string(), alts));
    }

    /// Applies quick candidate selection (`Alt+1`/`Alt+2`/`Alt+3` or mouse click)
    /// matching active composing candidates, mid-sentence caret re-selection
    /// (`tét <-> test`), or next-word predictions after Space.
    pub fn select_candidate(&mut self, index: usize) -> Vec<OutputCommand> {
        if !self.config.candidate_bar || !self.engine.is_enabled() {
            return Vec::new();
        }
        let visible = self.engine.buffer();
        let raw = self.engine.raw_buffer().to_string();
        if visible.is_empty() || raw.is_empty() {
            if let Some((word, alts)) = self.caret_reselection.take() {
                let Some(chosen) = alts.get(index).cloned() else {
                    self.candidate_ipc.hide();
                    return Vec::new();
                };
                self.reset_composition_state();
                self.candidate_ipc.hide();
                if chosen == word {
                    return Vec::new();
                }
                return vec![
                    OutputCommand::Backspace(word.chars().count()),
                    OutputCommand::Type(chosen),
                ];
            }
            let preds = self.engine.predict_next_words(3);
            let Some(chosen) = preds.get(index).cloned() else {
                return Vec::new();
            };
            self.reset_composition_state();
            self.candidate_ipc.hide();
            return vec![OutputCommand::Type(format!("{chosen} "))];
        }
        let mut slots = vec![visible.clone()];
        if raw != visible {
            slots.push(raw.clone());
        }
        if let Some(sug) = self.engine.candidate_suggestion() {
            if sug != visible && sug != raw {
                slots.push(sug);
            }
        }
        let Some(chosen) = slots.get(index).cloned() else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        if chosen != visible {
            commands.push(OutputCommand::Backspace(visible.chars().count()));
            commands.push(OutputCommand::Type(chosen));
        }
        self.reset_composition_state();
        self.candidate_ipc.hide();
        commands
    }

    pub fn process_key(&mut self, ch: char) -> Vec<OutputCommand> {
        let mut commands = Vec::new();
        self.caret_reselection = None;
        let allow_learning = !self.is_current_app_bypassed()
            && !self.app_state.is_terminal_app()
            && !self.app_state.is_password_active();
        self.engine.set_learning_enabled(allow_learning);

        if let Some(event) = self.engine.process_key(ch) {
            match event {
                EngineEvent::Flush(text) => {
                    commands.push(OutputCommand::Type(text));
                }
                EngineEvent::Insert(text) => {
                    commands.push(OutputCommand::Type(text));
                }
                EngineEvent::AutoRestore(word) => {
                    let len = word.chars().count();
                    commands.push(OutputCommand::Backspace(len));
                    commands.push(OutputCommand::Type(word));
                }
                EngineEvent::Replace(replacement) => {
                    commands.push(OutputCommand::Replace(replacement));
                }
                EngineEvent::UndoTones {
                    backspaces,
                    restored,
                } => {
                    commands.push(OutputCommand::Backspace(backspaces));
                    commands.push(OutputCommand::Type(restored));
                }
                EngineEvent::Paste(text) => {
                    self.engine.exit_paste_mode();
                    commands.push(OutputCommand::Type(text));
                }
            }
        }

        // Persist newly learned personal dictionary/bigram entries off the typing thread
        // (never in password fields, bypassed apps, or terminal emulators).
        if allow_learning {
            if let Some(serialized) = self.engine.take_user_dict_for_save() {
                if let Some(path) = crate::user_dict_store::user_dict_path() {
                    crate::user_dict_store::enqueue_user_dict_save(path, serialized);
                }
            }
        }

        self.sync_candidate_bar();
        commands
    }

    /// Account for the native key already delivered by an ungrabbed device.
    pub fn process_forwarded_key(&mut self, ch: char) -> Vec<OutputCommand> {
        let mut commands = self.process_key(ch);
        for command in &mut commands {
            if let OutputCommand::Replace(replacement) = command {
                if ch == '\x08' {
                    replacement.delete_before_cursor =
                        replacement.delete_before_cursor.saturating_sub(1);
                } else {
                    replacement.delete_before_cursor += 1;
                    if is_flush_char(ch) {
                        replacement.insert.push(ch);
                    }
                }
            }
        }
        commands
    }

    pub fn handle_transaction_outcome(
        &mut self,
        outcome: crate::commands::TransactionOutcome,
    ) -> bool {
        match outcome {
            crate::commands::TransactionOutcome::Applied { .. } => true,
            crate::commands::TransactionOutcome::Failed => {
                self.engine
                    .reduce(dboard_engine::PhysicalEvent::ReplacementFailed);
                false
            }
            crate::commands::TransactionOutcome::Stale { .. } => {
                self.engine.invalidate_composition();
                self.engine.reset_visible_revision();
                self.replay_reset();
                false
            }
        }
    }

    pub fn is_current_app_bypassed(&self) -> bool {
        if !self.config.app_state.enabled {
            return false;
        }
        self.app_state.is_current_app_bypassed()
    }

    pub fn replay_reset(&mut self) {
        self.event_store.clear();
        self.screen_output.clear();
        self.visible_revision = 0;
    }

    pub fn reset_composition_state(&mut self) {
        self.engine.reset();
        self.engine.reset_visible_revision();
        self.replay_reset();
        self.caret_reselection = None;
        self.candidate_ipc.hide();
    }

    pub fn check_app_change_with(&mut self, new_class: String) {
        if let Some(should_enable) = self.app_state.update_with_app(new_class) {
            self.engine.set_enabled(should_enable);
            self.write_status();
        }
        let effective = self.app_state.effective_method();
        let engine_method = match effective {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        self.engine.set_method(engine_method);
        self.engine
            .set_auto_capitalize(self.config.auto_capitalize && !self.app_state.is_terminal_app());
        self.reset_composition_state();
    }

    /// Restore engine enabled state, input method, and auto-capitalize back to
    /// global defaults when focus moves away from any tracked toplevel window
    /// (e.g. Polkit/launcher layer-shell surface).
    pub fn reset_app_focus_to_global(&mut self) {
        self.app_state.clear_current_app();
        let default_enabled = self.app_state.get_default_state();
        if self.engine.is_enabled() != default_enabled {
            self.engine.set_enabled(default_enabled);
            self.write_status();
        }
        let effective = self.app_state.effective_method();
        let engine_method = match effective {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        self.engine.set_method(engine_method);
        self.engine.set_auto_capitalize(self.config.auto_capitalize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dboard_engine::InputEvent;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    const BS: char = '\x08';

    fn apply(screen: &mut String, cmds: &[OutputCommand]) {
        for cmd in cmds {
            match cmd {
                OutputCommand::Backspace(n) => {
                    let len = screen.chars().count();
                    let keep = len.saturating_sub(*n);
                    *screen = screen.chars().take(keep).collect();
                }
                OutputCommand::Type(s) => screen.push_str(s),
                OutputCommand::Replace(replacement) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(replacement.delete_before_cursor);
                    *screen = screen.chars().take(keep).collect();
                    screen.push_str(&replacement.insert);
                }
            }
        }
    }

    #[test]
    fn forwarded_keys_preserve_prefix_after_backspace_and_transforms() {
        let config = Config {
            input_method: "telex".into(),
            start_enabled: true,
            ..Default::default()
        };
        let mut daemon = Daemon::new(
            config,
            PathBuf::from("/tmp/dboard-forwarded-test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        let mut screen = String::from("X ");
        for key in "as\x08s\x08\x08".chars() {
            if key == BS {
                screen.pop();
            } else {
                screen.push(key);
            }
            let commands = daemon.process_forwarded_key(key);
            apply(&mut screen, &commands);
            assert_eq!(screen, format!("X {}", daemon.engine.buffer()));
        }
        assert_eq!(screen, "X ");
    }

    #[test]
    fn focus_change_resets_engine_and_daemon_visible_revisions_together() {
        let mut daemon = Daemon::new(
            Config::default(),
            PathBuf::from("/tmp/dboard_focus_revision_test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);
        daemon.engine.set_method(InputMethod::Telex);

        let mut last_expected_revision = None;
        for ch in "hoor".chars() {
            for command in daemon.process_key(ch) {
                if let OutputCommand::Replace(replacement) = command {
                    last_expected_revision = Some(replacement.expected_visible_revision);
                }
            }
        }
        let last_expected_revision =
            last_expected_revision.expect("target A must produce a replacement");
        daemon.visible_revision = last_expected_revision + 1;

        daemon.reset_composition_state();

        let mut replacement_after_focus_change = None;
        for ch in "chaof".chars() {
            for command in daemon.process_key(ch) {
                if let OutputCommand::Replace(replacement) = command {
                    replacement_after_focus_change = Some(replacement);
                }
            }
        }

        let replacement = replacement_after_focus_change.expect("expected tone replacement");
        assert_eq!(replacement.expected_visible_revision, 0);
        assert_eq!(daemon.visible_revision, 0);
    }

    #[test]
    fn two_target_focus_change_does_not_replay_stale_composition() {
        let mut daemon = Daemon::new(
            Config::default(),
            PathBuf::from("/tmp/dboard_two_target_test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);

        for ch in "hoor".chars() {
            daemon.process_key(ch);
        }
        assert_eq!(daemon.engine.buffer(), "hổ");

        daemon.event_store.push(InputEvent::KeyTyped('r'));
        daemon.screen_output = "hổ".to_string();
        daemon.visible_revision = 4;
        daemon.reset_composition_state();

        let mut target_b = String::new();
        for ch in "xin chao".chars() {
            let commands = daemon.process_key(ch);
            apply(&mut target_b, &commands);
        }

        assert!(!target_b.contains("hoor"));
        assert!(!target_b.contains("hổ"));
        assert_eq!(daemon.engine.buffer(), "chao");
        assert!(daemon.event_store.is_empty());
        assert!(daemon.screen_output.is_empty());
        assert_eq!(daemon.visible_revision, 0);
    }

    #[test]
    fn focus_change_before_boundaries_does_not_commit_previous_target_composition() {
        for boundary in [' ', '.', '\n'] {
            let mut daemon = Daemon::new(
                Config::default(),
                PathBuf::from("focus-boundary-test.toml"),
                Arc::new(AtomicBool::new(true)),
            );
            daemon.engine.set_enabled(true);

            for ch in "tieeng".chars() {
                daemon.process_key(ch);
            }
            assert!(!daemon.engine.buffer().is_empty());

            daemon.reset_composition_state();
            let commands = daemon.process_key(boundary);
            let mut target_b = String::new();
            apply(&mut target_b, &commands);

            assert!(!target_b.contains("tieeng"));
            assert!(!target_b.contains("tiếng"));
            assert!(daemon.event_store.is_empty());
            assert!(daemon.screen_output.is_empty());
            assert_eq!(daemon.visible_revision, 0);
        }
    }

    #[test]
    fn closed_target_during_composition_discards_raw_keys_before_next_target() {
        let mut daemon = Daemon::new(
            Config::default(),
            PathBuf::from("closed-target-compose-test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);

        for ch in "dduow".chars() {
            daemon.process_key(ch);
        }
        assert!(!daemon.engine.buffer().is_empty());

        daemon.reset_composition_state();

        let mut next_target = String::new();
        for ch in "moi".chars() {
            let commands = daemon.process_key(ch);
            apply(&mut next_target, &commands);
        }

        assert!(!next_target.contains("dduow"));
        assert!(!next_target.contains("đư"));
        assert_eq!(daemon.engine.buffer(), "moi");
        assert_eq!(daemon.visible_revision, 0);
    }

    #[test]
    fn focus_change_reset_clears_all_composition_state() {
        let mut daemon = Daemon::new(
            Config::default(),
            PathBuf::from("/tmp/dboard_focus_reset_test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);
        daemon.process_key('h');
        daemon.process_key('o');
        daemon.process_key('r');
        daemon.process_key('r');
        daemon.event_store.push(InputEvent::KeyTyped('x'));
        daemon.screen_output = "stale".to_string();
        daemon.visible_revision = 7;

        daemon.reset_composition_state();

        assert!(daemon.engine.buffer().is_empty());
        assert!(daemon.event_store.is_empty());
        assert!(daemon.screen_output.is_empty());
        assert_eq!(daemon.visible_revision, 0);
    }

    #[test]
    fn toggle_modes_supports_one_tap_vn_en_and_three_way_rotation() {
        let two_way = vec!["vn".into(), "en".into()];
        assert_eq!(
            next_toggle_state(&two_way, true, "telex"),
            (false, "telex".into())
        );
        assert_eq!(
            next_toggle_state(&two_way, false, "telex"),
            (true, "telex".into())
        );
        // Preserves VNI when toggling VN <-> EN
        assert_eq!(
            next_toggle_state(&two_way, true, "vni"),
            (false, "vni".into())
        );
        assert_eq!(
            next_toggle_state(&two_way, false, "vni"),
            (true, "vni".into())
        );

        let three_way = vec!["en".into(), "vni".into(), "telex".into()];
        assert_eq!(
            next_toggle_state(&three_way, false, "telex"),
            (true, "vni".into())
        );
        assert_eq!(
            next_toggle_state(&three_way, true, "vni"),
            (true, "telex".into())
        );
        assert_eq!(
            next_toggle_state(&three_way, true, "telex"),
            (false, "telex".into())
        );

        // Invalid/empty lists fall back safely to 2-way VN <-> EN
        assert_eq!(
            next_toggle_state(&[], true, "telex"),
            (false, "telex".into())
        );
        assert_eq!(
            next_toggle_state(&["bogus".into()], false, "vni"),
            (true, "vni".into())
        );
    }

    #[test]
    fn select_candidate_supports_vn_ascii_and_third_autocorrect_slot() {
        let cfg = Config {
            candidate_bar: true,
            ..Config::default()
        };
        let mut daemon = Daemon::new(
            cfg,
            PathBuf::from("/tmp/dboard_cand_test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);
        daemon.engine.set_method(InputMethod::Telex);

        // Slot [1] (VN) vs Slot [2] (raw ASCII)
        let mut screen = String::new();
        for ch in "tieengs".chars() {
            let cmds = daemon.process_forwarded_key(ch);
            screen.push(ch);
            apply(&mut screen, &cmds);
        }
        assert_eq!(screen, "tiếng");
        let cmds = daemon.select_candidate(1);
        apply(&mut screen, &cmds);
        assert_eq!(screen, "tieengs");
        assert!(daemon.engine.buffer().is_empty());

        // Slot [3] (autocorrect suggestion for typo "tienegs" -> "tiếng")
        let mut screen2 = String::new();
        for ch in "tienegs".chars() {
            let cmds = daemon.process_forwarded_key(ch);
            screen2.push(ch);
            apply(&mut screen2, &cmds);
        }
        assert_eq!(
            daemon.engine.candidate_suggestion().as_deref(),
            Some("tiếng")
        );
        // Slots: [1]=composed ("tiếng" or "tiênegs"?), wait: "tienegs" composes to "tiênegs" or "tiếng"?
        // Regardless, pick the index matching "tiếng" (slot 2 when raw != visible, or slot 1 when raw == visible).
        let idx = if daemon.engine.raw_buffer() == daemon.engine.buffer() {
            1
        } else {
            2
        };
        let cmds = daemon.select_candidate(idx);
        apply(&mut screen2, &cmds);
        assert_eq!(screen2, "tiếng");
    }
}
