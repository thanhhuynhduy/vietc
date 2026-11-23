// SPDX-License-Identifier: MIT
//! Senior QC Automation Multi-Environment Exhaustive Stress Suite.
//!
//! Simulates real-world user workflows across 4 Linux desktop app environments
//! (Chromium/Chrome, Electron/VS Code/Discord, Qt/LibreOffice, Terminal/Password)
//! against the full `Daemon` + `AppStateManager` + `Engine` + `FocusSnapshot`
//! + `evdev` state pipeline, logging every anomaly or invariant breach.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use dboard_protocol::inject::InjectResult;
use dboard_protocol::KeyInjector;
use evdev::{AttributeSet, KeyCode};

use crate::config::Config;
use crate::daemon::Daemon;
use crate::event::{
    consume_released_or_repeated_key, is_flush_char, is_modifier_held_shift, is_modifier_pressed,
    is_navigation_or_control_key, is_toggle_combination_state, is_toggle_key, is_vn_control_key,
    key_to_char_with_case, update_keyboard_state,
};
use crate::focus::{take_omnibox_inline_selection, FocusSnapshot, SharedFocusSnapshot};
use crate::inject::execute_commands;
use crate::password_detector::{record_atspi_focus_state, reset_atspi_focus_state};
use crate::surrounding_context::{extract_surrounding_context, maybe_seed_surrounding_word};

/// Virtual screen + modifier/omnibox/bypass recorder acting as a deterministic
/// Wayland/uinput client for Chromium, Electron, Qt, and Terminal targets.
struct QcVirtualClient {
    screen: RefCell<String>,
    cursor: Cell<usize>,
    held_keys: RefCell<HashSet<u16>>,
    omnibox_selection_len: Cell<usize>,
    omnibox_shield_armed: Cell<bool>,
    caps_lock: Cell<bool>,
    num_lock: Cell<bool>,
    bypass_mode: Cell<bool>,
    terminal_app: Cell<bool>,
    active_window: RefCell<String>,
    shift_enter_count: Cell<usize>,
    raw_forwarded: RefCell<Vec<(u16, i32)>>,
}

impl QcVirtualClient {
    fn new() -> Self {
        Self {
            screen: RefCell::new(String::new()),
            cursor: Cell::new(0),
            held_keys: RefCell::new(HashSet::new()),
            omnibox_selection_len: Cell::new(0),
            omnibox_shield_armed: Cell::new(false),
            caps_lock: Cell::new(false),
            num_lock: Cell::new(false),
            bypass_mode: Cell::new(false),
            terminal_app: Cell::new(false),
            active_window: RefCell::new(String::new()),
            shift_enter_count: Cell::new(0),
            raw_forwarded: RefCell::new(Vec::new()),
        }
    }

    fn text(&self) -> String {
        self.screen.borrow().clone()
    }

    fn clear_screen(&self) {
        self.screen.borrow_mut().clear();
        self.cursor.set(0);
        self.omnibox_selection_len.set(0);
    }

    fn set_screen_and_cursor(&self, text: &str, cursor_chars: usize) {
        *self.screen.borrow_mut() = text.to_string();
        self.cursor.set(cursor_chars.min(text.chars().count()));
        self.omnibox_selection_len.set(0);
    }

    fn insert_str_at_cursor(&self, s: &str) {
        let mut buf = self.screen.borrow_mut();
        let pos = self.cursor.get().min(buf.chars().count());
        let byte_idx = buf
            .char_indices()
            .nth(pos)
            .map_or(buf.len(), |(idx, _)| idx);
        buf.insert_str(byte_idx, s);
        self.cursor.set(pos + s.chars().count());
    }

    fn backspace_at_cursor(&self, count: usize) {
        let mut buf = self.screen.borrow_mut();
        let pos = self.cursor.get().min(buf.chars().count());
        let del = count.min(pos);
        if del == 0 {
            return;
        }
        let start_byte = buf.char_indices().nth(pos - del).map_or(0, |(idx, _)| idx);
        let end_byte = buf
            .char_indices()
            .nth(pos)
            .map_or(buf.len(), |(idx, _)| idx);
        buf.replace_range(start_byte..end_byte, "");
        self.cursor.set(pos - del);
    }

    fn delete_after_cursor(&self, count: usize) {
        let mut buf = self.screen.borrow_mut();
        let pos = self.cursor.get().min(buf.chars().count());
        let avail = buf.chars().count().saturating_sub(pos);
        let del = count.min(avail);
        if del == 0 {
            return;
        }
        let start_byte = buf
            .char_indices()
            .nth(pos)
            .map_or(buf.len(), |(idx, _)| idx);
        let end_byte = buf
            .char_indices()
            .nth(pos + del)
            .map_or(buf.len(), |(idx, _)| idx);
        buf.replace_range(start_byte..end_byte, "");
    }
}

impl KeyInjector for QcVirtualClient {
    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        self.raw_forwarded.borrow_mut().push((keycode, value));
        if value == 1 {
            self.held_keys.borrow_mut().insert(keycode);
        } else if value == 0 {
            self.held_keys.borrow_mut().remove(&keycode);
        }

        if self.bypass_mode.get() {
            return InjectResult::Success;
        }

        if value == 1 || value == 2 {
            let held = self.held_keys.borrow();
            let ctrl = held.contains(&29) || held.contains(&97);
            let alt = held.contains(&56) || held.contains(&100);
            let meta = held.contains(&125) || held.contains(&126);
            let shift = held.contains(&42) || held.contains(&54);
            if ctrl || alt || meta {
                return InjectResult::Success;
            }
            if keycode == KeyCode::KEY_BACKSPACE.code() {
                if self.omnibox_selection_len.get() > 0 {
                    let sel = self.omnibox_selection_len.replace(0);
                    self.delete_after_cursor(sel);
                } else {
                    self.backspace_at_cursor(1);
                }
                return InjectResult::Success;
            }
            if keycode == KeyCode::KEY_DELETE.code() {
                if self.omnibox_selection_len.get() > 0 {
                    let sel = self.omnibox_selection_len.replace(0);
                    self.delete_after_cursor(sel);
                } else {
                    self.delete_after_cursor(1);
                }
                return InjectResult::Success;
            }
            if keycode == KeyCode::KEY_ENTER.code() || keycode == KeyCode::KEY_KPENTER.code() {
                if shift {
                    self.shift_enter_count.set(self.shift_enter_count.get() + 1);
                }
                self.omnibox_selection_len.set(0);
                self.insert_str_at_cursor("\n");
                return InjectResult::Success;
            }
            if let Some(ch) = key_to_char_with_case(KeyCode(keycode), shift, self.caps_lock.get()) {
                let sel = self.omnibox_selection_len.replace(0);
                if sel > 0 {
                    self.delete_after_cursor(sel);
                }
                let mut s = String::new();
                s.push(ch);
                self.insert_str_at_cursor(&s);
            }
        }
        InjectResult::Success
    }

    fn send_char(&self, ch: char) -> InjectResult {
        self.omnibox_selection_len.set(0);
        let mut s = String::new();
        s.push(ch);
        self.insert_str_at_cursor(&s);
        InjectResult::Success
    }

    fn send_string(&self, text: &str) -> InjectResult {
        self.omnibox_selection_len.set(0);
        self.insert_str_at_cursor(text);
        InjectResult::Success
    }

    fn send_backspace(&self) -> InjectResult {
        self.backspace_at_cursor(1);
        InjectResult::Success
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        if self.omnibox_shield_armed.replace(false) && self.omnibox_selection_len.get() > 0 {
            let sel = self.omnibox_selection_len.replace(0);
            self.delete_after_cursor(sel);
        } else if self.omnibox_selection_len.get() > 0 {
            // Simulate Chromium bug when shield is NOT armed: first backspace eats selection!
            let sel = self.omnibox_selection_len.replace(0);
            self.delete_after_cursor(sel);
            self.backspace_at_cursor(backspaces.saturating_sub(1));
            self.insert_str_at_cursor(text);
            return InjectResult::Success;
        }
        self.backspace_at_cursor(backspaces);
        self.insert_str_at_cursor(text);
        InjectResult::Success
    }

    fn set_caps_lock(&self, enabled: bool) -> InjectResult {
        self.caps_lock.set(enabled);
        InjectResult::Success
    }

    fn set_num_lock(&self, enabled: bool) -> InjectResult {
        self.num_lock.set(enabled);
        InjectResult::Success
    }

    fn set_terminal_app(&self, is_terminal: bool) -> InjectResult {
        self.terminal_app.set(is_terminal);
        InjectResult::Success
    }

    fn set_bypass_mode(&self, bypass: bool) -> InjectResult {
        self.bypass_mode.set(bypass);
        InjectResult::Success
    }

    fn set_active_window(&self, app_class: &str) -> InjectResult {
        *self.active_window.borrow_mut() = app_class.to_string();
        InjectResult::Success
    }

    fn set_omnibox_inline_selection(&self, active: bool) -> InjectResult {
        self.omnibox_shield_armed.set(active);
        InjectResult::Success
    }

    fn uses_virtual_keyboard(&self) -> bool {
        true
    }
}

struct QcHarness {
    _atspi_guard: std::sync::MutexGuard<'static, ()>,
    daemon: Daemon,
    client: QcVirtualClient,
    shared_focus: SharedFocusSnapshot,
    focus_sync: crate::focus_sync::FocusSyncState,
    key_state: AttributeSet<KeyCode>,
    consumed_keys: HashSet<u16>,
    caps_lock: bool,
    num_lock: bool,
}

impl QcHarness {
    fn new(mut config: Config) -> Self {
        let atspi_guard = crate::password_detector::atspi_test_lock();
        reset_atspi_focus_state();
        config.app_state.enabled = true;
        let engine_enabled = Arc::new(AtomicBool::new(config.start_enabled));
        let daemon = Daemon::new(
            config,
            PathBuf::from("/tmp/dboard-qc-nonexistent.toml"),
            engine_enabled,
        );
        let shared_focus = Arc::new(Mutex::new(FocusSnapshot::default()));
        let focus_sync =
            crate::focus_sync::FocusSyncState::from_initial(shared_focus.lock().unwrap().clone());
        Self {
            _atspi_guard: atspi_guard,
            daemon,
            client: QcVirtualClient::new(),
            shared_focus,
            focus_sync,
            key_state: AttributeSet::new(),
            consumed_keys: HashSet::new(),
            caps_lock: false,
            num_lock: false,
        }
    }

    fn focus_window(&mut self, identity: &str, app_class: &str, is_password: bool) {
        record_atspi_focus_state(is_password, Some(app_class.to_string()));
        {
            let mut snap = self.shared_focus.lock().unwrap();
            snap.observe(
                Some(format!("atspi:{identity}")),
                Some(app_class.to_string()),
            );
        }
        self.sync_focus_before_key();
    }

    fn sync_focus_before_key(&mut self) {
        self.focus_sync
            .sync_on_key_press(&mut self.daemon, &self.client, &self.shared_focus);
    }

    fn simulate_click_at(&mut self, cursor_chars: usize) {
        self.daemon.reset_composition_state();
        clear_omnibox_inline_selection(&self.client, &self.shared_focus);
        let text = self.client.text();
        let pos = cursor_chars.min(text.chars().count());
        self.client.cursor.set(pos);
        let byte_idx = text.char_indices().nth(pos).map_or(text.len(), |(i, _)| i);
        let (before, after) = text.split_at(byte_idx);
        let ctx = extract_surrounding_context(before, after);
        self.shared_focus
            .lock()
            .unwrap()
            .set_surrounding_context(ctx);
    }

    fn send_evdev(&mut self, key: KeyCode, value: i32) {
        let keycode = key.code();
        update_keyboard_state(
            &mut self.key_state,
            &mut self.caps_lock,
            &mut self.num_lock,
            key,
            value,
        );
        self.client.set_caps_lock(self.caps_lock);
        self.client.set_num_lock(self.num_lock);

        if value == 1 {
            self.sync_focus_before_key();
        }

        if self.daemon.is_current_app_bypassed() {
            self.client.set_bypass_mode(true);
            if consume_released_or_repeated_key(&mut self.consumed_keys, keycode, value) {
                return;
            }
            self.client.send_key_event(keycode, value);
            return;
        }

        if value == 1
            && is_toggle_key(key, &self.daemon.config.toggle_key)
            && is_toggle_combination_state(&self.key_state, &self.daemon.config.toggle_key)
        {
            self.consumed_keys.insert(keycode);
            clear_omnibox_inline_selection(&self.client, &self.shared_focus);
            self.daemon.toggle_method();
            return;
        }

        if !(value == 2 && key == KeyCode::KEY_BACKSPACE && !is_modifier_pressed(&self.key_state))
            && consume_released_or_repeated_key(&mut self.consumed_keys, keycode, value)
        {
            return;
        }

        if is_modifier_pressed(&self.key_state) {
            if value == 1
                && !matches!(
                    key,
                    KeyCode::KEY_LEFTCTRL
                        | KeyCode::KEY_RIGHTCTRL
                        | KeyCode::KEY_LEFTALT
                        | KeyCode::KEY_RIGHTALT
                        | KeyCode::KEY_LEFTMETA
                        | KeyCode::KEY_RIGHTMETA
                )
            {
                self.daemon.reset_composition_state();
                clear_omnibox_inline_selection(&self.client, &self.shared_focus);
            }
            self.client.send_key_event(keycode, value);
            return;
        }

        if !self.daemon.engine.is_enabled() {
            self.client.send_key_event(keycode, value);
            return;
        }

        if key == KeyCode::KEY_BACKSPACE {
            if value == 1 || value == 2 {
                crate::focus::clear_surrounding_word(&self.shared_focus);
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key('\x08');
                if commands.is_empty() {
                    clear_omnibox_inline_selection(&self.client, &self.shared_focus);
                    self.client.send_key_event(14, 1);
                    self.client.send_key_event(14, 0);
                } else {
                    if buf_before > 0 && take_omnibox_inline_selection(&self.shared_focus) {
                        self.client.set_omnibox_inline_selection(true);
                    }
                    execute_commands(
                        &self.client,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_sync.last_focus_generation,
                    );
                }
            }
            if value == 0 {
                self.consumed_keys.remove(&keycode);
            } else {
                self.consumed_keys.insert(keycode);
            }
            return;
        }

        if value == 1 {
            self.consumed_keys.remove(&keycode);
            if let Some(ch) =
                key_to_char_with_case(key, is_modifier_held_shift(&self.key_state), self.caps_lock)
            {
                maybe_seed_surrounding_word(
                    &mut self.daemon,
                    &self.shared_focus,
                    self.focus_sync.last_focus_generation,
                    ch,
                );
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key(ch);
                if !commands.is_empty() {
                    self.consumed_keys.insert(keycode);
                    if buf_before > 0 && take_omnibox_inline_selection(&self.shared_focus) {
                        self.client.set_omnibox_inline_selection(true);
                    } else {
                        clear_omnibox_inline_selection(&self.client, &self.shared_focus);
                    }
                    execute_commands(
                        &self.client,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_sync.last_focus_generation,
                    );
                    if key == KeyCode::KEY_SPACE
                        && !is_modifier_held_shift(&self.key_state)
                        && self.daemon.engine.is_enabled()
                    {
                        let _ = self.client.send_char(' ');
                        self.consumed_keys.insert(keycode);
                    } else if is_flush_char(ch) && self.daemon.engine.is_enabled() {
                        self.client.send_key_event(keycode, 1);
                        self.client.send_key_event(keycode, 0);
                        self.consumed_keys.insert(keycode);
                    }
                } else if self.daemon.engine.is_enabled()
                    && is_vn_control_key(self.daemon.app_state.effective_method(), ch)
                    && self.daemon.engine.buffer().chars().count() <= buf_before
                    && !self.daemon.engine.buffer().ends_with(ch)
                {
                    self.consumed_keys.insert(keycode);
                } else {
                    clear_omnibox_inline_selection(&self.client, &self.shared_focus);
                    if key == KeyCode::KEY_SPACE
                        && !is_modifier_held_shift(&self.key_state)
                        && self.daemon.engine.is_enabled()
                    {
                        let _ = self.client.send_char(' ');
                        self.consumed_keys.insert(keycode);
                    } else {
                        self.client.send_key_event(keycode, 1);
                    }
                }
            } else {
                if is_navigation_or_control_key(key) {
                    self.daemon.reset_composition_state();
                }
                self.client.send_key_event(keycode, 1);
            }
        } else if value == 0 {
            self.client.send_key_event(keycode, 0);
        }
    }

    fn tap_key(&mut self, key: KeyCode) {
        self.send_evdev(key, 1);
        self.send_evdev(key, 0);
    }

    fn type_ascii(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\x08' {
                self.tap_key(KeyCode::KEY_BACKSPACE);
                continue;
            }
            let (key, need_shift) = char_to_evdev(ch);
            if need_shift {
                self.send_evdev(KeyCode::KEY_LEFTSHIFT, 1);
            }
            self.tap_key(key);
            if need_shift {
                self.send_evdev(KeyCode::KEY_LEFTSHIFT, 0);
            }
        }
    }
}

fn clear_omnibox_inline_selection(client: &QcVirtualClient, shared: &SharedFocusSnapshot) {
    crate::focus::clear_omnibox_inline_selection(shared);
    client.set_omnibox_inline_selection(false);
}

fn char_to_evdev(ch: char) -> (KeyCode, bool) {
    match ch {
        'a'..='z' => (alpha_key(ch), false),
        'A'..='Z' => (alpha_key(ch.to_ascii_lowercase()), true),
        '0' => (KeyCode::KEY_0, false),
        '1' => (KeyCode::KEY_1, false),
        '2' => (KeyCode::KEY_2, false),
        '3' => (KeyCode::KEY_3, false),
        '4' => (KeyCode::KEY_4, false),
        '5' => (KeyCode::KEY_5, false),
        '6' => (KeyCode::KEY_6, false),
        '7' => (KeyCode::KEY_7, false),
        '8' => (KeyCode::KEY_8, false),
        '9' => (KeyCode::KEY_9, false),
        ' ' => (KeyCode::KEY_SPACE, false),
        '\n' => (KeyCode::KEY_ENTER, false),
        '.' => (KeyCode::KEY_DOT, false),
        ',' => (KeyCode::KEY_COMMA, false),
        '!' => (KeyCode::KEY_1, true),
        '?' => (KeyCode::KEY_SLASH, true),
        '/' => (KeyCode::KEY_SLASH, false),
        ':' => (KeyCode::KEY_SEMICOLON, true),
        ';' => (KeyCode::KEY_SEMICOLON, false),
        '_' => (KeyCode::KEY_MINUS, true),
        '-' => (KeyCode::KEY_MINUS, false),
        '(' => (KeyCode::KEY_9, true),
        ')' => (KeyCode::KEY_0, true),
        '"' => (KeyCode::KEY_APOSTROPHE, true),
        '\'' => (KeyCode::KEY_APOSTROPHE, false),
        _ => panic!("unsupported test char {ch:?}"),
    }
}

fn alpha_key(lower: char) -> KeyCode {
    match lower {
        'a' => KeyCode::KEY_A,
        'b' => KeyCode::KEY_B,
        'c' => KeyCode::KEY_C,
        'd' => KeyCode::KEY_D,
        'e' => KeyCode::KEY_E,
        'f' => KeyCode::KEY_F,
        'g' => KeyCode::KEY_G,
        'h' => KeyCode::KEY_H,
        'i' => KeyCode::KEY_I,
        'j' => KeyCode::KEY_J,
        'k' => KeyCode::KEY_K,
        'l' => KeyCode::KEY_L,
        'm' => KeyCode::KEY_M,
        'n' => KeyCode::KEY_N,
        'o' => KeyCode::KEY_O,
        'p' => KeyCode::KEY_P,
        'q' => KeyCode::KEY_Q,
        'r' => KeyCode::KEY_R,
        's' => KeyCode::KEY_S,
        't' => KeyCode::KEY_T,
        'u' => KeyCode::KEY_U,
        'v' => KeyCode::KEY_V,
        'w' => KeyCode::KEY_W,
        'x' => KeyCode::KEY_X,
        'y' => KeyCode::KEY_Y,
        'z' => KeyCode::KEY_Z,
        _ => unreachable!(),
    }
}

#[test]
fn qc_env1_chromium_omnibox_autocomplete_and_mid_sentence_click() {
    let mut cfg = Config {
        autocorrect: true,
        auto_capitalize: true,
        ..Default::default()
    };
    cfg.input_method = "telex".into();
    let mut h = QcHarness::new(cfg);

    // 1. Chrome Omnibox with inline autocomplete suggestion `[ttps://google.com]`
    h.focus_window("chrome-omnibox", "google-chrome", false);
    h.type_ascii("ho");
    // Browser inserts inline suggestion after caret: `ho[ttps://google.com]`
    h.client.insert_str_at_cursor("ttps://google.com");
    h.client.cursor.set(2);
    h.client
        .omnibox_selection_len
        .set("ttps://google.com".len());
    h.shared_focus
        .lock()
        .unwrap()
        .set_omnibox_inline_selection(true);
    h.type_ascii("af ");
    assert_eq!(
        h.client.text(),
        "hòa ",
        "Chromium Omnibox inline suggestion must be cleared cleanly before replacing `ho` -> `hòa`"
    );

    // 2. Chrome Web Content: Space key overlap (`Space` pressed, next char pressed BEFORE Space released!)
    h.client.clear_screen();
    h.focus_window("chrome-docs", "google-chrome", false);
    h.type_ascii("ma");
    h.type_ascii("yf");
    // Overlap: Space Down -> 'v' Down -> Space Up -> 'v' Up
    h.send_evdev(KeyCode::KEY_SPACE, 1);
    h.send_evdev(KeyCode::KEY_V, 1);
    h.send_evdev(KeyCode::KEY_SPACE, 0);
    h.send_evdev(KeyCode::KEY_V, 0);
    h.type_ascii("uwaf suwar ");
    assert_eq!(
        h.client.text(),
        "mày vừa sửa ",
        "Key overlap on Space must never glue words together in Chromium"
    );

    // 3. Click mid-sentence in Chrome Google Docs behind `"chao"` in `"xin chao cac ban"`
    h.client.set_screen_and_cursor("xin chao cac ban", 8);
    h.simulate_click_at(8);
    h.type_ascii("f");
    assert_eq!(
        h.client.text(),
        "xin chào cac ban",
        "Clicking behind word mid-sentence must rehydrate and add tone without touching suffix"
    );

    // 4. Click mid-word `"vi|et"` must NOT corrupt `"viet"`
    h.client.set_screen_and_cursor("viet", 2);
    h.simulate_click_at(2);
    h.type_ascii("x");
    assert_eq!(h.client.text(), "vixet");
}

#[test]
fn qc_env2_electron_vscode_discord_shortcuts_and_shift_enter() {
    let cfg = Config {
        autocorrect: true,
        auto_capitalize: true,
        ..Default::default()
    };
    let mut h = QcHarness::new(cfg);

    // 1. VS Code (Electron): starts in English (`english_apps`)
    h.focus_window("vscode-1", "code", false);
    h.type_ascii("test ");
    assert_eq!(h.client.text(), "test ");

    // Ctrl+Shift+Space (IntelliSense Parameter Hints) must NOT toggle Dboard!
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 1);
    h.send_evdev(KeyCode::KEY_LEFTSHIFT, 1);
    h.tap_key(KeyCode::KEY_SPACE);
    h.send_evdev(KeyCode::KEY_LEFTSHIFT, 0);
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 0);
    assert!(!h.daemon.engine.is_enabled());

    // Ctrl+Space (1-tap toggle_modes = ["vn", "en"]) switches VS Code to VN!
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 1);
    h.tap_key(KeyCode::KEY_SPACE);
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 0);
    assert!(h.daemon.engine.is_enabled());

    h.client.clear_screen();
    h.type_ascii("(\"hoaf\") foo_bar google.com v1.0 ");
    assert_eq!(
        h.client.text(),
        "(\"hòa\") foo_bar google.com v1.0 ",
        "VS Code brackets, snake_case, URLs and version numbers must stay intact"
    );

    // 2. Discord (Electron): Shift+Enter must preserve Shift modifier and NOT drop it
    h.client.clear_screen();
    h.focus_window("discord-1", "discord", false);
    h.type_ascii("xin chaof");
    h.send_evdev(KeyCode::KEY_LEFTSHIFT, 1);
    h.tap_key(KeyCode::KEY_ENTER);
    h.send_evdev(KeyCode::KEY_LEFTSHIFT, 0);
    h.type_ascii("tienegs vieetj!");
    assert_eq!(h.client.shift_enter_count.get(), 1);
    assert_eq!(h.client.text(), "xin chào\nTiếng việt!");

    // Returning to VS Code must preserve the user's explicit Telex toggle
    // instead of reverting back to English via `english_apps`.
    h.focus_window("vscode-1", "code", false);
    assert!(
        h.daemon.engine.is_enabled(),
        "VS Code must remember user's explicit Telex override after focus switch"
    );
}

#[test]
fn qc_env3_qt_capslock_shift_numlock_and_autocorrect_undo() {
    let cfg = Config {
        autocorrect: true,
        auto_capitalize: true,
        toggle_key: "capslock".into(),
        ..Default::default()
    };
    let mut h = QcHarness::new(cfg);
    h.focus_window("qt-editor", "kate", false);

    // 1. Ctrl+CapsLock toggle must NEVER flip CapsLock state
    assert!(!h.caps_lock);
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 1);
    h.tap_key(KeyCode::KEY_CAPSLOCK);
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 0);
    assert!(!h.caps_lock, "Ctrl+CapsLock toggle must not flip CapsLock");
    // Toggle back to VN
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 1);
    h.tap_key(KeyCode::KEY_CAPSLOCK);
    h.send_evdev(KeyCode::KEY_LEFTCTRL, 0);
    assert!(h.daemon.engine.is_enabled());

    // 2. Bare CapsLock + NumLock work normally
    h.tap_key(KeyCode::KEY_NUMLOCK);
    assert!(h.num_lock);
    h.tap_key(KeyCode::KEY_CAPSLOCK);
    assert!(h.caps_lock);
    h.type_ascii("vieetj nam ");
    assert_eq!(h.client.text(), "VIỆT NAM ");
    h.tap_key(KeyCode::KEY_CAPSLOCK);
    assert!(!h.caps_lock);

    // 3. Autocorrect + One-Tap Undo + Personal Dict persistence in-session
    h.client.clear_screen();
    h.type_ascii("teh \x08 ");
    assert_eq!(h.client.text(), "teh ");
    // Typing `teh ` again must NOT autocorrect anymore because UserDict learned `keep_raw:teh`
    h.type_ascii("teh ");
    assert_eq!(h.client.text(), "teh teh ");
}

#[test]
fn qc_env4_terminal_password_and_bypass_isolation() {
    let cfg = Config {
        autocorrect: true,
        auto_capitalize: true,
        ..Default::default()
    };
    let mut h = QcHarness::new(cfg);

    // 1. COSMIC Terminal: auto_capitalize MUST be disabled, method switches to terminal_input_method ("vni")
    h.focus_window("term-1", "cosmic-term", false);
    assert_eq!(h.daemon.app_state.effective_method(), "vni");
    h.type_ascii("echo ok. tie61ng ");
    assert_eq!(
        h.client.text(),
        "echo ok. tiếng ",
        "Terminal must disable auto_capitalize and use VNI"
    );

    // 2. Password field inside Chrome: 1st keystroke must disable IME & NEVER learn into UserDict
    h.client.clear_screen();
    h.focus_window("chrome-pw", "google-chrome", true);
    assert!(!h.daemon.engine.is_enabled());
    h.type_ascii("teh pasword");
    assert_eq!(h.client.text(), "teh pasword");
    assert!(h.daemon.engine.take_user_dict_for_save().is_none());

    // Leaving password field inside Chrome re-enables VN
    h.client.clear_screen();
    h.focus_window("chrome-normal", "google-chrome", false);
    assert!(h.daemon.engine.is_enabled());
    h.type_ascii("xin chaof ");
    assert_eq!(h.client.text(), "xin chào ");

    // 3. Bypass app (steam): raw keys pass through without composition
    h.client.clear_screen();
    h.client.raw_forwarded.borrow_mut().clear();
    h.focus_window("steam-game", "steam", false);
    assert!(h.daemon.is_current_app_bypassed());
    h.type_ascii("wasd ");
    let forwarded: Vec<(u16, i32)> = h.client.raw_forwarded.borrow().clone();
    assert_eq!(
        forwarded,
        vec![
            (KeyCode::KEY_W.code(), 1),
            (KeyCode::KEY_W.code(), 0),
            (KeyCode::KEY_A.code(), 1),
            (KeyCode::KEY_A.code(), 0),
            (KeyCode::KEY_S.code(), 1),
            (KeyCode::KEY_S.code(), 0),
            (KeyCode::KEY_D.code(), 1),
            (KeyCode::KEY_D.code(), 0),
            (KeyCode::KEY_SPACE.code(), 1),
            (KeyCode::KEY_SPACE.code(), 0),
        ]
    );
}

#[test]
fn qc_chaos_multi_app_focus_thrash_and_mixed_input_stress() {
    let cfg = Config {
        autocorrect: true,
        auto_capitalize: true,
        ..Default::default()
    };
    let mut h = QcHarness::new(cfg);
    let apps = [
        ("chrome-1", "google-chrome", false),
        ("vscode-1", "code", false),
        ("discord-1", "discord", false),
        ("qt-1", "kate", false),
        ("term-1", "cosmic-term", false),
        ("pw-1", "google-chrome", true),
        ("game-1", "steam", false),
    ];

    let snippets = [
        "xin chaof! hoom nay ",
        "tienegs vieetj ",
        "it has been ",
        "hoaf \x08n ",
        "teh \x08 ",
        "google.com v1.0 ",
        "vieetj\x08c ",
    ];

    let mut seed: u64 = 0x5EED_C0DE_2026;
    let mut next_rand = || -> usize {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 32) as usize
    };

    for step in 0..300 {
        let (id, cls, is_pw) = apps[next_rand() % apps.len()];
        h.client.clear_screen();
        h.focus_window(id, cls, is_pw);

        let snippet = snippets[next_rand() % snippets.len()];
        // Occasionally cut off mid-word and switch focus (worst-case user Alt-Tab)
        let cut = if step % 3 == 0 {
            snippet.chars().count().min(4)
        } else {
            snippet.chars().count()
        };
        let partial: String = snippet.chars().take(cut).collect();
        h.type_ascii(&partial);

        // Invariant: engine buffer never exceeds 64 chars
        assert!(h.daemon.engine.buffer().chars().count() <= 64);
        // Invariant: password field & bypass app never leave composed non-ASCII output
        if is_pw || cls == "steam" {
            assert!(
                h.client.text().is_ascii(),
                "Password/bypass app `{cls}` received non-ASCII composition: {:?}",
                h.client.text()
            );
        }
        // Invariant: no Shift key left stuck after typing
        assert!(!is_modifier_held_shift(&h.key_state));
    }
}
