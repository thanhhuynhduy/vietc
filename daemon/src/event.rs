/// Characters that flush the current word and start a new one.
pub fn is_flush_char(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\u{00A0}'
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
            | '—'
            | '–'
            | '…'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '«'
            | '»'
    )
}

pub fn is_vn_control_key(method: &str, ch: char) -> bool {
    match method {
        "telex" => matches!(
            ch.to_ascii_lowercase(),
            'f' | 's' | 'r' | 'x' | 'j' | 'w' | 'z'
        ),
        "vni" => matches!(
            ch,
            '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '0'
        ),
        _ => false,
    }
}

pub fn is_modifier_pressed(key_state: &evdev::AttributeSet<evdev::KeyCode>) -> bool {
    key_state.contains(evdev::KeyCode::KEY_LEFTCTRL)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTCTRL)
        || key_state.contains(evdev::KeyCode::KEY_LEFTALT)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTALT)
        || key_state.contains(evdev::KeyCode::KEY_LEFTMETA)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTMETA)
}

pub fn is_modifier_held_shift(key_state: &evdev::AttributeSet<evdev::KeyCode>) -> bool {
    key_state.contains(evdev::KeyCode::KEY_LEFTSHIFT)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTSHIFT)
}

pub fn is_caps_lock_on(device: &evdev::Device) -> bool {
    if let Ok(leds) = device.get_led_state() {
        leds.contains(evdev::LedCode::LED_CAPSL)
    } else {
        false
    }
}

pub fn is_num_lock_on(device: &evdev::Device) -> bool {
    if let Ok(leds) = device.get_led_state() {
        leds.contains(evdev::LedCode::LED_NUML)
    } else {
        false
    }
}

pub fn update_keyboard_state(
    key_state: &mut evdev::AttributeSet<evdev::KeyCode>,
    caps_lock: &mut bool,
    num_lock: &mut bool,
    key: evdev::KeyCode,
    value: i32,
) {
    if value == 1 {
        key_state.insert(key);
    } else if value == 0 {
        key_state.remove(key);
    }

    if key == evdev::KeyCode::KEY_CAPSLOCK && value == 1 {
        *caps_lock = !*caps_lock;
    } else if key == evdev::KeyCode::KEY_NUMLOCK && value == 1 {
        *num_lock = !*num_lock;
    }
}

pub fn is_toggle_key(key: evdev::KeyCode, key_name: &str) -> bool {
    let lower = key_name.to_lowercase();
    if matches!(lower.as_str(), "alt+z" | "z") {
        return matches!(
            key,
            evdev::KeyCode::KEY_Z | evdev::KeyCode::KEY_LEFTALT | evdev::KeyCode::KEY_RIGHTALT
        );
    }
    if key == evdev::KeyCode::KEY_LEFTCTRL || key == evdev::KeyCode::KEY_RIGHTCTRL {
        return true;
    }
    let target = match lower.as_str() {
        "space" | "ctrl+space" => evdev::KeyCode::KEY_SPACE,
        "shift" | "ctrl+shift" => {
            return key == evdev::KeyCode::KEY_LEFTSHIFT || key == evdev::KeyCode::KEY_RIGHTSHIFT
        }
        "capslock" => evdev::KeyCode::KEY_CAPSLOCK,
        "ctrl" => {
            return key == evdev::KeyCode::KEY_LEFTCTRL || key == evdev::KeyCode::KEY_RIGHTCTRL
        }
        "alt" | "ctrl+alt" => {
            return key == evdev::KeyCode::KEY_LEFTALT || key == evdev::KeyCode::KEY_RIGHTALT
        }
        _ => return false,
    };
    key == target
}

pub fn is_toggle_combination_state(
    key_state: &evdev::AttributeSet<evdev::KeyCode>,
    key: &str,
) -> bool {
    let lower = key.to_lowercase();
    let ctrl_pressed = key_state.contains(evdev::KeyCode::KEY_LEFTCTRL)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTCTRL);
    let alt_pressed = key_state.contains(evdev::KeyCode::KEY_LEFTALT)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTALT);
    let shift_pressed = key_state.contains(evdev::KeyCode::KEY_LEFTSHIFT)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTSHIFT);
    let meta_pressed = key_state.contains(evdev::KeyCode::KEY_LEFTMETA)
        || key_state.contains(evdev::KeyCode::KEY_RIGHTMETA);

    if matches!(lower.as_str(), "alt+z" | "z") {
        return alt_pressed
            && !ctrl_pressed
            && !shift_pressed
            && !meta_pressed
            && key_state.contains(evdev::KeyCode::KEY_Z);
    }

    if !ctrl_pressed || meta_pressed {
        return false;
    }

    match lower.as_str() {
        "space" | "ctrl+space" => {
            !alt_pressed && !shift_pressed && key_state.contains(evdev::KeyCode::KEY_SPACE)
        }
        "shift" | "ctrl+shift" => !alt_pressed && shift_pressed,
        "capslock" => {
            !alt_pressed && !shift_pressed && key_state.contains(evdev::KeyCode::KEY_CAPSLOCK)
        }
        "ctrl" => !alt_pressed && !shift_pressed,
        "alt" | "ctrl+alt" => alt_pressed && !shift_pressed,
        _ => false,
    }
}

/// Handle KeyUp (`value == 0`) and KeyRepeat (`value == 2`) for keys previously
/// consumed on KeyDown (e.g., toggle chord keys, Telex transform triggers, or
/// atomic Space). Must run before modifier-passthrough so releasing a consumed
/// key while Ctrl/Alt/Meta is held never leaks `consumed_keys` or emits phantom
/// shortcuts.
pub fn consume_released_or_repeated_key(
    consumed_keys: &mut std::collections::HashSet<u16>,
    keycode: u16,
    value: i32,
) -> bool {
    match value {
        0 => consumed_keys.remove(&keycode),
        2 => consumed_keys.contains(&keycode),
        _ => false,
    }
}

pub fn key_to_char_with_case(key: evdev::KeyCode, shift: bool, caps_lock: bool) -> Option<char> {
    key_to_char(key).map(|ch| {
        if ch.is_ascii_alphabetic() {
            if shift ^ caps_lock {
                ch.to_ascii_uppercase()
            } else {
                ch
            }
        } else if shift {
            match ch {
                '1' => '!',
                '2' => '@',
                '3' => '#',
                '4' => '$',
                '5' => '%',
                '6' => '^',
                '7' => '&',
                '8' => '*',
                '9' => '(',
                '0' => ')',
                '-' => '_',
                '=' => '+',
                '[' => '{',
                ']' => '}',
                '\\' => '|',
                ';' => ':',
                '\'' => '"',
                '`' => '~',
                ',' => '<',
                '.' => '>',
                '/' => '?',
                other => other,
            }
        } else {
            ch
        }
    })
}

pub fn key_to_char(key: evdev::KeyCode) -> Option<char> {
    match key {
        evdev::KeyCode::KEY_A => Some('a'),
        evdev::KeyCode::KEY_B => Some('b'),
        evdev::KeyCode::KEY_C => Some('c'),
        evdev::KeyCode::KEY_D => Some('d'),
        evdev::KeyCode::KEY_E => Some('e'),
        evdev::KeyCode::KEY_F => Some('f'),
        evdev::KeyCode::KEY_G => Some('g'),
        evdev::KeyCode::KEY_H => Some('h'),
        evdev::KeyCode::KEY_I => Some('i'),
        evdev::KeyCode::KEY_J => Some('j'),
        evdev::KeyCode::KEY_K => Some('k'),
        evdev::KeyCode::KEY_L => Some('l'),
        evdev::KeyCode::KEY_M => Some('m'),
        evdev::KeyCode::KEY_N => Some('n'),
        evdev::KeyCode::KEY_O => Some('o'),
        evdev::KeyCode::KEY_P => Some('p'),
        evdev::KeyCode::KEY_Q => Some('q'),
        evdev::KeyCode::KEY_R => Some('r'),
        evdev::KeyCode::KEY_S => Some('s'),
        evdev::KeyCode::KEY_T => Some('t'),
        evdev::KeyCode::KEY_U => Some('u'),
        evdev::KeyCode::KEY_V => Some('v'),
        evdev::KeyCode::KEY_W => Some('w'),
        evdev::KeyCode::KEY_X => Some('x'),
        evdev::KeyCode::KEY_Y => Some('y'),
        evdev::KeyCode::KEY_Z => Some('z'),
        evdev::KeyCode::KEY_0 => Some('0'),
        evdev::KeyCode::KEY_1 => Some('1'),
        evdev::KeyCode::KEY_2 => Some('2'),
        evdev::KeyCode::KEY_3 => Some('3'),
        evdev::KeyCode::KEY_4 => Some('4'),
        evdev::KeyCode::KEY_5 => Some('5'),
        evdev::KeyCode::KEY_6 => Some('6'),
        evdev::KeyCode::KEY_7 => Some('7'),
        evdev::KeyCode::KEY_8 => Some('8'),
        evdev::KeyCode::KEY_9 => Some('9'),
        evdev::KeyCode::KEY_SPACE => Some(' '),
        evdev::KeyCode::KEY_TAB => Some('\t'),
        evdev::KeyCode::KEY_DOT => Some('.'),
        evdev::KeyCode::KEY_COMMA => Some(','),
        evdev::KeyCode::KEY_MINUS => Some('-'),
        evdev::KeyCode::KEY_EQUAL => Some('='),
        evdev::KeyCode::KEY_LEFTBRACE => Some('['),
        evdev::KeyCode::KEY_RIGHTBRACE => Some(']'),
        evdev::KeyCode::KEY_BACKSLASH => Some('\\'),
        evdev::KeyCode::KEY_SEMICOLON => Some(';'),
        evdev::KeyCode::KEY_APOSTROPHE => Some('\''),
        evdev::KeyCode::KEY_GRAVE => Some('`'),
        evdev::KeyCode::KEY_SLASH | evdev::KeyCode::KEY_KPSLASH => Some('/'),
        evdev::KeyCode::KEY_KPASTERISK => Some('*'),
        evdev::KeyCode::KEY_KPPLUS => Some('+'),
        evdev::KeyCode::KEY_BACKSPACE => Some('\x08'),
        evdev::KeyCode::KEY_ENTER | evdev::KeyCode::KEY_KPENTER => Some('\n'),
        evdev::KeyCode::KEY_KPMINUS => Some('-'),
        _ => None,
    }
}

/// Keys that move the caret, toggle Insert/Overwrite mode, invoke function/menu
/// actions, or emit NumLock-dependent keypad actions — always reset composition.
pub fn is_navigation_or_control_key(key: evdev::KeyCode) -> bool {
    matches!(
        key,
        evdev::KeyCode::KEY_LEFT
            | evdev::KeyCode::KEY_RIGHT
            | evdev::KeyCode::KEY_UP
            | evdev::KeyCode::KEY_DOWN
            | evdev::KeyCode::KEY_HOME
            | evdev::KeyCode::KEY_END
            | evdev::KeyCode::KEY_PAGEUP
            | evdev::KeyCode::KEY_PAGEDOWN
            | evdev::KeyCode::KEY_DELETE
            | evdev::KeyCode::KEY_INSERT
            | evdev::KeyCode::KEY_ESC
            | evdev::KeyCode::KEY_TAB
            | evdev::KeyCode::KEY_MENU
            | evdev::KeyCode::KEY_F1
            | evdev::KeyCode::KEY_F2
            | evdev::KeyCode::KEY_F3
            | evdev::KeyCode::KEY_F4
            | evdev::KeyCode::KEY_F5
            | evdev::KeyCode::KEY_F6
            | evdev::KeyCode::KEY_F7
            | evdev::KeyCode::KEY_F8
            | evdev::KeyCode::KEY_F9
            | evdev::KeyCode::KEY_F10
            | evdev::KeyCode::KEY_F11
            | evdev::KeyCode::KEY_F12
            | evdev::KeyCode::KEY_KP0
            | evdev::KeyCode::KEY_KP1
            | evdev::KeyCode::KEY_KP2
            | evdev::KeyCode::KEY_KP3
            | evdev::KeyCode::KEY_KP4
            | evdev::KeyCode::KEY_KP5
            | evdev::KeyCode::KEY_KP6
            | evdev::KeyCode::KEY_KP7
            | evdev::KeyCode::KEY_KP8
            | evdev::KeyCode::KEY_KP9
            | evdev::KeyCode::KEY_KPDOT
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::OutputCommand;
    use vietc_engine::{Engine, EngineEvent, InputMethod};

    fn event_to_commands(event: Option<EngineEvent>) -> Vec<OutputCommand> {
        let mut commands = Vec::new();
        if let Some(event) = event {
            match event {
                EngineEvent::Flush(text) | EngineEvent::Insert(text) | EngineEvent::Paste(text) => {
                    commands.push(OutputCommand::Type(text));
                }
                EngineEvent::AutoRestore(word) => {
                    commands.push(OutputCommand::Backspace(word.chars().count()));
                    commands.push(OutputCommand::Type(word));
                }
                EngineEvent::Replace(replacement) => {
                    commands.push(OutputCommand::Backspace(replacement.delete_before_cursor));
                    commands.push(OutputCommand::Type(replacement.insert));
                }
                EngineEvent::UndoTones {
                    backspaces,
                    restored,
                } => {
                    commands.push(OutputCommand::Backspace(backspaces));
                    commands.push(OutputCommand::Type(restored));
                }
            }
        }
        commands
    }

    fn render(method_str: &str, keys: &str) -> String {
        let method = match method_str {
            "vni" => InputMethod::Vni,
            _ => InputMethod::Telex,
        };
        let mut engine = Engine::new(method);
        engine.set_enabled(true);
        engine.set_auto_restore(true);

        let mut screen: Vec<char> = Vec::new();
        for ch in keys.chars() {
            let buf_before = engine.buffer().chars().count();
            let commands = event_to_commands(engine.process_key(ch));
            if !commands.is_empty() {
                for cmd in &commands {
                    match cmd {
                        OutputCommand::Backspace(n) => {
                            for _ in 0..*n {
                                screen.pop();
                            }
                        }
                        OutputCommand::Type(text) => screen.extend(text.chars()),
                        OutputCommand::Replace(replacement) => {
                            for _ in 0..replacement.delete_before_cursor {
                                screen.pop();
                            }
                            screen.extend(replacement.insert.chars());
                        }
                    }
                }
                if is_flush_char(ch) {
                    screen.push(ch);
                }
            } else if is_vn_control_key(method_str, ch)
                && engine.buffer().chars().count() <= buf_before
            {
            } else {
                screen.push(ch);
            }
        }
        screen.into_iter().collect()
    }

    #[test]
    fn leading_control_letters_are_kept() {
        assert_eq!(render("telex", "xuaw"), "xưa");
        assert_eq!(render("telex", "trong"), "trong");
        assert_eq!(render("telex", "ruwngf"), "rừng");
    }

    #[test]
    fn spaces_between_words_are_preserved() {
        assert_eq!(render("telex", "Ngayf xuaw"), "Ngày xưa");
        assert_eq!(render("telex", "khu ruwngf raamj"), "khu rừng rậm");
        assert_eq!(render("telex", "con Voi raats"), "con Voi rất");
    }

    #[test]
    fn full_sentence_renders_correctly() {
        let keys = "Ngayf xuaw, trong mootj khu ruwngf raamj cos mootj con Voi raats hung duwx.";
        let expected = "Ngày xưa, trong một khu rừng rậm có một con Voi rất hung dữ.";
        assert_eq!(render("telex", keys), expected);
    }

    #[test]
    fn uppercase_telex_raw_keys_survive_runtime_injection_path() {
        let raw = [
            evdev::KeyCode::KEY_T,
            evdev::KeyCode::KEY_I,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_E,
            evdev::KeyCode::KEY_N,
            evdev::KeyCode::KEY_G,
            evdev::KeyCode::KEY_S,
        ];
        let keys = raw
            .into_iter()
            .map(|key| key_to_char_with_case(key, true, false).expect("printable key"))
            .collect::<String>();

        assert_eq!(keys, "TIEENGS");
        assert_eq!(render("telex", &keys), "TIẾNG");
    }

    #[test]
    fn test_toggle_key_filtering() {
        assert!(is_toggle_key(evdev::KeyCode::KEY_SPACE, "space"));
        assert!(is_toggle_key(evdev::KeyCode::KEY_LEFTCTRL, "space"));
        assert!(!is_toggle_key(evdev::KeyCode::KEY_T, "space"));
        assert!(is_toggle_key(evdev::KeyCode::KEY_LEFTSHIFT, "shift"));
        assert!(is_toggle_key(evdev::KeyCode::KEY_RIGHTSHIFT, "shift"));
    }

    #[test]
    fn shifted_punctuation_and_parentheses_translate_and_compose_cleanly() {
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_9, true, false),
            Some('(')
        );
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_0, true, false),
            Some(')')
        );
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_9, true, true),
            Some('(')
        );
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_1, true, false),
            Some('!')
        );
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_LEFTBRACE, false, false),
            Some('[')
        );
        assert_eq!(
            key_to_char_with_case(evdev::KeyCode::KEY_LEFTBRACE, true, false),
            Some('{')
        );

        // Physical sequence for "(nhuw)": Shift+9, n, h, u, w, Shift+0
        let seq = [
            (evdev::KeyCode::KEY_9, true),
            (evdev::KeyCode::KEY_N, false),
            (evdev::KeyCode::KEY_H, false),
            (evdev::KeyCode::KEY_U, false),
            (evdev::KeyCode::KEY_W, false),
            (evdev::KeyCode::KEY_0, true),
        ];
        let translated: String = seq
            .into_iter()
            .map(|(k, shift)| key_to_char_with_case(k, shift, false).unwrap())
            .collect();
        assert_eq!(translated, "(nhuw)");
        assert_eq!(render("telex", &translated), "(như)");

        // Physical sequence for "(nhuw" followed by Backspace must yield "(nhu", never "9nhu"
        let seq_with_bs = [
            (evdev::KeyCode::KEY_9, true),
            (evdev::KeyCode::KEY_N, false),
            (evdev::KeyCode::KEY_H, false),
            (evdev::KeyCode::KEY_U, false),
            (evdev::KeyCode::KEY_W, false),
            (evdev::KeyCode::KEY_BACKSPACE, false),
        ];
        let translated_bs: String = seq_with_bs
            .into_iter()
            .map(|(k, shift)| key_to_char_with_case(k, shift, false).unwrap())
            .collect();
        assert_eq!(render("telex", &translated_bs), "(nhu");
    }

    #[test]
    fn toggle_combination_requires_exact_modifiers() {
        let mut ks = evdev::AttributeSet::new();
        ks.insert(evdev::KeyCode::KEY_LEFTCTRL);
        ks.insert(evdev::KeyCode::KEY_SPACE);
        assert!(is_toggle_combination_state(&ks, "space"));
        assert!(is_toggle_combination_state(&ks, "ctrl+space"));

        // Ctrl+Shift+Space (VS Code Parameter Hints / LibreOffice NBSP) must NOT toggle!
        ks.insert(evdev::KeyCode::KEY_LEFTSHIFT);
        assert!(!is_toggle_combination_state(&ks, "space"));
        ks.remove(evdev::KeyCode::KEY_LEFTSHIFT);

        // Ctrl+Alt+Space and Ctrl+Super+Space must NOT toggle!
        ks.insert(evdev::KeyCode::KEY_LEFTALT);
        assert!(!is_toggle_combination_state(&ks, "space"));
        ks.remove(evdev::KeyCode::KEY_LEFTALT);

        ks.insert(evdev::KeyCode::KEY_LEFTMETA);
        assert!(!is_toggle_combination_state(&ks, "space"));
    }

    #[test]
    fn consumed_keys_clean_up_on_release_and_repeat() {
        let mut consumed = std::collections::HashSet::from([31u16, 57u16]);
        assert!(consume_released_or_repeated_key(&mut consumed, 31, 2));
        assert!(consumed.contains(&31));
        assert!(consume_released_or_repeated_key(&mut consumed, 31, 0));
        assert!(!consumed.contains(&31));
        assert!(!consume_released_or_repeated_key(&mut consumed, 31, 0));
        assert!(consume_released_or_repeated_key(&mut consumed, 57, 0));
        assert!(consumed.is_empty());
    }
}
