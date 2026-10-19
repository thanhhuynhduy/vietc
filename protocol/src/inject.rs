// SPDX-License-Identifier: MIT
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: u32,
    pub value: char,
    pub action: KeyAction,
}

impl KeyEvent {
    pub fn press(code: u32, value: char) -> Self {
        Self {
            code,
            value,
            action: KeyAction::Press,
        }
    }

    pub fn release(code: u32, value: char) -> Self {
        Self {
            code,
            value,
            action: KeyAction::Release,
        }
    }

    pub fn is_press(&self) -> bool {
        self.action == KeyAction::Press
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectResult {
    Success,
    Failed,
    NotSupported,
}

impl InjectResult {
    pub fn is_ok(&self) -> bool {
        *self == InjectResult::Success
    }
}

pub trait KeyInjector {
    fn send_backspace(&self) -> InjectResult;
    fn send_char(&self, ch: char) -> InjectResult;
    fn send_string(&self, s: &str) -> InjectResult;
    fn send_key_event(&self, _keycode: u16, _value: i32) -> InjectResult {
        InjectResult::NotSupported
    }
    fn set_active_window(&self, _window_class: &str) -> InjectResult {
        InjectResult::Success
    }
    fn flush(&self) -> InjectResult;

    fn send_backspaces(&self, count: usize) -> InjectResult {
        for _ in 0..count {
            if self.send_backspace() != InjectResult::Success {
                return InjectResult::Failed;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        InjectResult::Success
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        if self.send_backspaces(backspaces) != InjectResult::Success {
            return InjectResult::Failed;
        }
        self.send_string(text)
    }

    /// Record that Unicode text was pasted via clipboard (for future delete/backspace support)
    fn update_pasted_text(&self, _text: &str) -> InjectResult {
        // Stub implementation - actual text tracking happens in engine via OutputCommand::Type
        InjectResult::Success
    }
}

impl fmt::Display for InjectResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InjectResult::Success => write!(f, "Success"),
            InjectResult::Failed => write!(f, "Failed"),
            InjectResult::NotSupported => write!(f, "NotSupported"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test double whose backspace and insert steps succeed or fail on demand.
    struct ScriptedInjector {
        backspace_ok: bool,
        string_ok: bool,
    }

    impl KeyInjector for ScriptedInjector {
        fn send_backspace(&self) -> InjectResult {
            if self.backspace_ok {
                InjectResult::Success
            } else {
                InjectResult::Failed
            }
        }

        fn send_char(&self, _ch: char) -> InjectResult {
            InjectResult::Success
        }

        fn send_string(&self, _s: &str) -> InjectResult {
            if self.string_ok {
                InjectResult::Success
            } else {
                InjectResult::Failed
            }
        }

        fn flush(&self) -> InjectResult {
            InjectResult::Success
        }
    }

    #[test]
    fn replacement_succeeds_when_all_steps_succeed() {
        let inj = ScriptedInjector {
            backspace_ok: true,
            string_ok: true,
        };
        assert_eq!(inj.inject_replacement(2, "ế"), InjectResult::Success);
    }

    #[test]
    fn backspace_failure_propagates() {
        let inj = ScriptedInjector {
            backspace_ok: false,
            string_ok: true,
        };
        assert_eq!(inj.inject_replacement(2, "ế"), InjectResult::Failed);
    }

    #[test]
    fn insert_failure_propagates() {
        let inj = ScriptedInjector {
            backspace_ok: true,
            string_ok: false,
        };
        assert_eq!(inj.inject_replacement(2, "ế"), InjectResult::Failed);
    }

    #[test]
    fn zero_backspaces_still_reports_insert_failure() {
        let inj = ScriptedInjector {
            backspace_ok: true,
            string_ok: false,
        };
        assert_eq!(inj.inject_replacement(0, "a"), InjectResult::Failed);
    }
}
