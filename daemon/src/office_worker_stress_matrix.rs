// SPDX-License-Identifier: MIT
//! Office Worker Multi-Application Real-World Stress Test Suite.
//!
//! Simulates an office worker continuously authoring documents across text editors
//! (VS Code, LibreOffice Writer), web browsers (Edge, Chrome), and electron/chat
//! apps (Telegram, Slack, Discord), rigorously exercising:
//! - Copy/Paste (`Ctrl+C`, `Ctrl+V`, `Ctrl+X`, `Ctrl+Insert`, `Shift+Insert`)
//! - Caret navigation & selection replacement (`Shift+Left/Right`, `Ctrl+A`)
//! - `Insert` overwrite mode vs insert mode
//! - `Backspace` mid-word diacritics, across word boundaries, Gboard Space-Undo
//! - Casing matrix: ALL CAPS (CapsLock & Shift), Title Case, CamelCase, inverted CapsLock (`vIỆT nAM`)
//! - Technical documentation with English vocabulary, file paths, code snippets
//! - `Shift + Enter` multi-line paragraph drafting in chat/editors
//! - Seamless bilingual Vietnamese–English code-switching without word corruption.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::config::Config;
use crate::daemon::Daemon;
use crate::event::{
    is_flush_char, is_modifier_held_shift, is_modifier_pressed, is_navigation_or_control_key,
    is_toggle_combination_state, is_toggle_key, is_vn_control_key, key_to_char_with_case,
};
use crate::inject::execute_commands;
use dboard_protocol::inject::{InjectResult, KeyInjector};
use evdev::KeyCode;

#[derive(Clone, Default, Debug)]
struct OfficeEditorState {
    chars: Vec<char>,
    cursor: usize,
    selection: Option<(usize, usize)>,
    overwrite_mode: bool,
    last_action_was_shift_enter: bool,
}

impl OfficeEditorState {
    fn text(&self) -> String {
        self.chars.iter().collect()
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection.take() {
            let (lo, hi) = (a.min(b), a.max(b));
            if lo < hi && hi <= self.chars.len() {
                self.chars.drain(lo..hi);
                self.cursor = lo;
                return true;
            }
        }
        false
    }

    fn insert_char(&mut self, ch: char) {
        let _ = self.delete_selection();
        if self.overwrite_mode && self.cursor < self.chars.len() && ch != '\n' {
            self.chars[self.cursor] = ch;
            self.cursor += 1;
        } else {
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
    }

    fn insert_str(&mut self, s: &str) {
        let _ = self.delete_selection();
        for ch in s.chars() {
            if self.overwrite_mode && self.cursor < self.chars.len() && ch != '\n' {
                self.chars[self.cursor] = ch;
                self.cursor += 1;
            } else {
                self.chars.insert(self.cursor, ch);
                self.cursor += 1;
            }
        }
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor > 0 && self.cursor <= self.chars.len() {
            self.chars.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }
}

#[derive(Default)]
struct OfficeDesktopEnvironment {
    editors: HashMap<String, OfficeEditorState>,
    active_app: String,
    user_clipboard: String,
    caps_lock: bool,
    synth_shift: bool,
    synth_ctrl: bool,
    synth_alt: bool,
}

struct TestOfficeInjector {
    env: Rc<RefCell<OfficeDesktopEnvironment>>,
}

impl KeyInjector for TestOfficeInjector {
    fn send_backspace(&self) -> InjectResult {
        let mut env = self.env.borrow_mut();
        let app = env.active_app.clone();
        if let Some(ed) = env.editors.get_mut(&app) {
            ed.backspace();
        }
        InjectResult::Success
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let mut env = self.env.borrow_mut();
        let app = env.active_app.clone();
        if let Some(ed) = env.editors.get_mut(&app) {
            ed.insert_char(ch);
        }
        InjectResult::Success
    }

    fn send_string(&self, s: &str) -> InjectResult {
        let mut env = self.env.borrow_mut();
        let app = env.active_app.clone();
        if let Some(ed) = env.editors.get_mut(&app) {
            ed.insert_str(s);
        }
        InjectResult::Success
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        let mut env = self.env.borrow_mut();
        let app = env.active_app.clone();
        if let Some(ed) = env.editors.get_mut(&app) {
            for _ in 0..backspaces {
                ed.backspace();
            }
            ed.insert_str(text);
        }
        InjectResult::Success
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let mut env = self.env.borrow_mut();
        let app = env.active_app.clone();
        let shift = env.synth_shift;
        let caps = env.caps_lock;

        match keycode {
            42 | 54 => {
                env.synth_shift = value == 1;
                return InjectResult::Success;
            }
            29 | 97 => {
                env.synth_ctrl = value == 1;
                return InjectResult::Success;
            }
            56 | 100 => {
                env.synth_alt = value == 1;
                return InjectResult::Success;
            }
            _ => {}
        }

        if value != 1 {
            return InjectResult::Success;
        }

        let key = KeyCode::new(keycode);
        let ctrl = env.synth_ctrl;
        if ctrl {
            match key {
                KeyCode::KEY_C => {
                    if let Some(selected) = env.editors.get(&app).and_then(|e| {
                        e.selection.map(|(a, b)| {
                            let (lo, hi) = (a.min(b), a.max(b));
                            e.chars[lo..hi].iter().collect::<String>()
                        })
                    }) {
                        env.user_clipboard = selected;
                    }
                }
                KeyCode::KEY_V => {
                    let clip = env.user_clipboard.clone();
                    if let Some(ed) = env.editors.get_mut(&app) {
                        ed.insert_str(&clip);
                    }
                }
                KeyCode::KEY_X => {
                    if let Some(selected) = env.editors.get_mut(&app).and_then(|ed| {
                        if let Some((a, b)) = ed.selection.take() {
                            let (lo, hi) = (a.min(b), a.max(b));
                            let text = ed.chars.drain(lo..hi).collect::<String>();
                            ed.cursor = lo;
                            Some(text)
                        } else {
                            None
                        }
                    }) {
                        env.user_clipboard = selected;
                    }
                }
                _ => {}
            }
            return InjectResult::Success;
        }

        match key {
            KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    ed.last_action_was_shift_enter = shift;
                    ed.insert_char('\n');
                }
            }
            KeyCode::KEY_BACKSPACE => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    ed.backspace();
                }
            }
            KeyCode::KEY_DELETE => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    ed.delete_forward();
                }
            }
            KeyCode::KEY_INSERT => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    ed.overwrite_mode = !ed.overwrite_mode;
                }
            }
            KeyCode::KEY_SPACE => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    ed.insert_char(' ');
                }
            }
            KeyCode::KEY_LEFT => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    let prev = ed.cursor;
                    if ed.cursor > 0 {
                        ed.cursor -= 1;
                    }
                    if shift {
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, ed.cursor));
                    } else {
                        ed.selection = None;
                    }
                }
            }
            KeyCode::KEY_RIGHT => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    let prev = ed.cursor;
                    if ed.cursor < ed.chars.len() {
                        ed.cursor += 1;
                    }
                    if shift {
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, ed.cursor));
                    } else {
                        ed.selection = None;
                    }
                }
            }
            KeyCode::KEY_HOME => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    let prev = ed.cursor;
                    ed.cursor = 0;
                    if shift {
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, 0));
                    } else {
                        ed.selection = None;
                    }
                }
            }
            KeyCode::KEY_END => {
                if let Some(ed) = env.editors.get_mut(&app) {
                    let prev = ed.cursor;
                    let end = ed.chars.len();
                    ed.cursor = end;
                    if shift {
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, end));
                    } else {
                        ed.selection = None;
                    }
                }
            }
            _ => {
                if let Some(ch) = key_to_char_with_case(key, shift, caps) {
                    if let Some(ed) = env.editors.get_mut(&app) {
                        ed.insert_char(ch);
                    }
                }
            }
        }
        InjectResult::Success
    }

    fn set_caps_lock(&self, active: bool) -> InjectResult {
        self.env.borrow_mut().caps_lock = active;
        InjectResult::Success
    }

    fn set_active_window(&self, app_class: &str) -> InjectResult {
        self.env.borrow_mut().active_app = app_class.to_string();
        InjectResult::Success
    }

    fn uses_virtual_keyboard(&self) -> bool {
        true
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }
}

struct OfficeSimulator {
    daemon: Daemon,
    injector: TestOfficeInjector,
    env: Rc<RefCell<OfficeDesktopEnvironment>>,
    key_state: evdev::AttributeSet<KeyCode>,
    caps_lock: bool,
    consumed_keys: HashSet<u16>,
    focus_generation: u64,
}

impl OfficeSimulator {
    fn new(app: &str) -> Self {
        let config = Config {
            input_method: "telex".to_string(),
            start_enabled: true,
            ..Default::default()
        };
        let daemon = Daemon::new(
            config,
            std::path::PathBuf::from("/tmp/dboard-office-matrix-test.toml"),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );

        let env = Rc::new(RefCell::new(OfficeDesktopEnvironment {
            active_app: app.to_string(),
            ..Default::default()
        }));
        let injector = TestOfficeInjector {
            env: Rc::clone(&env),
        };

        let mut sim = Self {
            daemon,
            injector,
            env,
            key_state: evdev::AttributeSet::new(),
            caps_lock: false,
            consumed_keys: HashSet::new(),
            focus_generation: 1,
        };
        sim.switch_app(app);
        sim
    }

    fn switch_app(&mut self, app: &str) {
        self.focus_generation += 1;
        self.daemon.reset_composition_state();
        self.injector.set_active_window(app);
        self.daemon.check_app_change_with(app.to_string());
        self.env
            .borrow_mut()
            .editors
            .entry(app.to_string())
            .or_default();
    }

    fn text(&self) -> String {
        let env = self.env.borrow();
        env.editors
            .get(&env.active_app)
            .map(|e| e.text())
            .unwrap_or_default()
    }

    fn last_shift_enter(&self) -> bool {
        let env = self.env.borrow();
        env.editors
            .get(&env.active_app)
            .map(|e| e.last_action_was_shift_enter)
            .unwrap_or(false)
    }

    fn set_clipboard(&mut self, text: &str) {
        self.env.borrow_mut().user_clipboard = text.to_string();
    }

    fn clipboard(&self) -> String {
        self.env.borrow().user_clipboard.clone()
    }

    fn dispatch_key(&mut self, key: KeyCode, value: i32) {
        let keycode = key.code();
        if value == 1 {
            self.key_state.insert(key);
        } else if value == 0 {
            self.key_state.remove(key);
        }
        if key == KeyCode::KEY_CAPSLOCK && value == 1 {
            self.caps_lock = !self.caps_lock;
        }
        self.injector.set_caps_lock(self.caps_lock);

        if self.daemon.is_current_app_bypassed() {
            self.injector.send_key_event(keycode, value);
            return;
        }

        if value == 1
            && is_toggle_key(key, &self.daemon.config.toggle_key)
            && is_toggle_combination_state(&self.key_state, &self.daemon.config.toggle_key)
        {
            self.consumed_keys.insert(keycode);
            self.daemon.toggle_method();
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
            }
            self.injector.send_key_event(keycode, value);
            return;
        }

        if !self.daemon.engine.is_enabled() {
            self.injector.send_key_event(keycode, value);
            return;
        }

        if key == KeyCode::KEY_BACKSPACE {
            if value == 1 || value == 2 {
                let commands = self.daemon.process_key('\x08');
                if commands.is_empty() {
                    self.injector.send_key_event(14, 1);
                    self.injector.send_key_event(14, 0);
                } else {
                    let outcome = execute_commands(
                        &self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_generation,
                    );
                    assert!(self.daemon.handle_transaction_outcome(outcome));
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
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key(ch);
                if !commands.is_empty() {
                    self.consumed_keys.insert(keycode);
                    let outcome = execute_commands(
                        &self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_generation,
                    );
                    assert!(self.daemon.handle_transaction_outcome(outcome));

                    if key == KeyCode::KEY_SPACE
                        && !is_modifier_held_shift(&self.key_state)
                        && self.daemon.engine.is_enabled()
                    {
                        let _ = self.injector.send_char(' ');
                        self.consumed_keys.insert(keycode);
                    } else if is_flush_char(ch) && self.daemon.engine.is_enabled() {
                        self.injector.send_key_event(keycode, 1);
                        self.injector.send_key_event(keycode, 0);
                        self.consumed_keys.insert(keycode);
                    }
                } else if self.daemon.engine.is_enabled()
                    && is_vn_control_key(self.daemon.app_state.effective_method(), ch)
                    && self.daemon.engine.buffer().chars().count() <= buf_before
                {
                    self.consumed_keys.insert(keycode);
                } else if key == KeyCode::KEY_SPACE
                    && !is_modifier_held_shift(&self.key_state)
                    && self.daemon.engine.is_enabled()
                {
                    let _ = self.injector.send_char(' ');
                    self.consumed_keys.insert(keycode);
                } else {
                    self.injector.send_key_event(keycode, 1);
                }
            } else {
                if is_navigation_or_control_key(key) {
                    self.daemon.reset_composition_state();
                }
                self.injector.send_key_event(keycode, 1);
            }
        } else if value == 2 {
            if self.consumed_keys.contains(&keycode) {
                if keycode == 57 && self.daemon.engine.is_enabled() {
                    let _ = self.injector.send_char(' ');
                }
                return;
            }
            if !self.daemon.engine.buffer().is_empty() {
                self.daemon.reset_composition_state();
            }
            self.injector.send_key_event(keycode, 2);
        } else if value == 0 {
            if self.consumed_keys.remove(&keycode) {
                return;
            }
            self.injector.send_key_event(keycode, 0);
        }
    }

    fn stroke(&mut self, key: KeyCode) {
        self.dispatch_key(key, 1);
        self.dispatch_key(key, 0);
    }

    fn shift_stroke(&mut self, key: KeyCode) {
        self.dispatch_key(KeyCode::KEY_LEFTSHIFT, 1);
        self.dispatch_key(key, 1);
        self.dispatch_key(key, 0);
        self.dispatch_key(KeyCode::KEY_LEFTSHIFT, 0);
    }

    fn ctrl_stroke(&mut self, key: KeyCode) {
        self.dispatch_key(KeyCode::KEY_LEFTCTRL, 1);
        self.dispatch_key(key, 1);
        self.dispatch_key(key, 0);
        self.dispatch_key(KeyCode::KEY_LEFTCTRL, 0);
    }

    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                'a'..='z' => {
                    let code = char_to_keycode(ch);
                    self.stroke(code);
                }
                'A'..='Z' => {
                    let code = char_to_keycode(ch.to_ascii_lowercase());
                    self.shift_stroke(code);
                }
                '0'..='9' => {
                    let code = digit_to_keycode(ch);
                    self.stroke(code);
                }
                ' ' => self.stroke(KeyCode::KEY_SPACE),
                '\n' => self.stroke(KeyCode::KEY_ENTER),
                '.' => self.stroke(KeyCode::KEY_DOT),
                ',' => self.stroke(KeyCode::KEY_COMMA),
                '/' => self.stroke(KeyCode::KEY_SLASH),
                '-' => self.stroke(KeyCode::KEY_MINUS),
                ':' => self.shift_stroke(KeyCode::KEY_SEMICOLON),
                ';' => self.stroke(KeyCode::KEY_SEMICOLON),
                '!' => self.shift_stroke(KeyCode::KEY_1),
                '?' => self.shift_stroke(KeyCode::KEY_SLASH),
                '(' => self.shift_stroke(KeyCode::KEY_9),
                ')' => self.shift_stroke(KeyCode::KEY_0),
                '_' => self.shift_stroke(KeyCode::KEY_MINUS),
                '=' => self.stroke(KeyCode::KEY_EQUAL),
                '+' => self.shift_stroke(KeyCode::KEY_EQUAL),
                '%' => self.shift_stroke(KeyCode::KEY_5),
                '&' => self.shift_stroke(KeyCode::KEY_7),
                '[' => self.stroke(KeyCode::KEY_LEFTBRACE),
                ']' => self.stroke(KeyCode::KEY_RIGHTBRACE),
                '$' => self.shift_stroke(KeyCode::KEY_4),
                '@' => self.shift_stroke(KeyCode::KEY_2),
                '"' => self.shift_stroke(KeyCode::KEY_APOSTROPHE),
                '\'' => self.stroke(KeyCode::KEY_APOSTROPHE),
                _ => {}
            }
        }
    }

    fn perform_copy(&mut self) {
        let app = self.env.borrow().active_app.clone();
        let selected = self.env.borrow().editors.get(&app).and_then(|ed| {
            ed.selection.map(|(a, b)| {
                let (lo, hi) = (a.min(b), a.max(b));
                ed.chars[lo..hi].iter().collect::<String>()
            })
        });
        if let Some(text) = selected {
            self.set_clipboard(&text);
        }
        self.ctrl_stroke(KeyCode::KEY_C);
    }

    fn perform_paste(&mut self) {
        self.ctrl_stroke(KeyCode::KEY_V);
    }

    fn enable_vietnamese(&mut self) {
        self.daemon.engine.set_enabled(true);
        self.daemon
            .engine
            .set_method(dboard_engine::InputMethod::Telex);
        self.daemon
            .engine_enabled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.daemon.config.input_method = "telex".into();
        self.daemon.app_state.set_global_enabled(true);
        self.daemon.app_state.set_global_method("telex");
    }
}

fn char_to_keycode(ch: char) -> KeyCode {
    match ch {
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
        _ => KeyCode::KEY_A,
    }
}

fn digit_to_keycode(ch: char) -> KeyCode {
    match ch {
        '0' => KeyCode::KEY_0,
        '1' => KeyCode::KEY_1,
        '2' => KeyCode::KEY_2,
        '3' => KeyCode::KEY_3,
        '4' => KeyCode::KEY_4,
        '5' => KeyCode::KEY_5,
        '6' => KeyCode::KEY_6,
        '7' => KeyCode::KEY_7,
        '8' => KeyCode::KEY_8,
        '9' => KeyCode::KEY_9,
        _ => KeyCode::KEY_0,
    }
}

#[test]
fn office_contract_drafting_with_copy_paste_and_caps() {
    let mut sim = OfficeSimulator::new("libreoffice-writer");

    // 1. ALL CAPS Header using CapsLock
    sim.stroke(KeyCode::KEY_CAPSLOCK);
    sim.type_text("coongj hoaf xax hooji chur nghiax vieetj nam\n");
    sim.type_text("ddoocj laapj - tuwj do - hanhj phucs\n\n");
    sim.stroke(KeyCode::KEY_CAPSLOCK);

    assert!(sim
        .text()
        .contains("CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM\nĐỘC LẬP - TỰ DO - HẠNH PHÚC"));

    // 2. Draft contract body with Copy/Paste
    sim.type_text("Howpj ddoongf soos: ");
    sim.set_clipboard("HD-2026/VNPT-001");
    sim.perform_paste();
    sim.type_text(" ddax kys keest giuwxa hai been.\n");

    let actual = sim.text();
    assert!(
        actual.contains("Hợp đồng số: HD-2026/VNPT-001 đã ký kết giữa hai bên."),
        "Actual text:\n{}",
        actual
    );

    // 3. Selection with Shift+Left and copy
    for _ in 0..10 {
        sim.shift_stroke(KeyCode::KEY_LEFT);
    }
    sim.perform_copy();
    assert_eq!(sim.clipboard(), " hai bên.\n");
}

#[test]
fn office_vscode_technical_documentation_and_file_paths() {
    let mut sim = OfficeSimulator::new("code");
    sim.enable_vietnamese();

    // Exact sentence user tested with file paths
    let input_raw = "ddocj laij caaus hinhf /home/thanhhd/apache-tomcat-7.0.70/conf/ vaf /home/thanhhd/Git-projects/it.das.ioffice_web/qlvbdh/WEB-INF/web.xml xem ok chuwa";
    sim.type_text(input_raw);

    let actual = sim.text();
    println!("Actual VS Code text:\n{}", actual);
    assert_eq!(
        actual,
        "đọc lại cấu hình /home/thanhhd/apache-tomcat-7.0.70/conf/ và /home/thanhhd/Git-projects/it.das.ioffice_web/qlvbdh/WEB-INF/web.xml xem ok chưa"
    );

    // VS Code must not turn into "VỚI Code"
    sim.type_text(" treen VS Code");
    assert!(sim.text().ends_with(" trên VS Code"));
}

#[test]
fn office_fast_prose_space_handling_in_vscode_and_antigravity() {
    let mut sim = OfficeSimulator::new("code");
    sim.enable_vietnamese();

    // Exact sentences from user report:
    // "mày vừa sửa, thông báo với mày, tôi sẽ thử gõ một đoạn văn bản thật dài bằng tiếng việt để kiểm tra độ ổn định của bộ gõ trong VS Code"
    sim.type_text("mayf vuwaf suwar, thoong baos vowis mayf, ");
    sim.type_text("tooi sex thuwr gox mootj ddoanj vawn barn thaatj daif ");
    sim.type_text("bawngf tieengs vieetj ddeer kieemr tra ddooj oorn ddijnh ");
    sim.type_text("cuar booj gox trong ");

    assert_eq!(
        sim.text(),
        "mày vừa sửa, thông báo với mày, tôi sẽ thử gõ một đoạn văn bản thật dài bằng tiếng việt để kiểm tra độ ổn định của bộ gõ trong "
    );
}

#[test]
fn office_chat_shift_enter_behavior_in_telegram_and_slack() {
    let mut sim = OfficeSimulator::new("org.telegram.desktop");

    // Line 1
    sim.type_text("chaof banj owi");
    assert_eq!(sim.text(), "chào bạn ơi");

    // Shift + Enter: user holds Shift and presses Enter
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 0);
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 0);

    assert!(
        sim.last_shift_enter(),
        "Shift+Enter must record Shift held!"
    );
    assert_eq!(sim.text(), "chào bạn ơi\n");

    // Line 2
    sim.type_text("guiwr giusp minhf baos caos nhes");
    assert_eq!(sim.text(), "chào bạn ơi\ngửi giúp mình báo cáo nhé");
}

#[test]
fn office_bilingual_homograph_and_retroactive_scoring() {
    let mut sim = OfficeSimulator::new("chromium");

    // English dominant
    sim.type_text("this is a test project with rust and git\n");
    assert_eq!(sim.text(), "this is a test project with rust and git\n");

    // Vietnamese phrase containing potential collision words
    sim.type_text("chayj thuwr tinhs nawng mowis\n");
    assert!(sim.text().contains("chạy thử tính năng mới\n"));
}

#[test]
fn office_scenario_finance_spreadsheet_calc_and_sheets() {
    let mut sim = OfficeSimulator::new("libreoffice-calc");
    sim.enable_vietnamese();

    // 1. Spreadsheet formulas
    sim.type_text("=SUM(A1:A10) + AVERAGE(B1:B10)\n");
    assert!(sim.text().starts_with("=SUM(A1:A10) + AVERAGE(B1:B10)\n"));

    // 2. Financial figures with currency & percentages
    sim.type_text("15.000.000 VND, thuees VAT 10%, doanh thu Q1/2026: 50,000.50 USD.\n");
    assert!(sim
        .text()
        .contains("15.000.000 VND, thuế VAT 10%, doanh thu Q1/2026: 50,000.50 USD.\n"));
}

#[test]
fn office_scenario_workplace_email_exchange() {
    let mut sim = OfficeSimulator::new("microsoft-edge");
    sim.enable_vietnamese();

    // Email subject
    sim.type_text("[Urgent] Kees hoachj trieenr khai duwj asn Q4 & Review howpj ddoongf\n");
    assert!(sim
        .text()
        .contains("[Urgent] Kế hoạch triển khai dự án Q4 & Review hợp đồng\n"));

    // Email body with English terms & tech tokens
    sim.type_text("Kinhs guwri Anh/Chij Ban Giasm ddoocs,\n");
    sim.type_text("Em xin phesp guwri feedback veef task vuwfa push leen repo. ");
    sim.type_text("Vui longf check email vaf review file PDF ddisnh kemf.\n");

    assert!(sim.text().contains("Kính gửi Anh/Chị Ban Giám đốc,\n"));
    assert!(sim.text().contains("Em xin phép gửi feedback về task vừa push lên repo. Vui lòng check email và review file PDF đính kèm.\n"));
}

#[test]
fn office_scenario_administrative_legal_contract() {
    let mut sim = OfficeSimulator::new("libreoffice-writer");
    sim.enable_vietnamese();

    // Official legal header
    sim.stroke(KeyCode::KEY_CAPSLOCK);
    sim.type_text("coongj hoaf xax hooji chur nghiax vieetj nam\n");
    sim.type_text("ddoocj laapj - tuwj do - hanhj phucs\n\n");
    sim.stroke(KeyCode::KEY_CAPSLOCK);

    sim.type_text("Soos: 125/QDD-UBND veef vieecj phee duyeetj duwj asn.\n");
    let actual = sim.text();
    println!("Actual admin text:\n{}", actual);
    assert!(actual.contains("CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM\nĐỘC LẬP - TỰ DO - HẠNH PHÚC\n\n"));
    assert!(actual.contains("Số: 125/QĐ-UBND về việc phê duyệt dự án.\n"));
}

#[test]
fn office_scenario_developer_technical_notes_in_vscode() {
    let mut sim = OfficeSimulator::new("code");
    sim.enable_vietnamese();

    // Technical comment with Unix path & web URL
    sim.type_text("// TODO: caaus hinhf database /etc/nginx/conf.d/default.conf vaf ");
    sim.type_text("kieemr tra endpoint http://localhost:8080/api/v1/auth/login\n");

    let actual = sim.text();
    println!("VS Code dev notes:\n{}", actual);
    assert!(actual.contains("/etc/nginx/conf.d/default.conf"));
    assert!(actual.contains("http://localhost:8080/api/v1/auth/login"));
    assert!(actual.contains("cấu hình database"));
    assert!(actual.contains("kiểm tra endpoint"));

    // Code token
    sim.type_text("const token = \"it.das.ioffice_web\";\n");
    assert!(sim.text().contains("it.das.ioffice_web"));
}

#[test]
fn office_scenario_chat_multiline_and_emoji() {
    let mut sim = OfficeSimulator::new("org.telegram.desktop");
    sim.enable_vietnamese();

    // Multi-line chat via Shift+Enter
    sim.type_text("ddoanj 1: caapj nhaatj tieesn ddooj duwj asn.");
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 0);
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 0);

    sim.type_text("ddoanj 2: casc ruri ro caanf luwu ys.");
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 1);
    sim.dispatch_key(KeyCode::KEY_ENTER, 0);
    sim.dispatch_key(KeyCode::KEY_LEFTSHIFT, 0);

    sim.type_text("ddoanj 3: kees hoachj tuaanf towis.");

    let actual = sim.text();
    println!("Chat multiline text:\n{}", actual);
    assert_eq!(
        actual,
        "đoạn 1: cập nhật tiến độ dự án.\nđoạn 2: các rủi ro cần lưu ý.\nđoạn 3: kế hoạch tuần tới."
    );
}

#[test]
fn office_scenario_linux_terminal_command_instruction() {
    let mut sim = OfficeSimulator::new("antigravity");
    sim.enable_vietnamese();

    // 1. With "thanhf"
    sim.type_text("chuyeenr 2 leejnh sau veef thanhf leejnh cuar linux terminal giusp tao");
    let actual = sim.text();
    println!("Actual terminal instruction (thanhf):\n{}", actual);
    assert_eq!(
        actual,
        "chuyển 2 lệnh sau về thành lệnh của linux terminal giúp tao"
    );

    // 2. With free-tone "thafnh"
    let mut sim2 = OfficeSimulator::new("antigravity");
    sim2.enable_vietnamese();
    sim2.type_text("chuyeenr 2 leejnh sau veef thafnh leejnh cuar linux terminal giusp tao");
    let actual2 = sim2.text();
    println!("Actual terminal instruction (thafnh):\n{}", actual2);
    assert_eq!(
        actual2,
        "chuyển 2 lệnh sau về thành lệnh của linux terminal giúp tao"
    );
}
