// SPDX-License-Identifier: MIT
//! Quantized 3-Word (Trigram) Language Context & Idiom Disambiguator.
//!
//! Extends Dboard's 1-word bigram scorer (`bigram.rs`) to a 3-word sliding window
//! `(prev2, prev1, current)` backed by `CommittedRingBuffer`, resolving idioms
//! and neutral-homograph chains (`"it has been"`, `"in its own"`, `"làm cái gì"`,
//! `"đi ăn tô phở"`) in O(1) with zero allocations on the hot path beyond
//! single-word lowercasing.

use crate::english::{should_restore_word_decision, EnglishDict, LanguageContext};
use crate::ring_buffer::CommittedWordEntry;

/// High-frequency 2-syllable Vietnamese compounds where word #2 collides with
/// English (`"bánh tét"`, `"tô phở"`, `"hồ sơ"`, `"bác sĩ"`, `"cảm ơn"`...).
fn is_vn_compound_pair(p1: &str, comp: &str) -> bool {
    matches!(
        (p1, comp),
        ("bánh", "tét")
            | ("ô", "tô")
            | ("tất", "cả")
            | ("hai", "bên")
            | ("cái", "gì")
            | ("làm", "gì")
            | ("họ", "tên")
            | ("bộ", "môn")
            | ("cái", "đó")
            | ("ở", "đó")
            | ("một", "ít")
            | ("nước", "sơn")
            | ("mô", "tả")
            | ("sấm", "sét")
            | ("miếng", "lót")
            | ("dây", "cáp")
            | ("chim", "cút")
            | ("ca", "hát")
            | ("cái", "bát")
            | ("bãi", "cát")
            | ("bỏng", "rát")
            | ("rõ", "nét")
            | ("quả", "bơ")
            | ("ước", "mơ")
            | ("hồ", "sơ")
            | ("bác", "sĩ")
            | ("cảm", "ơn")
            | ("cá", "trê")
            | ("cây", "tre")
            | ("mất", "mát")
            | ("tốt", "nhất")
            | ("trà", "đá")
            | ("cà", "phê")
            | ("phở", "bò")
            | ("bún", "chả")
            | ("bánh", "mì")
            | ("bánh", "bao")
            | ("bánh", "cuốn")
            | ("bánh", "xèo")
            | ("bánh", "chưng")
    )
}

/// Unified bilingual decision combining 3-word Trigram LM, Vietnamese compound
/// bigrams, and unigram/morphological English scoring.
pub(crate) fn decide_should_restore(
    dict: &EnglishDict,
    composed: &str,
    raw: &str,
    prior_context: LanguageContext,
    last_three: (
        Option<&CommittedWordEntry>,
        Option<&CommittedWordEntry>,
        Option<&CommittedWordEntry>,
    ),
    boundary_context: Option<(Option<char>, char)>,
) -> bool {
    if composed == raw {
        return false;
    }
    let (prev3, prev2, prev1) = last_three;
    // Unambiguous English auxiliary pair across a Vietnamese-to-English code switch
    // (`"anh ta has been"` where `"has"` was temporarily composed as `"há"`):
    // restore `"been"` so retroactive 2-word correction rewrites `"há been" -> "has been"`.
    if raw.eq_ignore_ascii_case("been")
        && prev1.is_some_and(|e| {
            !e.user_override_vn
                && (e.raw_keys.eq_ignore_ascii_case("has")
                    || e.raw_keys.eq_ignore_ascii_case("had"))
        })
    {
        return true;
    }
    if let Some(decision) = evaluate_trigram(
        prev2.map(|e| e.visible_word.as_str()),
        prev1.map(|e| e.visible_word.as_str()),
        composed,
        raw,
        prev2.map_or(LanguageContext::None, |e| e.committed_context),
        prev1.map_or(prior_context, |e| e.committed_context),
    ) {
        return decision;
    }
    let prev1_is_all_caps = prev1.is_some_and(|e| {
        let mut has_alpha = false;
        e.visible_word.chars().all(|c| {
            if c.is_alphabetic() {
                has_alpha = true;
                c.is_uppercase()
            } else {
                true
            }
        }) && has_alpha
    });
    let is_all_caps_acronym =
        !prev1_is_all_caps && raw.len() >= 2 && raw.chars().all(|c| c.is_ascii_uppercase());
    let is_mixed_caps = raw.chars().any(|c| c.is_ascii_lowercase())
        && raw.chars().skip(1).any(|c| c.is_ascii_uppercase());
    let is_technical_boundary = boundary_context.is_some_and(|(lead, trail)| {
        matches!(lead, Some('.' | '/' | '_' | '@' | ':')) || matches!(trail, '/' | '_' | '@' | ':')
    });
    if !is_all_caps_acronym && !is_mixed_caps {
        if let Some(p1) = prev1.map(|e| e.visible_word.to_lowercase()) {
            if is_vn_compound_pair(&p1, &composed.to_lowercase()) {
                return false;
            }
        }
        if !is_technical_boundary {
            if let (Some(p2), Some(p1)) = (prev2, prev1) {
                let ngram_score = match prev3 {
                    Some(p3) => crate::ngram_lm::score_context(
                        &[
                            p3.visible_word.as_str(),
                            p2.visible_word.as_str(),
                            p1.visible_word.as_str(),
                        ],
                        composed,
                        raw,
                    ),
                    None => crate::ngram_lm::score_context(
                        &[p2.visible_word.as_str(), p1.visible_word.as_str()],
                        composed,
                        raw,
                    ),
                };
                if ngram_score >= 250 {
                    return true;
                }
                if ngram_score <= -250 {
                    return false;
                }
            }
            let stat_score = crate::stat_lm::score_bilingual(
                prev2.map(|e| e.visible_word.as_str()),
                prev1.map(|e| e.visible_word.as_str()),
                composed,
                raw,
            );
            if stat_score >= 250 {
                return true;
            }
            if stat_score <= -250 {
                return false;
            }
        }
    }
    should_restore_word_decision(
        dict,
        composed,
        raw,
        prior_context,
        prev1.map(|e| e.visible_word.as_str()),
        boundary_context,
    )
}

/// Evaluate a 3-word window `(prev2, prev1, current_raw)`.
/// - `Some(true)`  => strong trigram evidence for English (auto-restore).
/// - `Some(false)` => strong trigram evidence for Vietnamese (keep composed).
/// - `None`        => defer to bigram + unigram scorer.
pub(crate) fn evaluate_trigram(
    prev2_visible: Option<&str>,
    prev1_visible: Option<&str>,
    current_composed: &str,
    current_raw: &str,
    prev2_ctx: LanguageContext,
    prev1_ctx: LanguageContext,
) -> Option<bool> {
    let p2 = prev2_visible?.to_lowercase();
    let p1 = prev1_visible?.to_lowercase();
    let raw = current_raw.to_ascii_lowercase();
    let comp = current_composed.to_lowercase();

    // 1. High-confidence English 3-word idioms where word #3 collides with Telex.
    let is_english_trigram = matches!(
        (p2.as_str(), p1.as_str(), raw.as_str()),
        ("a", "lot", "of")
            | ("one", "of", "the")
            | ("out", "of", "the")
            | ("it", "has", "been")
            | ("it", "had", "been")
            | ("has", "not", "been")
            | ("had", "not", "been")
            | ("have", "not", "been")
            | ("would", "have", "been")
            | ("could", "have", "been")
            | ("should", "have", "been")
            | ("might", "have", "been")
            | ("must", "have", "been")
            | ("have", "you", "seen")
            | ("in", "the", "past")
            | ("at", "the", "most")
            | ("at", "the", "last")
            | ("for", "the", "rest")
            | ("in", "the", "case")
            | ("in", "this", "case")
            | ("on", "the", "list")
            | ("as", "a", "user")
            | ("how", "to", "use")
            | ("how", "to", "fix")
            | ("how", "to", "run")
            | ("how", "to", "test")
            | ("not", "too", "bad")
            | ("way", "too", "much")
            | ("far", "too", "many")
            | ("my", "own", "car")
            | ("on", "its", "own")
            | ("in", "its", "own")
            | ("by", "its", "own")
            | ("at", "its", "core")
            | ("at", "its", "best")
    );
    if is_english_trigram {
        return Some(true);
    }

    // 2. High-confidence Vietnamese 3-word idioms where word #3 collides with English.
    let is_vietnamese_trigram = matches!(
        (p2.as_str(), p1.as_str(), raw.as_str(), comp.as_str()),
        ("làm", "cái", "gif", "gì")
            | ("biết", "cái", "gif", "gì")
            | ("mua", "cái", "gif", "gì")
            | ("xem", "cái", "gif", "gì")
            | ("không", "có", "gif", "gì")
            | ("chẳng", "có", "gif", "gì")
            | ("có", "chuyện", "gif", "gì")
            | ("cái", "này", "ddos", "đó")
            | ("chỗ", "này", "ddos", "đó")
            | ("đi", "ăn", "too", "tô")
            | ("ăn", "một", "too", "tô")
            | ("ăn", "bánh", "test", "tét")
            | ("gói", "bánh", "test", "tét")
            | ("một", "con", "car", "cả")
            | ("nhà", "con", "car", "cả")
            | ("còn", "một", "its", "ít")
            | ("chỉ", "một", "its", "ít")
            | ("ở", "hai", "been", "bên")
            | ("từ", "hai", "been", "bên")
    );
    if is_vietnamese_trigram {
        return Some(false);
    }

    // 2b. Bilingual coordination of two English terms via Vietnamese conjunction
    //     (`"docker và rust"`, `"git hoặc svn"`).
    if prev2_ctx == LanguageContext::English
        && matches!(p1.as_str(), "và" | "hoặc")
        && crate::engine::english_dict().is_english_dominant_collision(&raw)
    {
        return Some(true);
    }

    // 3. Quantized 2-word momentum tiebreaker when `prev1` was neutral (`None`)
    //    but `prev2` had strong language momentum (`prev2_ctx != None`).
    if prev1_ctx == LanguageContext::None && prev2_ctx != LanguageContext::None {
        let score: i16 = match prev2_ctx {
            LanguageContext::English => 20,
            LanguageContext::Vietnamese => -20,
            LanguageContext::None => 0,
        };
        if score >= 20 && matches!(raw.as_str(), "been" | "too" | "its" | "car") {
            return Some(true);
        }
        if score <= -20 && matches!(raw.as_str(), "ddos" | "gif" | "been" | "car" | "its") {
            return Some(false);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::evaluate_trigram;
    use crate::english::LanguageContext;

    #[test]
    fn resolves_english_and_vietnamese_trigrams() {
        assert_eq!(
            evaluate_trigram(
                Some("it"),
                Some("has"),
                "bên",
                "been",
                LanguageContext::English,
                LanguageContext::English,
            ),
            Some(true)
        );
        assert_eq!(
            evaluate_trigram(
                Some("on"),
                Some("its"),
                "ơn",
                "own",
                LanguageContext::English,
                LanguageContext::English,
            ),
            Some(true)
        );
        assert_eq!(
            evaluate_trigram(
                Some("không"),
                Some("có"),
                "gì",
                "gif",
                LanguageContext::Vietnamese,
                LanguageContext::Vietnamese,
            ),
            Some(false)
        );
        assert_eq!(
            evaluate_trigram(
                Some("đi"),
                Some("ăn"),
                "tô",
                "too",
                LanguageContext::Vietnamese,
                LanguageContext::Vietnamese,
            ),
            Some(false)
        );
    }
}
