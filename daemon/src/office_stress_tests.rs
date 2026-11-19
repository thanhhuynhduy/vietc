// SPDX-License-Identifier: MIT
//! Realistic Office-Worker Multi-Editor & Hardware `evdev` Stress Test Suite.
//!
//! Simulates continuous document authoring across office editors, browsers, chat
//! apps, spreadsheets, and terminals (`cosmic-term`) combining:
//! - `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, `Ctrl+A`, `Ctrl+Insert`, `Shift+Insert`
//! - `Insert` key Overwrite-Mode toggling, `Delete`, `Home`/`End`/Arrow navigation, `F2`, Numpad
//! - `Backspace` mid-word, across word boundaries, after `Ctrl+V`, and Gboard Space-Undo
//! - Lowercase, Title Case, ALL CAPS (`CapsLock` & `Shift`), Inverted CapsLock, Early-Shift, Mixed-Case (`DDoS`, `iPhone`, `GraphQL`)
//! - Seamless bilingual Vietnamese–English code-switching (Telex & VNI, Old-Style tones)
//! - 12,000-event deterministic endurance fuzzing verifying zero panics, zero stuck modifiers,
//!   and 100% preservation of user `Ctrl+C` clipboard contents.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use evdev::KeyCode;
use vietc_protocol::inject::{InjectResult, KeyInjector};

use crate::commands::TransactionOutcome;
use crate::config::Config;
use crate::daemon::Daemon;
use crate::event::{
    is_flush_char, is_modifier_held_shift, is_modifier_pressed, is_navigation_or_control_key,
    is_toggle_combination_state, is_toggle_key, is_vn_control_key, key_to_char_with_case,
};
use crate::inject::execute_commands;

#[derive(Clone, Default)]
struct EditorBuffer {
    chars: Vec<char>,
    cursor: usize,
    selection: Option<(usize, usize)>,
    overwrite_mode: bool,
}

impl EditorBuffer {
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

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let (lo, hi) = (a.min(b), a.max(b));
        (lo < hi && hi <= self.chars.len()).then(|| self.chars[lo..hi].iter().collect())
    }

    fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        if self.overwrite_mode
            && ch != '\n'
            && self.cursor < self.chars.len()
            && self.chars[self.cursor] != '\n'
        {
            self.chars[self.cursor] = ch;
            self.cursor += 1;
            return;
        }
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        for ch in s.chars() {
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
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

struct SharedEnvState {
    editors: HashMap<String, EditorBuffer>,
    active_app: String,
    user_clipboard: String,
    caps_lock: bool,
    synth_left_shift: bool,
    synth_right_shift: bool,
    synth_ctrl: bool,
    synth_alt: bool,
    terminal_paste_count: usize,
    gui_paste_count: usize,
}

struct SimulatedInjector {
    state: Rc<RefCell<SharedEnvState>>,
    terminal_mode: std::cell::Cell<bool>,
}

impl SimulatedInjector {
    fn handle_synthesized_key(&self, code: u16, value: i32) {
        let mut st = self.state.borrow_mut();
        let key = KeyCode(code);
        match key {
            KeyCode::KEY_LEFTSHIFT => {
                st.synth_left_shift = value != 0;
                return;
            }
            KeyCode::KEY_RIGHTSHIFT => {
                st.synth_right_shift = value != 0;
                return;
            }
            KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => {
                st.synth_ctrl = value != 0;
                return;
            }
            KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => {
                st.synth_alt = value != 0;
                return;
            }
            _ => {}
        }
        if value == 0 {
            return;
        }
        let shift = st.synth_left_shift || st.synth_right_shift;
        let ctrl = st.synth_ctrl;
        let caps = st.caps_lock;
        let active = st.active_app.clone();

        if ctrl {
            match key {
                KeyCode::KEY_A => {
                    if let Some(ed) = st.editors.get_mut(&active) {
                        let len = ed.chars.len();
                        ed.selection = (len > 0).then_some((0, len));
                        ed.cursor = len;
                    }
                }
                KeyCode::KEY_C | KeyCode::KEY_INSERT => {
                    if let Some(selected) = st.editors.get(&active).and_then(|e| e.selected_text())
                    {
                        st.user_clipboard = selected;
                    }
                }
                KeyCode::KEY_X => {
                    if let Some(selected) = st.editors.get(&active).and_then(|e| e.selected_text())
                    {
                        st.user_clipboard = selected;
                        if let Some(ed) = st.editors.get_mut(&active) {
                            ed.delete_selection();
                        }
                    }
                }
                KeyCode::KEY_V => {
                    let clip = st.user_clipboard.clone();
                    let is_term = self.terminal_mode.get();
                    if !is_term || shift {
                        if let Some(ed) = st.editors.get_mut(&active) {
                            ed.insert_str(&clip);
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        if shift && key == KeyCode::KEY_INSERT {
            let clip = st.user_clipboard.clone();
            if let Some(ed) = st.editors.get_mut(&active) {
                ed.insert_str(&clip);
            }
            return;
        }

        match key {
            KeyCode::KEY_BACKSPACE => {
                if let Some(ed) = st.editors.get_mut(&active) {
                    ed.backspace();
                }
            }
            KeyCode::KEY_DELETE => {
                if let Some(ed) = st.editors.get_mut(&active) {
                    ed.delete_forward();
                }
            }
            KeyCode::KEY_INSERT => {
                if let Some(ed) = st.editors.get_mut(&active) {
                    ed.overwrite_mode = !ed.overwrite_mode;
                }
            }
            KeyCode::KEY_LEFT => {
                if let Some(ed) = st.editors.get_mut(&active) {
                    if shift {
                        let prev = ed.cursor;
                        ed.cursor = ed.cursor.saturating_sub(1);
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, ed.cursor));
                    } else if let Some((a, b)) = ed.selection.take() {
                        ed.cursor = a.min(b);
                    } else {
                        ed.cursor = ed.cursor.saturating_sub(1);
                    }
                }
            }
            KeyCode::KEY_RIGHT => {
                if let Some(ed) = st.editors.get_mut(&active) {
                    if shift {
                        let prev = ed.cursor;
                        ed.cursor = (ed.cursor + 1).min(ed.chars.len());
                        let anchor = ed.selection.map(|(a, _)| a).unwrap_or(prev);
                        ed.selection = Some((anchor, ed.cursor));
                    } else if let Some((a, b)) = ed.selection.take() {
                        ed.cursor = a.max(b);
                    } else {
                        ed.cursor = (ed.cursor + 1).min(ed.chars.len());
                    }
                }
            }
            KeyCode::KEY_HOME => {
                if let Some(ed) = st.editors.get_mut(&active) {
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
                if let Some(ed) = st.editors.get_mut(&active) {
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
                    if let Some(ed) = st.editors.get_mut(&active) {
                        ed.insert_char(ch);
                    }
                }
            }
        }
    }
}

impl KeyInjector for SimulatedInjector {
    fn send_backspace(&self) -> InjectResult {
        self.handle_synthesized_key(KeyCode::KEY_BACKSPACE.code(), 1);
        self.handle_synthesized_key(KeyCode::KEY_BACKSPACE.code(), 0);
        InjectResult::Success
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let active = st.active_app.clone();
        if let Some(ed) = st.editors.get_mut(&active) {
            ed.insert_char(ch);
        }
        InjectResult::Success
    }

    fn send_string(&self, s: &str) -> InjectResult {
        if s.is_ascii() {
            for ch in s.chars() {
                let _ = self.send_char(ch);
            }
            return InjectResult::Success;
        }
        self.inject_replacement(0, s)
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        let mut st = self.state.borrow_mut();
        assert!(
            !st.synth_left_shift && !st.synth_right_shift,
            "Shift must be released before replacement injection so Shift+Backspaces never select/misfire!"
        );
        // Simulate Wayland/X11 atomic clipboard transaction: snapshot user clipboard,
        // publish replacement payload, delete `backspaces`, paste (`Ctrl+Shift+V` in terminals,
        // `Ctrl+V` in GUI apps), and restore user clipboard intact!
        let saved_clipboard = st.user_clipboard.clone();
        if !text.is_ascii() {
            st.user_clipboard = text.to_string();
            if self.terminal_mode.get() {
                st.terminal_paste_count += 1;
            } else {
                st.gui_paste_count += 1;
            }
        }
        let active = st.active_app.clone();
        if let Some(ed) = st.editors.get_mut(&active) {
            for _ in 0..backspaces {
                ed.backspace();
            }
            ed.insert_str(text);
        }
        st.user_clipboard = saved_clipboard;
        InjectResult::Success
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        self.handle_synthesized_key(keycode, value);
        InjectResult::Success
    }

    fn set_caps_lock(&self, active: bool) -> InjectResult {
        self.state.borrow_mut().caps_lock = active;
        InjectResult::Success
    }

    fn set_active_window(&self, app_class: &str) -> InjectResult {
        self.state.borrow_mut().active_app = app_class.to_string();
        InjectResult::Success
    }

    fn set_terminal_app(&self, is_terminal: bool) -> InjectResult {
        self.terminal_mode.set(is_terminal);
        InjectResult::Success
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }
}

struct OfficeDesktopRig {
    daemon: Daemon,
    injector: SimulatedInjector,
    env: Rc<RefCell<SharedEnvState>>,
    key_state: evdev::AttributeSet<KeyCode>,
    caps_lock: bool,
    consumed_keys: HashSet<u16>,
    focus_generation: u64,
}

impl OfficeDesktopRig {
    fn new(method: &str, initial_app: &str) -> Self {
        let config = Config {
            input_method: method.to_string(),
            start_enabled: true,
            ..Default::default()
        };
        let daemon = Daemon::new(
            config,
            PathBuf::from("/tmp/vietc-office-stress-nonexistent.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        let mut editors = HashMap::new();
        for app in [
            "libreoffice-writer",
            "chromium",
            "org.telegram.desktop",
            "code",
            "cosmic-term",
            "libreoffice-calc",
        ] {
            editors.insert(app.to_string(), EditorBuffer::default());
        }
        let env = Rc::new(RefCell::new(SharedEnvState {
            editors,
            active_app: initial_app.to_string(),
            user_clipboard: String::new(),
            caps_lock: false,
            synth_left_shift: false,
            synth_right_shift: false,
            synth_ctrl: false,
            synth_alt: false,
            terminal_paste_count: 0,
            gui_paste_count: 0,
        }));
        let injector = SimulatedInjector {
            state: Rc::clone(&env),
            terminal_mode: std::cell::Cell::new(false),
        };
        let mut rig = Self {
            daemon,
            injector,
            env,
            key_state: evdev::AttributeSet::new(),
            caps_lock: false,
            consumed_keys: HashSet::new(),
            focus_generation: 1,
        };
        rig.switch_window(initial_app);
        rig
    }

    fn switch_window(&mut self, app_class: &str) {
        self.focus_generation += 1;
        self.daemon.reset_composition_state();
        self.injector.set_active_window(app_class);
        self.daemon.check_app_change_with(app_class.to_string());
        self.injector
            .set_terminal_app(self.daemon.app_state.is_terminal_app());
        self.env
            .borrow_mut()
            .editors
            .entry(app_class.to_string())
            .or_default();
    }

    fn editor_text(&self, app_class: &str) -> String {
        self.env
            .borrow()
            .editors
            .get(app_class)
            .map(|e| e.text())
            .unwrap_or_default()
    }

    fn active_text(&self) -> String {
        let active = self.env.borrow().active_app.clone();
        self.editor_text(&active)
    }

    fn clipboard(&self) -> String {
        self.env.borrow().user_clipboard.clone()
    }

    fn set_clipboard(&mut self, text: &str) {
        self.env.borrow_mut().user_clipboard = text.to_string();
    }

    fn raw_evdev_event(&mut self, key: KeyCode, value: i32) {
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
                    let outcome = self.exec_with_shift_released(&commands);
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
            let shift = is_modifier_held_shift(&self.key_state);
            if let Some(ch) = key_to_char_with_case(key, shift, self.caps_lock) {
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key(ch);
                if !commands.is_empty() {
                    self.consumed_keys.insert(keycode);
                    let outcome = self.exec_with_shift_released(&commands);
                    assert!(self.daemon.handle_transaction_outcome(outcome));
                    if is_flush_char(ch) && self.daemon.engine.is_enabled() {
                        let _ = self.injector.send_char(ch);
                        self.consumed_keys.insert(keycode);
                    }
                } else if self.daemon.engine.is_enabled()
                    && is_vn_control_key(self.daemon.app_state.effective_method(), ch)
                    && self.daemon.engine.buffer().chars().count() <= buf_before
                {
                    self.consumed_keys.insert(keycode);
                } else if is_flush_char(ch) && self.daemon.engine.is_enabled() {
                    let _ = self.injector.send_char(ch);
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
                if keycode == KeyCode::KEY_SPACE.code() && self.daemon.engine.is_enabled() {
                    let _ = self.injector.send_char(' ');
                }
            } else {
                self.injector.send_key_event(keycode, 2);
            }
        } else if value == 0 && !self.consumed_keys.remove(&keycode) {
            self.injector.send_key_event(keycode, 0);
        }
    }

    fn exec_with_shift_released(
        &mut self,
        commands: &[crate::commands::OutputCommand],
    ) -> TransactionOutcome {
        let shifts = [KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT];
        for k in shifts {
            if self.key_state.contains(k) {
                self.injector.send_key_event(k.code(), 0);
            }
        }
        let outcome = execute_commands(
            &self.injector,
            commands,
            &mut self.daemon.visible_revision,
            self.focus_generation,
        );
        for k in shifts {
            if self.key_state.contains(k) {
                self.injector.send_key_event(k.code(), 1);
            }
        }
        outcome
    }

    fn tap(&mut self, key: KeyCode) {
        self.raw_evdev_event(key, 1);
        self.raw_evdev_event(key, 0);
    }

    fn chord(&mut self, modifier: KeyCode, key: KeyCode) {
        self.raw_evdev_event(modifier, 1);
        self.raw_evdev_event(key, 1);
        self.raw_evdev_event(key, 0);
        self.raw_evdev_event(modifier, 0);
    }

    /// Type an ASCII keystroke string using genuine physical `evdev` keycodes + `KEY_LEFTSHIFT`
    /// (taking current `caps_lock` into account only when `respect_caps` is false; here we
    /// translate uppercase letters in `seq` as physical `Shift` + key press!).
    fn type_physical_ascii(&mut self, seq: &str) {
        for ch in seq.chars() {
            if ch == '\x08' {
                self.tap(KeyCode::KEY_BACKSPACE);
                continue;
            }
            let (key, need_shift) = ascii_to_evdev(ch);
            if need_shift {
                self.chord(KeyCode::KEY_LEFTSHIFT, key);
            } else {
                self.tap(key);
            }
        }
    }

    /// Type raw physical keys WITHOUT pressing Shift (useful when `CapsLock` is ON!).
    fn type_unshifted_keys(&mut self, seq: &str) {
        for ch in seq.chars() {
            let (key, _) = ascii_to_evdev(ch.to_ascii_lowercase());
            self.tap(key);
        }
    }

    fn assert_no_stuck_modifiers(&self) {
        let st = self.env.borrow();
        assert!(!st.synth_left_shift, "Left Shift stuck!");
        assert!(!st.synth_right_shift, "Right Shift stuck!");
        assert!(!st.synth_ctrl, "Ctrl stuck!");
        assert!(!st.synth_alt, "Alt stuck!");
    }
}

fn ascii_to_evdev(ch: char) -> (KeyCode, bool) {
    for shift in [false, true] {
        for code in 1..=60u16 {
            let key = KeyCode(code);
            if key_to_char_with_case(key, shift, false) == Some(ch) {
                return (key, shift);
            }
        }
    }
    panic!("unsupported test char: {ch:?}");
}

#[test]
fn office_email_drafting_ctrl_c_ctrl_v_and_mixed_case_telex_and_vni() {
    let mut rig = OfficeDesktopRig::new("telex", "chromium");

    // Step 1: Type a contract code in browser, Ctrl+A -> Ctrl+C to copy it to OS clipboard
    rig.type_physical_ascii("HDKT-2026/089");
    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_A);
    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_C);
    assert_eq!(rig.clipboard(), "HDKT-2026/089");

    // Step 2: Switch to LibreOffice Writer and draft a bilingual Vietnamese-English office email
    rig.switch_window("libreoffice-writer");
    rig.type_physical_ascii("COONGJ HOAF XAX HOOIJ CHUR NGHIAX VIEETJ NAM\n");
    rig.type_physical_ascii(
        "Kinhs guwir: Ban Giams Ddoocs & Ddooij ngux GraphQL / DDoS Security,\n",
    );
    rig.type_physical_ascii("Veef vieecj nghieemj thu howpj ddoongf soos ");

    // Paste contract code via Ctrl+V right after typing Vietnamese!
    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V);
    // Immediately continue typing Vietnamese + English technical terms + Old-Style tones
    rig.type_physical_ascii(
        ": doanh thu quyf 3 ddatj 12.5 tyr ddoongf, suwcs khoer heej thoongs server u32/i64 oonr ddinhj.\n",
    );
    rig.type_physical_ascii("Vui longf xem Google docs kemf theo max tham chieeus: ");
    // Paste AGAIN via Shift+Insert to verify clipboard survived dozens of Vietnamese replacements!
    rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_INSERT);
    rig.type_physical_ascii(".");

    assert_eq!(rig.clipboard(), "HDKT-2026/089");
    assert_eq!(
        rig.active_text(),
        "CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM\n\
         Kính gửi: Ban Giám Đốc & Đội ngũ GraphQL / DDoS Security,\n\
         Về việc nghiệm thu hợp đồng số HDKT-2026/089: doanh thu quỳ 3 đạt 12.5 tỷ đồng, sức khỏe hệ thống server u32/i64 ổn định.\n\
         Vui lòng xem Google docs kèm theo mã tham chiếu: HDKT-2026/089."
    );
    rig.assert_no_stuck_modifiers();

    let mut vni_rig = OfficeDesktopRig::new("vni", "libreoffice-writer");
    vni_rig.set_clipboard(" Phu lục 02");
    vni_rig.type_physical_ascii("D9o62ng ho2a su71c khoe3 thu3y chung:");
    vni_rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V);
    vni_rig.type_physical_ascii(" - ban2 giao server u64.");
    assert_eq!(
        vni_rig.active_text(),
        "Đồng hòa sức khỏe thủy chung: Phu lục 02 - bàn giao server u64."
    );
    assert_eq!(vni_rig.clipboard(), " Phu lục 02");
    vni_rig.assert_no_stuck_modifiers();
}

#[test]
fn office_insert_key_overwrite_mode_ctrl_insert_and_shift_insert() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-writer");

    rig.type_physical_ascii("Bao cao nam 2025");
    for _ in 0..4 {
        rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_LEFT);
    }
    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_INSERT);
    assert_eq!(rig.clipboard(), "2025");

    rig.tap(KeyCode::KEY_LEFT);
    rig.tap(KeyCode::KEY_INSERT);
    rig.type_physical_ascii("2026");
    rig.tap(KeyCode::KEY_INSERT);
    rig.type_physical_ascii(" so vowis nawm ");
    rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_INSERT);

    assert_eq!(rig.active_text(), "Bao cao nam 2026 so với năm 2025");
    assert_eq!(rig.clipboard(), "2025");
    rig.assert_no_stuck_modifiers();
}

#[test]
fn office_backspace_torture_midword_across_words_after_paste_and_gboard_undo() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-writer");

    rig.type_physical_ascii("hofan");
    assert_eq!(rig.active_text(), "hoàn");
    rig.tap(KeyCode::KEY_BACKSPACE);
    assert_eq!(rig.active_text(), "hòa");
    rig.type_physical_ascii(" binhf ");
    assert_eq!(rig.active_text(), "hòa bình ");

    for _ in 0..5 {
        rig.tap(KeyCode::KEY_BACKSPACE);
    }
    assert_eq!(rig.active_text(), "hòa ");
    rig.type_physical_ascii("thuaanj ");
    assert_eq!(rig.active_text(), "hòa thuận ");

    rig.set_clipboard("hợp đồng X");
    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V);
    rig.tap(KeyCode::KEY_BACKSPACE);
    rig.type_physical_ascii("soos 1 ");
    assert_eq!(rig.active_text(), "hòa thuận hợp đồng số 1 ");

    rig.type_physical_ascii("test ");
    assert_eq!(rig.active_text(), "hòa thuận hợp đồng số 1 test ");
    rig.tap(KeyCode::KEY_BACKSPACE);
    assert_eq!(rig.active_text(), "hòa thuận hợp đồng số 1 tét");
    rig.type_physical_ascii(" ");
    assert_eq!(rig.active_text(), "hòa thuận hợp đồng số 1 tét ");
    rig.assert_no_stuck_modifiers();
}

#[test]
fn office_uppercase_lowercase_capslock_early_shift_and_mixed_case_matrix() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-writer");

    rig.type_physical_ascii("Ddaf Nawxng, Ddoongf Nai, Aau Cow, Oong Baf, Uowcs Mow, Huouw Sao. ");
    assert_eq!(
        rig.active_text(),
        "Đà Nẵng, Đồng Nai, Âu Cơ, Ông Bà, Ước Mơ, Hươu Sao. "
    );

    rig.select_and_clear();
    rig.type_physical_ascii("xin chaoF Anh vaf tieengS VieetJ ");
    assert_eq!(rig.active_text(), "xin chào Anh và tiếng Việt ");

    rig.select_and_clear();
    rig.tap(KeyCode::KEY_CAPSLOCK);
    assert!(rig.caps_lock);
    rig.type_unshifted_keys("baos caos taif chinhs quyf iv ");
    assert_eq!(rig.active_text(), "BÁO CÁO TÀI CHÍNH QUỲ IV ");

    rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_V);
    rig.type_unshifted_keys("ieetj ");
    assert_eq!(rig.active_text(), "BÁO CÁO TÀI CHÍNH QUỲ IV vIỆT ");

    rig.tap(KeyCode::KEY_CAPSLOCK);
    assert!(!rig.caps_lock);
    rig.type_physical_ascii("iPhone macOS GraphQL PyTorch DDoS ");
    assert_eq!(
        rig.active_text(),
        "BÁO CÁO TÀI CHÍNH QUỲ IV vIỆT iPhone macOS GraphQL PyTorch DDoS "
    );
    rig.assert_no_stuck_modifiers();
}

impl OfficeDesktopRig {
    fn select_and_clear(&mut self) {
        self.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_A);
        self.tap(KeyCode::KEY_BACKSPACE);
        self.daemon.reset_composition_state();
    }
}

#[test]
fn office_bilingual_english_vietnamese_code_switching_sentences() {
    let mut rig = OfficeDesktopRig::new("telex", "org.telegram.desktop");

    let cases = [
        (
            "anh owi check giups em file report vaf guwir Google docs truwowcs 5h nhes. ",
            "anh ơi check giúp em file report và gửi Google docs trước 5h nhé. ",
        ),
        (
            "team minhf lets meet lucs 2h ddeer review data sets vaf floor mats. ",
            "team mình lets meet lúc 2h để review data sets và floor mats. ",
        ),
        (
            "banhs tets ngon hown hay cargo test nhanh hown? ",
            "bánh tét ngon hơn hay cargo test nhanh hơn? ",
        ),
        (
            "camr own anh vif my own server ddax build pass rooif! ",
            "cảm ơn anh vì my own server đã build pass rồi! ",
        ),
        (
            "hai been have been lamf vieecj with lots of business partners. ",
            "hai bên have been làm việc with lots of business partners. ",
        ),
        (
            "check ddiaj chir address vaf uoongs cheese tea sau giowf meeting. ",
            "check địa chỉ address và uống cheese tea sau giờ meeting. ",
        ),
    ];

    for (typed, expected) in cases {
        rig.select_and_clear();
        rig.type_physical_ascii(typed);
        assert_eq!(rig.active_text(), expected, "failed on input: {typed:?}");
        rig.assert_no_stuck_modifiers();
    }
}

#[test]
fn office_numpad_enter_operators_and_f2_spreadsheet_navigation() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-calc");

    rig.type_physical_ascii("hoas");
    rig.tap(KeyCode::KEY_KPENTER);
    rig.type_physical_ascii("nhaan suwj");
    rig.tap(KeyCode::KEY_KPENTER);
    rig.type_physical_ascii("test");
    rig.tap(KeyCode::KEY_KPENTER);
    assert_eq!(rig.active_text(), "hóa\nnhân sự\ntest\n");

    rig.select_and_clear();
    rig.type_physical_ascii("ngayf 18");
    rig.tap(KeyCode::KEY_KPSLASH);
    rig.type_physical_ascii("11");
    rig.tap(KeyCode::KEY_KPMINUS);
    rig.type_physical_ascii("2026");
    rig.tap(KeyCode::KEY_F2);
    rig.type_physical_ascii(" ok ");
    assert_eq!(rig.active_text(), "ngày 18/11-2026 ok ");
    rig.assert_no_stuck_modifiers();
}

#[test]
fn office_multi_app_focus_switching_writer_vscode_telegram_and_cosmic_term() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-writer");
    rig.set_clipboard("CLIP_GUARD_999");

    rig.type_physical_ascii("Taif lieeuj kyx thuaatj ");
    assert_eq!(rig.editor_text("libreoffice-writer"), "Tài liệu kỹ thuật ");

    rig.switch_window("code");
    rig.type_physical_ascii("fn main() { let server_status = true; }");
    assert_eq!(
        rig.editor_text("code"),
        "fn main() { let server_status = true; }"
    );

    rig.switch_window("org.telegram.desktop");
    rig.type_physical_ascii("em ddax push code leen git rooif anh nhes ");
    assert_eq!(
        rig.editor_text("org.telegram.desktop"),
        "em đã push code lên git rồi anh nhé "
    );

    rig.switch_window("cosmic-term");
    if !rig.daemon.engine.is_enabled() {
        rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_SPACE);
    }
    let term_pastes_before = rig.env.borrow().terminal_paste_count;
    rig.type_physical_ascii("echo \"xin cha2o d9o62ng nghie65p\" ");
    assert_eq!(
        rig.editor_text("cosmic-term"),
        "echo \"xin chào đồng nghiệp\" "
    );
    assert!(
        rig.env.borrow().terminal_paste_count > term_pastes_before,
        "cosmic-term must receive terminal-mode paste (Ctrl+Shift+V)"
    );
    assert_eq!(rig.clipboard(), "CLIP_GUARD_999");
    rig.assert_no_stuck_modifiers();
}

#[test]
fn office_12000_keystroke_endurance_stress_fuzz_preserves_clipboard_and_invariants() {
    let mut rig = OfficeDesktopRig::new("telex", "libreoffice-writer");
    rig.set_clipboard("IMPORTANT_OFFICE_CLIPBOARD_PAYLOAD_2026");

    let mut seed: u64 = 0x5649_4554_435F_2026;
    let mut next_u32 = || -> u32 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 16) as u32
    };

    let tokens: Vec<&str> = "chao |chaof |tieengs |Vieetj |dduwowngf |hoaf |hofan |khoer |thuyr |huouw |ruouwj |test |server |Google |docs |GraphQL |DDoS |cheese |business |address |COONGJ |HOAF |123.45 |(ok) ".split('|').collect();
    let apps: Vec<&str> = "libreoffice-writer|chromium|org.telegram.desktop|code|cosmic-term"
        .split('|')
        .collect();

    let mut total_events = 0usize;
    while total_events < 12_000 {
        let roll = next_u32() % 100;
        match roll {
            0..=59 => {
                let token = tokens[(next_u32() as usize) % tokens.len()];
                rig.type_physical_ascii(token);
                total_events += token.len() * 2;
            }
            60..=71 => {
                let count = ((next_u32() % 6) + 1) as usize;
                for _ in 0..count {
                    rig.tap(KeyCode::KEY_BACKSPACE);
                }
                total_events += count * 2;
            }
            72..=77 => {
                let before_clip = rig.clipboard();
                if next_u32() % 2 == 0 {
                    rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V);
                } else {
                    rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_INSERT);
                }
                assert_eq!(rig.clipboard(), before_clip);
                total_events += 4;
            }
            78..=82 => {
                rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_LEFT);
                rig.chord(KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_LEFT);
                rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_C);
                rig.tap(KeyCode::KEY_END);
                total_events += 14;
            }
            83..=87 => {
                let nav = match next_u32() % 6 {
                    0 => KeyCode::KEY_LEFT,
                    1 => KeyCode::KEY_RIGHT,
                    2 => KeyCode::KEY_HOME,
                    3 => KeyCode::KEY_END,
                    4 => KeyCode::KEY_INSERT,
                    _ => KeyCode::KEY_KPENTER,
                };
                rig.tap(nav);
                total_events += 2;
            }
            88..=92 => {
                rig.tap(KeyCode::KEY_CAPSLOCK);
                total_events += 2;
            }
            93..=96 => {
                rig.chord(KeyCode::KEY_LEFTCTRL, KeyCode::KEY_SPACE);
                total_events += 4;
            }
            _ => {
                let app = apps[(next_u32() as usize) % apps.len()];
                rig.switch_window(app);
                total_events += 1;
            }
        }
        rig.assert_no_stuck_modifiers();
    }
}
