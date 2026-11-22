// SPDX-License-Identifier: MIT
//! US QWERTY physical keyboard mapping.
//!
//! Single Source of Truth shared between:
//! - `dboard-daemon` (translating raw `evdev` scancodes into characters for the
//!   linguistic engine), and
//! - `dboard-protocol` (generating the matching XKB v1 8-level keymap for
//!   `zwp_virtual_keyboard_v1` and mapping replacement characters back to
//!   `(evdev_code, shift)` strokes).

/// Evdev keycodes for the printable main-block keys (excluding keypad & space/tab/enter).
pub const PRINTABLE_EVDEV_CODES: &[u16] = &[
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, // AE01..AE12
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, // AD01..AD12
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43, // AC01..AC11, TLDE, BKSL
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, // AB01..AB10
    86, // LSGT (ISO 102nd key)
];

/// Returns `(base_char, shifted_char)` for a printable US QWERTY evdev keycode.
pub fn us_key_pair(code: u16) -> Option<(char, char)> {
    Some(match code {
        2 => ('1', '!'),
        3 => ('2', '@'),
        4 => ('3', '#'),
        5 => ('4', '$'),
        6 => ('5', '%'),
        7 => ('6', '^'),
        8 => ('7', '&'),
        9 => ('8', '*'),
        10 => ('9', '('),
        11 => ('0', ')'),
        12 => ('-', '_'),
        13 => ('=', '+'),
        16 => ('q', 'Q'),
        17 => ('w', 'W'),
        18 => ('e', 'E'),
        19 => ('r', 'R'),
        20 => ('t', 'T'),
        21 => ('y', 'Y'),
        22 => ('u', 'U'),
        23 => ('i', 'I'),
        24 => ('o', 'O'),
        25 => ('p', 'P'),
        26 => ('[', '{'),
        27 => (']', '}'),
        30 => ('a', 'A'),
        31 => ('s', 'S'),
        32 => ('d', 'D'),
        33 => ('f', 'F'),
        34 => ('g', 'G'),
        35 => ('h', 'H'),
        36 => ('j', 'J'),
        37 => ('k', 'K'),
        38 => ('l', 'L'),
        39 => (';', ':'),
        40 => ('\'', '"'),
        41 => ('`', '~'),
        43 => ('\\', '|'),
        44 => ('z', 'Z'),
        45 => ('x', 'X'),
        46 => ('c', 'C'),
        47 => ('v', 'V'),
        48 => ('b', 'B'),
        49 => ('n', 'N'),
        50 => ('m', 'M'),
        51 => (',', '<'),
        52 => ('.', '>'),
        53 => ('/', '?'),
        86 => ('<', '>'),
        _ => return None,
    })
}

/// Decode a raw Linux evdev keycode into its typed character on US QWERTY.
/// NumLock-dependent keypad digits (`KEY_KP0..KEY_KP9`, `KEY_KPDOT`) intentionally
/// return `None` so `evdev_loop` resets composition and forwards them natively via uinput.
pub fn keycode_to_char(evdev_code: u16, shift: bool, caps_lock: bool) -> Option<char> {
    match evdev_code {
        14 => return Some('\x08'),
        57 => return Some(' '),
        28 | 96 => return Some('\n'),
        15 => return Some('\t'),
        // NumLock-independent numpad operators
        98 => return Some('/'),
        55 => return Some('*'),
        74 => return Some('-'),
        78 => return Some('+'),
        _ => {}
    }
    let (base, shifted) = us_key_pair(evdev_code)?;
    let use_shifted = if base.is_ascii_alphabetic() {
        shift ^ caps_lock
    } else {
        shift
    };
    Some(if use_shifted { shifted } else { base })
}

/// Reverse lookup: find `(evdev_code, needs_shift)` for a printable US QWERTY char.
pub fn char_to_evdev_and_shift(ch: char) -> Option<(u32, bool)> {
    match ch {
        ' ' => return Some((57, false)),
        '\n' | '\r' => return Some((28, false)),
        '\t' => return Some((15, false)),
        _ => {}
    }
    for &code in PRINTABLE_EVDEV_CODES {
        if let Some((base, shifted)) = us_key_pair(code) {
            if base == ch {
                return Some((code as u32, false));
            }
            if shifted == ch {
                return Some((code as u32, true));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_qwerty_covers_all_26_ascii_letters_and_10_digits_bijectively() {
        for ch in 'a'..='z' {
            let (code, shifted) =
                char_to_evdev_and_shift(ch).expect("lowercase ASCII must be mapped");
            assert!(!shifted);
            assert_eq!(keycode_to_char(code as u16, false, false), Some(ch));
            assert_eq!(
                keycode_to_char(code as u16, true, false),
                Some(ch.to_ascii_uppercase())
            );
            assert_eq!(
                keycode_to_char(code as u16, false, true),
                Some(ch.to_ascii_uppercase())
            );
            assert_eq!(keycode_to_char(code as u16, true, true), Some(ch));
        }
        for ch in '0'..='9' {
            let (code, shifted) = char_to_evdev_and_shift(ch).expect("ASCII digit must be mapped");
            assert!(!shifted);
            assert_eq!(keycode_to_char(code as u16, false, false), Some(ch));
        }
        // NumLock-dependent keypad keys must return None so evdev_loop preserves native navigation
        for kp_code in [71u16, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83] {
            assert_eq!(keycode_to_char(kp_code, false, false), None);
        }
    }
}
