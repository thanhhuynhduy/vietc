// SPDX-License-Identifier: MIT
//! Crisp 8x16 Unicode Vietnamese bitmap font renderer for `dboard-popup`.
//!
//! Renders full lowercase/uppercase ASCII and all 134 Vietnamese accented
//! characters (circumflex, breve, horn, `đ`/`Đ`, and all 5 tones including
//! stacked diacritics such as `ế`, `ề`, `ể`, `ễ`, `ệ`, `ắ`, `ừ`, `ợ`) with
//! zero external dependencies and zero `unsafe` code.

pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VowelShape {
    Plain,
    Circumflex,
    Breve,
    Horn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    None,
    Acute,
    Grave,
    Hook,
    Tilde,
    DotBelow,
}

#[derive(Debug, Clone, Copy)]
struct DecomposedChar {
    base: char,
    shape: VowelShape,
    tone: Tone,
    barred_d: bool,
}

fn decompose(ch: char) -> DecomposedChar {
    use Tone::*;
    use VowelShape::*;
    let (base, shape, tone, barred_d) = match ch {
        // Lowercase a / ă / â
        'á' => ('a', Plain, Acute, false),
        'à' => ('a', Plain, Grave, false),
        'ả' => ('a', Plain, Hook, false),
        'ã' => ('a', Plain, Tilde, false),
        'ạ' => ('a', Plain, DotBelow, false),
        'ă' => ('a', Breve, None, false),
        'ắ' => ('a', Breve, Acute, false),
        'ằ' => ('a', Breve, Grave, false),
        'ẳ' => ('a', Breve, Hook, false),
        'ẵ' => ('a', Breve, Tilde, false),
        'ặ' => ('a', Breve, DotBelow, false),
        'â' => ('a', Circumflex, None, false),
        'ấ' => ('a', Circumflex, Acute, false),
        'ầ' => ('a', Circumflex, Grave, false),
        'ẩ' => ('a', Circumflex, Hook, false),
        'ẫ' => ('a', Circumflex, Tilde, false),
        'ậ' => ('a', Circumflex, DotBelow, false),
        // Uppercase A / Ă / Â
        'Á' => ('A', Plain, Acute, false),
        'À' => ('A', Plain, Grave, false),
        'Ả' => ('A', Plain, Hook, false),
        'Ã' => ('A', Plain, Tilde, false),
        'Ạ' => ('A', Plain, DotBelow, false),
        'Ă' => ('A', Breve, None, false),
        'Ắ' => ('A', Breve, Acute, false),
        'Ằ' => ('A', Breve, Grave, false),
        'Ẳ' => ('A', Breve, Hook, false),
        'Ẵ' => ('A', Breve, Tilde, false),
        'Ặ' => ('A', Breve, DotBelow, false),
        'Â' => ('A', Circumflex, None, false),
        'Ấ' => ('A', Circumflex, Acute, false),
        'Ầ' => ('A', Circumflex, Grave, false),
        'Ẩ' => ('A', Circumflex, Hook, false),
        'Ẫ' => ('A', Circumflex, Tilde, false),
        'Ậ' => ('A', Circumflex, DotBelow, false),
        // Lowercase e / ê
        'é' => ('e', Plain, Acute, false),
        'è' => ('e', Plain, Grave, false),
        'ẻ' => ('e', Plain, Hook, false),
        'ẽ' => ('e', Plain, Tilde, false),
        'ẹ' => ('e', Plain, DotBelow, false),
        'ê' => ('e', Circumflex, None, false),
        'ế' => ('e', Circumflex, Acute, false),
        'ề' => ('e', Circumflex, Grave, false),
        'ể' => ('e', Circumflex, Hook, false),
        'ễ' => ('e', Circumflex, Tilde, false),
        'ệ' => ('e', Circumflex, DotBelow, false),
        // Uppercase E / Ê
        'É' => ('E', Plain, Acute, false),
        'È' => ('E', Plain, Grave, false),
        'Ẻ' => ('E', Plain, Hook, false),
        'Ẽ' => ('E', Plain, Tilde, false),
        'Ẹ' => ('E', Plain, DotBelow, false),
        'Ê' => ('E', Circumflex, None, false),
        'Ế' => ('E', Circumflex, Acute, false),
        'Ề' => ('E', Circumflex, Grave, false),
        'Ể' => ('E', Circumflex, Hook, false),
        'Ễ' => ('E', Circumflex, Tilde, false),
        'Ệ' => ('E', Circumflex, DotBelow, false),
        // Lowercase / Uppercase i / I
        'í' => ('i', Plain, Acute, false),
        'ì' => ('i', Plain, Grave, false),
        'ỉ' => ('i', Plain, Hook, false),
        'ĩ' => ('i', Plain, Tilde, false),
        'ị' => ('i', Plain, DotBelow, false),
        'Í' => ('I', Plain, Acute, false),
        'Ì' => ('I', Plain, Grave, false),
        'Ỉ' => ('I', Plain, Hook, false),
        'Ĩ' => ('I', Plain, Tilde, false),
        'Ị' => ('I', Plain, DotBelow, false),
        // Lowercase o / ô / ơ
        'ó' => ('o', Plain, Acute, false),
        'ò' => ('o', Plain, Grave, false),
        'ỏ' => ('o', Plain, Hook, false),
        'õ' => ('o', Plain, Tilde, false),
        'ọ' => ('o', Plain, DotBelow, false),
        'ô' => ('o', Circumflex, None, false),
        'ố' => ('o', Circumflex, Acute, false),
        'ồ' => ('o', Circumflex, Grave, false),
        'ổ' => ('o', Circumflex, Hook, false),
        'ỗ' => ('o', Circumflex, Tilde, false),
        'ộ' => ('o', Circumflex, DotBelow, false),
        'ơ' => ('o', Horn, None, false),
        'ớ' => ('o', Horn, Acute, false),
        'ờ' => ('o', Horn, Grave, false),
        'ở' => ('o', Horn, Hook, false),
        'ỡ' => ('o', Horn, Tilde, false),
        'ợ' => ('o', Horn, DotBelow, false),
        // Uppercase O / Ô / Ơ
        'Ó' => ('O', Plain, Acute, false),
        'Ò' => ('O', Plain, Grave, false),
        'Ỏ' => ('O', Plain, Hook, false),
        'Õ' => ('O', Plain, Tilde, false),
        'Ọ' => ('O', Plain, DotBelow, false),
        'Ô' => ('O', Circumflex, None, false),
        'Ố' => ('O', Circumflex, Acute, false),
        'Ồ' => ('O', Circumflex, Grave, false),
        'Ổ' => ('O', Circumflex, Hook, false),
        'Ỗ' => ('O', Circumflex, Tilde, false),
        'Ộ' => ('O', Circumflex, DotBelow, false),
        'Ơ' => ('O', Horn, None, false),
        'Ớ' => ('O', Horn, Acute, false),
        'Ờ' => ('O', Horn, Grave, false),
        'Ở' => ('O', Horn, Hook, false),
        'Ỡ' => ('O', Horn, Tilde, false),
        'Ợ' => ('O', Horn, DotBelow, false),
        // Lowercase u / ư
        'ú' => ('u', Plain, Acute, false),
        'ù' => ('u', Plain, Grave, false),
        'ủ' => ('u', Plain, Hook, false),
        'ũ' => ('u', Plain, Tilde, false),
        'ụ' => ('u', Plain, DotBelow, false),
        'ư' => ('u', Horn, None, false),
        'ứ' => ('u', Horn, Acute, false),
        'ừ' => ('u', Horn, Grave, false),
        'ử' => ('u', Horn, Hook, false),
        'ữ' => ('u', Horn, Tilde, false),
        'ự' => ('u', Horn, DotBelow, false),
        // Uppercase U / Ư
        'Ú' => ('U', Plain, Acute, false),
        'Ù' => ('U', Plain, Grave, false),
        'Ủ' => ('U', Plain, Hook, false),
        'Ũ' => ('U', Plain, Tilde, false),
        'Ụ' => ('U', Plain, DotBelow, false),
        'Ư' => ('U', Horn, None, false),
        'Ứ' => ('U', Horn, Acute, false),
        'Ừ' => ('U', Horn, Grave, false),
        'Ử' => ('U', Horn, Hook, false),
        'Ữ' => ('U', Horn, Tilde, false),
        'Ự' => ('U', Horn, DotBelow, false),
        // Lowercase / Uppercase y / Y
        'ý' => ('y', Plain, Acute, false),
        'ỳ' => ('y', Plain, Grave, false),
        'ỷ' => ('y', Plain, Hook, false),
        'ỹ' => ('y', Plain, Tilde, false),
        'ỵ' => ('y', Plain, DotBelow, false),
        'Ý' => ('Y', Plain, Acute, false),
        'Ỳ' => ('Y', Plain, Grave, false),
        'Ỷ' => ('Y', Plain, Hook, false),
        'Ỹ' => ('Y', Plain, Tilde, false),
        'Ỵ' => ('Y', Plain, DotBelow, false),
        // Barred d / D
        'đ' => ('d', Plain, None, true),
        'Đ' => ('D', Plain, None, true),
        other => (other, Plain, None, false),
    };
    DecomposedChar {
        base,
        shape,
        tone,
        barred_d,
    }
}

/// 8x16 glyph bitmap (each `u8` is one row, bit 7 = leftmost pixel, bit 0 = rightmost).
/// Uppercase occupies rows `4..=12`, lowercase x-height occupies `6..=12`,
/// descenders occupy `13..=14`, diacritics occupy `0..=5` and `14..=15`.
fn base_glyph_8x16(ch: char, strip_i_dot: bool) -> [u8; 16] {
    match ch {
        // Lowercase a-z
        'a' => [
            0, 0, 0, 0, 0, 0, 0x3C, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0,
        ],
        'b' => [
            0, 0, 0, 0, 0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x7C, 0, 0, 0,
        ],
        'c' => [
            0, 0, 0, 0, 0, 0, 0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0, 0, 0,
        ],
        'd' => [
            0, 0, 0, 0, 0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0,
        ],
        'e' => [
            0, 0, 0, 0, 0, 0, 0x3C, 0x66, 0x7E, 0x60, 0x60, 0x66, 0x3C, 0, 0, 0,
        ],
        'f' => [
            0, 0, 0, 0, 0x1C, 0x30, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x30, 0, 0, 0,
        ],
        'g' => [
            0, 0, 0, 0, 0, 0, 0x3E, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0,
        ],
        'h' => [
            0, 0, 0, 0, 0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0, 0, 0,
        ],
        'i' => {
            let dot = if strip_i_dot { 0 } else { 0x18 };
            [
                0, 0, 0, 0, dot, 0, 0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0,
            ]
        }
        'j' => [
            0, 0, 0, 0, 0x0C, 0, 0x1C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0,
        ],
        'k' => [
            0, 0, 0, 0, 0x60, 0x60, 0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0, 0, 0,
        ],
        'l' => [
            0, 0, 0, 0, 0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0,
        ],
        'm' => [
            0, 0, 0, 0, 0, 0, 0x6C, 0xFE, 0xD6, 0xD6, 0xC6, 0xC6, 0xC6, 0, 0, 0,
        ],
        'n' => [
            0, 0, 0, 0, 0, 0, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0, 0, 0,
        ],
        'o' => [
            0, 0, 0, 0, 0, 0, 0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        'p' => [
            0, 0, 0, 0, 0, 0, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0,
        ],
        'q' => [
            0, 0, 0, 0, 0, 0, 0x3E, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x06, 0x06, 0,
        ],
        'r' => [
            0, 0, 0, 0, 0, 0, 0x5C, 0x66, 0x60, 0x60, 0x60, 0x60, 0x60, 0, 0, 0,
        ],
        's' => [
            0, 0, 0, 0, 0, 0, 0x3E, 0x60, 0x60, 0x3C, 0x06, 0x06, 0x7C, 0, 0, 0,
        ],
        't' => [
            0, 0, 0, 0, 0x30, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x36, 0x1C, 0, 0, 0,
        ],
        'u' => [
            0, 0, 0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0,
        ],
        'v' => [
            0, 0, 0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0,
        ],
        'w' => [
            0, 0, 0, 0, 0, 0, 0xC6, 0xC6, 0xC6, 0xD6, 0xD6, 0xFE, 0x6C, 0, 0, 0,
        ],
        'x' => [
            0, 0, 0, 0, 0, 0, 0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0, 0, 0,
        ],
        'y' => [
            0, 0, 0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0,
        ],
        'z' => [
            0, 0, 0, 0, 0, 0, 0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0, 0, 0,
        ],
        // Uppercase A-Z
        'A' => [
            0, 0, 0, 0, 0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0, 0, 0,
        ],
        'B' => [
            0, 0, 0, 0, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0, 0, 0,
        ],
        'C' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x60, 0x60, 0x60, 0x60, 0x60, 0x66, 0x3C, 0, 0, 0,
        ],
        'D' => [
            0, 0, 0, 0, 0x78, 0x6C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x6C, 0x78, 0, 0, 0,
        ],
        'E' => [
            0, 0, 0, 0, 0x7E, 0x60, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x7E, 0, 0, 0,
        ],
        'F' => [
            0, 0, 0, 0, 0x7E, 0x60, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x60, 0, 0, 0,
        ],
        'G' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x60, 0x60, 0x6E, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0,
        ],
        'H' => [
            0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0, 0, 0,
        ],
        'I' => [
            0, 0, 0, 0, 0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0,
        ],
        'J' => [
            0, 0, 0, 0, 0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x6C, 0x38, 0, 0, 0,
        ],
        'K' => [
            0, 0, 0, 0, 0x66, 0x6C, 0x78, 0x70, 0x70, 0x78, 0x6C, 0x66, 0x66, 0, 0, 0,
        ],
        'L' => [
            0, 0, 0, 0, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0, 0, 0,
        ],
        'M' => [
            0, 0, 0, 0, 0xC6, 0xEE, 0xFE, 0xD6, 0xC6, 0xC6, 0xC6, 0xC6, 0xC6, 0, 0, 0,
        ],
        'N' => [
            0, 0, 0, 0, 0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x66, 0x66, 0, 0, 0,
        ],
        'O' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        'P' => [
            0, 0, 0, 0, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x60, 0, 0, 0,
        ],
        'Q' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x6E, 0x3C, 0x06, 0, 0, 0,
        ],
        'R' => [
            0, 0, 0, 0, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x70, 0x6C, 0x66, 0x66, 0, 0, 0,
        ],
        'S' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x60, 0x38, 0x0C, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0,
        ],
        'T' => [
            0, 0, 0, 0, 0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0,
        ],
        'U' => [
            0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        'V' => [
            0, 0, 0, 0, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0,
        ],
        'W' => [
            0, 0, 0, 0, 0xC6, 0xC6, 0xC6, 0xC6, 0xC6, 0xD6, 0xFE, 0xEE, 0xC6, 0, 0, 0,
        ],
        'X' => [
            0, 0, 0, 0, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x3C, 0x66, 0x66, 0, 0, 0,
        ],
        'Y' => [
            0, 0, 0, 0, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0,
        ],
        'Z' => [
            0, 0, 0, 0, 0x7E, 0x06, 0x0C, 0x18, 0x18, 0x30, 0x60, 0x60, 0x7E, 0, 0, 0,
        ],
        // Digits 0-9
        '0' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        '1' => [
            0, 0, 0, 0, 0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0,
        ],
        '2' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x60, 0x7E, 0, 0, 0,
        ],
        '3' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x06, 0x06, 0x1C, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0,
        ],
        '4' => [
            0, 0, 0, 0, 0x0C, 0x1C, 0x3C, 0x6C, 0x6C, 0x7E, 0x0C, 0x0C, 0x0C, 0, 0, 0,
        ],
        '5' => [
            0, 0, 0, 0, 0x7E, 0x60, 0x60, 0x7C, 0x06, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0,
        ],
        '6' => [
            0, 0, 0, 0, 0x3C, 0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        '7' => [
            0, 0, 0, 0, 0x7E, 0x06, 0x0C, 0x0C, 0x18, 0x18, 0x30, 0x30, 0x30, 0, 0, 0,
        ],
        '8' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0,
        ],
        '9' => [
            0, 0, 0, 0, 0x3C, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0,
        ],
        // Punctuation
        '[' => [
            0, 0, 0, 0, 0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0, 0, 0,
        ],
        ']' => [
            0, 0, 0, 0, 0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0, 0, 0,
        ],
        '|' => [
            0, 0, 0, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0,
        ],
        '-' => [0, 0, 0, 0, 0, 0, 0, 0, 0x7E, 0, 0, 0, 0, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x7E, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0, 0, 0],
        ',' => [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0x30, 0, 0],
        ':' => [0, 0, 0, 0, 0, 0, 0x18, 0x18, 0, 0, 0, 0x18, 0x18, 0, 0, 0],
        '/' => [
            0, 0, 0, 0, 0x06, 0x0C, 0x0C, 0x18, 0x18, 0x30, 0x30, 0x60, 0x60, 0, 0, 0,
        ],
        '\'' => [0, 0, 0, 0, 0x18, 0x18, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ' ' => [0; 16],
        _ => [
            0, 0, 0, 0, 0x3C, 0x66, 0x06, 0x0C, 0x18, 0x18, 0, 0x18, 0x18, 0, 0, 0,
        ],
    }
}

/// Composes an 8x16 Unicode glyph bitmap for any ASCII or Vietnamese character.
pub fn glyph_8x16(ch: char) -> [u8; 16] {
    let dec = decompose(ch);
    let strip_i_dot = dec.base == 'i' && !matches!(dec.tone, Tone::None | Tone::DotBelow);
    let mut rows = base_glyph_8x16(dec.base, strip_i_dot);
    let is_upper = dec.base.is_ascii_uppercase();

    if dec.barred_d {
        if is_upper {
            // Crossbar on left stem of 'Đ'
            rows[8] |= 0xF0;
        } else {
            // Crossbar through top stem of 'đ'
            rows[4] |= 0x1F;
        }
        return rows;
    }

    // 1. Apply vowel shape (circumflex ^, breve ˘, horn ˒)
    let has_top_shape = matches!(dec.shape, VowelShape::Circumflex | VowelShape::Breve);
    match dec.shape {
        VowelShape::Circumflex => {
            let r = if is_upper { 1 } else { 3 };
            rows[r] |= 0x18;
            rows[r + 1] |= 0x24;
        }
        VowelShape::Breve => {
            let r = if is_upper { 1 } else { 3 };
            rows[r] |= 0x42;
            rows[r + 1] |= 0x3C;
        }
        VowelShape::Horn => {
            if is_upper {
                rows[4] |= 0x03;
                rows[5] |= 0x01;
                rows[6] |= 0x02;
            } else {
                rows[5] |= 0x03;
                rows[6] |= 0x01;
                rows[7] |= 0x02;
            }
        }
        VowelShape::Plain => {}
    }

    // 2. Apply tone mark (stacked cleanly above/beside circumflex or breve)
    match dec.tone {
        Tone::None => {}
        Tone::DotBelow => {
            rows[14] |= 0x18;
            rows[15] |= 0x18;
        }
        Tone::Acute => {
            if has_top_shape {
                let r = if is_upper { 0 } else { 1 };
                rows[r] |= 0x06;
                rows[r + 1] |= 0x0C;
            } else {
                let r = if is_upper { 1 } else { 3 };
                rows[r] |= 0x0C;
                rows[r + 1] |= 0x18;
            }
        }
        Tone::Grave => {
            if has_top_shape {
                let r = if is_upper { 0 } else { 1 };
                rows[r] |= 0x60;
                rows[r + 1] |= 0x30;
            } else {
                let r = if is_upper { 1 } else { 3 };
                rows[r] |= 0x30;
                rows[r + 1] |= 0x18;
            }
        }
        Tone::Hook => {
            if has_top_shape {
                rows[0] |= 0x1C;
                rows[1] |= 0x04;
                rows[2] |= 0x08;
            } else {
                let r = if is_upper { 0 } else { 2 };
                rows[r] |= 0x38;
                rows[r + 1] |= 0x08;
                rows[r + 2] |= 0x10;
            }
        }
        Tone::Tilde => {
            if has_top_shape {
                let r = if is_upper { 0 } else { 1 };
                rows[r] |= 0x32;
                rows[r + 1] |= 0x4C;
            } else {
                let r = if is_upper { 1 } else { 3 };
                rows[r] |= 0x32;
                rows[r + 1] |= 0x4C;
            }
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vietnamese_diacritics_and_case_produce_distinct_glyphs() {
        assert_ne!(glyph_8x16('a'), glyph_8x16('A'));
        assert_ne!(glyph_8x16('d'), glyph_8x16('đ'));
        assert_ne!(glyph_8x16('D'), glyph_8x16('Đ'));
        assert_ne!(glyph_8x16('e'), glyph_8x16('ê'));
        assert_ne!(glyph_8x16('ê'), glyph_8x16('ế'));
        assert_ne!(glyph_8x16('ế'), glyph_8x16('ề'));
        assert_ne!(glyph_8x16('ề'), glyph_8x16('ể'));
        assert_ne!(glyph_8x16('ể'), glyph_8x16('ễ'));
        assert_ne!(glyph_8x16('ễ'), glyph_8x16('ệ'));
        assert_ne!(glyph_8x16('o'), glyph_8x16('ơ'));
        assert_ne!(glyph_8x16('ơ'), glyph_8x16('ợ'));
        assert_ne!(glyph_8x16('u'), glyph_8x16('ư'));
        assert_ne!(glyph_8x16('ư'), glyph_8x16('ứ'));
    }
}
