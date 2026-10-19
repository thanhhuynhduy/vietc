// SPDX-License-Identifier: MIT
//! Shared preedit-planning logic for the input-method front-ends.
//!
//! Both the Wayland `zwp_input_method_v2` path and the IBus engine path feed
//! keystrokes to `vietc_engine::Engine` and need to decide, for each key, what
//! to show the user: keep an in-progress preedit, finalize (commit) a word, or
//! forward the raw key. Keeping that decision in one place means the two
//! front-ends behave identically.

use vietc_engine::{Engine, EngineEvent};

/// Result of deciding what to do with a single composed character.
#[derive(Debug, PartialEq, Eq)]
pub enum ImAction {
    /// Show `s` as the in-progress preedit (composing, not committed yet).
    SetPreedit(String),
    /// Finalize `s` into the focused app and clear the preedit.
    Commit(String),
    /// Forward the raw physical key (already grabbed, so the app needs it).
    ForwardKey(u32),
}

/// Characters that end a word and should be committed, with the character
/// itself forwarded to the app.
pub fn is_flush_char(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '.'
            | ','
            | '!'
            | '?'
            | ';'
            | ':'
            | '/'
            | '\\'
            | '-'
            | '_'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '='
            | '+'
            | '*'
            | '&'
            | '%'
            | '$'
            | '#'
            | '@'
            | '~'
            | '`'
            | '"'
            | '\''
            | '|'
            | '^'
    )
}

/// Decide how to present `ch` (from physical key `key`) to the app, given the
/// engine state and the current preedit. Mirrors how the evdev/daemon model
/// treats every raw keystroke as already on screen and the engine events as
/// corrections: here `None`/`Replace` keep a preedit, while `Flush`/
/// `AutoRestore`/`UndoTones`/`Paste` finalize, and flush characters commit the
/// word and let the separator through.
pub fn plan_char(
    enabled: bool,
    engine: &mut Engine,
    preedit: &str,
    ch: char,
    key: u32,
) -> Vec<ImAction> {
    if !enabled {
        let mut a = Vec::new();
        if !preedit.is_empty() {
            a.push(ImAction::Commit(preedit.to_string()));
        }
        a.push(ImAction::ForwardKey(key));
        return a;
    }

    let is_flush = is_flush_char(ch);
    let mut actions = Vec::new();
    match engine.process_key(ch) {
        Some(EngineEvent::Insert(_)) => {
            actions.push(ImAction::ForwardKey(key));
        }
        Some(EngineEvent::Replace(replacement)) => {
            if is_flush {
                actions.push(ImAction::Commit(replacement.insert));
                actions.push(ImAction::ForwardKey(key));
            } else {
                actions.push(ImAction::SetPreedit(replacement.insert));
            }
        }
        Some(EngineEvent::AutoRestore(s))
        | Some(EngineEvent::UndoTones { restored: s, .. })
        | Some(EngineEvent::Flush(s))
        | Some(EngineEvent::Paste(s)) => {
            actions.push(ImAction::Commit(s));
            if is_flush {
                actions.push(ImAction::ForwardKey(key));
            }
        }
        None => {
            if is_flush {
                if !preedit.is_empty() {
                    actions.push(ImAction::Commit(preedit.to_string()));
                }
                actions.push(ImAction::ForwardKey(key));
            } else {
                actions.push(ImAction::SetPreedit(engine.buffer().to_string()));
            }
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use vietc_engine::InputMethod;

    fn render(actions: Vec<ImAction>, preedit: &mut String, committed: &mut String) {
        for action in actions {
            match action {
                ImAction::SetPreedit(value) => *preedit = value,
                ImAction::Commit(value) => {
                    committed.push_str(&value);
                    preedit.clear();
                }
                ImAction::ForwardKey(key) => {
                    if key == 57 {
                        committed.push(' ');
                    }
                }
            }
        }
    }

    #[test]
    fn wayland_and_ibus_shared_plan_preserves_text_without_duplicate_or_drop() {
        for input in ["tieengs ", "ATXX ", "dduwowngf ", "cargo "] {
            let mut outputs = Vec::new();
            for _frontend in ["wayland", "ibus"] {
                let mut engine = Engine::new(InputMethod::Telex);
                engine.set_auto_restore(false);
                let mut preedit = String::new();
                let mut committed = String::new();

                for ch in input.chars() {
                    let key = if ch == ' ' { 57 } else { ch as u32 };
                    let actions = plan_char(true, &mut engine, &preedit, ch, key);
                    render(actions, &mut preedit, &mut committed);
                }
                committed.push_str(&preedit);
                outputs.push(committed);
            }

            assert_eq!(outputs[0], outputs[1], "input={input}");
        }
    }
}
