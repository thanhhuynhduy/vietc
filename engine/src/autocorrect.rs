// SPDX-License-Identifier: MIT
//! Gboard-style Spatial & Transposition Autocorrect.
//!
//! Covers 4 fast-typing error classes with strict zero-false-positive guards:
//! 1. High-precision English/dev typos + unambiguous Core English transpositions.
//! 2. Vietnamese single adjacent key transposition (`tienegs -> tiếng`, `khonog -> không`).
//! 3. Vietnamese accidental duplicate non-Telex consonant (`khoongg -> không`, `vieettj -> việt`).
//! 4. Vietnamese QWERTY spatial key-proximity substitution (`key_distance <= 1.25`:
//!    `tieemgs -> tiếng`, `khoonh -> không`, `vieetk -> việt`, `dduowch -> được`).
//!
//! Safety invariants:
//! - Never modifies a valid Vietnamese dictionary word, valid English word,
//!   known personal dictionary word, ALL_CAPS acronym, or token with digits/symbols.
//! - Only accepts a candidate when it is the **sole unambiguous** match.
//! - Always paired with One-Tap Undo (`pending_restore_undo`).

use crate::engine_helpers::match_casing;
use crate::english::{EnglishDict, LanguageContext};
use crate::input_method::InputMethod;
use crate::user_dict::UserDict;

const MAX_SPATIAL_KEY_DISTANCE: f32 = 1.25;
const QWERTY_ALPHA: [char; 26] = [
    'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', 'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l',
    'z', 'x', 'c', 'v', 'b', 'n', 'm',
];
const QWERTY_VNI_DIGITS: [char; 9] = ['1', '2', '3', '4', '5', '6', '7', '8', '9'];

/// Physical keyboard layout used for spatial typo proximity scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutocorrectLayout {
    #[default]
    UsQwerty,
    Dvorak,
    Colemak,
}

impl AutocorrectLayout {
    pub fn from_str_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "dvorak" | "us-dvorak" | "us_dvorak" => Self::Dvorak,
            "colemak" | "us-colemak" | "us_colemak" => Self::Colemak,
            _ => Self::UsQwerty,
        }
    }
}

/// Physical key coordinates `(row, col)` for spatial proximity scoring across
/// US QWERTY, Dvorak, and Colemak layouts (synchronized with `protocol/src/layout.rs`).
fn key_coord_for_layout(layout: AutocorrectLayout, ch: char) -> Option<(f32, f32)> {
    let lower = ch.to_ascii_lowercase();
    let digit_coord = match lower {
        '1' => Some((-1.0, 0.0)),
        '2' => Some((-1.0, 1.0)),
        '3' => Some((-1.0, 2.0)),
        '4' => Some((-1.0, 3.0)),
        '5' => Some((-1.0, 4.0)),
        '6' => Some((-1.0, 5.0)),
        '7' => Some((-1.0, 6.0)),
        '8' => Some((-1.0, 7.0)),
        '9' => Some((-1.0, 8.0)),
        '0' => Some((-1.0, 9.0)),
        _ => None,
    };
    if digit_coord.is_some() {
        return digit_coord;
    }
    let (row, col) = match layout {
        AutocorrectLayout::UsQwerty => match lower {
            'q' => (0.0, 0.0),
            'w' => (0.0, 1.0),
            'e' => (0.0, 2.0),
            'r' => (0.0, 3.0),
            't' => (0.0, 4.0),
            'y' => (0.0, 5.0),
            'u' => (0.0, 6.0),
            'i' => (0.0, 7.0),
            'o' => (0.0, 8.0),
            'p' => (0.0, 9.0),
            'a' => (1.0, 0.25),
            's' => (1.0, 1.25),
            'd' => (1.0, 2.25),
            'f' => (1.0, 3.25),
            'g' => (1.0, 4.25),
            'h' => (1.0, 5.25),
            'j' => (1.0, 6.25),
            'k' => (1.0, 7.25),
            'l' => (1.0, 8.25),
            'z' => (2.0, 0.75),
            'x' => (2.0, 1.75),
            'c' => (2.0, 2.75),
            'v' => (2.0, 3.75),
            'b' => (2.0, 4.75),
            'n' => (2.0, 5.75),
            'm' => (2.0, 6.75),
            _ => return None,
        },
        AutocorrectLayout::Dvorak => match lower {
            'p' => (0.0, 3.0),
            'y' => (0.0, 4.0),
            'f' => (0.0, 5.0),
            'g' => (0.0, 6.0),
            'c' => (0.0, 7.0),
            'r' => (0.0, 8.0),
            'l' => (0.0, 9.0),
            'a' => (1.0, 0.25),
            'o' => (1.0, 1.25),
            'e' => (1.0, 2.25),
            'u' => (1.0, 3.25),
            'i' => (1.0, 4.25),
            'd' => (1.0, 5.25),
            'h' => (1.0, 6.25),
            't' => (1.0, 7.25),
            'n' => (1.0, 8.25),
            's' => (1.0, 9.25),
            'q' => (2.0, 1.75),
            'j' => (2.0, 2.75),
            'k' => (2.0, 3.75),
            'x' => (2.0, 4.75),
            'b' => (2.0, 5.75),
            'm' => (2.0, 6.75),
            'w' => (2.0, 7.75),
            'v' => (2.0, 8.75),
            'z' => (2.0, 9.75),
            _ => return None,
        },
        AutocorrectLayout::Colemak => match lower {
            'q' => (0.0, 0.0),
            'w' => (0.0, 1.0),
            'f' => (0.0, 2.0),
            'p' => (0.0, 3.0),
            'g' => (0.0, 4.0),
            'j' => (0.0, 5.0),
            'l' => (0.0, 6.0),
            'u' => (0.0, 7.0),
            'y' => (0.0, 8.0),
            'a' => (1.0, 0.25),
            'r' => (1.0, 1.25),
            's' => (1.0, 2.25),
            't' => (1.0, 3.25),
            'd' => (1.0, 4.25),
            'h' => (1.0, 5.25),
            'n' => (1.0, 6.25),
            'e' => (1.0, 7.25),
            'i' => (1.0, 8.25),
            'o' => (1.0, 9.25),
            'z' => (2.0, 0.75),
            'x' => (2.0, 1.75),
            'c' => (2.0, 2.75),
            'v' => (2.0, 3.75),
            'b' => (2.0, 4.75),
            'k' => (2.0, 5.75),
            'm' => (2.0, 6.75),
            _ => return None,
        },
    };
    Some((row, col))
}

pub(crate) fn key_distance_for_layout(layout: AutocorrectLayout, a: char, b: char) -> f32 {
    match (
        key_coord_for_layout(layout, a),
        key_coord_for_layout(layout, b),
    ) {
        (Some((r1, c1)), Some((r2, c2))) => ((r1 - r2).powi(2) + (c1 - c2).powi(2)).sqrt(),
        _ => f32::INFINITY,
    }
}

/// Euclidean distance on a US QWERTY physical keyboard (adjacent keys <= 1.25).
#[cfg(test)]
pub(crate) fn key_distance(a: char, b: char) -> f32 {
    key_distance_for_layout(AutocorrectLayout::UsQwerty, a, b)
}

fn is_valid_diacritic_vn_word(candidate: &str) -> bool {
    !candidate.is_ascii()
        && crate::spelling::is_valid_vietnamese_syllable(candidate)
        && crate::spelling::is_in_vietnamese_dictionary(candidate)
}

/// High-precision English & developer typo table (strictly invalid in both
/// English and Vietnamese dictionaries).
fn common_english_typo(raw_lower: &str) -> Option<&'static str> {
    match raw_lower {
        "teh" => Some("the"),
        "adn" => Some("and"),
        "taht" => Some("that"),
        "wiht" => Some("with"),
        "thsi" => Some("this"),
        "recieve" => Some("receive"),
        "beleive" => Some("believe"),
        "seperate" => Some("separate"),
        "occured" => Some("occurred"),
        "untill" => Some("until"),
        "wich" => Some("which"),
        "becuase" => Some("because"),
        "thier" => Some("their"),
        "freind" => Some("friend"),
        "definately" => Some("definitely"),
        "enviroment" => Some("environment"),
        "langauge" => Some("language"),
        "fucntion" => Some("function"),
        "retunr" => Some("return"),
        "improt" => Some("import"),
        "exprot" => Some("export"),
        "asycn" => Some("async"),
        "awiat" => Some("await"),
        "stuct" => Some("struct"),
        "comit" => Some("commit"),
        "puhs" => Some("push"),
        "mereg" => Some("merge"),
        "bracnh" => Some("branch"),
        "sercer" => Some("server"),
        "clinet" => Some("client"),
        "databse" => Some("database"),
        "passwrod" => Some("password"),
        "keybaord" => Some("keyboard"),
        "widnows" => Some("windows"),
        "cosimc" => Some("cosmic"),
        "wayalnd" => Some("wayland"),
        _ => None,
    }
}

fn is_method_double_undo_letter(method: InputMethod, ch: char) -> bool {
    match method {
        InputMethod::Telex => matches!(
            ch,
            'a' | 'e' | 'o' | 'd' | 'w' | 's' | 'f' | 'r' | 'x' | 'j' | 'z'
        ),
        InputMethod::Vni => ch.is_ascii_digit(),
    }
}

fn try_vn_transposition(chars: &[char], compose: &impl Fn(&str) -> String) -> Option<String> {
    let mut unique: Option<String> = None;
    for i in 0..chars.len().saturating_sub(1) {
        if chars[i] == chars[i + 1] {
            continue;
        }
        let mut swapped = chars.to_vec();
        swapped.swap(i, i + 1);
        let cand_raw: String = swapped.into_iter().collect();
        let cand_vn = compose(&cand_raw);
        if is_valid_diacritic_vn_word(&cand_vn) {
            match &unique {
                None => unique = Some(cand_vn),
                Some(existing) if existing == &cand_vn => {}
                Some(_) => return None,
            }
        }
    }
    unique
}

fn try_vn_duplicate_consonant_delete(
    chars: &[char],
    method: InputMethod,
    compose: &impl Fn(&str) -> String,
) -> Option<String> {
    let mut unique: Option<String> = None;
    for i in 0..chars.len().saturating_sub(1) {
        if chars[i] == chars[i + 1] && !is_method_double_undo_letter(method, chars[i]) {
            let mut deduped = Vec::with_capacity(chars.len() - 1);
            deduped.extend_from_slice(&chars[..i]);
            deduped.extend_from_slice(&chars[i + 1..]);
            let cand_raw: String = deduped.into_iter().collect();
            let cand_vn = compose(&cand_raw);
            if is_valid_diacritic_vn_word(&cand_vn) {
                match &unique {
                    None => unique = Some(cand_vn),
                    Some(existing) if existing == &cand_vn => {}
                    Some(_) => return None,
                }
            }
        }
    }
    unique
}

fn try_vn_spatial_substitution(
    chars: &[char],
    method: InputMethod,
    layout: AutocorrectLayout,
    compose: &impl Fn(&str) -> String,
) -> Option<String> {
    if chars.len() < 4 {
        return None;
    }
    let mut unique: Option<String> = None;
    for i in 0..chars.len() {
        let orig = chars[i];
        let neighbors: &[char] = if orig.is_ascii_digit() {
            if method == InputMethod::Vni {
                &QWERTY_VNI_DIGITS
            } else {
                continue;
            }
        } else if orig.is_ascii_alphabetic() {
            &QWERTY_ALPHA
        } else {
            continue;
        };
        for &neighbor in neighbors {
            if neighbor == orig
                || key_distance_for_layout(layout, orig, neighbor) > MAX_SPATIAL_KEY_DISTANCE
            {
                continue;
            }
            let mut mutated = chars.to_vec();
            mutated[i] = neighbor;
            let cand_raw: String = mutated.into_iter().collect();
            let cand_vn = compose(&cand_raw);
            if is_valid_diacritic_vn_word(&cand_vn)
                && crate::ring_buffer::visible_to_canonical_raw(&cand_vn, method).is_some_and(
                    |canon| {
                        canon.len() == cand_raw.len()
                            || (method == InputMethod::Vni
                                && cand_raw.contains('7')
                                && canon.len() == cand_raw.len() + 1)
                    },
                )
            {
                match &unique {
                    None => unique = Some(cand_vn),
                    Some(existing) if existing == &cand_vn => {}
                    Some(_) => return None,
                }
            }
        }
    }
    unique
}

fn try_core_english_transposition(chars: &[char], dict: &EnglishDict) -> Option<String> {
    if chars.len() < 4 {
        return None;
    }
    let mut unique: Option<String> = None;
    for i in 0..chars.len().saturating_sub(1) {
        if chars[i] == chars[i + 1] {
            continue;
        }
        let mut swapped = chars.to_vec();
        swapped.swap(i, i + 1);
        let cand: String = swapped.into_iter().collect();
        if dict.is_core_english_word(&cand) {
            match &unique {
                None => unique = Some(cand),
                Some(existing) if existing == &cand => {}
                Some(_) => return None,
            }
        }
    }
    unique
}

/// Suggest an unambiguous typo correction on Space, returning `None` whenever
/// the input is already valid, protected by `UserDict`, or ambiguous.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn suggest_correction(
    dict: &EnglishDict,
    user_dict: &UserDict,
    composed: &str,
    raw: &str,
    method: InputMethod,
    prev_ctx: LanguageContext,
    compose: impl Fn(&str) -> String,
) -> Option<String> {
    suggest_correction_for_layout(
        dict,
        user_dict,
        composed,
        raw,
        method,
        AutocorrectLayout::UsQwerty,
        prev_ctx,
        compose,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn suggest_correction_for_layout(
    dict: &EnglishDict,
    user_dict: &UserDict,
    composed: &str,
    raw: &str,
    method: InputMethod,
    layout: AutocorrectLayout,
    prev_ctx: LanguageContext,
    compose: impl Fn(&str) -> String,
) -> Option<String> {
    suggest_correction_with_context(
        dict,
        user_dict,
        composed,
        raw,
        method,
        layout,
        prev_ctx,
        &[],
        compose,
    )
}

fn is_known_english_inflection(dict: &EnglishDict, lower: &str) -> bool {
    if lower.len() < 4 || !lower.is_ascii() {
        return false;
    }
    // Regular plural / 3rd-person singular / past / agentive / adverbial suffixes
    for suffix in ["s", "es", "ed", "d", "er", "r", "ing", "ly", "ion"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            if stem.len() >= 3 && dict.is_english_word(stem) {
                return true;
            }
            if matches!(suffix, "ing" | "er" | "ion") && stem.len() >= 3 {
                let mut with_e = String::with_capacity(stem.len() + 1);
                with_e.push_str(stem);
                with_e.push('e');
                if dict.is_english_word(&with_e) {
                    return true;
                }
            }
        }
    }
    for suffix in ["ies", "ied"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            if stem.len() >= 2 {
                let mut with_y = String::with_capacity(stem.len() + 1);
                with_y.push_str(stem);
                with_y.push('y');
                if dict.is_english_word(&with_y) {
                    return true;
                }
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn suggest_correction_with_context(
    dict: &EnglishDict,
    user_dict: &UserDict,
    composed: &str,
    raw: &str,
    method: InputMethod,
    layout: AutocorrectLayout,
    prev_ctx: LanguageContext,
    prev_words: &[&str],
    compose: impl Fn(&str) -> String,
) -> Option<String> {
    if raw.len() < 3 || raw.len() > 24 || !raw.is_ascii() {
        return None;
    }
    let allow_digits = method == InputMethod::Vni;
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphabetic() || (allow_digits && c.is_ascii_digit()))
    {
        return None;
    }
    if raw.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2
        && raw
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .all(|c| c.is_ascii_uppercase())
    {
        return None;
    }
    // Skip mixed-case acronyms / Pascal-acronym identifiers (`IBus`, `macOS`, `WaylandVK`).
    let upper_count = raw.chars().filter(|c| c.is_ascii_uppercase()).count();
    let lower_count = raw.chars().filter(|c| c.is_ascii_lowercase()).count();
    if upper_count >= 2 && lower_count >= 1 {
        return None;
    }
    if crate::english::has_mixed_internal_caps(raw, composed) {
        return None;
    }

    let raw_lower = raw.to_ascii_lowercase();
    if user_dict.is_keep_raw(&raw_lower)
        || user_dict.is_known_personal_word(&raw_lower)
        || (crate::spelling::is_valid_vietnamese_syllable(composed)
            && crate::spelling::is_in_vietnamese_dictionary(composed))
        || dict.is_english_word(&raw_lower)
        || is_known_english_inflection(dict, &raw_lower)
    {
        return None;
    }

    // 1. High-precision English/dev typo table (preserve Vietnamese "adn" = DNA).
    if !(raw_lower == "adn" && prev_ctx == LanguageContext::Vietnamese) {
        if let Some(fixed) = common_english_typo(&raw_lower) {
            return Some(match_casing(raw, fixed));
        }
    }

    let chars: Vec<char> = raw_lower.chars().collect();

    // When already in English context, never corrupt OOV English/dev tokens into Vietnamese.
    if prev_ctx == LanguageContext::English {
        if let Some(en) = try_core_english_transposition(&chars, dict) {
            return Some(match_casing(raw, &en));
        }
        return crate::symspell::suggest_multi_error(
            user_dict, &raw_lower, method, layout, prev_ctx, prev_words, &compose,
        )
        .map(|en| match_casing(raw, &en));
    }

    // Vietnamese syllables never exceed 10 raw keys; skip expensive VN passes on long tokens.
    if chars.len() <= 10 {
        // 2. Vietnamese single adjacent key transposition (`tienegs -> tiếng`).
        if let Some(vn) = try_vn_transposition(&chars, &compose) {
            return Some(match_casing(raw, &vn));
        }

        // 3. Vietnamese accidental duplicate non-undo key (`khoongg -> không`, VNI `khoong6 -> không`).
        if let Some(vn) = try_vn_duplicate_consonant_delete(&chars, method, &compose) {
            return Some(match_casing(raw, &vn));
        }

        // 4. Vietnamese layout-aware spatial key-proximity substitution (`tieemgs -> tiếng`, `khoonh -> không`).
        if let Some(vn) = try_vn_spatial_substitution(&chars, method, layout, &compose) {
            return Some(match_casing(raw, &vn));
        }
    }

    // 5. Core English single adjacent transposition (`proejct -> project`, `netwrok -> network`).
    if let Some(en) = try_core_english_transposition(&chars, dict) {
        return Some(match_casing(raw, &en));
    }

    // 6. Weighted Spatial SymSpell (missing/extra/multi-key + n-gram & Personal Bigram
    //    context ranking + missing-space Vietnamese compound splitting).
    crate::symspell::suggest_multi_error(
        user_dict, &raw_lower, method, layout, prev_ctx, prev_words, &compose,
    )
    .map(|fixed| match_casing(raw, &fixed))
}

#[cfg(test)]
mod tests {
    use super::{key_distance, suggest_correction};
    use crate::english::{EnglishDict, LanguageContext};
    use crate::input_method::InputMethod;
    use crate::user_dict::UserDict;

    #[test]
    fn layout_key_distance_reflects_physical_adjacency() {
        assert!(key_distance('e', 'r') <= 1.1);
        assert!(key_distance('d', 's') <= 1.1);
        assert!(key_distance('h', 'g') <= 1.1);
        assert!(key_distance('k', 'j') <= 1.1);
        assert!(key_distance('q', 'm') > 5.0);
    }

    #[test]
    fn fixes_all_four_error_classes_without_touching_valid_or_personal_words() {
        let dict = EnglishDict::new();
        let mut user = UserDict::new();
        let compose = |r: &str| crate::telex::resolve_and_render(r);

        // Class 1 & 5: English table + Core English transposition
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                "Teh",
                "Teh",
                InputMethod::Telex,
                LanguageContext::English,
                compose
            )
            .as_deref(),
            Some("The")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                "proejct",
                "proejct",
                InputMethod::Telex,
                LanguageContext::English,
                compose
            )
            .as_deref(),
            Some("project")
        );

        // Class 2: Vietnamese transposition
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("tienegs"),
                "tienegs",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("tiếng")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("khonog"),
                "khonog",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("không")
        );

        // Class 3: Duplicate non-Telex consonant deletion
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("khoongg"),
                "khoongg",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("không")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("vieettj"),
                "vieettj",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("việt")
        );

        // Class 4: Spatial QWERTY key-proximity substitution (`m` next to `n`, `h` next to `g`/`j`, `k` next to `j`)
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("tieemgs"),
                "tieemgs",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("tiếng")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("khoonh"),
                "khoonh",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("không")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("vieetk"),
                "vieetk",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("việt")
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("dduowch"),
                "dduowch",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            )
            .as_deref(),
            Some("được")
        );
        // Ambiguous spatial slip (`d` sits between tone keys `s`/`f`/`r`/`x`) must stay None
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                &compose("tieengd"),
                "tieengd",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            ),
            None
        );

        // Learned personal word is NEVER autocorrected
        user.record_word_usage("proejct");
        user.record_word_usage("proejct");
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                "proejct",
                "proejct",
                InputMethod::Telex,
                LanguageContext::English,
                compose
            ),
            None
        );

        // Mixed-internal-caps (camelCase) and Vietnamese "adn" (DNA) are NEVER corrupted
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                "proEjct",
                "proEjct",
                InputMethod::Telex,
                LanguageContext::English,
                compose
            ),
            None
        );
        assert_eq!(
            suggest_correction(
                &dict,
                &user,
                "adn",
                "adn",
                InputMethod::Telex,
                LanguageContext::Vietnamese,
                compose
            ),
            None
        );
    }
}
