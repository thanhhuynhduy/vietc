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

/// Physical key coordinates `(row, col)` on US QWERTY for spatial proximity scoring.
fn key_coord(ch: char) -> Option<(f32, f32)> {
    let (row, col) = match ch.to_ascii_lowercase() {
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
    };
    Some((row, col))
}

/// Euclidean distance on a US QWERTY physical keyboard (adjacent keys <= 1.25).
pub(crate) fn key_distance(a: char, b: char) -> f32 {
    match (key_coord(a), key_coord(b)) {
        (Some((r1, c1)), Some((r2, c2))) => ((r1 - r2).powi(2) + (c1 - c2).powi(2)).sqrt(),
        _ => f32::INFINITY,
    }
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
    compose: &impl Fn(&str) -> String,
) -> Option<String> {
    if chars.len() < 4 {
        return None;
    }
    let mut unique: Option<String> = None;
    for i in 0..chars.len() {
        let orig = chars[i];
        for &neighbor in &QWERTY_ALPHA {
            if neighbor == orig || key_distance(orig, neighbor) > MAX_SPATIAL_KEY_DISTANCE {
                continue;
            }
            let mut mutated = chars.to_vec();
            mutated[i] = neighbor;
            let cand_raw: String = mutated.into_iter().collect();
            let cand_vn = compose(&cand_raw);
            if is_valid_diacritic_vn_word(&cand_vn)
                && crate::ring_buffer::visible_to_canonical_raw(&cand_vn, method)
                    .is_some_and(|canon| canon.len() == cand_raw.len())
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
    if crate::english::has_mixed_internal_caps(raw, composed) {
        return None;
    }

    let raw_lower = raw.to_ascii_lowercase();
    if user_dict.is_keep_raw(&raw_lower)
        || user_dict.is_known_personal_word(&raw_lower)
        || (crate::spelling::is_valid_vietnamese_syllable(composed)
            && crate::spelling::is_in_vietnamese_dictionary(composed))
        || dict.is_english_word(&raw_lower)
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
        return try_core_english_transposition(&chars, dict).map(|en| match_casing(raw, &en));
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

        // 4. Vietnamese US QWERTY spatial key-proximity substitution (`tieemgs -> tiếng`, `khoonh -> không`).
        if let Some(vn) = try_vn_spatial_substitution(&chars, method, &compose) {
            return Some(match_casing(raw, &vn));
        }
    }

    // 5. Core English single adjacent transposition (`proejct -> project`, `netwrok -> network`).
    if let Some(en) = try_core_english_transposition(&chars, dict) {
        return Some(match_casing(raw, &en));
    }

    None
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
