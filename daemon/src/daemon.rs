use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use vietc_engine::{Engine, EngineEvent, EventStore, InputMethod};

use crate::app_state::AppStateManager;
use crate::commands::OutputCommand;
use crate::config::Config;
use crate::event::is_flush_char;
use crate::log::log_info;

fn show_osd(summary: &str, body: &str) {
    if let Ok(mut child) = std::process::Command::new("notify-send")
        .args([
            "-t",
            "1000",
            "-h",
            "string:x-canonical-private-synchronous:vietc-osd",
            "-a",
            "VietC",
            summary,
            body,
        ])
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
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
        }
    }

    fn status_file_path(&self) -> PathBuf {
        crate::config::user_ipc_file_path("status")
    }

    fn method_file_path(&self) -> PathBuf {
        crate::config::user_ipc_file_path("method")
    }

    fn write_ipc_file(path: &std::path::Path, content: &str) {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && parent != std::path::Path::new("/tmp") {
                let _ = std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent);
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            let _ = f.write_all(content.as_bytes());
        }
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

        let (new_enabled, new_method) = if !is_enabled {
            (true, "vni")
        } else if current_method == "vni" {
            (true, "telex")
        } else {
            (false, "telex")
        };

        self.engine.set_enabled(new_enabled);
        self.engine_enabled.store(new_enabled, Ordering::SeqCst);
        self.config.input_method = new_method.into();
        self.app_state.set_global_enabled(new_enabled);
        self.app_state.set_global_method(new_method);
        self.app_state.clear_overrides();
        let effective = self.app_state.effective_method().to_string();
        let engine_method = match effective.as_str() {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        self.engine.set_method(engine_method);
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
        show_osd("Viet+", &format!("Mode: {}", label));
        log_info(&format!(
            "[vietc] Mode rotated (Ctrl+Space): enabled={}, method={}, effective={}",
            new_enabled, new_method, effective
        ));
    }

    pub fn sync_status_file(&mut self) {
        let status_path = self.status_file_path();
        if let Ok(content) = fs::read_to_string(&status_path) {
            let s = content.trim();
            if s == "vn" || s == "en" {
                let expect_enabled = s == "vn";
                if self.engine.is_enabled() != expect_enabled {
                    self.engine.set_enabled(expect_enabled);
                    self.engine_enabled.store(expect_enabled, Ordering::SeqCst);
                    self.app_state.set_global_enabled(expect_enabled);
                }
            }
        }
        let method_path = self.method_file_path();
        if let Ok(content) = fs::read_to_string(&method_path) {
            let expect_method = content.trim();
            if (expect_method == "vni" || expect_method == "telex")
                && self.config.input_method != expect_method
            {
                self.config.input_method = expect_method.to_string();
                self.app_state.set_global_method(expect_method);
                let effective = self.app_state.effective_method();
                let engine_method = match effective {
                    "vni" => InputMethod::Vni,
                    _ => InputMethod::Telex,
                };
                self.engine.set_method(engine_method);
            }
        }
    }

    pub fn reload_config(&mut self) -> bool {
        // Promote ~/.config/vietc/config.toml if the user created/saved it after
        // the daemon initially booted from /etc/vietc/config.toml (.deb install).
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
                self.config = new_config;
                self.config_modified = modified;
                true
            }
            Err(_) => false,
        }
    }

    pub fn process_key(&mut self, ch: char) -> Vec<OutputCommand> {
        let mut commands = Vec::new();

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
                    .reduce(vietc_engine::PhysicalEvent::ReplacementFailed);
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
    }

    #[allow(dead_code)]
    pub fn has_composition_state(&self) -> bool {
        !self.engine.buffer().is_empty()
            || !self.event_store.is_empty()
            || !self.screen_output.is_empty()
            || self.visible_revision != 0
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
        self.reset_composition_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use vietc_engine::InputEvent;

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
            PathBuf::from("/tmp/vietc-forwarded-test.toml"),
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
            PathBuf::from("/tmp/vietc_focus_revision_test.toml"),
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
            PathBuf::from("/tmp/vietc_two_target_test.toml"),
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
            PathBuf::from("/tmp/vietc_focus_reset_test.toml"),
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
}
