// SPDX-License-Identifier: MIT
//! Adversarial Exhaustive Office Worker Stress Suite
//!
//! Simulates real-world office workers continuously creating and editing documents
//! with realistic physical key overlap, clipboard copy/paste, cut, Insert (overwrite),
//! Backspace cascades, CapsLock/Shift variations, bilingual code-switching, and
//! cross-application switching (VS Code, LibreOffice, Edge/Chrome, Telegram/Slack).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::config::Config;
use crate::daemon::Daemon;
use crate::event::{
    is_flush_char, is_modifier_held_shift, is_navigation_or_control_key, is_vn_control_key,
    key_to_char_with_case,
};
use crate::inject::execute_commands;
use evdev::KeyCode;
use vietc_protocol::inject::{InjectResult, KeyInjector};

#[derive(Clone, Default, Debug)]
struct OfficeDocumentBuffer {
    chars: Vec<char>,
    caret: usize,
    selection: Option<(usize, usize)>,
    overwrite: bool,
}

impl OfficeDocumentBuffer {
    fn text(&self) -> String {
        self.chars.iter().collect()
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection.take() {
            let (lo, hi) = (start.min(end), start.max(end));
            if lo < hi && hi <= self.chars.len() {
                self.chars.drain(lo..hi);
                self.caret = lo;
                return true;
            }
        }
        false
    }

    fn insert_char(&mut self, ch: char) {
        let _ = self.delete_selection();
        if self.overwrite && self.caret < self.chars.len() && ch != '\n' {
            self.chars[self.caret] = ch;
            self.caret += 1;
        } else {
            self.chars.insert(self.caret, ch);
            self.caret += 1;
        }
    }

    fn insert_str(&mut self, s: &str) {
        let _ = self.delete_selection();
        for ch in s.chars() {
            if self.overwrite && self.caret < self.chars.len() && ch != '\n' {
                self.chars[self.caret] = ch;
                self.caret += 1;
            } else {
                self.chars.insert(self.caret, ch);
                self.caret += 1;
            }
        }
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret > 0 && self.caret <= self.chars.len() {
            self.chars.remove(self.caret - 1);
            self.caret -= 1;
        }
    }

    fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret < self.chars.len() {
            self.chars.remove(self.caret);
        }
    }
}

#[derive(Default)]
struct VirtualDesktop {
    documents: HashMap<String, OfficeDocumentBuffer>,
    focused_app: String,
    system_clipboard: String,
    caps_lock_state: bool,
    shift_pressed: bool,
    ctrl_pressed: bool,
    alt_pressed: bool,
    /// Simulates hardware keyboard controller holding keycodes currently down
    held_down_keycodes: HashSet<u16>,
}

struct AdversarialTestInjector {
    desktop: Rc<RefCell<VirtualDesktop>>,
}

impl KeyInjector for AdversarialTestInjector {
    fn send_backspace(&self) -> InjectResult {
        let mut d = self.desktop.borrow_mut();
        let app = d.focused_app.clone();
        if let Some(doc) = d.documents.get_mut(&app) {
            doc.backspace();
        }
        InjectResult::Success
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let mut d = self.desktop.borrow_mut();
        let app = d.focused_app.clone();
        if let Some(doc) = d.documents.get_mut(&app) {
            doc.insert_char(ch);
        }
        InjectResult::Success
    }

    fn send_string(&self, s: &str) -> InjectResult {
        let mut d = self.desktop.borrow_mut();
        let app = d.focused_app.clone();
        if let Some(doc) = d.documents.get_mut(&app) {
            doc.insert_str(s);
        }
        InjectResult::Success
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        let mut d = self.desktop.borrow_mut();
        let app = d.focused_app.clone();
        if let Some(doc) = d.documents.get_mut(&app) {
            for _ in 0..backspaces {
                doc.backspace();
            }
            doc.insert_str(text);
        }
        InjectResult::Success
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let mut d = self.desktop.borrow_mut();
        let app = d.focused_app.clone();
        match keycode {
            42 | 54 => {
                d.shift_pressed = value == 1;
                return InjectResult::Success;
            }
            29 | 97 => {
                d.ctrl_pressed = value == 1;
                return InjectResult::Success;
            }
            56 | 100 => {
                d.alt_pressed = value == 1;
                return InjectResult::Success;
            }
            _ => {}
        }

        if value == 1 {
            d.held_down_keycodes.insert(keycode);
        } else if value == 0 {
            d.held_down_keycodes.remove(&keycode);
            return InjectResult::Success;
        }

        let key = KeyCode::new(keycode);
        if d.ctrl_pressed {
            match key {
                KeyCode::KEY_C => {
                    if let Some(sel) = d.documents.get(&app).and_then(|doc| {
                        doc.selection.map(|(a, b)| {
                            let (lo, hi) = (a.min(b), a.max(b));
                            doc.chars[lo..hi].iter().collect::<String>()
                        })
                    }) {
                        d.system_clipboard = sel;
                    }
                }
                KeyCode::KEY_V => {
                    let clip = d.system_clipboard.clone();
                    if let Some(doc) = d.documents.get_mut(&app) {
                        doc.insert_str(&clip);
                    }
                }
                KeyCode::KEY_X => {
                    if let Some(sel) = d.documents.get_mut(&app).and_then(|doc| {
                        if let Some((a, b)) = doc.selection.take() {
                            let (lo, hi) = (a.min(b), a.max(b));
                            let t = doc.chars.drain(lo..hi).collect::<String>();
                            doc.caret = lo;
                            Some(t)
                        } else {
                            None
                        }
                    }) {
                        d.system_clipboard = sel;
                    }
                }
                _ => {}
            }
            return InjectResult::Success;
        }

        match key {
            KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER => {
                if let Some(doc) = d.documents.get_mut(&app) {
                    doc.insert_char('\n');
                }
            }
            KeyCode::KEY_BACKSPACE => {
                if let Some(doc) = d.documents.get_mut(&app) {
                    doc.backspace();
                }
            }
            KeyCode::KEY_DELETE => {
                if let Some(doc) = d.documents.get_mut(&app) {
                    doc.delete_forward();
                }
            }
            KeyCode::KEY_INSERT => {
                if let Some(doc) = d.documents.get_mut(&app) {
                    doc.overwrite = !doc.overwrite;
                }
            }
            KeyCode::KEY_SPACE => {
                if let Some(doc) = d.documents.get_mut(&app) {
                    doc.insert_char(' ');
                }
            }
            KeyCode::KEY_LEFT => {
                let shift_pressed = d.shift_pressed;
                if let Some(doc) = d.documents.get_mut(&app) {
                    let prev = doc.caret;
                    if doc.caret > 0 {
                        doc.caret -= 1;
                    }
                    if shift_pressed {
                        let anchor = doc.selection.map(|(a, _)| a).unwrap_or(prev);
                        doc.selection = Some((anchor, doc.caret));
                    } else {
                        doc.selection = None;
                    }
                }
            }
            KeyCode::KEY_RIGHT => {
                let shift_pressed = d.shift_pressed;
                if let Some(doc) = d.documents.get_mut(&app) {
                    let prev = doc.caret;
                    if doc.caret < doc.chars.len() {
                        doc.caret += 1;
                    }
                    if shift_pressed {
                        let anchor = doc.selection.map(|(a, _)| a).unwrap_or(prev);
                        doc.selection = Some((anchor, doc.caret));
                    } else {
                        doc.selection = None;
                    }
                }
            }
            _ => {
                let shift = d.shift_pressed;
                let caps = d.caps_lock_state;
                if let Some(ch) = key_to_char_with_case(key, shift, caps) {
                    if let Some(doc) = d.documents.get_mut(&app) {
                        doc.insert_char(ch);
                    }
                }
            }
        }
        InjectResult::Success
    }
}

struct AdversarialOfficeRig {
    daemon: Daemon,
    desktop: Rc<RefCell<VirtualDesktop>>,
    injector: Box<dyn KeyInjector>,
    evdev_key_state: evdev::AttributeSet<KeyCode>,
    consumed: HashSet<u16>,
    focus_generation: u64,
}

impl AdversarialOfficeRig {
    fn new(initial_app: &str) -> Self {
        let config = Config::default();
        let config_path = std::path::PathBuf::from("/etc/vietc/config.toml");
        let engine_enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut daemon = Daemon::new(config, config_path, engine_enabled);
        daemon.engine.set_enabled(true);
        daemon.engine.set_method(vietc_engine::InputMethod::Telex);

        let desktop = Rc::new(RefCell::new(VirtualDesktop::default()));
        {
            let mut d = desktop.borrow_mut();
            d.focused_app = initial_app.to_string();
            d.documents
                .insert(initial_app.to_string(), OfficeDocumentBuffer::default());
        }

        let injector = Box::new(AdversarialTestInjector {
            desktop: desktop.clone(),
        });

        Self {
            daemon,
            desktop,
            injector,
            evdev_key_state: evdev::AttributeSet::new(),
            consumed: HashSet::new(),
            focus_generation: 1,
        }
    }

    fn text(&self) -> String {
        let d = self.desktop.borrow();
        d.documents
            .get(&d.focused_app)
            .map(|doc| doc.text())
            .unwrap_or_default()
    }

    fn switch_app(&mut self, app_id: &str) {
        {
            let mut d = self.desktop.borrow_mut();
            d.focused_app = app_id.to_string();
            d.documents.entry(app_id.to_string()).or_default();
        }
        self.focus_generation += 1;
        self.daemon.reset_composition_state();
        self.daemon.check_app_change_with(app_id.to_string());
    }

    #[allow(dead_code)]
    fn set_clipboard(&mut self, text: &str) {
        self.desktop.borrow_mut().system_clipboard = text.to_string();
    }

    fn clipboard(&self) -> String {
        self.desktop.borrow().system_clipboard.clone()
    }

    fn toggle_caps_lock(&mut self) {
        let current = self.desktop.borrow().caps_lock_state;
        self.desktop.borrow_mut().caps_lock_state = !current;
    }

    fn dispatch_event(&mut self, keycode: u16, value: i32) {
        let key = KeyCode::new(keycode);
        if value == 1 {
            self.evdev_key_state.insert(key);
        } else if value == 0 {
            self.evdev_key_state.remove(key);
        }

        if key == KeyCode::KEY_CAPSLOCK && value == 1 {
            self.toggle_caps_lock();
        }

        if !self.daemon.grab_enabled {
            return;
        }

        let is_mod = self.evdev_key_state.contains(KeyCode::KEY_LEFTCTRL)
            || self.evdev_key_state.contains(KeyCode::KEY_RIGHTCTRL)
            || self.evdev_key_state.contains(KeyCode::KEY_LEFTALT)
            || self.evdev_key_state.contains(KeyCode::KEY_RIGHTALT)
            || self.evdev_key_state.contains(KeyCode::KEY_LEFTMETA)
            || self.evdev_key_state.contains(KeyCode::KEY_RIGHTMETA);

        if is_mod {
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

        if key == KeyCode::KEY_BACKSPACE {
            if value == 1 || value == 2 {
                let commands = self.daemon.process_key('\x08');
                if commands.is_empty() {
                    self.injector.send_key_event(14, 1);
                    self.injector.send_key_event(14, 0);
                } else {
                    let outcome = execute_commands(
                        &*self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_generation,
                    );
                    assert!(self.daemon.handle_transaction_outcome(outcome));
                }
            }
            if value == 0 {
                self.consumed.remove(&keycode);
            } else {
                self.consumed.insert(keycode);
            }
            return;
        }

        if value == 1 {
            self.consumed.remove(&keycode);
            let shift = is_modifier_held_shift(&self.evdev_key_state);
            let caps = self.desktop.borrow().caps_lock_state;

            if let Some(ch) = key_to_char_with_case(key, shift, caps) {
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key(ch);
                if !commands.is_empty() {
                    self.consumed.insert(keycode);
                    let outcome = execute_commands(
                        &*self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.focus_generation,
                    );
                    assert!(self.daemon.handle_transaction_outcome(outcome));

                    if key == KeyCode::KEY_SPACE && !shift && self.daemon.engine.is_enabled() {
                        let _ = self.injector.send_char(' ');
                        self.consumed.insert(keycode);
                    } else if is_flush_char(ch) && self.daemon.engine.is_enabled() {
                        self.injector.send_key_event(keycode, 1);
                        self.injector.send_key_event(keycode, 0);
                        self.consumed.insert(keycode);
                    }
                } else if self.daemon.engine.is_enabled()
                    && is_vn_control_key(self.daemon.app_state.effective_method(), ch)
                    && self.daemon.engine.buffer().chars().count() <= buf_before
                {
                    self.consumed.insert(keycode);
                } else if key == KeyCode::KEY_SPACE && !shift && self.daemon.engine.is_enabled() {
                    let _ = self.injector.send_char(' ');
                    self.consumed.insert(keycode);
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
            if self.consumed.contains(&keycode) {
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
            if self.consumed.remove(&keycode) {
                return;
            }
            self.injector.send_key_event(keycode, 0);
        }
    }

    /// Simulates natural fast typing where each key may overlap with the previous one
    fn type_overlapping_sequence(&mut self, keycodes: &[u16]) {
        let mut held_prev: Option<u16> = None;
        for &code in keycodes {
            self.dispatch_event(code, 1); // KeyDown current
            if let Some(prev) = held_prev.take() {
                self.dispatch_event(prev, 0); // Release previous key while current is held
            }
            held_prev = Some(code);
        }
        if let Some(last) = held_prev {
            self.dispatch_event(last, 0);
        }
    }

    /// Converts string of characters to scancodes and types them sequentially
    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => {
                    self.dispatch_event(28, 1);
                    self.dispatch_event(28, 0);
                }
                ' ' => {
                    self.dispatch_event(57, 1);
                    self.dispatch_event(57, 0);
                }
                'a'..='z' | 'A'..='Z' => {
                    let shift = ch.is_uppercase();
                    let lower = ch.to_ascii_lowercase();
                    let code = char_to_evdev(lower);
                    if shift {
                        self.dispatch_event(42, 1);
                    }
                    self.dispatch_event(code, 1);
                    self.dispatch_event(code, 0);
                    if shift {
                        self.dispatch_event(42, 0);
                    }
                }
                '0'..='9' => {
                    let code = match ch {
                        '1' => 2,
                        '2' => 3,
                        '3' => 4,
                        '4' => 5,
                        '5' => 6,
                        '6' => 7,
                        '7' => 8,
                        '8' => 9,
                        '9' => 10,
                        '0' => 11,
                        _ => 11,
                    };
                    self.dispatch_event(code, 1);
                    self.dispatch_event(code, 0);
                }
                _ => {
                    if let Some((code, shift)) = symbol_to_evdev(ch) {
                        if shift {
                            self.dispatch_event(42, 1);
                        }
                        self.dispatch_event(code, 1);
                        self.dispatch_event(code, 0);
                        if shift {
                            self.dispatch_event(42, 0);
                        }
                    }
                }
            }
        }
    }

    fn shortcut_ctrl(&mut self, key: KeyCode) {
        self.dispatch_event(29, 1); // Ctrl Down
        self.dispatch_event(key.code(), 1);
        self.dispatch_event(key.code(), 0);
        self.dispatch_event(29, 0); // Ctrl Up
    }

    fn press_key(&mut self, key: KeyCode) {
        self.dispatch_event(key.code(), 1);
        self.dispatch_event(key.code(), 0);
    }
}

fn char_to_evdev(c: char) -> u16 {
    match c {
        'a' => 30,
        'b' => 48,
        'c' => 46,
        'd' => 32,
        'e' => 18,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'i' => 23,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'm' => 50,
        'n' => 49,
        'o' => 24,
        'p' => 25,
        'q' => 16,
        'r' => 19,
        's' => 31,
        't' => 20,
        'u' => 22,
        'v' => 47,
        'w' => 17,
        'x' => 45,
        'y' => 21,
        'z' => 44,
        _ => 57,
    }
}

fn symbol_to_evdev(c: char) -> Option<(u16, bool)> {
    match c {
        '-' => Some((12, false)),
        '_' => Some((12, true)),
        '=' => Some((13, false)),
        '+' => Some((13, true)),
        '/' => Some((53, false)),
        '?' => Some((53, true)),
        '.' => Some((52, false)),
        '>' => Some((52, true)),
        ',' => Some((51, false)),
        '<' => Some((51, true)),
        ';' => Some((39, false)),
        ':' => Some((39, true)),
        '\'' => Some((40, false)),
        '"' => Some((40, true)),
        '`' => Some((41, false)),
        '~' => Some((41, true)),
        '[' => Some((26, false)),
        '{' => Some((26, true)),
        ']' => Some((27, false)),
        '}' => Some((27, true)),
        '\\' => Some((43, false)),
        '|' => Some((43, true)),
        '!' => Some((2, true)),
        '@' => Some((3, true)),
        '#' => Some((4, true)),
        '$' => Some((5, true)),
        '%' => Some((6, true)),
        '^' => Some((7, true)),
        '&' => Some((8, true)),
        '*' => Some((9, true)),
        '(' => Some((10, true)),
        ')' => Some((11, true)),
        _ => None,
    }
}

#[test]
fn test_adversarial_typing_overlap_rapid_consonants_and_space() {
    let mut rig = AdversarialOfficeRig::new("code");

    // 1. "thành" with overlapping keystrokes (t -> h -> a -> n -> h -> f)
    // t (20), h (35), a (30), n (49), h (35), f (33), Space (57)
    let thanh_strokes = [20, 35, 30, 49, 35, 33, 57];
    rig.type_overlapping_sequence(&thanh_strokes);
    assert_eq!(
        rig.text(),
        "thành ",
        "Overlapping 'thanhf ' must yield 'thành '"
    );

    // 2. "các" with overlapping keystrokes (c -> a -> c -> s -> space)
    // c (46), a (30), c (46), s (31), Space (57)
    let cac_strokes = [46, 30, 46, 31, 57];
    rig.type_overlapping_sequence(&cac_strokes);
    assert_eq!(
        rig.text(),
        "thành các ",
        "Overlapping 'cacs ' must yield 'các '"
    );

    // 3. "mày thấy" with overlapping space
    // m (50), a (30), y (21), f (33), Space (57), t (20), h (35), a (30), y (21), a (30), s (31), Space (57)
    let may_thay_strokes = [50, 30, 21, 33, 57, 20, 35, 30, 21, 30, 31, 57];
    rig.type_overlapping_sequence(&may_thay_strokes);
    assert_eq!(
        rig.text(),
        "thành các mày thấy ",
        "Overlapping 'mayf thayas ' must yield 'mày thấy '"
    );
}

#[test]
fn test_adversarial_full_user_problematic_sentence_exhaustive() {
    let mut rig = AdversarialOfficeRig::new("code");

    // User's exact prompt with correct Telex keystrokes:
    // "chuyển 2 lệnh sau về thành lệnh của linux terminal giúp tao, vẫn còn lỗi nè, tao nghĩ mày nên test thật nhiều từ vào, giả lập như người dùng để vét cạn hết các lỗi thay vì sử dụng test suite có sẵn, mày thấy trong câu này có bao nhiêu chỗ còn sai không"
    let sentence = "chuyeenr 2 leejnh sau veef thanhf leejnh cuar linux terminal giusp tao, vaanx cofn looxi nef, tao nghix mayf neen test thaatj nhieeuf tuwf vaof, giar laapj nhuw nguowif dungf ddeer vets cajn heest cacs looxi thay vif suwr dujng test suite cos sawnx, mayf thaays trong caau nafy cos bao nhieeu choox cofn sai khoong";
    rig.type_text(sentence);

    let actual = rig.text();
    println!("Full user paragraph stress result:\n{}", actual);

    // Verify all 3 previously failing spots:
    assert!(
        actual.contains("về thành lệnh"),
        "Must produce 'về thành lệnh', not 'tành'"
    );
    assert!(
        actual.contains("vét cạn hết các lỗi"),
        "Must produce 'hết các lỗi', not 'ác'"
    );
    assert!(
        actual.contains("mày thấy trong câu này"),
        "Must produce 'mày thấy', not 'màythấy'"
    );
    assert!(
        actual.contains("nên test thật nhiều"),
        "Must produce 'nên test', not 'tét'"
    );
    assert!(
        actual.contains("sử dụng test suite"),
        "Must produce 'sử dụng test suite'"
    );
    assert!(actual.contains("bao nhiêu chỗ còn sai không"));
}

#[test]
fn test_adversarial_cross_application_clipboard_workflow() {
    let mut rig = AdversarialOfficeRig::new("microsoft-edge");

    // Step 1: Draft legal decree title in browser
    rig.type_text("Nghij ddijnh soos: 15/2026/NDD-CP veef chuyeenr ddooir soos\n");
    assert!(rig
        .text()
        .contains("Nghị định số: 15/2026/NĐ-CP về chuyển đổi số\n"));

    // Step 2: Select and Copy with Ctrl+C
    for _ in 0..15 {
        rig.dispatch_event(42, 1); // Shift
        rig.press_key(KeyCode::KEY_LEFT);
        rig.dispatch_event(42, 0);
    }
    rig.shortcut_ctrl(KeyCode::KEY_C);
    assert!(!rig.clipboard().is_empty());

    // Step 3: Switch to LibreOffice Writer
    rig.switch_app("libreoffice-writer");
    rig.type_text("Kinhs guwri: Ban Chir ddaoj.\n");
    rig.type_text("Thuwcj hieenj theo ");
    rig.shortcut_ctrl(KeyCode::KEY_V);
    rig.type_text(" ddax dduowjc ban hanhf.\n");

    let writer_text = rig.text();
    println!("Writer text after paste:\n{}", writer_text);
    assert!(writer_text.contains("Kính gửi: Ban Chỉ đạo.\n"));
    assert!(writer_text.contains("Thực hiện theo"));
    assert!(writer_text.contains("đã được ban hành."));

    // Step 4: Switch to VS Code and paste inside technical comment
    rig.switch_app("code");
    rig.type_text("// Config for ");
    rig.shortcut_ctrl(KeyCode::KEY_V);
    rig.type_text(" in /etc/vietc/config.toml\n");

    let code_text = rig.text();
    println!("VS Code text:\n{}", code_text);
    assert!(code_text.contains("// Config for "));
    assert!(code_text.contains("in /etc/vietc/config.toml"));
}

#[test]
fn test_adversarial_overwrite_mode_and_backspace_editing() {
    let mut rig = AdversarialOfficeRig::new("libreoffice-writer");

    // 1. Type initial text
    rig.type_text("Hop dong kinh te so 01");
    assert_eq!(rig.text(), "Hop dong kinh te so 01");

    // 2. Backspace 5 times to erase "so 01"
    for _ in 0..5 {
        rig.press_key(KeyCode::KEY_BACKSPACE);
    }
    assert_eq!(rig.text(), "Hop dong kinh te ");

    // 3. Type Vietnamese with diacritics
    rig.type_text("soos: 99/2026/HD-VN");
    assert_eq!(rig.text(), "Hop dong kinh te số: 99/2026/HD-VN");

    // 4. Toggle Insert (Overwrite Mode)
    rig.press_key(KeyCode::KEY_INSERT);
    // Move cursor left 5 characters to overwrite "HD-VN"
    for _ in 0..5 {
        rig.press_key(KeyCode::KEY_LEFT);
    }
    rig.type_text("VN-01");
    rig.press_key(KeyCode::KEY_INSERT); // Toggle back to insert
    assert_eq!(rig.text(), "Hop dong kinh te số: 99/2026/VN-01");
}

#[test]
fn test_adversarial_bilingual_it_infrastructure_commands() {
    let mut rig = AdversarialOfficeRig::new("code");

    // Mix of shell commands, English keywords, and Vietnamese instructions
    rig.type_text("1. chayj leejnh: sudo systemctl restart vietc.service\n");
    rig.type_text("2. kieemr tra log taij: tail -f ~/.local/state/vietc/vietc.log\n");
    rig.type_text("3. neesu cos looxi thif run: git status vaf git diff --stat\n");
    rig.type_text(
        "4. curl -X POST http://localhost:8080/api/v1/auth/token -d '{\"username\":\"admin\"}'\n",
    );

    let actual = rig.text();
    println!("Bilingual commands text:\n{}", actual);
    assert!(actual.contains("1. chạy lệnh: sudo systemctl restart vietc.service\n"));
    assert!(actual.contains("2. kiểm tra log tại: tail -f ~/.local/state/vietc/vietc.log\n"));
    assert!(actual.contains("3. nếu có lỗi thì run: git status và git diff --stat\n"));
    assert!(actual.contains("http://localhost:8080/api/v1/auth/token"));
}

#[test]
fn test_adversarial_capslock_and_acronym_stress() {
    let mut rig = AdversarialOfficeRig::new("libreoffice-writer");

    // All-caps official document title with CapsLock
    rig.press_key(KeyCode::KEY_CAPSLOCK);
    rig.type_text("coongj hoaf xax hooji chur nghiax vieetj nam\n");
    rig.type_text("ddoocj laapj - tuwj do - hanhj phucs\n");
    rig.press_key(KeyCode::KEY_CAPSLOCK);

    assert_eq!(
        rig.text(),
        "CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM\nĐỘC LẬP - TỰ DO - HẠNH PHÚC\n"
    );

    // TitleCase with Shift
    rig.type_text("Uy Ban Nhaan Daan Thanh Pho Ho Chi Minh");
    assert!(rig.text().contains("Uy Ban Nhân Dân Thanh Pho Ho Chi Minh"));
}
