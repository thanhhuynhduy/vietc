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
const PERSONAL_WORD_PROMOTE_COUNT: u16 = 2;

#[derive(Debug, Clone)]
pub(crate) struct AutoRestoreUndo {
    pub(crate) restored_ascii: String,
    pub(crate) original_vn: String,
    pub(crate) raw_keys: String,
    pub(crate) boundary_char: char,
    pub(crate) is_autocorrect: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDict {
    keep_vn: BTreeSet<String>,
    keep_raw: BTreeSet<String>,
    learned_words: BTreeMap<String, u16>,
    /// FIFO queue of single-observation (`count == 1`) candidates so one-off typos
    /// are evicted in O(1) instead of permanently saturating `learned_words`.
    pending_once: VecDeque<String>,
    dirty: bool,
}

fn is_learnable_token(raw: &str) -> bool {
    let len = raw.chars().count();
    (2..=32).contains(&len) && raw.chars().all(|c| c.is_ascii_alphabetic())
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

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

    pub(crate) fn on_flush_char(&mut self, ch: char) {
        if !self.enabled {
            self.reset();
            return;
        }
        match ch {
            '.' | '!' | '?' => {
                self.pending_sentence_end = true;
                self.expect_cap = false;
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
        ac.on_flush_char('.');
        assert_eq!(ac.maybe_capitalize('c', true), None);

        // `chào. hôm` -> `.` followed by `' '` DOES capitalize `h` -> `H`
        ac.on_flush_char('.');
        ac.on_flush_char(' ');
        assert_eq!(ac.maybe_capitalize('h', true), Some('H'));
        // Subsequent letter is NOT capitalized
        assert_eq!(ac.maybe_capitalize('o', false), None);

        // Newline arms capitalization
        ac.on_flush_char('\n');
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
