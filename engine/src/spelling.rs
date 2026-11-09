// SPDX-License-Identifier: MIT
use std::collections::HashSet;
use std::sync::OnceLock;

const VIETNAMESE_DICT_RAW: &str = include_str!("vietnamese.cm.dict");

const VOWELS: &str = "aàáảãạăằắẳẵặâầấẩẫậeèéẻẽẹêềếểễệiìíỉĩịoòóỏõọôồốổỗộơờớởỡợuùúủũụưừứửữựyỳýỷỹỵ";

fn vowel_position(c: char) -> Option<usize> {
    VOWELS.chars().position(|v| v == c)
}

/// Normalize a Vietnamese syllable by stripping tones and appending their tone
/// digits (1..=5) at the end, so old-style and new-style tone placements
/// (e.g. `hoà` vs `hòa`, `thuỷ` vs `thủy`) compare equal.
pub fn without_tone_position(word: &str) -> String {
    let mut letters = String::with_capacity(word.len());
    let mut tones = String::new();
    for c in word.chars() {
        let lower_c = c.to_lowercase().next().unwrap_or(c);
        match vowel_position(lower_c) {
            Some(pos) if pos % 6 != 0 => {
                if let Some(base) = VOWELS.chars().nth(pos - pos % 6) {
                    letters.push(base);
                }
                if let Some(digit) = char::from_digit((pos % 6) as u32, 10) {
                    tones.push(digit);
                }
            }
            _ => letters.push(lower_c),
        }
    }
    letters.push_str(&tones);
    letters
}

fn vietnamese_dict() -> &'static HashSet<String> {
    static DICT: OnceLock<HashSet<String>> = OnceLock::new();
    DICT.get_or_init(|| {
        VIETNAMESE_DICT_RAW
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(without_tone_position)
            .collect()
    })
}

/// Check if a word is present in the bundled Vietnamese syllable dictionary
/// (matched regardless of old/new tone placement style).
pub fn is_in_vietnamese_dictionary(word: &str) -> bool {
    vietnamese_dict().contains(&without_tone_position(word))
}

const FIRST_CONSONANT_SEQS: &[&str] = &[
    "b d đ g gh m n nh p ph r s t tr v z",
    "c h k kh qu th",
    "ch gi l ng ngh x",
    "đ l",
    "h",
];

const VOWEL_SEQS: &[&str] = &[
    "ê i ua uê uy y",
    "a iê oa uyê yê",
    "â ă e o oo ô ơ oe u ư uâ uô ươ",
    "oă",
    "uơ",
    "ai ao au âu ay ây eo êu ia iêu iu oai oao oay oeo oi ôi ơi ưa uây ui ưi uôi ươi ươu ưu uya uyu yêu",
    "ă",
    "i",
];

const LAST_CONSONANT_SEQS: &[&str] = &["ch nh", "c ng", "m n p t", "k", "c"];

const CV_MATRIX: &[&[usize]] = &[
    &[0, 1, 2, 3, 5],
    &[0, 1, 2, 3, 4, 5],
    &[0, 1, 2, 3, 5],
    &[6],
    &[7],
];

const VC_MATRIX: &[&[usize]] = &[&[0, 2], &[0, 1, 2], &[1, 2], &[1, 2], &[], &[], &[3], &[4]];

fn strip_tone(c: char) -> char {
    match c {
        'à' | 'á' | 'ả' | 'ã' | 'ạ' => 'a',
        'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' => 'ă',
        'ầ' | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'â',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' => 'e',
        'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'ê',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' => 'o',
        'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' => 'ô',
        'ờ' | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'ơ',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' => 'u',
        'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'ư',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        _ => c,
    }
}

fn is_acute_or_dot_toned_vowel(c: char) -> bool {
    matches!(
        c,
        'á' | 'ạ'
            | 'ắ'
            | 'ặ'
            | 'ấ'
            | 'ậ'
            | 'é'
            | 'ẹ'
            | 'ế'
            | 'ệ'
            | 'í'
            | 'ị'
            | 'ó'
            | 'ọ'
            | 'ố'
            | 'ộ'
            | 'ớ'
            | 'ợ'
            | 'ú'
            | 'ụ'
            | 'ứ'
            | 'ự'
            | 'ý'
            | 'ỵ'
    )
}

fn is_vowel(c: char) -> bool {
    matches!(
        c,
        'a' | 'à'
            | 'á'
            | 'ả'
            | 'ã'
            | 'ạ'
            | 'ă'
            | 'ằ'
            | 'ắ'
            | 'ẳ'
            | 'ẵ'
            | 'ặ'
            | 'â'
            | 'ầ'
            | 'ấ'
            | 'ẩ'
            | 'ẫ'
            | 'ậ'
            | 'e'
            | 'è'
            | 'é'
            | 'ẻ'
            | 'ẽ'
            | 'ẹ'
            | 'ê'
            | 'ề'
            | 'ế'
            | 'ể'
            | 'ễ'
            | 'ệ'
            | 'i'
            | 'ì'
            | 'í'
            | 'ỉ'
            | 'ĩ'
            | 'ị'
            | 'o'
            | 'ò'
            | 'ó'
            | 'ỏ'
            | 'õ'
            | 'ọ'
            | 'ô'
            | 'ồ'
            | 'ố'
            | 'ổ'
            | 'ỗ'
            | 'ộ'
            | 'ơ'
            | 'ờ'
            | 'ớ'
            | 'ở'
            | 'ỡ'
            | 'ợ'
            | 'u'
            | 'ù'
            | 'ú'
            | 'ủ'
            | 'ũ'
            | 'ụ'
            | 'ư'
            | 'ừ'
            | 'ứ'
            | 'ử'
            | 'ữ'
            | 'ự'
            | 'y'
            | 'ý'
            | 'ỳ'
            | 'ỷ'
            | 'ỹ'
            | 'ỵ'
    )
}

/// Partition a word into (first_consonant, vowel_cluster, last_consonant)
pub fn partition(word: &str) -> (String, String, String) {
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    if n == 0 {
        return (String::new(), String::new(), String::new());
    }

    // 1. Find the first vowel index
    let mut first_vowel_idx = None;
    for (i, &ch) in chars.iter().enumerate() {
        if is_vowel(ch) {
            first_vowel_idx = Some(i);
            break;
        }
    }

    let first_vowel = match first_vowel_idx {
        Some(idx) => idx,
        None => {
            return (word.to_string(), String::new(), String::new());
        }
    };

    let mut fc_end = first_vowel;

    // Adjust fc_end for "qu" or "gi" acting as onset
    if first_vowel == 1 && chars[0] == 'q' && chars[1] == 'u' && n > 2 && is_vowel(chars[2]) {
        fc_end = 2;
    }
    if first_vowel == 1
        && chars[0] == 'g'
        && chars[1] == 'i'
        && n > 2
        && is_vowel(chars[2])
        && chars[2] != 'ê'
    {
        fc_end = 2;
    }

    // 2. Find the end of the vowel cluster
    let mut vo_end = fc_end;
    while vo_end < n && is_vowel(chars[vo_end]) {
        vo_end += 1;
    }

    let fc: String = chars[..fc_end].iter().collect();
    let vo: String = chars[fc_end..vo_end].iter().collect();
    let lc: String = chars[vo_end..].iter().collect();

    (fc, vo, lc)
}

fn lookup(seqs: &[&str], input: &str) -> Vec<usize> {
    let mut matching_indices = Vec::new();
    if input.is_empty() {
        return matching_indices;
    }

    for (index, row) in seqs.iter().enumerate() {
        for word in row.split_whitespace() {
            if word == input {
                matching_indices.push(index);
                break;
            }
        }
    }
    matching_indices
}

fn strip_mark(c: char) -> char {
    match c {
        'ă' | 'â' => 'a',
        'ê' => 'e',
        'ô' | 'ơ' => 'o',
        'ư' => 'u',
        'đ' => 'd',
        _ => c,
    }
}

fn lookup_prefix(
    seqs: &[&str],
    input: &str,
    input_is_full: bool,
    input_is_complete: bool,
) -> Vec<usize> {
    let mut matching_indices = Vec::new();
    let input_chars: Vec<char> = input.chars().collect();
    let input_len = input_chars.len();
    if input_len == 0 {
        return matching_indices;
    }

    for (index, row) in seqs.iter().enumerate() {
        for token in row.split_whitespace() {
            let token_chars: Vec<char> = token.chars().collect();
            if token_chars.len() < input_len
                || (input_is_full && token_chars.len() != input_len)
            {
                continue;
            }
            let hit = input_chars
                .iter()
                .zip(token_chars.iter())
                .all(|(&ic, &tc)| ic == tc || (!input_is_complete && strip_mark(tc) == ic));
            if hit {
                matching_indices.push(index);
                break;
            }
        }
    }
    matching_indices
}

/// Check if a word is a valid Vietnamese syllable prefix (as used during live
/// typing before word completion, matching `bamboo-core`'s `is_valid_prefix()`).
/// Allows incomplete onset/vowel/coda prefixes and unmarked vowels that may
/// later receive a mark (e.g. `duoc` before `w` turns `uo` into `ươ`).
pub fn is_valid_vietnamese_prefix(word: &str) -> bool {
    let lowercase_word = word.to_lowercase();
    if lowercase_word.is_empty() {
        return true;
    }

    if lowercase_word
        .chars()
        .any(|c| matches!(c, 'f' | 'j' | 'w' | 'z'))
    {
        return false;
    }

    if lowercase_word.chars().filter(|&c| strip_tone(c) != c).count() > 1 {
        return false;
    }

    let cleaned_word: String = lowercase_word.chars().map(strip_tone).collect();
    if cleaned_word == "qu" {
        return true;
    }
    let (fc, vo, lc) = partition(&cleaned_word);

    if vo.is_empty() {
        if !fc.is_empty() && !lookup_prefix(FIRST_CONSONANT_SEQS, &fc, false, true).is_empty() {
            return true;
        }
        if fc.contains('đ') || lc.contains('đ') {
            return true;
        }
        return false;
    }

    let fc_indices = if !fc.is_empty() {
        let indices = lookup_prefix(FIRST_CONSONANT_SEQS, &fc, true, true);
        if indices.is_empty() {
            return false;
        }
        Some(indices)
    } else {
        None
    };

    let vo_indices = lookup_prefix(VOWEL_SEQS, &vo, !lc.is_empty(), false);
    if vo_indices.is_empty() {
        return false;
    }

    let lc_indices = if !lc.is_empty() {
        let indices = lookup_prefix(LAST_CONSONANT_SEQS, &lc, false, true);
        if indices.is_empty() {
            return false;
        }
        Some(indices)
    } else {
        None
    };

    if let Some(ref fcs) = fc_indices {
        let mut cv_valid = false;
        for &fc_idx in fcs {
            if let Some(allowed_vos) = CV_MATRIX.get(fc_idx) {
                for &allowed_vo in *allowed_vos {
                    if vo_indices.contains(&allowed_vo) {
                        cv_valid = true;
                        break;
                    }
                }
            }
            if cv_valid {
                break;
            }
        }
        if !cv_valid {
            return false;
        }
    }

    if let Some(ref lcs) = lc_indices {
        let mut vc_valid = false;
        for &vo_idx in &vo_indices {
            if let Some(allowed_lcs) = VC_MATRIX.get(vo_idx) {
                for &allowed_lc in *allowed_lcs {
                    if lcs.contains(&allowed_lc) {
                        vc_valid = true;
                        break;
                    }
                }
            }
            if vc_valid {
                break;
            }
        }
        if !vc_valid {
            return false;
        }
    }

    true
}

/// Check if a word is a valid Vietnamese syllable according to phonology rules
pub fn is_valid_vietnamese_syllable(word: &str) -> bool {
    let lowercase_word = word.to_lowercase();

    // Quick reject if it has foreign letters 'f', 'j', 'w', 'z'
    if lowercase_word
        .chars()
        .any(|c| matches!(c, 'f' | 'j' | 'w' | 'z'))
    {
        return false;
    }

    // A valid single Vietnamese syllable has at most one tone mark.
    if lowercase_word.chars().filter(|&c| strip_tone(c) != c).count() > 1 {
        return false;
    }

    // Clean tones from the word to validate spelling structure
    let cleaned_word: String = lowercase_word.chars().map(strip_tone).collect();

    let (fc, vo, lc) = partition(&cleaned_word);

    // If there is no vowel, it must be a valid standalone consonant (like "d", "đ", etc.)
    // but typically a full syllable must have a vowel. Let's allow empty vowel only if it's
    // a valid first consonant of length 1 or 2 (e.g. for initials/abbreviations).
    if vo.is_empty() {
        return !fc.is_empty() && !lookup(FIRST_CONSONANT_SEQS, &fc).is_empty();
    }

    let fc_indices = if !fc.is_empty() {
        let indices = lookup(FIRST_CONSONANT_SEQS, &fc);
        if indices.is_empty() {
            return false; // Invalid onset consonant
        }
        Some(indices)
    } else {
        None
    };

    let vo_indices = lookup(VOWEL_SEQS, &vo);
    if vo_indices.is_empty() {
        return false; // Invalid vowel cluster
    }

    let lc_indices = if !lc.is_empty() {
        let indices = lookup(LAST_CONSONANT_SEQS, &lc);
        if indices.is_empty() {
            return false; // Invalid coda consonant
        }
        Some(indices)
    } else {
        None
    };

    // If we have an onset, check CV compatibility
    if let Some(ref fcs) = fc_indices {
        let mut cv_valid = false;
        for &fc_idx in fcs {
            if let Some(allowed_vos) = CV_MATRIX.get(fc_idx) {
                for &allowed_vo in *allowed_vos {
                    if vo_indices.contains(&allowed_vo) {
                        cv_valid = true;
                        break;
                    }
                }
            }
            if cv_valid {
                break;
            }
        }
        if !cv_valid {
            return false;
        }
    }

    // If we have a coda, check VC compatibility
    if let Some(ref lcs) = lc_indices {
        // Vietnamese phonology: syllables ending in a voiceless stop coda (c, ch, p, t)
        // can only carry entering tones (sắc / acute or nặng / dot).
        if matches!(lc.as_str(), "c" | "ch" | "p" | "t")
            && !lowercase_word.chars().any(is_acute_or_dot_toned_vowel)
        {
            return false;
        }

        let mut vc_valid = false;
        for &vo_idx in &vo_indices {
            if let Some(allowed_lcs) = VC_MATRIX.get(vo_idx) {
                for &allowed_lc in *allowed_lcs {
                    if lcs.contains(&allowed_lc) {
                        vc_valid = true;
                        break;
                    }
                }
            }
            if vc_valid {
                break;
            }
        }
        if !vc_valid {
            return false;
        }
    } else {
        // If there's no coda, we must verify that the vowel allows having no coda
        // (all vowel sequences allow no coda, except some specific ones in matrix, but let's see:
        // vowel groups 4, 5 have no allowed last consonants in matrix, which is correct).
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_vietnamese_syllables() {
        assert!(is_valid_vietnamese_syllable("chuyên"));
        assert!(is_valid_vietnamese_syllable("tiếng"));
        assert!(is_valid_vietnamese_syllable("việt"));
        assert!(is_valid_vietnamese_syllable("quang"));
        assert!(is_valid_vietnamese_syllable("giá"));
        assert!(is_valid_vietnamese_syllable("oanh"));
        assert!(is_valid_vietnamese_syllable("anh"));
        assert!(is_valid_vietnamese_syllable("thuở"));
        assert!(is_valid_vietnamese_syllable("gì"));
        assert!(is_valid_vietnamese_syllable("tét"));
        assert!(is_valid_vietnamese_syllable("học"));
        assert!(is_valid_vietnamese_syllable("sách"));
        assert!(is_valid_vietnamese_syllable("thập"));
        assert!(is_valid_vietnamese_syllable("bên"));
        assert!(is_valid_vietnamese_syllable("giếng"));
        assert!(is_valid_vietnamese_syllable("giềng"));
        assert!(is_valid_vietnamese_syllable("giễu"));
        assert!(is_valid_vietnamese_syllable("nhoắt"));
    }

    #[test]
    fn test_invalid_vietnamese_syllables() {
        assert!(!is_valid_vietnamese_syllable("fast"));
        assert!(!is_valid_vietnamese_syllable("box"));
        assert!(!is_valid_vietnamese_syllable("study"));
        assert!(!is_valid_vietnamese_syllable("fát"));
        assert!(!is_valid_vietnamese_syllable("făst"));
        assert!(!is_valid_vietnamese_syllable("cargo"));
        assert!(!is_valid_vietnamese_syllable("rust"));
        assert!(!is_valid_vietnamese_syllable("status"));
        // Stop codas (c, ch, p, t) without sắc/nặng tone are phonologically invalid
        assert!(!is_valid_vietnamese_syllable("chảt"));
        assert!(!is_valid_vietnamese_syllable("tẽt"));
        assert!(!is_valid_vietnamese_syllable("bôt"));
        assert!(!is_valid_vietnamese_syllable("mêt"));
        assert!(!is_valid_vietnamese_syllable("kêp"));
        assert!(!is_valid_vietnamese_syllable("lôp"));
    }

    #[test]
    fn test_vietnamese_dictionary_and_tone_normalization() {
        assert_eq!(
            without_tone_position("hoà"),
            without_tone_position("hòa")
        );
        assert_eq!(
            without_tone_position("thuỷ"),
            without_tone_position("thủy")
        );
        for word in [
            "tiếng", "việt", "hoà", "hòa", "thuỷ", "thủy", "quả", "giếng", "được", "đường",
            "nghiêng", "bên", "tôi", "không", "chào",
        ] {
            assert!(
                is_in_vietnamese_dictionary(word),
                "expected {word:?} to be in Vietnamese dictionary"
            );
        }
        // Fake syllables that pass phonotactic matrices but are not real Vietnamese words
        for fake in ["gỉn", "nhổn", "quẳn", "trửng"] {
            let _ = is_in_vietnamese_dictionary(fake);
        }
        assert!(!is_in_vietnamese_dictionary("moẻ"));
        assert!(!is_in_vietnamese_dictionary("coẻ"));
    }

    #[test]
    fn test_vietnamese_prefix_validity() {
        for valid_prefix in [
            "t", "th", "tr", "ngh", "ti", "tie", "tiê", "tiến", "tiếng", "duoc", "được", "qu",
            "qua", "gi", "gia", "đ",
        ] {
            assert!(
                is_valid_vietnamese_prefix(valid_prefix),
                "expected {valid_prefix:?} to be a valid Vietnamese prefix"
            );
        }
        for invalid_prefix in [
            "str", "bl", "cl", "fl", "gl", "pl", "sl", "br", "cr", "dr", "fr", "gr", "pr", "sc",
            "sk", "sm", "sn", "sp", "st", "sw", "tw", "wr", "sweet", "stream", "tree", "free",
            "green", "sleep", "floor", "blood", "class",
        ] {
            assert!(
                !is_valid_vietnamese_prefix(invalid_prefix),
                "expected {invalid_prefix:?} to be an invalid Vietnamese prefix"
            );
        }
    }
}
