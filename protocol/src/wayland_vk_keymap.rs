// SPDX-License-Identifier: MIT
//! Complete standards-compliant XKB v1 keymap for VietC's persistent Wayland
//! virtual keyboard (`zwp_virtual_keyboard_v1`).
//!
//! Design invariants:
//! 1. Uploaded ONCE per connection (zero dynamic keymap churn while typing).
//! 2. Defines all 8 real XKB modifiers (`Shift`, `Lock`, `Control`, `Mod1`..`Mod5`)
//!    and standard virtual modifiers (`NumLock`, `Alt`, `Super`, `Meta`) so
//!    Chromium/Edge/Electron, GTK4, and Qt6 never encounter `XKB_MOD_INVALID`.
//! 3. Keeps all XKB keycodes strictly within `8..=255` (`evdev + 8`), ensuring
//!    8-bit XWayland compatibility alongside native Wayland clients.
//! 4. Maps all 134 precomposed Vietnamese Unicode characters (`à..ỹ`, `À..Ỹ`,
//!    `đ`, `Đ`) onto dedicated `ONE_LEVEL` evdev keycodes (`112..=124` and
//!    `127..=247`), making Vietnamese output immune to physical Shift/CapsLock races.

use std::fmt::Write;

pub(crate) const XKB_MOD_SHIFT: u32 = 1 << 0;
pub(crate) const XKB_MOD_LOCK: u32 = 1 << 1;

pub(crate) const VIETNAMESE_CHARS: [char; 134] = [
    // Lowercase (67)
    'à', 'á', 'ả', 'ã', 'ạ', 'ă', 'ằ', 'ắ', 'ẳ', 'ẵ', 'ặ', 'â', 'ầ', 'ấ', 'ẩ', 'ẫ', 'ậ', 'è', 'é',
    'ẻ', 'ẽ', 'ẹ', 'ê', 'ề', 'ế', 'ể', 'ễ', 'ệ', 'ì', 'í', 'ỉ', 'ĩ', 'ị', 'ò', 'ó', 'ỏ', 'õ', 'ọ',
    'ô', 'ồ', 'ố', 'ổ', 'ỗ', 'ộ', 'ơ', 'ờ', 'ớ', 'ở', 'ỡ', 'ợ', 'ù', 'ú', 'ủ', 'ũ', 'ụ', 'ư', 'ừ',
    'ứ', 'ử', 'ữ', 'ự', 'ỳ', 'ý', 'ỷ', 'ỹ', 'ỵ', 'đ', // Uppercase (67)
    'À', 'Á', 'Ả', 'Ã', 'Ạ', 'Ă', 'Ằ', 'Ắ', 'Ẳ', 'Ẵ', 'Ặ', 'Â', 'Ầ', 'Ấ', 'Ẩ', 'Ẫ', 'Ậ', 'È', 'É',
    'Ẻ', 'Ẽ', 'Ẹ', 'Ê', 'Ề', 'Ế', 'Ể', 'Ễ', 'Ệ', 'Ì', 'Í', 'Ỉ', 'Ĩ', 'Ị', 'Ò', 'Ó', 'Ỏ', 'Õ', 'Ọ',
    'Ô', 'Ồ', 'Ố', 'Ổ', 'Ỗ', 'Ộ', 'Ơ', 'Ờ', 'Ớ', 'Ở', 'Ỡ', 'Ợ', 'Ù', 'Ú', 'Ủ', 'Ũ', 'Ụ', 'Ư', 'Ừ',
    'Ứ', 'Ử', 'Ữ', 'Ự', 'Ỳ', 'Ý', 'Ỷ', 'Ỹ', 'Ỵ', 'Đ',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VkKeyStroke {
    pub evdev_code: u32,
    pub shift: bool,
    pub one_level: bool,
}

#[inline]
pub(crate) fn vn_index_to_evdev(idx: usize) -> u32 {
    debug_assert!(idx < VIETNAMESE_CHARS.len());
    // Skip 125 (KEY_LEFTMETA) and 126 (KEY_RIGHTMETA) so <LWIN>/<RWIN> keep
    // their standard evdev slots and maximum XKB keycode is 247 + 8 = 255.
    if idx < 13 {
        112 + idx as u32
    } else {
        127 + (idx - 13) as u32
    }
}

pub(crate) fn char_to_vk_stroke(ch: char) -> Option<VkKeyStroke> {
    if let Some(idx) = VIETNAMESE_CHARS.iter().position(|&c| c == ch) {
        return Some(VkKeyStroke {
            evdev_code: vn_index_to_evdev(idx),
            shift: false,
            one_level: true,
        });
    }

    let (evdev_code, shift, one_level) = match ch {
        'a'..='z' => (ascii_letter_evdev(ch), false, false),
        'A'..='Z' => (ascii_letter_evdev(ch.to_ascii_lowercase()), true, false),
        '1' => (2, false, false),
        '!' => (2, true, false),
        '2' => (3, false, false),
        '@' => (3, true, false),
        '3' => (4, false, false),
        '#' => (4, true, false),
        '4' => (5, false, false),
        '$' => (5, true, false),
        '5' => (6, false, false),
        '%' => (6, true, false),
        '6' => (7, false, false),
        '^' => (7, true, false),
        '7' => (8, false, false),
        '&' => (8, true, false),
        '8' => (9, false, false),
        '*' => (9, true, false),
        '9' => (10, false, false),
        '(' => (10, true, false),
        '0' => (11, false, false),
        ')' => (11, true, false),
        '-' => (12, false, false),
        '_' => (12, true, false),
        '=' => (13, false, false),
        '+' => (13, true, false),
        '\t' => (15, false, true),
        '[' => (26, false, false),
        '{' => (26, true, false),
        ']' => (27, false, false),
        '}' => (27, true, false),
        '\n' => (28, false, true),
        ';' => (39, false, false),
        ':' => (39, true, false),
        '\'' => (40, false, false),
        '"' => (40, true, false),
        '`' => (41, false, false),
        '~' => (41, true, false),
        '\\' => (43, false, false),
        '|' => (43, true, false),
        ',' => (51, false, false),
        '<' => (51, true, false),
        '.' => (52, false, false),
        '>' => (52, true, false),
        '/' => (53, false, false),
        '?' => (53, true, false),
        ' ' => (57, false, true),
        _ => return None,
    };

    Some(VkKeyStroke {
        evdev_code,
        shift,
        one_level,
    })
}

fn ascii_letter_evdev(lower: char) -> u32 {
    match lower {
        'q' => 16,
        'w' => 17,
        'e' => 18,
        'r' => 19,
        't' => 20,
        'y' => 21,
        'u' => 22,
        'i' => 23,
        'o' => 24,
        'p' => 25,
        'a' => 30,
        's' => 31,
        'd' => 32,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'z' => 44,
        'x' => 45,
        'c' => 46,
        'v' => 47,
        'b' => 48,
        'n' => 49,
        'm' => 50,
        _ => unreachable!(),
    }
}

/// Returns `true` if `keycode` is a standard typing, editing, or navigation key
/// defined in our virtual-keyboard XKB keymap (excluding system modifiers, CapsLock,
/// and physical Numpad keys so compositor shortcuts, hardware LEDs, and NumLock OFF
/// keypad navigation stay 100% native on uinput).
pub(crate) fn is_vk_typing_or_nav_keycode(keycode: u16) -> bool {
    matches!(
        keycode,
        1..=28 | 30..=43 | 44..=53 | 57 | 59..=68 | 87..=88 | 102..=111
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
    ("AD01", 16, "ALPHABETIC", "[ q, Q ]"),
    ("AD02", 17, "ALPHABETIC", "[ w, W ]"),
    ("AD03", 18, "ALPHABETIC", "[ e, E ]"),
    ("AD04", 19, "ALPHABETIC", "[ r, R ]"),
    ("AD05", 20, "ALPHABETIC", "[ t, T ]"),
    ("AD06", 21, "ALPHABETIC", "[ y, Y ]"),
    ("AD07", 22, "ALPHABETIC", "[ u, U ]"),
    ("AD08", 23, "ALPHABETIC", "[ i, I ]"),
    ("AD09", 24, "ALPHABETIC", "[ o, O ]"),
    ("AD10", 25, "ALPHABETIC", "[ p, P ]"),
    ("AD11", 26, "TWO_LEVEL", "[ bracketleft, braceleft ]"),
    ("AD12", 27, "TWO_LEVEL", "[ bracketright, braceright ]"),
    ("RTRN", 28, "ONE_LEVEL", "[ Return ]"),
    ("LCTL", 29, "ONE_LEVEL", "[ Control_L ]"),
    ("AC01", 30, "ALPHABETIC", "[ a, A ]"),
    ("AC02", 31, "ALPHABETIC", "[ s, S ]"),
    ("AC03", 32, "ALPHABETIC", "[ d, D ]"),
    ("AC04", 33, "ALPHABETIC", "[ f, F ]"),
    ("AC05", 34, "ALPHABETIC", "[ g, G ]"),
    ("AC06", 35, "ALPHABETIC", "[ h, H ]"),
    ("AC07", 36, "ALPHABETIC", "[ j, J ]"),
    ("AC08", 37, "ALPHABETIC", "[ k, K ]"),
    ("AC09", 38, "ALPHABETIC", "[ l, L ]"),
    ("AC10", 39, "TWO_LEVEL", "[ semicolon, colon ]"),
    ("AC11", 40, "TWO_LEVEL", "[ apostrophe, quotedbl ]"),
    ("TLDE", 41, "TWO_LEVEL", "[ grave, asciitilde ]"),
    ("LFSH", 42, "ONE_LEVEL", "[ Shift_L ]"),
    ("BKSL", 43, "TWO_LEVEL", "[ backslash, bar ]"),
    ("AB01", 44, "ALPHABETIC", "[ z, Z ]"),
    ("AB02", 45, "ALPHABETIC", "[ x, X ]"),
    ("AB03", 46, "ALPHABETIC", "[ c, C ]"),
    ("AB04", 47, "ALPHABETIC", "[ v, V ]"),
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
    ("FK11", 87, "ONE_LEVEL", "[ F11 ]"),
    ("FK12", 88, "ONE_LEVEL", "[ F12 ]"),
    ("KPEN", 96, "ONE_LEVEL", "[ Return ]"),
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

pub(crate) fn build_xkb_keymap() -> String {
    let mut out = String::with_capacity(14_336);
    out.push_str("xkb_keymap {\n");
    out.push_str("xkb_keycodes \"vietc_vk\" {\n");
    out.push_str("    minimum = 8;\n");
    out.push_str("    maximum = 255;\n");
    for &(name, evdev, _, _) in STANDARD_KEYS {
        let _ = writeln!(out, "    <{name}> = {};", evdev + 8);
    }
    for idx in 0..VIETNAMESE_CHARS.len() {
        let xkb_code = vn_index_to_evdev(idx) + 8;
        let _ = writeln!(out, "    <V{idx:02X}> = {xkb_code};");
    }
    out.push_str("};\n");

    out.push_str(
        r#"xkb_types "vietc_vk" {
    virtual_modifiers NumLock,Alt,LevelThree,Super,Meta,Hyper,ScrollLock;
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
    type "VIETC_EXACT" {
        modifiers= Shift+Lock;
        level_name[Level1]= "Exact";
    };
};
xkb_compatibility "vietc_vk" {
    virtual_modifiers NumLock,Alt,LevelThree,Super,Meta,Hyper,ScrollLock;
    interpret.useModMapMods= AnyLevel;
    interpret.repeat= False;
    interpret Shift_L+AnyOf(all) { action= SetMods(modifiers=Shift,clearLocks); };
    interpret Shift_R+AnyOf(all) { action= SetMods(modifiers=Shift,clearLocks); };
    interpret Caps_Lock+AnyOf(all) { action= LockMods(modifiers=Lock); };
    interpret Control_L+AnyOf(all) { action= SetMods(modifiers=Control,clearLocks); };
    interpret Control_R+AnyOf(all) { action= SetMods(modifiers=Control,clearLocks); };
    interpret Alt_L+AnyOf(all) { virtualModifier= Alt; action= SetMods(modifiers=modMapMods,clearLocks); };
    interpret Alt_R+AnyOf(all) { virtualModifier= Alt; action= SetMods(modifiers=modMapMods,clearLocks); };
    interpret Num_Lock+AnyOf(all) { virtualModifier= NumLock; action= LockMods(modifiers=NumLock); };
    interpret Super_L+AnyOf(all) { virtualModifier= Super; action= SetMods(modifiers=modMapMods,clearLocks); };
    interpret Super_R+AnyOf(all) { virtualModifier= Super; action= SetMods(modifiers=modMapMods,clearLocks); };
    interpret Any+AnyOf(all) { action= NoAction(); };
};
xkb_symbols "vietc_vk" {
    name[Group1]= "English (US, VietC Direct Unicode)";
"#,
    );

    for &(name, _, ty, syms) in STANDARD_KEYS {
        let _ = writeln!(
            out,
            "    key <{name}> {{ type[Group1] = \"{ty}\", symbols[Group1] = {syms} }};"
        );
    }
    for (idx, &ch) in VIETNAMESE_CHARS.iter().enumerate() {
        let _ = writeln!(
            out,
            "    key <V{idx:02X}> {{ type[Group1] = \"VIETC_EXACT\", symbols[Group1] = [ U{:04X} ] }};",
            ch as u32
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};

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
        for (idx, &ch) in VIETNAMESE_CHARS.iter().enumerate() {
            let evdev = vn_index_to_evdev(idx);
            let xkb = evdev + 8;
            assert!(
                (8..=255).contains(&xkb),
                "{ch} out of 8-bit XKB range: {xkb}"
            );
            assert!(
                seen.insert(evdev),
                "duplicate evdev keycode {evdev} for {ch}"
            );
            let stroke = char_to_vk_stroke(ch).expect("Vietnamese char mapped");
            assert_eq!(stroke.evdev_code, evdev);
            assert!(stroke.one_level);
            assert!(!stroke.shift);
        }
    }

    #[test]
    fn libxkbcommon_accepts_keymap_and_decodes_all_vietnamese_and_ascii_chars() {
        unsafe {
            let lib = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_NOW);
            if lib.is_null() {
                return;
            }
            type CtxNew = unsafe extern "C" fn(c_int) -> *mut c_void;
            type CtxUnref = unsafe extern "C" fn(*mut c_void);
            type KmNew =
                unsafe extern "C" fn(*mut c_void, *const c_char, c_int, c_int) -> *mut c_void;
            type KmUnref = unsafe extern "C" fn(*mut c_void);
            type ModIdx = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_uint;
            type StNew = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
            type StUnref = unsafe extern "C" fn(*mut c_void);
            type StUpdateMask = unsafe extern "C" fn(
                *mut c_void,
                c_uint,
                c_uint,
                c_uint,
                c_uint,
                c_uint,
                c_uint,
            ) -> c_int;
            type StGetUtf8 = unsafe extern "C" fn(*mut c_void, c_uint, *mut c_char, usize) -> c_int;
            type StGetSym = unsafe extern "C" fn(*mut c_void, c_uint) -> u32;

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
            let st_mask: StUpdateMask =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_update_mask".as_ptr()));
            let st_utf8: StGetUtf8 =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_key_get_utf8".as_ptr()));
            let st_sym: StGetSym =
                std::mem::transmute(libc::dlsym(lib, c"xkb_state_key_get_one_sym".as_ptr()));

            let ctx = ctx_new(0);
            assert!(!ctx.is_null());
            let keymap_src = CString::new(build_xkb_keymap()).unwrap();
            let km = km_new(ctx, keymap_src.as_ptr(), 1, 0);
            assert!(!km.is_null(), "libxkbcommon rejected build_xkb_keymap()");

            // All real XKB modifiers required by Chromium/Ozone/GTK/Qt must exist!
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

            let state = st_new(km);
            assert!(!state.is_null());

            // Verify BackSpace (14 -> 22 = 0xff08) and Delete (111 -> 119 = 0xffff)
            assert_eq!(st_sym(state, 14 + 8), 0xff08);
            assert_eq!(st_sym(state, 111 + 8), 0xffff);

            // Verify every Vietnamese char decodes identically even when CapsLock OR Shift is active!
            for &ch in &VIETNAMESE_CHARS {
                let stroke = char_to_vk_stroke(ch).unwrap();
                for (depressed, locked) in [(0, 0), (XKB_MOD_SHIFT, 0), (0, XKB_MOD_LOCK)] {
                    st_mask(state, depressed, 0, locked, 0, 0, 0);
                    let mut buf = [0 as c_char; 16];
                    let len = st_utf8(state, stroke.evdev_code + 8, buf.as_mut_ptr(), buf.len());
                    assert!(len > 0);
                    let decoded = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
                    assert_eq!(decoded, ch.to_string(), "mismatch for {ch}");
                }
            }

            // Verify all 95 printable ASCII characters
            for byte in 0x20u8..=0x7eu8 {
                let ch = byte as char;
                let stroke = char_to_vk_stroke(ch).unwrap();
                let depressed = if stroke.shift { XKB_MOD_SHIFT } else { 0 };
                st_mask(state, depressed, 0, 0, 0, 0, 0);
                let mut buf = [0 as c_char; 16];
                let len = st_utf8(state, stroke.evdev_code + 8, buf.as_mut_ptr(), buf.len());
                assert!(len > 0);
                let decoded = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
                assert_eq!(decoded, ch.to_string(), "ASCII mismatch for {ch:?}");
            }

            st_unref(state);
            km_unref(km);
            ctx_unref(ctx);
            libc::dlclose(lib);
        }
    }
}
