// SPDX-License-Identifier: MIT
//! Physical keyboard layout mapping (US QWERTY, US Dvorak, US Colemak).
//!
//! Single Source of Truth shared between:
//! - `dboard-daemon` (translating raw `evdev` scancodes into characters for the
//!   linguistic engine, and reporting the active layout in `dboard doctor`),
//! - `dboard-protocol` (generating the matching XKB v1 8-level keymap for
//!   `zwp_virtual_keyboard_v1` and mapping replacement characters back to
//!   `(evdev_code, shift)` strokes), and
//! - spatial autocorrect geometry (`key_coord_for_layout`).

/// Supported physical ANSI/ISO keyboard layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardLayout {
    #[default]
    UsQwerty,
    Dvorak,
    Colemak,
}

impl KeyboardLayout {
    /// Parse a user-facing layout identifier from `dboard.toml` (`keyboard_layout`).
    pub fn from_str_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "dvorak" | "us-dvorak" | "us_dvorak" => Self::Dvorak,
            "colemak" | "us-colemak" | "us_colemak" => Self::Colemak,
            _ => Self::UsQwerty,
        }
    }

    /// Canonical short name used in configuration and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsQwerty => "us",
            Self::Dvorak => "dvorak",
            Self::Colemak => "colemak",
        }
    }

    /// Human-readable label shown by `dboard doctor`.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::UsQwerty => "us (English (US QWERTY))",
            Self::Dvorak => "dvorak (English (US Dvorak))",
            Self::Colemak => "colemak (English (US Colemak))",
        }
    }
}

/// Evdev keycodes for the printable main-block keys (excluding keypad & space/tab/enter).
pub const PRINTABLE_EVDEV_CODES: &[u16] = &[
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, // AE01..AE12
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, // AD01..AD12
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43, // AC01..AC11, TLDE, BKSL
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, // AB01..AB10
    86, // LSGT (ISO 102nd key)
];

/// Returns `(base_char, shifted_char)` for a printable evdev keycode on the given layout.
pub fn key_pair_for_layout(layout: KeyboardLayout, code: u16) -> Option<(char, char)> {
    match layout {
        KeyboardLayout::UsQwerty => us_qwerty_pair(code),
        KeyboardLayout::Dvorak => dvorak_pair(code),
        KeyboardLayout::Colemak => colemak_pair(code),
    }
}

/// Returns `(base_char, shifted_char)` for a printable US QWERTY evdev keycode.
pub fn us_key_pair(code: u16) -> Option<(char, char)> {
    us_qwerty_pair(code)
}

fn us_qwerty_pair(code: u16) -> Option<(char, char)> {
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

fn dvorak_pair(code: u16) -> Option<(char, char)> {
    Some(match code {
        12 => ('[', '{'),
        13 => (']', '}'),
        16 => ('\'', '"'),
        17 => (',', '<'),
        18 => ('.', '>'),
        19 => ('p', 'P'),
        20 => ('y', 'Y'),
        21 => ('f', 'F'),
        22 => ('g', 'G'),
        23 => ('c', 'C'),
        24 => ('r', 'R'),
        25 => ('l', 'L'),
        26 => ('/', '?'),
        27 => ('=', '+'),
        30 => ('a', 'A'),
        31 => ('o', 'O'),
        32 => ('e', 'E'),
        33 => ('u', 'U'),
        34 => ('i', 'I'),
        35 => ('d', 'D'),
        36 => ('h', 'H'),
        37 => ('t', 'T'),
        38 => ('n', 'N'),
        39 => ('s', 'S'),
        40 => ('-', '_'),
        44 => (';', ':'),
        45 => ('q', 'Q'),
        46 => ('j', 'J'),
        47 => ('k', 'K'),
        48 => ('x', 'X'),
        49 => ('b', 'B'),
        50 => ('m', 'M'),
        51 => ('w', 'W'),
        52 => ('v', 'V'),
        53 => ('z', 'Z'),
        _ => return us_qwerty_pair(code),
    })
}

fn colemak_pair(code: u16) -> Option<(char, char)> {
    Some(match code {
        18 => ('f', 'F'),
        19 => ('p', 'P'),
        20 => ('g', 'G'),
        21 => ('j', 'J'),
        22 => ('l', 'L'),
        23 => ('u', 'U'),
        24 => ('y', 'Y'),
        25 => (';', ':'),
        31 => ('r', 'R'),
        32 => ('s', 'S'),
        33 => ('t', 'T'),
        34 => ('d', 'D'),
        36 => ('n', 'N'),
        37 => ('e', 'E'),
        38 => ('i', 'I'),
        39 => ('o', 'O'),
        49 => ('k', 'K'),
        _ => return us_qwerty_pair(code),
    })
}

/// Decode a raw Linux evdev keycode into its typed character on the specified layout.
/// NumLock-dependent keypad digits (`KEY_KP0..KEY_KP9`, `KEY_KPDOT`) intentionally
/// return `None` so `evdev_loop` resets composition and forwards them natively via uinput.
pub fn keycode_to_char_for_layout(
    layout: KeyboardLayout,
    evdev_code: u16,
    shift: bool,
    caps_lock: bool,
) -> Option<char> {
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
    let (base, shifted) = key_pair_for_layout(layout, evdev_code)?;
    let use_shifted = if base.is_ascii_alphabetic() {
        shift ^ caps_lock
    } else {
        shift
    };
    Some(if use_shifted { shifted } else { base })
}

/// Decode a raw Linux evdev keycode into its typed character on US QWERTY.
pub fn keycode_to_char(evdev_code: u16, shift: bool, caps_lock: bool) -> Option<char> {
    keycode_to_char_for_layout(KeyboardLayout::UsQwerty, evdev_code, shift, caps_lock)
}

/// Reverse lookup: find `(evdev_code, needs_shift)` for a printable char on the given layout.
pub fn char_to_evdev_and_shift_for_layout(layout: KeyboardLayout, ch: char) -> Option<(u32, bool)> {
    match ch {
        ' ' => return Some((57, false)),
        '\n' | '\r' => return Some((28, false)),
        '\t' => return Some((15, false)),
        _ => {}
    }
    for &code in PRINTABLE_EVDEV_CODES {
        if let Some((base, shifted)) = key_pair_for_layout(layout, code) {
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

/// Reverse lookup: find `(evdev_code, needs_shift)` for a printable US QWERTY char.
pub fn char_to_evdev_and_shift(ch: char) -> Option<(u32, bool)> {
    char_to_evdev_and_shift_for_layout(KeyboardLayout::UsQwerty, ch)
}

/// Physical `(row, col)` coordinates for an ANSI evdev scancode, shared across layouts.
fn physical_evdev_coord(code: u16) -> Option<(f32, f32)> {
    match code {
        2..=11 => Some((-1.0, (code - 2) as f32)),
        16..=25 => Some((0.0, (code - 16) as f32)),
        30..=39 => Some((1.0, (code - 30) as f32 + 0.25)),
        44..=53 => Some((2.0, (code - 44) as f32 + 0.75)),
        _ => None,
    }
}

/// Return physical `(row, col)` key coordinates for a character under `layout`,
/// derived from the single-source-of-truth evdev layout table.
pub fn key_coord_for_layout(layout: KeyboardLayout, ch: char) -> Option<(f32, f32)> {
    let lower = ch.to_ascii_lowercase();
    let (code, _) = char_to_evdev_and_shift_for_layout(layout, lower)?;
    physical_evdev_coord(code as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_layouts_cover_all_26_ascii_letters_and_10_digits_bijectively() {
        for layout in [
            KeyboardLayout::UsQwerty,
            KeyboardLayout::Dvorak,
            KeyboardLayout::Colemak,
        ] {
            for ch in 'a'..='z' {
                let (code, shifted) = char_to_evdev_and_shift_for_layout(layout, ch)
                    .expect("lowercase ASCII must be mapped");
                assert!(!shifted, "layout {layout:?} letter {ch} must be unshifted");
                assert_eq!(
                    keycode_to_char_for_layout(layout, code as u16, false, false),
                    Some(ch)
                );
                assert_eq!(
                    keycode_to_char_for_layout(layout, code as u16, true, false),
                    Some(ch.to_ascii_uppercase())
                );
                assert_eq!(
                    keycode_to_char_for_layout(layout, code as u16, false, true),
                    Some(ch.to_ascii_uppercase())
                );
                assert_eq!(
                    keycode_to_char_for_layout(layout, code as u16, true, true),
                    Some(ch)
                );
            }
            for ch in '0'..='9' {
                let (code, shifted) = char_to_evdev_and_shift_for_layout(layout, ch)
                    .expect("ASCII digit must be mapped");
                assert!(!shifted);
                assert_eq!(
                    keycode_to_char_for_layout(layout, code as u16, false, false),
                    Some(ch)
                );
            }
            // NumLock-dependent keypad keys must return None so evdev_loop preserves native navigation
            for kp_code in [71u16, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83] {
                assert_eq!(
                    keycode_to_char_for_layout(layout, kp_code, false, false),
                    None
                );
            }
        }
    }

    #[test]
    fn physical_coordinates_reflect_each_layout_geometry() {
        // US QWERTY: 'a' and 's' are adjacent on home row
        let (ra, ca) = key_coord_for_layout(KeyboardLayout::UsQwerty, 'a').unwrap();
        let (rs, cs) = key_coord_for_layout(KeyboardLayout::UsQwerty, 's').unwrap();
        assert_eq!(ra, rs);
        assert!((cs - ca - 1.0).abs() < 1e-4);

        // Dvorak: 'a' and 'o' are adjacent on home row
        let (ra, ca) = key_coord_for_layout(KeyboardLayout::Dvorak, 'a').unwrap();
        let (ro, co) = key_coord_for_layout(KeyboardLayout::Dvorak, 'o').unwrap();
        assert_eq!(ra, ro);
        assert!((co - ca - 1.0).abs() < 1e-4);

        // Colemak: 'a' and 'r' are adjacent on home row
        let (ra, ca) = key_coord_for_layout(KeyboardLayout::Colemak, 'a').unwrap();
        let (rr, cr) = key_coord_for_layout(KeyboardLayout::Colemak, 'r').unwrap();
        assert_eq!(ra, rr);
        assert!((cr - ca - 1.0).abs() < 1e-4);
    }

    #[test]
    fn layout_name_parsing_and_display_names() {
        assert_eq!(
            KeyboardLayout::from_str_name("us"),
            KeyboardLayout::UsQwerty
        );
        assert_eq!(
            KeyboardLayout::from_str_name("dvorak"),
            KeyboardLayout::Dvorak
        );
        assert_eq!(
            KeyboardLayout::from_str_name("Colemak"),
            KeyboardLayout::Colemak
        );
        assert_eq!(
            KeyboardLayout::from_str_name("unknown"),
            KeyboardLayout::UsQwerty
        );
        assert!(KeyboardLayout::Dvorak.display_name().contains("Dvorak"));
        assert!(KeyboardLayout::Colemak.display_name().contains("Colemak"));
    }
}
