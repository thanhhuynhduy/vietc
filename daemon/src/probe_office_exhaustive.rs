// SPDX-License-Identifier: MIT
//! Exhaustive Adversarial Office Worker Stress Probe
//!
//! Tests continuous document authoring across editors, browsers (Omnibox inline autocomplete),
//! and Electron apps combining Ctrl+C/V/X/Z/Backspace, Insert (Overwrite), Backspace cascades,
//! uppercase/lowercase/mixed-case Shift & CapsLock, and 260+ bilingual English/Vietnamese words.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use evdev::KeyCode;
use vietc_protocol::inject::{InjectResult, KeyInjector};

use crate::config::Config;
use crate::daemon::Daemon;
use crate::event::{
    is_flush_char, is_modifier_held_shift, is_modifier_pressed, is_navigation_or_control_key,
    is_toggle_combination_state, is_toggle_key, is_vn_control_key, key_to_char_with_case,
};
use crate::focus::{FocusSnapshot, SharedFocusSnapshot};
use crate::inject::execute_commands;

#[derive(Clone, Default, Debug)]
struct RealisticAppBuffer {
    chars: Vec<char>,
    caret: usize,
    selection: Option<(usize, usize)>,
    overwrite_mode: bool,
    undo_stack: Vec<(Vec<char>, usize)>,
}

impl RealisticAppBuffer {
    fn text(&self) -> String {
        self.chars.iter().collect()
    }

    fn save_undo(&mut self) {
        self.undo_stack.push((self.chars.clone(), self.caret));
        if self.undo_stack.len() > 32 {
            self.undo_stack.remove(0);
        }
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection.take() {
            let (lo, hi) = (a.min(b), a.max(b));
            if lo < hi && hi <= self.chars.len() {
                self.save_undo();
                self.chars.drain(lo..hi);
                self.caret = lo;
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
            && self.caret < self.chars.len()
            && self.chars[self.caret] != '\n'
        {
            self.chars[self.caret] = ch;
            self.caret += 1;
        } else {
            self.chars.insert(self.caret, ch);
            self.caret += 1;
        }
    }

    fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret > 0 && self.caret <= self.chars.len() {
            self.caret -= 1;
            self.chars.remove(self.caret);
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

    fn delete_word_backward(&mut self) {
        if self.delete_selection() {
            return;
        }
        while self.caret > 0 && self.chars[self.caret - 1].is_whitespace() {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
        while self.caret > 0 && !self.chars[self.caret - 1].is_whitespace() {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
    }

    fn delete_word_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        while self.caret < self.chars.len() && self.chars[self.caret].is_whitespace() {
            self.chars.remove(self.caret);
        }
        while self.caret < self.chars.len() && !self.chars[self.caret].is_whitespace() {
            self.chars.remove(self.caret);
        }
    }
}

struct CompositorSimState {
    apps: HashMap<String, RealisticAppBuffer>,
    active_app: String,
    clipboard: String,
    vk_left_shift: bool,
    vk_right_shift: bool,
    vk_caps_lock: bool,
    uinput_left_shift: bool,
    uinput_right_shift: bool,
    uinput_ctrl: bool,
    uinput_alt: bool,
    uinput_meta: bool,
    omnibox_inline_selection: bool,
    virtual_keyboard_enabled: bool,
    held_vk_keys: HashSet<u16>,
}

impl Default for CompositorSimState {
    fn default() -> Self {
        Self {
            apps: HashMap::new(),
            active_app: "editor".to_string(),
            clipboard: String::new(),
            vk_left_shift: false,
            vk_right_shift: false,
            vk_caps_lock: false,
            uinput_left_shift: false,
            uinput_right_shift: false,
            uinput_ctrl: false,
            uinput_alt: false,
            uinput_meta: false,
            omnibox_inline_selection: false,
            virtual_keyboard_enabled: true,
            held_vk_keys: HashSet::new(),
        }
    }
}

struct FullStackProbeInjector {
    state: Rc<RefCell<CompositorSimState>>,
}

impl KeyInjector for FullStackProbeInjector {
    fn uses_virtual_keyboard(&self) -> bool {
        self.state.borrow().virtual_keyboard_enabled
    }

    fn set_caps_lock(&self, enabled: bool) -> InjectResult {
        self.state.borrow_mut().vk_caps_lock = enabled;
        InjectResult::Success
    }

    fn set_omnibox_inline_selection(&self, active: bool) -> InjectResult {
        self.state.borrow_mut().omnibox_inline_selection = active;
        InjectResult::Success
    }

    fn send_backspace(&self) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let app = st.active_app.clone();
        if let Some(buf) = st.apps.get_mut(&app) {
            buf.backspace();
        }
        InjectResult::Success
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let app = st.active_app.clone();
        if let Some(buf) = st.apps.get_mut(&app) {
            buf.insert_char(ch);
        }
        InjectResult::Success
    }

    fn send_string(&self, s: &str) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let app = st.active_app.clone();
        if let Some(buf) = st.apps.get_mut(&app) {
            buf.insert_str(s);
        }
        InjectResult::Success
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let extra_del = std::mem::replace(&mut st.omnibox_inline_selection, false);
        let app = st.active_app.clone();
        if let Some(buf) = st.apps.get_mut(&app) {
            if extra_del {
                buf.delete_forward();
            }
            for _ in 0..backspaces {
                buf.backspace();
            }
            buf.insert_str(text);
        }
        InjectResult::Success
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let mut st = self.state.borrow_mut();
        let key = KeyCode::new(keycode);

        match key {
            KeyCode::KEY_LEFTSHIFT => {
                if st.uinput_ctrl || st.uinput_alt || st.uinput_meta || !st.virtual_keyboard_enabled
                {
                    st.uinput_left_shift = value != 0;
                } else {
                    st.vk_left_shift = value != 0;
                }
                return InjectResult::Success;
            }
            KeyCode::KEY_RIGHTSHIFT => {
                if st.uinput_ctrl || st.uinput_alt || st.uinput_meta || !st.virtual_keyboard_enabled
                {
                    st.uinput_right_shift = value != 0;
                } else {
                    st.vk_right_shift = value != 0;
                }
                return InjectResult::Success;
            }
            KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => {
                let was_ctrl = st.uinput_ctrl;
                st.uinput_ctrl = value != 0;
                if !was_ctrl && st.uinput_ctrl {
                    if st.vk_left_shift {
                        st.vk_left_shift = false;
                        st.uinput_left_shift = true;
                    }
                    if st.vk_right_shift {
                        st.vk_right_shift = false;
                        st.uinput_right_shift = true;
                    }
                } else if was_ctrl && !st.uinput_ctrl && !st.uinput_alt && !st.uinput_meta {
                    if st.uinput_left_shift {
                        st.uinput_left_shift = false;
                        st.vk_left_shift = true;
                    }
                    if st.uinput_right_shift {
                        st.uinput_right_shift = false;
                        st.vk_right_shift = true;
                    }
                }
                return InjectResult::Success;
            }
            KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => {
                st.uinput_alt = value != 0;
                return InjectResult::Success;
            }
            KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => {
                st.uinput_meta = value != 0;
                return InjectResult::Success;
            }
            _ => {}
        }

        if value == 0 {
            st.held_vk_keys.remove(&keycode);
            return InjectResult::Success;
        }
        if value == 1 {
            st.held_vk_keys.insert(keycode);
        }

        let shift =
            st.vk_left_shift || st.vk_right_shift || st.uinput_left_shift || st.uinput_right_shift;
        let ctrl = st.uinput_ctrl;
        let caps = st.vk_caps_lock;
        let app = st.active_app.clone();

        if ctrl {
            match key {
                KeyCode::KEY_A => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        let len = buf.chars.len();
                        buf.selection = (len > 0).then_some((0, len));
                        buf.caret = len;
                    }
                }
                KeyCode::KEY_C | KeyCode::KEY_INSERT => {
                    if let Some(sel) = st.apps.get(&app).and_then(|b| b.selected_text()) {
                        st.clipboard = sel;
                    }
                }
                KeyCode::KEY_X => {
                    if let Some(sel) = st.apps.get(&app).and_then(|b| b.selected_text()) {
                        st.clipboard = sel;
                        if let Some(buf) = st.apps.get_mut(&app) {
                            buf.delete_selection();
                        }
                    }
                }
                KeyCode::KEY_V => {
                    let clip = st.clipboard.clone();
                    if let Some(buf) = st.apps.get_mut(&app) {
                        buf.insert_str(&clip);
                    }
                }
                KeyCode::KEY_Z => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        if let Some((prev_chars, prev_caret)) = buf.undo_stack.pop() {
                            buf.chars = prev_chars;
                            buf.caret = prev_caret.min(buf.chars.len());
                            buf.selection = None;
                        }
                    }
                }
                KeyCode::KEY_BACKSPACE => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        buf.delete_word_backward();
                    }
                }
                KeyCode::KEY_DELETE => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        buf.delete_word_forward();
                    }
                }
                KeyCode::KEY_LEFT => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        let prev = buf.caret;
                        while buf.caret > 0 && buf.chars[buf.caret - 1].is_whitespace() {
                            buf.caret -= 1;
                        }
                        while buf.caret > 0 && !buf.chars[buf.caret - 1].is_whitespace() {
                            buf.caret -= 1;
                        }
                        if shift {
                            let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                            buf.selection = Some((anchor, buf.caret));
                        } else {
                            buf.selection = None;
                        }
                    }
                }
                KeyCode::KEY_RIGHT => {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        let prev = buf.caret;
                        while buf.caret < buf.chars.len() && buf.chars[buf.caret].is_whitespace() {
                            buf.caret += 1;
                        }
                        while buf.caret < buf.chars.len() && !buf.chars[buf.caret].is_whitespace() {
                            buf.caret += 1;
                        }
                        if shift {
                            let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                            buf.selection = Some((anchor, buf.caret));
                        } else {
                            buf.selection = None;
                        }
                    }
                }
                _ => {}
            }
            return InjectResult::Success;
        }

        if shift && key == KeyCode::KEY_INSERT {
            let clip = st.clipboard.clone();
            if let Some(buf) = st.apps.get_mut(&app) {
                buf.insert_str(&clip);
            }
            return InjectResult::Success;
        }

        match key {
            KeyCode::KEY_BACKSPACE => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    buf.backspace();
                }
            }
            KeyCode::KEY_DELETE => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    buf.delete_forward();
                }
            }
            KeyCode::KEY_INSERT => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    buf.overwrite_mode = !buf.overwrite_mode;
                }
            }
            KeyCode::KEY_LEFT => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    if shift {
                        let prev = buf.caret;
                        buf.caret = buf.caret.saturating_sub(1);
                        let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                        buf.selection = Some((anchor, buf.caret));
                    } else if let Some((a, b)) = buf.selection.take() {
                        buf.caret = a.min(b);
                    } else {
                        buf.caret = buf.caret.saturating_sub(1);
                    }
                }
            }
            KeyCode::KEY_RIGHT => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    if shift {
                        let prev = buf.caret;
                        buf.caret = (buf.caret + 1).min(buf.chars.len());
                        let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                        buf.selection = Some((anchor, buf.caret));
                    } else if let Some((a, b)) = buf.selection.take() {
                        buf.caret = a.max(b);
                    } else {
                        buf.caret = (buf.caret + 1).min(buf.chars.len());
                    }
                }
            }
            KeyCode::KEY_HOME => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    let prev = buf.caret;
                    buf.caret = 0;
                    if shift {
                        let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                        buf.selection = Some((anchor, 0));
                    } else {
                        buf.selection = None;
                    }
                }
            }
            KeyCode::KEY_END => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    let prev = buf.caret;
                    let end = buf.chars.len();
                    buf.caret = end;
                    if shift {
                        let anchor = buf.selection.map(|(a, _)| a).unwrap_or(prev);
                        buf.selection = Some((anchor, end));
                    } else {
                        buf.selection = None;
                    }
                }
            }
            KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    buf.insert_char('\n');
                }
            }
            KeyCode::KEY_TAB => {
                if let Some(buf) = st.apps.get_mut(&app) {
                    buf.insert_char('\t');
                }
            }
            _ => {
                if let Some(ch) = key_to_char_with_case(key, shift, caps) {
                    if let Some(buf) = st.apps.get_mut(&app) {
                        buf.insert_char(ch);
                    }
                }
            }
        }
        InjectResult::Success
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }
}

struct OfficeProbeHarness {
    daemon: Daemon,
    compositor: Rc<RefCell<CompositorSimState>>,
    injector: FullStackProbeInjector,
    shared_focus: SharedFocusSnapshot,
    key_state: evdev::AttributeSet<KeyCode>,
    consumed_keys: HashSet<u16>,
    global_caps_lock: bool,
    last_focus_generation: u64,
}

impl OfficeProbeHarness {
    fn new(app_name: &str) -> Self {
        let config = Config::default();
        let mut daemon = Daemon::new(
            config,
            PathBuf::from("/tmp/vietc-probe-config.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);

        let compositor = Rc::new(RefCell::new(CompositorSimState::default()));
        {
            let mut c = compositor.borrow_mut();
            c.active_app = app_name.to_string();
            c.apps
                .insert(app_name.to_string(), RealisticAppBuffer::default());
        }
        let injector = FullStackProbeInjector {
            state: Rc::clone(&compositor),
        };
        let shared_focus = Arc::new(Mutex::new(FocusSnapshot::default()));
        shared_focus
            .lock()
            .unwrap()
            .observe(Some(format!("win-{app_name}")), Some(app_name.to_string()));
        let last_focus_generation = shared_focus.lock().unwrap().generation;

        Self {
            daemon,
            compositor,
            injector,
            shared_focus,
            key_state: evdev::AttributeSet::new(),
            consumed_keys: HashSet::new(),
            global_caps_lock: false,
            last_focus_generation,
        }
    }

    fn text(&self) -> String {
        let c = self.compositor.borrow();
        c.apps
            .get(&c.active_app)
            .map(|b| b.text())
            .unwrap_or_default()
    }

    fn clear_text(&mut self) {
        self.daemon.reset_composition_state();
        let mut c = self.compositor.borrow_mut();
        let app = c.active_app.clone();
        c.apps.insert(app, RealisticAppBuffer::default());
    }

    fn feed_evdev(&mut self, keycode: u16, value: i32) {
        let key = KeyCode::new(keycode);
        if value == 1 {
            self.key_state.insert(key);
        } else if value == 0 {
            self.key_state.remove(key);
        }
        if key == KeyCode::KEY_CAPSLOCK && value == 1 {
            self.global_caps_lock = !self.global_caps_lock;
        }
        self.injector.set_caps_lock(self.global_caps_lock);

        if value == 1
            && is_toggle_key(key, &self.daemon.config.toggle_key)
            && is_toggle_combination_state(&self.key_state, &self.daemon.config.toggle_key)
        {
            if key == KeyCode::KEY_CAPSLOCK {
                self.global_caps_lock = !self.global_caps_lock;
                self.injector.set_caps_lock(self.global_caps_lock);
            }
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
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key('\x08');
                if commands.is_empty() {
                    self.injector.set_omnibox_inline_selection(false);
                    self.injector.send_key_event(14, 1);
                    self.injector.send_key_event(14, 0);
                } else {
                    if buf_before > 0
                        && crate::focus::take_omnibox_inline_selection(&self.shared_focus)
                    {
                        self.injector.set_omnibox_inline_selection(true);
                    } else {
                        self.injector.set_omnibox_inline_selection(false);
                    }
                    let outcome = execute_commands(
                        &self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.last_focus_generation,
                    );
                    self.daemon.handle_transaction_outcome(outcome);
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
            if let Some(ch) = key_to_char_with_case(
                key,
                is_modifier_held_shift(&self.key_state),
                self.global_caps_lock,
            ) {
                let buf_before = self.daemon.engine.buffer().chars().count();
                let commands = self.daemon.process_key(ch);
                if !commands.is_empty() {
                    self.consumed_keys.insert(keycode);
                    if buf_before > 0
                        && crate::focus::take_omnibox_inline_selection(&self.shared_focus)
                    {
                        self.injector.set_omnibox_inline_selection(true);
                    } else {
                        self.injector.set_omnibox_inline_selection(false);
                    }
                    let outcome = execute_commands(
                        &self.injector,
                        &commands,
                        &mut self.daemon.visible_revision,
                        self.last_focus_generation,
                    );
                    self.daemon.handle_transaction_outcome(outcome);

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
                } else {
                    self.injector.set_omnibox_inline_selection(false);
                    if key == KeyCode::KEY_SPACE
                        && !is_modifier_held_shift(&self.key_state)
                        && self.daemon.engine.is_enabled()
                    {
                        let _ = self.injector.send_char(' ');
                        self.consumed_keys.insert(keycode);
                    } else {
                        self.injector.send_key_event(keycode, 1);
                    }
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

    fn tap_key(&mut self, key: KeyCode) {
        self.feed_evdev(key.code(), 1);
        self.feed_evdev(key.code(), 0);
    }

    fn type_ascii_str(&mut self, s: &str) {
        for ch in s.chars() {
            match ch {
                '\x08' => self.tap_key(KeyCode::KEY_BACKSPACE),
                '\n' => self.tap_key(KeyCode::KEY_ENTER),
                '\t' => self.tap_key(KeyCode::KEY_TAB),
                ' ' => self.tap_key(KeyCode::KEY_SPACE),
                _ => {
                    let (code, needs_shift) = char_to_physical_evdev(ch, self.global_caps_lock);
                    if needs_shift {
                        self.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 1);
                    }
                    self.feed_evdev(code, 1);
                    self.feed_evdev(code, 0);
                    if needs_shift {
                        self.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 0);
                    }
                }
            }
        }
    }
}

fn char_to_physical_evdev(ch: char, caps_lock: bool) -> (u16, bool) {
    if ch.is_ascii_alphabetic() {
        let needs_shift = ch.is_ascii_uppercase() ^ caps_lock;
        let idx = "qwertyuiop____asdfghjkl_____zxcvbnm"
            .find(ch.to_ascii_lowercase())
            .unwrap();
        return ((16 + idx) as u16, needs_shift);
    }
    if let Some(pos) = "1234567890-=".find(ch) {
        return ((2 + pos) as u16, false);
    }
    if let Some(pos) = "!@#$%^&*()_+".find(ch) {
        return ((2 + pos) as u16, true);
    }
    match ch {
        '[' => (26, false),
        '{' => (26, true),
        ']' => (27, false),
        '}' => (27, true),
        '\\' => (43, false),
        '|' => (43, true),
        ';' => (39, false),
        ':' => (39, true),
        '\'' => (40, false),
        '"' => (40, true),
        '`' => (41, false),
        '~' => (41, true),
        ',' => (51, false),
        '<' => (51, true),
        '.' => (52, false),
        '>' => (52, true),
        '/' => (53, false),
        '?' => (53, true),
        _ => (57, false),
    }
}

#[test]
fn run_exhaustive_adversarial_office_probe() {
    let mut h = OfficeProbeHarness::new("code");

    let cases: &[(&str, &str)] = &[
        ("COONGJ HOAF XAX HOOIJ CHUR NGHIAX VIEETJ NAM ", "CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM "),
        ("Nguyeenx Huwux Thanhf ", "Nguyễn Hữu Thành "),
        ("THanhf VIeetj NGuyeenx DDuowngf ", "THành VIệt NGuyễn Đường "),
        ("Dduowngf DDuowngf DDUOWNGF ", "Đường Đường ĐƯỜNG "),
        ("vieeTj VIEEts nguowIf dduowNgf ", "việt VIẾt người đường "),
        ("Aas Ees Oos Uws Ows Aws ", "Ấ Ế Ố Ứ Ớ Ắ "),
        ("AAs EEs OOs UWs OWs AWs ", "Ấ Ế Ố Ứ Ớ Ắ "),
        ("tHANHF vIEETJ dDUOWNGF ", "tHÀNH vIỆT đƯỜNG "),
        ("useState useEffect FastAPI PyTorch GraphQL TypeScript NextJS macOS iOS iPadOS ", "useState useEffect FastAPI PyTorch GraphQL TypeScript NextJS macOS iOS iPadOS "),
        ("getUserInfo setWindowSize createRoomList onPressButton isFreeTier ", "getUserInfo setWindowSize createRoomList onPressButton isFreeTier "),
        ("window windows workflow network framework password forward awkward ", "window windows workflow network framework password forward awkward "),
        ("book took look good food mood root boot tool pool loop room soon ", "book took look good food mood root boot tool pool loop room soon "),
        ("need feed seed weed speed free tree see keep deep week meet address middle added wedding reddit padding hidden sudden forbidden ladder rudder toddler cuddle puddle saddle paddle riddle fiddle needle ", "need feed seed weed speed free tree see keep deep week meet address middle added wedding reddit padding hidden sudden forbidden ladder rudder toddler cuddle puddle saddle paddle riddle fiddle needle "),
        ("anh check giusp em file report trong folder docs treen server dev nhes ", "anh check giúp em file report trong folder docs trên server dev nhé "),
        ("em guwri anh link meeting vaf slide demo duwj ans mowis ", "em gửi anh link meeting và slide demo dự án mới "),
        ("team minhf review code xong thif merge branch main vaf deploy leen staging luoon ", "team mình review code xong thì merge branch main và deploy lên staging luôn "),
        ("user server docker cluster folder render filter router error monitor ", "user server docker cluster folder render filter router error monitor "),
        ("for from with without within into onto upon under over after before ", "for from with without within into onto upon under over after before "),
        ("officeword excel powerpoint outlook teams slack zoom jira confluence figma notion ", "officeword excel powerpoint outlook teams slack zoom jira confluence figma notion "),
        ("browser electron terminal editor cursor buffer mutex thread async await future stream ", "browser electron terminal editor cursor buffer mutex thread async await future stream "),
        ("budget target profit margin invoice receipt contract client partner manager director ", "budget target profit margin invoice receipt contract client partner manager director "),
        ("vieetj\x08c ", "việc "),
        ("dduowngf\x08\x08cj ", "được "),
        ("toans\x08t ", "toát "),
        ("hoaf\x08n ", "hòn "),
        ("thuowngf\x08\x08cj ", "thược "),
        ("xin chao \x08f ", "xin chào "),
        ("caaus \x08hinhf ", "cấuhình "),
        ("xin chao  \x08\x08f ", "xin chào "),
        ("chaof \x08\x08\x08\x08\x08mowis ", "mới "),
        ("windows \x08s ", "windowss "),
        ("test \x08", "tét"),
        ("Last nuwax \x08\x08\x08\x08sau ", "Lát sau "),
        ("chaof.\x08! ", "chào! "),
        ("Last nuwax, Lots of. Too much! Max soos: ", "Lát nữa, Lots of. Too much! Mã số: "),
        ("Chaof \x08s HOAF \x08N ", "Cháo HOÀN "),
    ];

    for &(seq, expected) in cases {
        h.clear_text();
        h.type_ascii_str(seq);
        assert_eq!(h.text(), expected, "failed sequence: {seq:?}");
    }

    // Ctrl+V followed immediately by Shift-uppercase Vietnamese
    h.clear_text();
    h.compositor.borrow_mut().clipboard = "BÁO CÁO ".to_string();
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 1);
    h.tap_key(KeyCode::KEY_V);
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 0);
    h.type_ascii_str("TAIF CHINHS ");
    assert_eq!(h.text(), "BÁO CÁO TÀI CHÍNH ");

    // Ctrl+Shift+Left word selection then release Ctrl while keeping Shift held to type Vietnamese
    h.clear_text();
    h.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 1);
    h.type_ascii_str("abc ");
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 1);
    h.tap_key(KeyCode::KEY_LEFT);
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 0);
    for key in [
        KeyCode::KEY_D,
        KeyCode::KEY_D,
        KeyCode::KEY_I,
        KeyCode::KEY_N,
        KeyCode::KEY_H,
        KeyCode::KEY_J,
        KeyCode::KEY_SPACE,
    ] {
        h.tap_key(key);
    }
    h.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 0);
    assert_eq!(h.text(), "ĐỊNH ");

    // Shift+Space when auto-restore triggers on Space
    h.clear_text();
    h.type_ascii_str("windows");
    h.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 1);
    h.tap_key(KeyCode::KEY_SPACE);
    h.tap_key(KeyCode::KEY_T);
    h.feed_evdev(KeyCode::KEY_LEFTSHIFT.code(), 0);
    h.type_ascii_str("oots ");
    assert_eq!(h.text(), "windows Tốt ");

    // Insert (Overwrite Mode) after Vietnamese composition
    h.clear_text();
    h.type_ascii_str("toans: 0000");
    for _ in 0..4 {
        h.tap_key(KeyCode::KEY_LEFT);
    }
    h.tap_key(KeyCode::KEY_INSERT);
    h.type_ascii_str("2026");
    h.tap_key(KeyCode::KEY_INSERT);
    assert_eq!(h.text(), "toán: 2026");

    // Ctrl+Backspace after 'Last ' then typing 'nuwax '
    h.clear_text();
    h.type_ascii_str("xin Last ");
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 1);
    h.tap_key(KeyCode::KEY_BACKSPACE);
    h.feed_evdev(KeyCode::KEY_LEFTCTRL.code(), 0);
    h.type_ascii_str("nuwax ");
    assert_eq!(h.text(), "xin nữa ");

    // Browser Omnibox Inline Autocomplete Shield on Tone & Backspace
    h.clear_text();
    h.type_ascii_str("ho");
    {
        let mut c = h.compositor.borrow_mut();
        let app = c.active_app.clone();
        let buf = c.apps.get_mut(&app).unwrap();
        let start = buf.caret;
        buf.chars.extend("tmail.com".chars());
        buf.selection = Some((start, buf.chars.len()));
    }
    h.shared_focus
        .lock()
        .unwrap()
        .set_omnibox_inline_selection(true);
    h.type_ascii_str("f");
    assert_eq!(h.text(), "hò");

    {
        let mut c = h.compositor.borrow_mut();
        let app = c.active_app.clone();
        let buf = c.apps.get_mut(&app).unwrap();
        let start = buf.caret;
        buf.chars.extend("a.vn".chars());
        buf.selection = Some((start, buf.chars.len()));
    }
    h.shared_focus
        .lock()
        .unwrap()
        .set_omnibox_inline_selection(true);
    h.tap_key(KeyCode::KEY_BACKSPACE);
    assert_eq!(h.text(), "ho");

    // VNI mode mid-word Backspace after tone/mark ('vie6t5' + BS + 'c' -> 'việc')
    h.clear_text();
    h.daemon.engine.set_method(vietc_engine::InputMethod::Vni);
    h.type_ascii_str("vie6t5\x08c ");
    assert_eq!(h.text(), "việc ");
    h.daemon.engine.set_method(vietc_engine::InputMethod::Telex);

    // Holding Backspace (KeyRepeat value=2) across Space into committed word
    h.clear_text();
    h.type_ascii_str("xin chaof ");
    h.feed_evdev(KeyCode::KEY_BACKSPACE.code(), 1);
    h.feed_evdev(KeyCode::KEY_BACKSPACE.code(), 2);
    h.feed_evdev(KeyCode::KEY_BACKSPACE.code(), 2);
    h.feed_evdev(KeyCode::KEY_BACKSPACE.code(), 0);
    h.type_ascii_str("oans ");
    assert_eq!(h.text(), "xin choán ");

    // 250+ Office English Vocabulary Sweep (dd, ee, oo, w, s, f, r, x, j)
    let vocab = "add added adding addition additional addon address addressed addressee addresses \
        bidding buddies buddy caddie cheddar cuddle daddy eddy embedded fiddle fodder forbidden \
        goddess hidden huddle Kidd ladder maddening middle middleware modding muddy nodded odd \
        oddly odds Order padded padding paddle pudding puddle reddit reddish riddle ridden rudder \
        saddle sadden shredded shredder shudder skidded studded sudden suddenly teddies teddy \
        toddler udders wedding weddings agree agreed agreement bamboo beef beer beetle Between \
        blood bloom boost booth breeze breed brook broom career cheer cheese choose coffee \
        committee cookie cool cooperate coordinate creed creek creep decree deed deem deer degree \
        door employee engineer exceed fee feed feedback feel feeling feet flee fleet flood floor \
        food fool foot footage football footprint freeze free freedom freelance freely gee geese \
        gloom good goodbye goodness goods goose greed greedy green greet greeting groom groove \
        guaranteed heel hood hook hoop indoor indeed jeep keen keep keeper knee kneel Lagoon leech \
        leek leer look lookup loom loop loose loot meet meeting meek mood moor moose need needle \
        noodle noon noose outdoor outlook overlook oversee peek peel peer pioneer pool poor \
        proceed proof queen queer reed reef reel referee reboot redeem roof rook rookie room \
        roommate root screen screenshot see seed seek seem seen seep seer sheep sheer sheet shoot \
        shopkeeper Mainstream sleek sleep sleeve sleet smooth sneeze snoop soon soothe speed \
        speech sphere spoon spool spook squeeze steed steel steep steer stool stoop street \
        succeed sweep sweet tattoo tee teeth three tool toolbar toolkit tooth took tree troop \
        trustee tweet volunteer weed week weekday weekend weekly weep wheel wood wooden wool \
        yahoo zoom workflow workspace worksheet workshop workstation worldwide framework homework \
        network password keyword forward backward toward towards upward downward inward outward \
        onward afterward awkward software hardware firmware malware spyware adware freeware \
        shareware courseware groupware bloatware ransomware aware unaware warehouse warfare \
        welfare farewell firewall waterproof weather website webpage webcam webinar webhook \
        webSocket webview webmaster webpack weblog welcome western whatever whenever wherever \
        whether whichever whistle wholesale widespread wildcard wildlife willing window windows \
        winner winter wireless wisdom withdraw withdrawal withhold within without withstand \
        witness wizard wonder wonderful worker working workout workplace world worried worry \
        worse worship worst worth worthwhile worthy would wound wrapper wrestle wrist write \
        writer writing written wrong";

    for word in vocab.split_whitespace() {
        h.clear_text();
        let seq = format!("{word} ");
        h.type_ascii_str(&seq);
        assert_eq!(h.text(), seq, "English word corrupted: {word}");
    }
}
