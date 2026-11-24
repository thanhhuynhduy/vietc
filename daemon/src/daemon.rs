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
    displayed_composing_suggestion: Option<String>,
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
            displayed_composing_suggestion: None,
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
        let status_str = if enabled { "vn" } else { "en" };
        Self::write_ipc_file(&self.status_file_path(), status_str);
        self.engine_enabled.store(enabled, Ordering::SeqCst);
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
        if !self.app_state.is_password_active()
            && !crate::user_dict_store::has_pending_write(&status_path)
        {
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
        }
        let method_path = self.method_file_path();
        if !crate::user_dict_store::has_pending_write(&method_path) {
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
        if self.config_path.starts_with("/etc/") {
            if let Some(user_cfg) = crate::config::user_config_path() {
                if user_cfg != self.config_path && user_cfg.exists() {
                    self.config_path = user_cfg;
                    self.config_modified = std::time::UNIX_EPOCH;
                }
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
                if !new_config
                    .keyboard_layout
                    .eq_ignore_ascii_case(&self.config.keyboard_layout)
                {
                    // Keep `engine.keyboard_layout` aligned with the active session's
                    // single-upload Wayland XKB keymap (`evdev_loop::active_layout`) until
                    // the daemon restarts, preventing QWERTY/Dvorak spatial mismatch.
                    log_info(&format!(
                        "[dboard] keyboard_layout changed '{}' -> '{}': restart dboard.service to reload Wayland XKB keymap",
                        self.config.keyboard_layout, new_config.keyboard_layout
                    ));
                    show_osd(
                        "Dboard",
                        &format!(
                            "Layout '{}' saved — restart dboard.service to apply.",
                            new_config.keyboard_layout
                        ),
                    );
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

                if new_config.app_state.enabled {
                    let desired =
                        self.app_state.get_default_state() && !self.app_state.is_password_active();
                    if self.engine.is_enabled() != desired {
                        self.engine.set_enabled(desired);
                        self.write_status();
                    }
                } else if self.app_state.is_password_active() && self.engine.is_enabled() {
                    self.engine.set_enabled(false);
                    self.write_status();
                }

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

    pub fn sync_candidate_bar(&mut self, include_suggestion: bool) {
        if !self.candidate_ipc.is_enabled() {
            return;
        }
        let sensitive_or_bypass = !self.engine.is_enabled()
            || self.is_current_app_bypassed()
            || self.app_state.is_password_active()
            || self.engine.has_pasted_prefix();
        if sensitive_or_bypass {
            self.displayed_composing_suggestion = None;
            self.candidate_ipc.hide();
            return;
        }
        let visible = self.engine.buffer();
        if visible.is_empty() {
            self.displayed_composing_suggestion = None;
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
        let sug = include_suggestion
            .then(|| self.engine.candidate_suggestion())
            .flatten();
        self.displayed_composing_suggestion = sug.clone();
        self.candidate_ipc
            .sync_from_engine(false, &visible, &raw, sug.as_deref());
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

    /// Applies quick candidate selection (`Alt+1`/`Alt+2`/`Alt+3` or mouse click).
    /// Returns `None` when no matching candidate slot exists so keyboard shortcuts
    /// pass through to the active application unconsumed.
    /// When `include_predictions` is `false` (keyboard `Alt+1/2/3`), idle next-word
    /// predictions are not intercepted so browser/terminal tab switching stays intact.
    pub fn try_select_candidate(
        &mut self,
        index: usize,
        include_predictions: bool,
    ) -> Option<Vec<OutputCommand>> {
        if !self.config.candidate_bar
            || !self.engine.is_enabled()
            || self.is_current_app_bypassed()
            || self.app_state.is_password_active()
            || self.engine.has_pasted_prefix()
        {
            return None;
        }
        let allow_learning = !self.app_state.is_terminal_app();
        self.engine.set_learning_enabled(allow_learning);

        let visible = self.engine.buffer();
        let raw = self.engine.raw_buffer().to_string();
        if visible.is_empty() || raw.is_empty() {
            if let Some((word, alts)) = self.caret_reselection.as_ref() {
                let chosen = alts.get(index)?.clone();
                let word = word.clone();
                self.caret_reselection = None;
                self.reset_composition_state();
                self.candidate_ipc.hide();
                if chosen == word {
                    return Some(Vec::new());
                }
                return Some(vec![
                    OutputCommand::Backspace(word.chars().count()),
                    OutputCommand::Type(chosen),
                ]);
            }
            if !include_predictions {
                return None;
            }
            let preds = self.engine.predict_next_words(3);
            let chosen = preds.get(index)?.clone();
            self.engine.commit_selected_word(&chosen, true);
            if allow_learning {
                if let Some(serialized) = self.engine.take_user_dict_for_save() {
                    if let Some(path) = crate::user_dict_store::user_dict_path() {
                        crate::user_dict_store::enqueue_user_dict_save(path, serialized);
                    }
                }
            }
            self.sync_candidate_bar(false);
            return Some(vec![OutputCommand::Type(format!("{chosen} "))]);
        }
        let mut slots = vec![visible.clone()];
        if raw != visible {
            slots.push(raw.clone());
        }
        let suggestion = if include_predictions {
            self.displayed_composing_suggestion
                .clone()
                .or_else(|| self.engine.candidate_suggestion())
        } else {
            self.displayed_composing_suggestion.clone()
        };
        if let Some(sug) = suggestion {
            if sug != visible && sug != raw {
                slots.push(sug);
            }
        }
        let chosen = slots.get(index)?.clone();
        let mut commands = Vec::new();
        if chosen != visible {
            commands.push(OutputCommand::Backspace(visible.chars().count()));
            commands.push(OutputCommand::Type(chosen.clone()));
        }
        self.displayed_composing_suggestion = None;
        self.engine.commit_selected_word(&chosen, false);
        if let Some(serialized) = self.engine.take_user_dict_for_save() {
            if let Some(path) = crate::user_dict_store::user_dict_path() {
                crate::user_dict_store::enqueue_user_dict_save(path, serialized);
            }
        }
        self.candidate_ipc.hide();
        Some(commands)
    }

    /// Applies quick candidate selection across composing, re-selection, and
    /// next-word prediction slots.
    pub fn select_candidate(&mut self, index: usize) -> Vec<OutputCommand> {
        self.try_select_candidate(index, true).unwrap_or_default()
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

        self.sync_candidate_bar(false);
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
        self.displayed_composing_suggestion = None;
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
#[cfg(test)]
#[path = "daemon_tests.rs"]
mod tests;
