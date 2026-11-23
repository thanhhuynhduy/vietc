// SPDX-License-Identifier: MIT
//! On-device Personal Dictionary & Smart Auto-Capitalization state.
//!
//! Privacy & safety invariants:
//! - Only pure alphabetic tokens (length 2..=32, no digits/symbols) are ever
//!   learned, preventing accidental persistence of passwords, tokens, or IDs.
//! - Bounded to `MAX_USER_DICT_ENTRIES` so memory and disk stay strictly small.
//! - `AutoCapState` only capitalizes the first lowercase ASCII letter after a
//!   sentence-ending punctuation mark (`.`, `!`, `?`) followed by Space/Enter,
//!   never inside URLs (`google.com`) or version numbers (`v1.0`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const MAX_USER_DICT_ENTRIES: usize = 2000;
pub const MAX_PERSONAL_BIGRAMS: usize = 1000;
const PERSONAL_WORD_PROMOTE_COUNT: u16 = 2;
const PERSONAL_BIGRAM_PROMOTE_COUNT: u16 = 2;

#[derive(Debug, Clone)]
pub(crate) enum AutoRestoreUndo {
    SingleWord {
        restored_ascii: String,
        original_vn: String,
        raw_keys: String,
        boundary_char: char,
        is_autocorrect: bool,
    },
    RetroactivePair {
        rewritten_pair: String,
        original_pair: String,
        original_w1_entry: crate::ring_buffer::CommittedWordEntry,
        w2_raw_keys: String,
        boundary_char: char,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDict {
    keep_vn: BTreeSet<String>,
    keep_raw: BTreeSet<String>,
    learned_words: BTreeMap<String, u16>,
    /// FIFO queue of single-observation (`count == 1`) candidates so one-off typos
    /// are evicted in O(1) instead of permanently saturating `learned_words`.
    pending_once: VecDeque<String>,
    /// Personal 2-word collocations `(w1, w2) -> count` (Vietnamese or bilingual
    /// alphabetic pairs only — never digits, symbols, or passwords).
    personal_bigrams: BTreeMap<(String, String), u16>,
    pending_bigram_once: VecDeque<(String, String)>,
    dirty: bool,
}

fn is_learnable_token(raw: &str) -> bool {
    let len = raw.chars().count();
    (2..=32).contains(&len) && raw.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_learnable_bigram_word(word: &str) -> bool {
    let len = word.chars().count();
    (2..=24).contains(&len) && word.chars().all(char::is_alphabetic)
}

impl UserDict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_from_str(content: &str) -> Self {
        let mut dict = Self::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(tok) = trimmed.strip_prefix("keep_vn:") {
                let lower = tok.trim().to_ascii_lowercase();
                if is_learnable_token(&lower) && dict.len() < MAX_USER_DICT_ENTRIES {
                    dict.keep_vn.insert(lower);
                }
            } else if let Some(tok) = trimmed.strip_prefix("keep_raw:") {
                let lower = tok.trim().to_ascii_lowercase();
                if is_learnable_token(&lower) && dict.len() < MAX_USER_DICT_ENTRIES {
                    dict.keep_raw.insert(lower);
                }
            } else if let Some(rest) = trimmed.strip_prefix("learned:") {
                if let Some((tok, cnt_str)) = rest.split_once(':') {
                    let lower = tok.trim().to_ascii_lowercase();
                    let count = cnt_str.trim().parse::<u16>().unwrap_or(0);
                    if is_learnable_token(&lower)
                        && count >= PERSONAL_WORD_PROMOTE_COUNT
                        && dict.len() < MAX_USER_DICT_ENTRIES
                    {
                        dict.learned_words.insert(lower, count);
                    }
                }
            } else if let Some(rest) = trimmed.strip_prefix("bigram:") {
                if let Some((pair, cnt_str)) = rest.split_once(':') {
                    if let Some((w1, w2)) = pair.split_once('|') {
                        let l1 = w1.trim().to_lowercase();
                        let l2 = w2.trim().to_lowercase();
                        let count = cnt_str.trim().parse::<u16>().unwrap_or(0);
                        if is_learnable_bigram_word(&l1)
                            && is_learnable_bigram_word(&l2)
                            && l1 != l2
                            && count >= PERSONAL_BIGRAM_PROMOTE_COUNT
                            && dict.promoted_bigram_count() < MAX_PERSONAL_BIGRAMS
                        {
                            dict.personal_bigrams.insert((l1, l2), count);
                        }
                    }
                }
            }
        }
        dict.dirty = false;
        dict
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# Dboard Personal Dictionary (auto-managed, mode 0600)\n");
        for tok in &self.keep_vn {
            out.push_str("keep_vn:");
            out.push_str(tok);
            out.push('\n');
        }
        for tok in &self.keep_raw {
            out.push_str("keep_raw:");
            out.push_str(tok);
            out.push('\n');
        }
        for (tok, &count) in &self.learned_words {
            if count >= PERSONAL_WORD_PROMOTE_COUNT {
                out.push_str("learned:");
                out.push_str(tok);
                out.push(':');
                out.push_str(&count.to_string());
                out.push('\n');
            }
        }
        for ((w1, w2), &count) in &self.personal_bigrams {
            if count >= PERSONAL_BIGRAM_PROMOTE_COUNT {
                out.push_str("bigram:");
                out.push_str(w1);
                out.push('|');
                out.push_str(w2);
                out.push(':');
                out.push_str(&count.to_string());
                out.push('\n');
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.keep_vn.len()
            + self.keep_raw.len()
            + self
                .learned_words
                .values()
                .filter(|&&c| c >= PERSONAL_WORD_PROMOTE_COUNT)
                .count()
    }

    pub fn promoted_bigram_count(&self) -> usize {
        self.personal_bigrams
            .values()
            .filter(|&&c| c >= PERSONAL_BIGRAM_PROMOTE_COUNT)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0 && self.promoted_bigram_count() == 0
    }

    /// Observe a committed alphabetic token; once seen `>= 2` times it becomes
    /// a protected personal word (e.g. project/brand/person name).
    pub fn record_word_usage(&mut self, word: &str) {
        let lower = word.to_ascii_lowercase();
        if !is_learnable_token(&lower) {
            return;
        }
        if !self.learned_words.contains_key(&lower) {
            if self.len() >= MAX_USER_DICT_ENTRIES {
                return;
            }
            while self.learned_words.len() >= MAX_USER_DICT_ENTRIES * 2 {
                let Some(oldest) = self.pending_once.pop_front() else {
                    return;
                };
                if self.learned_words.get(&oldest).copied() == Some(1) {
                    self.learned_words.remove(&oldest);
                }
            }
            self.pending_once.push_back(lower.clone());
        }
        let entry = self.learned_words.entry(lower).or_insert(0);
        let prev = *entry;
        *entry = entry.saturating_add(1);
        if prev < PERSONAL_WORD_PROMOTE_COUNT && *entry >= PERSONAL_WORD_PROMOTE_COUNT {
            self.dirty = true;
        }
    }

    /// Observe a committed 2-word alphabetic pair (Vietnamese or bilingual).
    /// Once seen `>= 2` times it is promoted for personal next-word prediction
    /// and adaptive autocorrect ranking.
    pub fn record_bigram_usage(&mut self, prev_word: &str, next_word: &str) {
        let l1 = prev_word.to_lowercase();
        let l2 = next_word.to_lowercase();
        if l1 == l2 || !is_learnable_bigram_word(&l1) || !is_learnable_bigram_word(&l2) {
            return;
        }
        let key = (l1, l2);
        if !self.personal_bigrams.contains_key(&key) {
            if self.promoted_bigram_count() >= MAX_PERSONAL_BIGRAMS {
                return;
            }
            while self.personal_bigrams.len() >= MAX_PERSONAL_BIGRAMS * 2 {
                let Some(oldest) = self.pending_bigram_once.pop_front() else {
                    return;
                };
                if self.personal_bigrams.get(&oldest).copied() == Some(1) {
                    self.personal_bigrams.remove(&oldest);
                }
            }
            self.pending_bigram_once.push_back(key.clone());
        }
        let entry = self.personal_bigrams.entry(key).or_insert(0);
        let prev_cnt = *entry;
        *entry = entry.saturating_add(1);
        if (prev_cnt < PERSONAL_BIGRAM_PROMOTE_COUNT && *entry >= PERSONAL_BIGRAM_PROMOTE_COUNT)
            || (*entry > PERSONAL_BIGRAM_PROMOTE_COUNT && (*entry).is_multiple_of(4))
        {
            self.dirty = true;
        }
    }

    /// Adaptive ranking bonus for SymSpell / contextual autocorrect when
    /// `(prev_word, candidate)` is a promoted personal bigram.
    pub fn personal_bigram_bonus(&self, prev_word: &str, candidate: &str) -> i32 {
        let key = (prev_word.to_lowercase(), candidate.to_lowercase());
        match self.personal_bigrams.get(&key).copied() {
            Some(cnt) if cnt >= PERSONAL_BIGRAM_PROMOTE_COUNT => 220 + i32::from(cnt.min(20)) * 2,
            _ => 0,
        }
    }

    /// Return up to `limit` promoted personal next-word predictions following `prev_word`,
    /// ordered by observed usage count descending.
    pub fn predict_next_words(&self, prev_word: &str, limit: usize) -> Vec<String> {
        if limit == 0 || self.personal_bigrams.is_empty() {
            return Vec::new();
        }
        let p1 = prev_word.to_lowercase();
        let mut matches: Vec<(u16, &str)> = self
            .personal_bigrams
            .range((p1.clone(), String::new())..)
            .take_while(|((w1, _), _)| w1 == &p1)
            .filter_map(|((_, w2), &cnt)| {
                (cnt >= PERSONAL_BIGRAM_PROMOTE_COUNT).then_some((cnt, w2.as_str()))
            })
            .collect();
        matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        matches
            .into_iter()
            .take(limit)
            .map(|(_, w)| w.to_string())
            .collect()
    }

    /// Learn an untouched out-of-vocabulary ASCII word (e.g. a username, handle,
    /// or project codename) after repeated deliberate commits.
    pub(crate) fn maybe_learn_committed(
        &mut self,
        final_word: &str,
        raw: &str,
        english: &crate::english::EnglishDict,
    ) {
        if final_word != raw || raw.len() < 3 || !raw.is_ascii() {
            return;
        }
        let lower = raw.to_ascii_lowercase();
        if english.is_english_word(&lower) || crate::spelling::is_in_vietnamese_dictionary(&lower) {
            return;
        }
        self.record_word_usage(&lower);
    }

    pub fn is_known_personal_word(&self, raw: &str) -> bool {
        self.learned_words
            .get(&raw.to_ascii_lowercase())
            .is_some_and(|&c| c >= PERSONAL_WORD_PROMOTE_COUNT)
    }

    pub fn record_keep_vn(&mut self, raw: &str) {
        let lower = raw.to_ascii_lowercase();
        if !is_learnable_token(&lower) || self.len() >= MAX_USER_DICT_ENTRIES {
            return;
        }
        self.keep_raw.remove(&lower);
        if self.keep_vn.insert(lower) {
            self.dirty = true;
        }
    }

    pub fn record_keep_raw(&mut self, raw: &str) {
        let lower = raw.to_ascii_lowercase();
        if !is_learnable_token(&lower) || self.len() >= MAX_USER_DICT_ENTRIES {
            return;
        }
        self.keep_vn.remove(&lower);
        if self.keep_raw.insert(lower) {
            self.dirty = true;
        }
    }

    pub fn is_keep_vn(&self, raw: &str) -> bool {
        self.keep_vn.contains(&raw.to_ascii_lowercase())
    }

    pub fn is_keep_raw(&self, raw: &str) -> bool {
        self.keep_raw.contains(&raw.to_ascii_lowercase())
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }
}

/// Finite state machine for Gboard-style Smart Auto-Capitalization.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AutoCapState {
    pub(crate) enabled: bool,
    pending_sentence_end: bool,
    expect_cap: bool,
}

fn is_auto_cap_abbreviation(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "mr" | "mrs"
            | "ms"
            | "dr"
            | "prof"
            | "st"
            | "vs"
            | "etc"
            | "eg"
            | "ie"
            | "tp"
            | "ts"
            | "ths"
            | "pgs"
            | "gs"
            | "ks"
            | "bs"
            | "v"
            | "vv"
    )
}

impl AutoCapState {
    pub(crate) fn reset(&mut self) {
        self.pending_sentence_end = false;
        self.expect_cap = false;
    }

    pub(crate) fn arm(&mut self) {
        if self.enabled {
            self.pending_sentence_end = false;
            self.expect_cap = true;
        }
    }

    pub(crate) fn on_flush_char(&mut self, ch: char, prev_word: Option<&str>) {
        if !self.enabled {
            self.reset();
            return;
        }
        match ch {
            '.' => {
                if prev_word.is_some_and(is_auto_cap_abbreviation) {
                    self.reset();
                } else {
                    self.pending_sentence_end = true;
                    self.expect_cap = false;
                }
            }
            '!' | '?' => {
                self.pending_sentence_end = true;
                self.expect_cap = false;
            }
            '"' | '\'' | ')' | ']' | '»' | '”' | '’' => {
                if !self.pending_sentence_end {
                    self.reset();
                }
            }
            ' ' | '\t' => {
                if self.pending_sentence_end {
                    self.expect_cap = true;
                    self.pending_sentence_end = false;
                }
            }
            '\n' | '\r' => {
                self.pending_sentence_end = false;
                self.expect_cap = true;
            }
            _ => {
                self.reset();
            }
        }
    }

    /// If auto-capitalization is armed and `ch` is the first lowercase ASCII
    /// letter of a new word, consume the armed state and return `Some(UPPER)`.
    pub(crate) fn maybe_capitalize(&mut self, ch: char, is_word_start: bool) -> Option<char> {
        self.pending_sentence_end = false;
        if !self.enabled || !is_word_start {
            self.expect_cap = false;
            return None;
        }
        let armed = std::mem::replace(&mut self.expect_cap, false);
        if armed && ch.is_ascii_lowercase() {
            Some(ch.to_ascii_uppercase())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoCapState, UserDict};

    #[test]
    fn user_dict_roundtrips_and_rejects_non_alphabetic_tokens() {
        let mut dict = UserDict::new();
        dict.record_keep_vn("lets");
        dict.record_keep_raw("teh");
        dict.record_keep_raw("p@ss123"); // Must be ignored
        dict.record_keep_vn("a"); // Too short
        assert!(dict.take_dirty());
        assert!(!dict.take_dirty());
        assert_eq!(dict.len(), 2);
        assert!(dict.is_keep_vn("LETS"));
        assert!(dict.is_keep_raw("teh"));

        let restored = UserDict::load_from_str(&dict.serialize());
        assert_eq!(restored, dict);
        assert!(!restored.clone().take_dirty());
    }

    #[test]
    fn auto_cap_arms_only_after_sentence_punctuation_plus_space_or_newline() {
        let mut ac = AutoCapState {
            enabled: true,
            ..Default::default()
        };
        // `google.com` -> `.` followed directly by `c` (no space) must NOT capitalize
        ac.on_flush_char('.', Some("google"));
        assert_eq!(ac.maybe_capitalize('c', true), None);

        // `chào. hôm` -> `.` followed by `' '` DOES capitalize `h` -> `H`
        ac.on_flush_char('.', Some("chào"));
        ac.on_flush_char(' ', None);
        assert_eq!(ac.maybe_capitalize('h', true), Some('H'));
        // Subsequent letter is NOT capitalized
        assert_eq!(ac.maybe_capitalize('o', false), None);

        // Closing quotes & brackets preserve pending sentence end (`"Chào." Hôm`, `“Chào.” Hôm`, `(Xong!) Tiếp`)
        for closer in ['"', '\'', ')', ']', '»', '”', '’'] {
            ac.on_flush_char('.', Some("chào"));
            ac.on_flush_char(closer, None);
            ac.on_flush_char(' ', None);
            assert_eq!(
                ac.maybe_capitalize('h', true),
                Some('H'),
                "closer {closer:?} must preserve sentence-end auto-cap"
            );
        }

        // Abbreviations before `.` must NOT arm auto-cap (`TP. hcm`, `Mr. nam`, `TS. nguyễn`, `etc. tiếp`)
        for abbr in [
            "TP", "Mr", "Mrs", "Dr", "TS", "ThS", "PGS", "GS", "etc", "vs",
        ] {
            ac.on_flush_char('.', Some(abbr));
            ac.on_flush_char(' ', None);
            assert_eq!(
                ac.maybe_capitalize('n', true),
                None,
                "abbreviation {abbr:?}. must NOT arm auto-cap"
            );
        }

        // Newline arms capitalization
        ac.on_flush_char('\n', None);
        assert_eq!(ac.maybe_capitalize('t', true), Some('T'));
    }

    #[test]
    fn one_off_tokens_evict_fifo_without_saturating_personal_word_learning() {
        let mut dict = UserDict::new();
        // Record > 2 * MAX_USER_DICT_ENTRIES unique one-off typos
        for i in 0..(super::MAX_USER_DICT_ENTRIES * 2 + 50) {
            let tok = format!(
                "tok{}{}{}{}",
                (b'a' + (i % 26) as u8) as char,
                (b'a' + ((i / 26) % 26) as u8) as char,
                (b'a' + ((i / 676) % 26) as u8) as char,
                (b'a' + ((i / 17576) % 26) as u8) as char,
            );
            dict.record_word_usage(&tok);
        }
        // A genuinely repeated personal word must still be learned after eviction
        dict.record_word_usage("dboard");
        dict.record_word_usage("dboard");
        assert!(dict.is_known_personal_word("dboard"));
    }
}
