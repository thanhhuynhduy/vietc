// SPDX-License-Identifier: MIT
//! Complete standards-compliant XKB v1 keymap for Dboard's persistent Wayland
//! virtual keyboard (`zwp_virtual_keyboard_v1`).
//!
//! Design invariants:
//! 1. Uploaded ONCE per connection (zero dynamic keymap churn while typing).
//! 2. Defines all 8 real XKB modifiers (`Shift`, `Lock`, `Control`, `Mod1`..`Mod5`)
//!    and standard virtual modifiers (`NumLock`, `Alt`, `Super`, `Meta`) so
//!    Chromium/Edge/Electron, GTK4, and Qt6 never encounter `XKB_MOD_INVALID`.
//! 3. Keeps all XKB keycodes strictly within `8..=255` (`evdev + 8`), ensuring
//!    8-bit XWayland compatibility alongside native Wayland clients.
//! 4. Maps all 134 precomposed Vietnamese Unicode characters onto dedicated 8-level
//!    alphanumeric evdev keycodes (`16..=47`), completely eliminating high keycodes
//!    (>= 112) that trigger Chromium DomCode collisions (`KEY_UNDO` 131, `KEY_COMPOSE` 127).

use std::fmt::Write;

use crate::layout::{char_to_evdev_and_shift_for_layout, key_pair_for_layout, KeyboardLayout};

pub(crate) const XKB_MOD_SHIFT: u32 = 1 << 0;
pub(crate) const XKB_MOD_LOCK: u32 = 1 << 1;
pub(crate) const XKB_MOD_MOD2: u32 = 1 << 4;
pub(crate) const XKB_MOD_MOD3: u32 = 1 << 5;
pub(crate) const XKB_MOD_MOD5: u32 = 1 << 7;

pub(crate) const VIETNAMESE_PAIRS: [(char, char); 67] = [
    // a & vowels
    ('à', 'À'),
    ('á', 'Á'),
    ('ả', 'Ả'),
    ('ã', 'Ã'),
    ('ạ', 'Ạ'),
    ('ă', 'Ă'),
    ('ằ', 'Ằ'),
    ('ắ', 'Ắ'),
    ('ẳ', 'Ẳ'),
    ('ẵ', 'Ẵ'),
    ('ặ', 'Ặ'),
    ('â', 'Â'),
    ('ầ', 'Ầ'),
    ('ấ', 'Ấ'),
    ('ẩ', 'Ẩ'),
    ('ẫ', 'Ẫ'),
    ('ậ', 'Ậ'),
    ('è', 'È'),
    ('é', 'É'),
    ('ẻ', 'Ẻ'),
    ('ẽ', 'Ẽ'),
    ('ẹ', 'Ẹ'),
    ('ê', 'Ê'),
    ('ề', 'Ề'),
    ('ế', 'Ế'),
    ('ể', 'Ể'),
    ('ễ', 'Ễ'),
    ('ệ', 'Ệ'),
    ('ì', 'Ì'),
    ('í', 'Í'),
    ('ỉ', 'Ỉ'),
    ('ĩ', 'Ĩ'),
    ('ị', 'Ị'),
    ('ò', 'Ò'),
    ('ó', 'Ó'),
    ('ỏ', 'Ỏ'),
    ('õ', 'Õ'),
    ('ọ', 'Ọ'),
    ('ô', 'Ô'),
    ('ồ', 'Ồ'),
    ('ố', 'Ố'),
    ('ổ', 'Ổ'),
    ('ỗ', 'Ỗ'),
    ('ộ', 'Ộ'),
    ('ơ', 'Ơ'),
    ('ờ', 'Ờ'),
    ('ớ', 'Ớ'),
    ('ở', 'Ở'),
    ('ỡ', 'Ỡ'),
    ('ợ', 'Ợ'),
    ('ù', 'Ù'),
    ('ú', 'Ú'),
    ('ủ', 'Ủ'),
    ('ũ', 'Ũ'),
    ('ụ', 'Ụ'),
    ('ư', 'Ư'),
    ('ừ', 'Ừ'),
    ('ứ', 'Ứ'),
    ('ử', 'Ử'),
    ('ữ', 'Ữ'),
    ('ự', 'Ự'),
    ('ỳ', 'Ỳ'),
    ('ý', 'Ý'),
    ('ỷ', 'Ỷ'),
    ('ỹ', 'Ỹ'),
    ('ỵ', 'Ỵ'),
    ('đ', 'Đ'),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VkKeyStroke {
    pub evdev_code: u32,
    pub modifiers: u32,
}

const EIGHT_LEVEL_KEYS: [(&str, u32, &str, &str); 23] = [
    ("AD01", 16, "q", "Q"),
    ("AD02", 17, "w", "W"),
    ("AD03", 18, "e", "E"),
    ("AD04", 19, "r", "R"),
    ("AD05", 20, "t", "T"),
    ("AD06", 21, "y", "Y"),
    ("AD07", 22, "u", "U"),
    ("AD08", 23, "i", "I"),
    ("AD09", 24, "o", "O"),
    ("AD10", 25, "p", "P"),
    ("AC01", 30, "a", "A"),
    ("AC02", 31, "s", "S"),
    ("AC03", 32, "d", "D"),
    ("AC04", 33, "f", "F"),
    ("AC05", 34, "g", "G"),
    ("AC06", 35, "h", "H"),
    ("AC07", 36, "j", "J"),
    ("AC08", 37, "k", "K"),
    ("AC09", 38, "l", "L"),
    ("AB01", 44, "z", "Z"),
    ("AB02", 45, "x", "X"),
    ("AB03", 46, "c", "C"),
    ("AB04", 47, "v", "V"),
];

/// Returns `true` if `evdev_code` is bound to `DBOARD_EIGHT_LEVEL`, whose
/// levels are shifted by `Lock` (`CapsLock`) regardless of base ASCII layout
/// (e.g. Dvorak `AD01..AD03`/`AB01` carry Vietnamese vowels on levels 3..8).
pub(crate) fn is_eight_level_evdev(evdev_code: u32) -> bool {
    EIGHT_LEVEL_KEYS
        .iter()
        .any(|&(_, code, _, _)| code == evdev_code)
}

pub(crate) fn char_to_vk_stroke_for_layout(
    layout: KeyboardLayout,
    ch: char,
) -> Option<VkKeyStroke> {
    for (pair_idx, &(lo, up)) in VIETNAMESE_PAIRS.iter().enumerate() {
        if ch == lo || ch == up {
            let key_idx = pair_idx / 3;
            let slot = pair_idx % 3;
            let (_, evdev_code, _, _) = EIGHT_LEVEL_KEYS[key_idx];
            let is_upper = ch == up;
            let modifiers = match (slot, is_upper) {
                (0, false) => XKB_MOD_MOD5,
                (0, true) => XKB_MOD_SHIFT | XKB_MOD_MOD5,
                (1, false) => XKB_MOD_MOD3,
                (1, true) => XKB_MOD_SHIFT | XKB_MOD_MOD3,
                (2, false) => XKB_MOD_MOD5 | XKB_MOD_MOD3,
                (2, true) => XKB_MOD_SHIFT | XKB_MOD_MOD5 | XKB_MOD_MOD3,
                _ => unreachable!(),
            };
            return Some(VkKeyStroke {
                evdev_code,
                modifiers,
            });
        }
    }

    let (evdev_code, needs_shift) = char_to_evdev_and_shift_for_layout(layout, ch)?;
    Some(VkKeyStroke {
        evdev_code,
        modifiers: if needs_shift { XKB_MOD_SHIFT } else { 0 },
    })
}

#[cfg(test)]
pub(crate) fn char_to_vk_stroke(ch: char) -> Option<VkKeyStroke> {
    char_to_vk_stroke_for_layout(KeyboardLayout::UsQwerty, ch)
}

/// Returns `true` if `keycode` is a standard typing, editing, navigation, or
/// NumLock-independent Numpad delimiter key defined in our virtual-keyboard XKB
/// keymap (excluding system modifiers, CapsLock, and NumLock-dependent keypad
/// digits so compositor shortcuts, hardware LEDs, and NumLock OFF navigation
/// stay 100% native on uinput).
pub(crate) fn is_vk_typing_or_nav_keycode(keycode: u16) -> bool {
    matches!(
        keycode,
        1..=28
            | 30..=43
            | 44..=53
            | 55
            | 57
            | 59..=68
            | 74
            | 78
            | 86..=88
            | 96
            | 98
            | 102..=111
    )
}

const STANDARD_KEYS: &[(&str, u32, &str, &str)] = &[
    ("ESC", 1, "ONE_LEVEL", "[ Escape ]"),
    ("AE01", 2, "TWO_LEVEL", "[ 1, exclam ]"),
    ("AE02", 3, "TWO_LEVEL", "[ 2, at ]"),
    ("AE03", 4, "TWO_LEVEL", "[ 3, numbersign ]"),
    ("AE04", 5, "TWO_LEVEL", "[ 4, dollar ]"),
    ("AE05", 6, "TWO_LEVEL", "[ 5, percent ]"),
    ("AE06", 7, "TWO_LEVEL", "[ 6, asciicircum ]"),
    ("AE07", 8, "TWO_LEVEL", "[ 7, ampersand ]"),
    ("AE08", 9, "TWO_LEVEL", "[ 8, asterisk ]"),
    ("AE09", 10, "TWO_LEVEL", "[ 9, parenleft ]"),
    ("AE10", 11, "TWO_LEVEL", "[ 0, parenright ]"),
    ("AE11", 12, "TWO_LEVEL", "[ minus, underscore ]"),
    ("AE12", 13, "TWO_LEVEL", "[ equal, plus ]"),
    ("BKSP", 14, "ONE_LEVEL", "[ BackSpace ]"),
    ("TAB", 15, "TWO_LEVEL", "[ Tab, ISO_Left_Tab ]"),
    ("AD11", 26, "TWO_LEVEL", "[ bracketleft, braceleft ]"),
    ("AD12", 27, "TWO_LEVEL", "[ bracketright, braceright ]"),
    ("RTRN", 28, "TWO_LEVEL", "[ Return, Return ]"),
    ("LCTL", 29, "ONE_LEVEL", "[ Control_L ]"),
    ("AC10", 39, "TWO_LEVEL", "[ semicolon, colon ]"),
    ("AC11", 40, "TWO_LEVEL", "[ apostrophe, quotedbl ]"),
    ("TLDE", 41, "TWO_LEVEL", "[ grave, asciitilde ]"),
    ("LFSH", 42, "ONE_LEVEL", "[ Shift_L ]"),
    ("BKSL", 43, "TWO_LEVEL", "[ backslash, bar ]"),
    ("AB05", 48, "ALPHABETIC", "[ b, B ]"),
    ("AB06", 49, "ALPHABETIC", "[ n, N ]"),
    ("AB07", 50, "ALPHABETIC", "[ m, M ]"),
    ("AB08", 51, "TWO_LEVEL", "[ comma, less ]"),
    ("AB09", 52, "TWO_LEVEL", "[ period, greater ]"),
    ("AB10", 53, "TWO_LEVEL", "[ slash, question ]"),
    ("RTSH", 54, "ONE_LEVEL", "[ Shift_R ]"),
    ("KPMU", 55, "ONE_LEVEL", "[ asterisk ]"),
    ("LALT", 56, "ONE_LEVEL", "[ Alt_L ]"),
    ("SPCE", 57, "ONE_LEVEL", "[ space ]"),
    ("CAPS", 58, "ONE_LEVEL", "[ Caps_Lock ]"),
    ("FK01", 59, "ONE_LEVEL", "[ F1 ]"),
    ("FK02", 60, "ONE_LEVEL", "[ F2 ]"),
    ("FK03", 61, "ONE_LEVEL", "[ F3 ]"),
    ("FK04", 62, "ONE_LEVEL", "[ F4 ]"),
    ("FK05", 63, "ONE_LEVEL", "[ F5 ]"),
    ("FK06", 64, "ONE_LEVEL", "[ F6 ]"),
    ("FK07", 65, "ONE_LEVEL", "[ F7 ]"),
    ("FK08", 66, "ONE_LEVEL", "[ F8 ]"),
    ("FK09", 67, "ONE_LEVEL", "[ F9 ]"),
    ("FK10", 68, "ONE_LEVEL", "[ F10 ]"),
    ("NMLK", 69, "ONE_LEVEL", "[ Num_Lock ]"),
    ("KP7", 71, "ONE_LEVEL", "[ 7 ]"),
    ("KP8", 72, "ONE_LEVEL", "[ 8 ]"),
    ("KP9", 73, "ONE_LEVEL", "[ 9 ]"),
    ("KPSU", 74, "ONE_LEVEL", "[ minus ]"),
    ("KP4", 75, "ONE_LEVEL", "[ 4 ]"),
    ("KP5", 76, "ONE_LEVEL", "[ 5 ]"),
    ("KP6", 77, "ONE_LEVEL", "[ 6 ]"),
    ("KPAD", 78, "ONE_LEVEL", "[ plus ]"),
    ("KP1", 79, "ONE_LEVEL", "[ 1 ]"),
    ("KP2", 80, "ONE_LEVEL", "[ 2 ]"),
    ("KP3", 81, "ONE_LEVEL", "[ 3 ]"),
    ("KP0", 82, "ONE_LEVEL", "[ 0 ]"),
    ("KPDL", 83, "ONE_LEVEL", "[ period ]"),
    ("LSGT", 86, "TWO_LEVEL", "[ less, greater ]"),
    ("FK11", 87, "ONE_LEVEL", "[ F11 ]"),
    ("FK12", 88, "ONE_LEVEL", "[ F12 ]"),
    ("KPEN", 96, "TWO_LEVEL", "[ Return, Return ]"),
    ("RCTL", 97, "ONE_LEVEL", "[ Control_R ]"),
    ("KPDV", 98, "ONE_LEVEL", "[ slash ]"),
    ("RALT", 100, "ONE_LEVEL", "[ Alt_R ]"),
    ("HOME", 102, "ONE_LEVEL", "[ Home ]"),
    ("UP", 103, "ONE_LEVEL", "[ Up ]"),
    ("PGUP", 104, "ONE_LEVEL", "[ Prior ]"),
    ("LEFT", 105, "ONE_LEVEL", "[ Left ]"),
    ("RGHT", 106, "ONE_LEVEL", "[ Right ]"),
    ("END", 107, "ONE_LEVEL", "[ End ]"),
    ("DOWN", 108, "ONE_LEVEL", "[ Down ]"),
    ("PGDN", 109, "ONE_LEVEL", "[ Next ]"),
    ("INS", 110, "ONE_LEVEL", "[ Insert ]"),
    ("DELE", 111, "ONE_LEVEL", "[ Delete ]"),
    ("LWIN", 125, "ONE_LEVEL", "[ Super_L ]"),
    ("RWIN", 126, "ONE_LEVEL", "[ Super_R ]"),
];

fn ascii_to_xkb_keysym(ch: char) -> String {
    match ch {
        'a'..='z' | 'A'..='Z' | '0'..='9' => ch.to_string(),
        '!' => "exclam".into(),
        '@' => "at".into(),
        '#' => "numbersign".into(),
        '$' => "dollar".into(),
        '%' => "percent".into(),
        '^' => "asciicircum".into(),
        '&' => "ampersand".into(),
        '*' => "asterisk".into(),
        '(' => "parenleft".into(),
        ')' => "parenright".into(),
        '-' => "minus".into(),
        '_' => "underscore".into(),
        '=' => "equal".into(),
        '+' => "plus".into(),
        '[' => "bracketleft".into(),
        '{' => "braceleft".into(),
        ']' => "bracketright".into(),
        '}' => "braceright".into(),
        ';' => "semicolon".into(),
        ':' => "colon".into(),
        '\'' => "apostrophe".into(),
        '"' => "quotedbl".into(),
        '`' => "grave".into(),
        '~' => "asciitilde".into(),
        '\\' => "backslash".into(),
        '|' => "bar".into(),
        ',' => "comma".into(),
        '<' => "less".into(),
        '.' => "period".into(),
        '>' => "greater".into(),
        '/' => "slash".into(),
        '?' => "question".into(),
        _ => format!("U{:04X}", ch as u32),
    }
}

pub(crate) fn is_layout_alpha_evdev(layout: KeyboardLayout, evdev_code: u32) -> bool {
    key_pair_for_layout(layout, evdev_code as u16)
        .is_some_and(|(unshifted, _)| unshifted.is_ascii_alphabetic())
}

pub(crate) fn build_xkb_keymap_for_layout(layout: KeyboardLayout) -> String {
    let mut out = String::with_capacity(16_384);
    out.push_str("xkb_keymap {\n");
    out.push_str("xkb_keycodes \"dboard_vk\" {\n");
    out.push_str("    minimum = 8;\n");
    out.push_str("    maximum = 255;\n");
    for &(name, evdev, _, _) in STANDARD_KEYS {
        let _ = writeln!(out, "    <{name}> = {};", evdev + 8);
    }
    for &(name, evdev, _, _) in &EIGHT_LEVEL_KEYS {
        let _ = writeln!(out, "    <{name}> = {};", evdev + 8);
    }
    out.push_str("    indicator 1 = \"Caps Lock\";\n");
    out.push_str("    indicator 2 = \"Num Lock\";\n");
    out.push_str("    indicator 3 = \"Scroll Lock\";\n");
    out.push_str("};\n");

    out.push_str(
        r#"xkb_types "dboard_vk" {
    virtual_modifiers NumLock, Alt, Super, Meta;
    type "ONE_LEVEL" {
        modifiers= none;
        level_name[Level1]= "Any";
    };
    type "TWO_LEVEL" {
        modifiers= Shift;
        map[Shift]= Level2;
        level_name[Level1]= "Base";
        level_name[Level2]= "Shift";
    };
    type "ALPHABETIC" {
        modifiers= Shift+Lock;
        map[Shift]= Level2;
        map[Lock]= Level2;
        level_name[Level1]= "Base";
        level_name[Level2]= "Caps";
    };
    type "DBOARD_EIGHT_LEVEL" {
        modifiers = Shift + Lock + Mod5 + Mod3;
        map[none] = Level1;
        map[Shift] = Level2;
        map[Lock] = Level2;
        map[Mod5] = Level3;
        map[Shift+Mod5] = Level4;
        map[Lock+Mod5] = Level4;
        map[Mod3] = Level5;
        map[Shift+Mod3] = Level6;
        map[Lock+Mod3] = Level6;
        map[Mod5+Mod3] = Level7;
        map[Shift+Mod5+Mod3] = Level8;
        map[Lock+Mod5+Mod3] = Level8;
        level_name[Level1] = "1";
        level_name[Level2] = "2";
        level_name[Level3] = "3";
        level_name[Level4] = "4";
        level_name[Level5] = "5";
        level_name[Level6] = "6";
        level_name[Level7] = "7";
        level_name[Level8] = "8";
    };
    type "DBOARD_EIGHT_LEVEL_NONALPHA" {
        modifiers = Shift + Lock + Mod5 + Mod3;
        map[none] = Level1;
        map[Shift] = Level2;
        map[Mod5] = Level3;
        map[Shift+Mod5] = Level4;
        map[Lock+Mod5] = Level4;
        map[Mod3] = Level5;
        map[Shift+Mod3] = Level6;
        map[Lock+Mod3] = Level6;
        map[Mod5+Mod3] = Level7;
        map[Shift+Mod5+Mod3] = Level8;
        map[Lock+Mod5+Mod3] = Level8;
        level_name[Level1] = "1";
        level_name[Level2] = "2";
        level_name[Level3] = "3";
        level_name[Level4] = "4";
        level_name[Level5] = "5";
        level_name[Level6] = "6";
        level_name[Level7] = "7";
        level_name[Level8] = "8";
    };
};
xkb_compatibility "dboard_vk" {
    interpret VoidSymbol { repeat= False; };
    interpret.repeat= True;
    interpret Shift_L {
        action= SetMods(modifiers=Shift);
    };
    interpret Shift_R {
        action= SetMods(modifiers=Shift);
    };
    interpret Shift_Lock+AnyOf(Shift+Lock) {
        action= LockMods(modifiers=Lock);
    };
    interpret Caps_Lock+AnyOf(all) {
        action= LockMods(modifiers=Lock);
    };
    interpret Alt_L+AnyOf(all) {
        virtualModifier= Alt;
        action= SetMods(modifiers=Alt);
    };
    interpret Alt_R+AnyOf(all) {
        virtualModifier= Alt;
        action= SetMods(modifiers=Alt);
    };
    interpret Meta_L+AnyOf(all) {
        virtualModifier= Meta;
        action= SetMods(modifiers=Meta);
    };
    interpret Meta_R+AnyOf(all) {
        virtualModifier= Meta;
        action= SetMods(modifiers=Meta);
    };
    interpret Super_L+AnyOf(all) {
        virtualModifier= Super;
        action= SetMods(modifiers=Super);
    };
    interpret Super_R+AnyOf(all) {
        virtualModifier= Super;
        action= SetMods(modifiers=Super);
    };
    interpret Num_Lock+AnyOf(all) {
        virtualModifier= NumLock;
        action= LockMods(modifiers=NumLock);
    };
    interpret ISO_Level3_Shift+AnyOf(all) {
        action= SetMods(modifiers=Mod5);
    };
    interpret ISO_Level5_Shift+AnyOf(all) {
        action= SetMods(modifiers=Mod3);
    };
    indicator "Caps Lock" {
        whichModState= locked;
        modifiers= Lock;
    };
    indicator "Num Lock" {
        whichModState= locked;
        modifiers= NumLock;
    };
};
xkb_symbols "dboard_vk" {
"#,
    );
    let group_name = match layout {
        KeyboardLayout::UsQwerty => "English (US)",
        KeyboardLayout::Dvorak => "English (US Dvorak)",
        KeyboardLayout::Colemak => "English (US Colemak)",
    };
    let _ = writeln!(out, "    name[Group1] = \"{group_name}\";");

    for &(name, evdev, ty, syms) in STANDARD_KEYS {
        if let Some((base, shifted)) = key_pair_for_layout(layout, evdev as u16) {
            let eff_ty = if base.is_ascii_alphabetic() {
                "ALPHABETIC"
            } else {
                "TWO_LEVEL"
            };
            let s1 = ascii_to_xkb_keysym(base);
            let s2 = ascii_to_xkb_keysym(shifted);
            let _ = writeln!(
                out,
                "    key <{name}> {{ type[Group1] = \"{eff_ty}\", symbols[Group1] = [ {s1}, {s2} ] }};"
            );
        } else {
            let _ = writeln!(
                out,
                "    key <{name}> {{ type[Group1] = \"{ty}\", symbols[Group1] = {syms} }};"
            );
        }
    }
    for (i, &(name, evdev, default_lower, default_upper)) in EIGHT_LEVEL_KEYS.iter().enumerate() {
        let (ty, l1, l2) = if let Some((base, shifted)) = key_pair_for_layout(layout, evdev as u16)
        {
            let ty = if base.is_ascii_alphabetic() {
                "DBOARD_EIGHT_LEVEL"
            } else {
                "DBOARD_EIGHT_LEVEL_NONALPHA"
            };
            (ty, ascii_to_xkb_keysym(base), ascii_to_xkb_keysym(shifted))
        } else {
            (
                "DBOARD_EIGHT_LEVEL",
                default_lower.to_string(),
                default_upper.to_string(),
            )
        };
        let mut syms = format!("[ {l1}, {l2}");
        for slot in 0..3 {
            let pair_idx = i * 3 + slot;
            if pair_idx < VIETNAMESE_PAIRS.len() {
                let (lo, up) = VIETNAMESE_PAIRS[pair_idx];
                let _ = write!(
                    out_sym(&mut syms),
                    ", U{:04X}, U{:04X}",
                    lo as u32,
                    up as u32
                );
            } else {
                syms.push_str(", VoidSymbol, VoidSymbol");
            }
        }
        syms.push_str(" ]");
        let _ = writeln!(
            out,
            "    key <{name}> {{ type[Group1] = \"{ty}\", symbols[Group1] = {syms} }};"
        );
    }

    out.push_str(
        r#"    modifier_map Shift { <LFSH>, <RTSH> };
    modifier_map Lock { <CAPS> };
    modifier_map Control { <LCTL>, <RCTL> };
    modifier_map Mod1 { <LALT>, <RALT> };
    modifier_map Mod2 { <NMLK> };
    modifier_map Mod4 { <LWIN>, <RWIN> };
};
};
"#,
    );

    out
}

#[inline]
fn out_sym(s: &mut String) -> &mut String {
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_vietnamese_and_ascii_keycodes_are_unique_and_within_8bit_xkb_limit() {
        let mut seen = HashSet::new();
        for &(name, evdev, _, _) in STANDARD_KEYS {
            let xkb = evdev + 8;
            assert!((8..=255).contains(&xkb), "{name} out of range: {xkb}");
            assert!(
                seen.insert(evdev),
                "duplicate evdev keycode {evdev} ({name})"
            );
        }
        for &(name, evdev, _, _) in &EIGHT_LEVEL_KEYS {
            let xkb = evdev + 8;
            assert!((8..=255).contains(&xkb), "{name} out of range: {xkb}");
            assert!(
                seen.insert(evdev),
                "duplicate evdev keycode {evdev} ({name})"
            );
        }
        for &(lo, up) in &VIETNAMESE_PAIRS {
            let stroke_lo = char_to_vk_stroke(lo).expect("Vietnamese lower char mapped");
            let stroke_up = char_to_vk_stroke(up).expect("Vietnamese upper char mapped");
            assert!(
                stroke_lo.evdev_code <= 47,
                "Vietnamese char {lo} mapped to high evdev code {}",
                stroke_lo.evdev_code
            );
            assert!(
                stroke_up.evdev_code <= 47,
                "Vietnamese char {up} mapped to high evdev code {}",
                stroke_up.evdev_code
            );
        }
    }

    #[test]
    fn libxkbcommon_accepts_keymap_and_decodes_all_vietnamese_and_ascii_chars() {
        use std::ffi::{c_char, c_int, c_void, CStr, CString};

        type XkbContext = *mut c_void;
        type XkbKeymap = *mut c_void;
        type XkbState = *mut c_void;
        type CtxNew = unsafe extern "C" fn(flags: c_int) -> XkbContext;
        type CtxUnref = unsafe extern "C" fn(ctx: XkbContext);
        type KmNew = unsafe extern "C" fn(
            ctx: XkbContext,
            string: *const c_char,
            format: c_int,
            flags: c_int,
        ) -> XkbKeymap;
        type KmUnref = unsafe extern "C" fn(km: XkbKeymap);
        type ModIdx = unsafe extern "C" fn(km: XkbKeymap, name: *const c_char) -> u32;
        type StNew = unsafe extern "C" fn(km: XkbKeymap) -> XkbState;
        type StUnref = unsafe extern "C" fn(st: XkbState);
        type StMask = unsafe extern "C" fn(
            st: XkbState,
            depressed_mods: u32,
            latched_mods: u32,
            locked_mods: u32,
            depressed_layout: u32,
            latched_layout: u32,
            locked_layout: u32,
        ) -> u32;
        type StGetUtf8 =
            unsafe extern "C" fn(st: XkbState, key: u32, buffer: *mut c_char, size: usize) -> c_int;
        type StGetSym = unsafe extern "C" fn(st: XkbState, key: u32) -> u32;
        type StLedActive = unsafe extern "C" fn(st: XkbState, name: *const c_char) -> c_int;

        unsafe {
            let lib = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_LAZY);
            if lib.is_null() {
                eprintln!("libxkbcommon.so.0 not found; skipping dynamic decode verification");
                return;
            }

            let ctx_new: CtxNew =
                std::mem::transmute(libc::dlsym(lib, c"xkb_context_new".as_ptr()));
            let ctx_unref: CtxUnref =
                std::mem::transmute(libc::dlsym(lib, c"xkb_context_unref".as_ptr()));
            let km_new: KmNew =
                std::mem::transmute(libc::dlsym(lib, c"xkb_keymap_new_from_string".as_ptr()));
            let km_unref: KmUnref =
                std::mem::transmute(libc::dlsym(lib, c"xkb_keymap_unref".as_ptr()));
            let mod_idx: ModIdx =
                std::mem::transmute(libc::dlsym(lib, c"xkb_keymap_mod_get_index".as_ptr()));
            let st_new: StNew = std::mem::transmute(libc::dlsym(lib, c"xkb_state_new".as_ptr()));
            let st_unref: StUnref =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_unref".as_ptr()));
            let st_mask: StMask =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_update_mask".as_ptr()));
            let st_utf8: StGetUtf8 =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_key_get_utf8".as_ptr()));
            let _st_sym: StGetSym =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_key_get_one_sym".as_ptr()));
            let st_led_active: StLedActive =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_led_name_is_active".as_ptr()));

            let ctx = ctx_new(0);
            assert!(!ctx.is_null());

            for layout in [
                KeyboardLayout::UsQwerty,
                KeyboardLayout::Dvorak,
                KeyboardLayout::Colemak,
            ] {
                let keymap_src = CString::new(build_xkb_keymap_for_layout(layout)).unwrap();
                let km = km_new(ctx, keymap_src.as_ptr(), 1, 0);
                assert!(!km.is_null(), "libxkbcommon rejected {layout:?} keymap");

                for (expected_idx, name) in [
                    (0u32, c"Shift"),
                    (1, c"Lock"),
                    (2, c"Control"),
                    (3, c"Mod1"),
                    (4, c"Mod2"),
                    (5, c"Mod3"),
                    (6, c"Mod4"),
                    (7, c"Mod5"),
                ] {
                    assert_eq!(mod_idx(km, name.as_ptr()), expected_idx);
                }
                for vmod in [c"Alt", c"Super", c"Meta", c"NumLock"] {
                    assert_ne!(
                        mod_idx(km, vmod.as_ptr()),
                        u32::MAX,
                        "virtual modifier {vmod:?} must be defined in {layout:?} keymap"
                    );
                }

                let state = st_new(km);
                assert!(!state.is_null());

                // Verify Caps Lock and Num Lock XKB LED indicators respond to locked modifiers
                // so compositors (COSMIC/Smithay) never turn off the hardware NumLock LED.
                st_mask(state, 0, 0, XKB_MOD_MOD2, 0, 0, 0);
                assert_eq!(st_led_active(state, c"Num Lock".as_ptr()), 1);
                assert_eq!(st_led_active(state, c"Caps Lock".as_ptr()), 0);
                st_mask(state, 0, 0, XKB_MOD_LOCK | XKB_MOD_MOD2, 0, 0, 0);
                assert_eq!(st_led_active(state, c"Caps Lock".as_ptr()), 1);
                assert_eq!(st_led_active(state, c"Num Lock".as_ptr()), 1);
                st_mask(state, 0, 0, 0, 0, 0, 0);
                assert_eq!(st_led_active(state, c"Num Lock".as_ptr()), 0);
                assert_eq!(st_led_active(state, c"Caps Lock".as_ptr()), 0);

                for &(lo, up) in &VIETNAMESE_PAIRS {
                    for &ch in &[lo, up] {
                        let stroke = char_to_vk_stroke_for_layout(layout, ch).unwrap();
                        st_mask(state, stroke.modifiers, 0, 0, 0, 0, 0);
                        let mut buf = [0 as c_char; 16];
                        let len =
                            st_utf8(state, stroke.evdev_code + 8, buf.as_mut_ptr(), buf.len());
                        assert!(len > 0, "failed len for {ch} on {layout:?}");
                        let decoded = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
                        assert_eq!(decoded, ch.to_string(), "mismatch for {ch} on {layout:?}");
                    }
                }

                for byte in 0x20u8..=0x7eu8 {
                    let ch = byte as char;
                    let stroke = char_to_vk_stroke_for_layout(layout, ch).unwrap();
                    st_mask(state, stroke.modifiers, 0, 0, 0, 0, 0);
                    let mut buf = [0 as c_char; 16];
                    let len = st_utf8(state, stroke.evdev_code + 8, buf.as_mut_ptr(), buf.len());
                    assert!(len > 0);
                    let decoded = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
                    assert_eq!(
                        decoded,
                        ch.to_string(),
                        "ASCII mismatch for {ch:?} on {layout:?}"
                    );
                }

                if layout == KeyboardLayout::Dvorak {
                    for (evdev_code, expected) in [(16u32, "'"), (17, ","), (18, "."), (44, ";")] {
                        st_mask(state, 0, 0, XKB_MOD_LOCK, 0, 0, 0);
                        let mut buf = [0 as c_char; 16];
                        let len = st_utf8(state, evdev_code + 8, buf.as_mut_ptr(), buf.len());
                        assert!(len > 0);
                        let decoded = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
                        assert_eq!(
                            decoded, expected,
                            "Dvorak punctuation key {evdev_code} must not shift under CapsLock"
                        );
                    }
                }

                st_unref(state);
                km_unref(km);
            }
            ctx_unref(ctx);
            libc::dlclose(lib);
        }
    }

    #[test]
    fn numpad_delimiters_route_via_vk_while_numlock_digits_stay_on_uinput() {
        // Invariant 3: NumLock-independent Numpad delimiters share the VK channel
        for code in [55u16, 74, 78, 96, 98] {
            assert!(
                is_vk_typing_or_nav_keycode(code),
                "expected Numpad delimiter keycode {code} to route via VK"
            );
        }
        // NumLock-dependent keypad digits/dot stay native on uinput
        for code in [71u16, 72, 73, 75, 76, 77, 79, 80, 81, 82, 83] {
            assert!(
                !is_vk_typing_or_nav_keycode(code),
                "expected NumLock-dependent keycode {code} to stay on uinput"
            );
        }
    }
}
