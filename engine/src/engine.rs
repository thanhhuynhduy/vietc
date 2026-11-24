// SPDX-License-Identifier: MIT
use crate::bamboo::BambooEngine;
use crate::bigram::evaluate_retroactive_first_word;
use crate::engine_helpers::{
    adapt_macro_casing, build_composition_state, build_surrounding_prev_entry,
    commit_rehydrated_ascii_word, commit_retroactive_pair, is_flush_char,
    is_seedable_surrounding_word, is_sentence_boundary, match_casing, resolve_coda_edit_canon,
    resolve_retroactive_undo_w2,
};
use crate::english::{EnglishDict, LanguageContext};
use crate::filename_restore::{try_dot_filename_restore, DotStemEntry};
use crate::input_method::InputMethod;
use crate::ring_buffer::{visible_to_canonical_raw, CommittedRingBuffer, CommittedWordEntry};
use crate::state::{
    BoundaryState, CompositionState, Replacement, ReplacementReason, TransformIdentity,
    TransitionKind,
};
use crate::user_dict::{AutoCapState, AutoRestoreUndo, UserDict};
use std::collections::HashMap;
use std::sync::OnceLock;

#[path = "engine_backspace.rs"]
mod engine_backspace;
#[path = "engine_candidates.rs"]
mod engine_candidates;

pub(crate) fn english_dict() -> &'static EnglishDict {
    static DICT: OnceLock<EnglishDict> = OnceLock::new();
    DICT.get_or_init(EnglishDict::new)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum EngineEvent {
    Replace(Replacement),
    Insert(String),
    Flush(String),
    AutoRestore(String),
    UndoTones { backspaces: usize, restored: String },
    Paste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelexBackend {
    Recomposition,
    BambooRollback,
}

pub struct Engine {
    bamboo: BambooEngine,
    method: InputMethod,
    telex_backend: TelexBackend,
    macros: HashMap<String, String>,
    raw_buffer: String,
    pasted_prefix: String,
    previous_screen_len: Option<usize>,
    recovered_screen_len: Option<usize>,
    paste_mode: bool,
    auto_restore: bool,
    autocorrect: bool,
    keyboard_layout: crate::autocorrect::AutocorrectLayout,
    auto_cap: AutoCapState,
    user_dict: UserDict,
    learning_enabled: bool,
    prev_context: LanguageContext,
    ring_buffer: CommittedRingBuffer,
    last_dot_stem: Option<DotStemEntry>,
    rehydrated_word: bool,
    rehydrated_edited: bool,
    rehydrated_ascii_only: bool,
    pending_coda_edit_canon: Option<String>,
    pending_restore_undo: Option<AutoRestoreUndo>,
    override_keep_vietnamese: bool,
    active_transform: Option<TransformIdentity>,
    generation: u64,
    visible_revision: u64,
    boundary: BoundaryState,
    last_transition: Option<TransitionKind>,
    last_boundary_char: Option<char>,
    pub(super) saturated_until_flush: bool,
}

impl Engine {
    pub fn new(method: InputMethod) -> Self {
        let telex_backend = if cfg!(feature = "bamboo-telex-rollback") {
            TelexBackend::BambooRollback
        } else {
            TelexBackend::Recomposition
        };
        Self::with_telex_backend(method, telex_backend)
    }

    fn with_telex_backend(method: InputMethod, telex_backend: TelexBackend) -> Self {
        Self {
            bamboo: BambooEngine::new(method),
            method,
            telex_backend,
            macros: HashMap::new(),
            raw_buffer: String::new(),
            pasted_prefix: String::new(),
            previous_screen_len: None,
            recovered_screen_len: None,
            paste_mode: false,
            auto_restore: true,
            autocorrect: false,
            keyboard_layout: crate::autocorrect::AutocorrectLayout::UsQwerty,
            auto_cap: AutoCapState::default(),
            user_dict: UserDict::new(),
            learning_enabled: true,
            prev_context: LanguageContext::None,
            ring_buffer: CommittedRingBuffer::new(),
            last_dot_stem: None,
            rehydrated_word: false,
            rehydrated_edited: false,
            rehydrated_ascii_only: false,
            pending_coda_edit_canon: None,
            pending_restore_undo: None,
            override_keep_vietnamese: false,
            active_transform: None,
            generation: 0,
            visible_revision: 0,
            boundary: BoundaryState::Committed,
            last_transition: None,
            last_boundary_char: None,
            saturated_until_flush: false,
        }
    }

    pub fn set_auto_restore(&mut self, enabled: bool) {
        self.auto_restore = enabled;
    }

    pub fn set_autocorrect(&mut self, enabled: bool) {
        self.autocorrect = enabled;
    }

    pub fn set_keyboard_layout_name(&mut self, name: &str) {
        self.keyboard_layout = crate::autocorrect::AutocorrectLayout::from_str_name(name);
    }

    pub fn set_auto_capitalize(&mut self, enabled: bool) {
        self.auto_cap.enabled = enabled;
        if !enabled {
            self.auto_cap.reset();
        }
    }

    pub fn set_user_dict(&mut self, dict: UserDict) {
        self.user_dict = dict;
    }

    pub fn set_learning_enabled(&mut self, enabled: bool) {
        self.learning_enabled = enabled;
    }

    pub fn take_user_dict_for_save(&mut self) -> Option<String> {
        self.user_dict
            .take_dirty()
            .then(|| self.user_dict.serialize())
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.bamboo.set_enabled(enabled);
        if !enabled {
            self.reset();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.bamboo.is_enabled()
    }

    pub fn set_method(&mut self, method: InputMethod) {
        self.method = method;
        self.bamboo.set_method(method);
        self.reset();
    }

    fn uses_recomposition_backend(&self) -> bool {
        self.method == InputMethod::Telex && self.telex_backend == TelexBackend::Recomposition
    }

    fn uses_recomposition(&self) -> bool {
        self.uses_recomposition_backend() && self.boundary != BoundaryState::Invalidated
    }

    fn compose_candidate(&self, raw: &str) -> String {
        if raw.is_empty() {
            return String::new();
        }
        if self.uses_recomposition_backend() {
            match_casing(raw, &crate::telex::resolve_and_render(raw))
        } else {
            crate::engine_helpers::compose_with_bamboo(self.method, raw)
        }
    }

    fn composed_tail(&self) -> String {
        if self.raw_buffer.is_empty() {
            return String::new();
        }
        if self.rehydrated_ascii_only {
            return self.raw_buffer.clone();
        }
        if self.uses_recomposition() {
            match_casing(
                &self.raw_buffer,
                &crate::telex::resolve_and_render(&self.raw_buffer),
            )
        } else {
            match_casing(&self.raw_buffer, &self.bamboo.get_output())
        }
    }

    pub(crate) fn visible_output(&self) -> String {
        if self.pasted_prefix.is_empty() {
            self.composed_tail()
        } else {
            format!("{}{}", self.pasted_prefix, self.composed_tail())
        }
    }

    pub fn enter_paste_mode(&mut self) {
        self.paste_mode = true;
    }

    pub fn exit_paste_mode(&mut self) {
        self.paste_mode = false;
    }

    pub fn paste(&mut self, text: &str) -> EngineEvent {
        self.invalidate_composition();
        self.pasted_prefix.push_str(text);
        EngineEvent::Paste(text.to_string())
    }

    pub fn update_with_pasted_text(&mut self, text: &str) {
        self.invalidate_composition();
        self.pasted_prefix.push_str(text);
    }

    pub(crate) fn clear_word_only(&mut self) {
        self.bamboo.reset();
        self.raw_buffer.clear();
        self.pasted_prefix.clear();
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
        self.rehydrated_word = false;
        self.rehydrated_edited = false;
        self.rehydrated_ascii_only = false;
        self.pending_coda_edit_canon = None;
        self.override_keep_vietnamese = false;
        self.active_transform = None;
        self.boundary = BoundaryState::Committed;
    }

    pub fn reset(&mut self) {
        self.clear_word_only();
        self.prev_context = LanguageContext::None;
        self.ring_buffer.clear();
        self.last_dot_stem = None;
        self.pending_restore_undo = None;
        self.last_boundary_char = None;
        self.saturated_until_flush = false;
        self.auto_cap.reset();
    }

    /// Commits a user-selected candidate or next-word prediction without wiping
    /// `ring_buffer`, enabling chained next-word prediction, personal bigram
    /// learning, and Backspace rehydration.
    pub fn commit_selected_word(&mut self, chosen: &str, trailing_space: bool) {
        let prev_word = self
            .learning_enabled
            .then(|| self.ring_buffer.last().map(|e| e.visible_word.clone()))
            .flatten();
        self.clear_word_only();
        self.last_dot_stem = None;
        self.pending_restore_undo = None;
        self.saturated_until_flush = false;
        if chosen.is_empty() {
            return;
        }
        let dict = english_dict();
        let parts: Vec<&str> = chosen.split(' ').filter(|p| !p.is_empty()).collect();
        let mut prev = prev_word;
        for (idx, part) in parts.iter().enumerate() {
            let is_last = idx + 1 == parts.len();
            let (mut entry, mut ctx) = build_surrounding_prev_entry(part, dict);
            if ctx == LanguageContext::English && !dict.is_english_word(&part.to_ascii_lowercase())
            {
                ctx = LanguageContext::None;
                entry.committed_context = LanguageContext::None;
                entry.was_auto_restored = false;
            }
            entry.raw_keys =
                visible_to_canonical_raw(part, self.method).unwrap_or_else(|| (*part).to_string());
            entry.trailing_delimiters = if !is_last || trailing_space {
                " ".to_string()
            } else {
                String::new()
            };
            self.prev_context = ctx;
            self.ring_buffer.push(entry);
            if self.learning_enabled {
                if let Some(ref p) = prev {
                    self.user_dict.record_bigram_usage(p, part);
                }
            }
            prev = Some((*part).to_string());
        }
        self.auto_cap.reset();
        self.last_boundary_char = trailing_space.then_some(' ');
    }

    pub fn reset_visible_revision(&mut self) {
        self.visible_revision = 0;
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
    }

    pub fn seed_surrounding_context(
        &mut self,
        word_at_caret: Option<&str>,
        prev_word: Option<&str>,
        after_sentence_boundary: bool,
    ) -> bool {
        if !(self.raw_buffer.is_empty() && self.pasted_prefix.is_empty())
            || (word_at_caret.is_none() && prev_word.is_none() && !after_sentence_boundary)
        {
            return false;
        }
        self.invalidate_composition();
        let mut seeded = false;
        if after_sentence_boundary {
            self.auto_cap.arm();
            self.prev_context = LanguageContext::None;
            seeded = true;
        } else if let Some(pw) = prev_word {
            let (entry, ctx) = build_surrounding_prev_entry(pw, english_dict());
            self.prev_context = ctx;
            self.ring_buffer.push(entry);
            self.last_boundary_char = Some(' ');
            seeded = true;
        }
        if let Some(word) = word_at_caret.filter(|w| is_seedable_surrounding_word(w)) {
            if let Some(raw) = visible_to_canonical_raw(word, self.method)
                .filter(|r| self.compose_candidate(r) == word)
            {
                self.raw_buffer = raw;
                self.replay_bamboo_from_raw();
                self.rehydrated_word = true;
                self.rehydrated_ascii_only = false;
                self.boundary = BoundaryState::Composing;
                return true;
            }
        }
        seeded
    }

    pub fn invalidate_composition(&mut self) {
        self.clear_word_only();
        self.prev_context = LanguageContext::None;
        self.ring_buffer.clear();
        self.last_dot_stem = None;
        self.last_boundary_char = None;
        self.pending_restore_undo = None;
        self.saturated_until_flush = false;
        self.auto_cap.reset();
        self.boundary = BoundaryState::Invalidated;
        self.last_transition = Some(TransitionKind::Invalidate);
    }

    pub fn recover_composition(&mut self) {
        if let Some(prev_len) = self.previous_screen_len.take() {
            self.visible_revision = self.visible_revision.saturating_sub(1);
            if self.last_transition == Some(TransitionKind::Commit) {
                self.recovered_screen_len = None;
                self.ring_buffer.clear();
                self.pending_restore_undo = None;
            } else {
                self.recovered_screen_len = Some(prev_len);
            }
        } else if self.last_transition != Some(TransitionKind::Recover) {
            self.visible_revision = self.visible_revision.saturating_sub(1);
        }
        self.last_transition = Some(TransitionKind::Recover);
    }

    pub fn flush(&mut self) -> Option<EngineEvent> {
        if self.paste_mode && !self.raw_buffer.is_empty() && !self.raw_buffer.is_ascii() {
            let word = std::mem::take(&mut self.raw_buffer);
            self.pasted_prefix.clear();
            self.bamboo.reset();
            self.paste_mode = false;
            return Some(EngineEvent::Flush(word));
        }
        None
    }

    pub fn add_macro(&mut self, shortcut: String, expansion: String) {
        self.macros.insert(shortcut.to_lowercase(), expansion);
    }

    pub fn clear_macros(&mut self) {
        self.macros.clear();
    }

    fn replacement(
        &mut self,
        delete_before_cursor: usize,
        insert: String,
        reason: ReplacementReason,
    ) -> EngineEvent {
        let replacement = Replacement {
            version: Replacement::VERSION,
            delete_before_cursor,
            insert,
            expected_visible_revision: self.visible_revision,
            transform_generation: self.generation,
            reason,
        };
        self.visible_revision += 1;
        EngineEvent::Replace(replacement)
    }

    fn replay_bamboo_from_raw(&mut self) {
        crate::engine_helpers::replay_bamboo(&mut self.bamboo, &self.raw_buffer);
    }

    pub fn process_key(&mut self, ch: char) -> Option<EngineEvent> {
        if !self.bamboo.is_enabled() {
            return Some(EngineEvent::Insert(ch.to_string()));
        }

        if ch == '\x08' {
            return self.handle_backspace();
        }

        if ch.is_control() && !is_flush_char(ch) {
            self.invalidate_composition();
            return None;
        }

        self.pending_restore_undo = None;

        if is_flush_char(ch) {
            self.saturated_until_flush = false;
            let prev_boundary = self.last_boundary_char;
            self.last_boundary_char = Some(ch);
            let prev_word = (!self.raw_buffer.is_empty()).then_some(self.raw_buffer.as_str());
            self.auto_cap.on_flush_char(ch, prev_word);
            if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
                self.last_dot_stem = None;
                self.ring_buffer.append_delimiter_or_clear(ch);
                if is_sentence_boundary(ch) {
                    self.prev_context = LanguageContext::None;
                }
                return None;
            }

            if self.rehydrated_ascii_only {
                let word = self.raw_buffer.clone();
                let prior_context = self.prev_context;
                self.clear_word_only();
                self.last_dot_stem = None;
                self.prev_context = commit_rehydrated_ascii_word(
                    &mut self.ring_buffer,
                    english_dict(),
                    word,
                    prior_context,
                    ch,
                );
                self.last_transition = Some(TransitionKind::Commit);
                return None;
            }

            let previous = self.composed_tail();
            let prev_len = self
                .recovered_screen_len
                .take()
                .unwrap_or_else(|| previous.chars().count());

            if self.pasted_prefix.is_empty() {
                if let Some(expansion) = self.macros.get(&self.raw_buffer.to_lowercase()).cloned() {
                    let adapted = adapt_macro_casing(&self.raw_buffer, &expansion);
                    let prior_context = self.prev_context;
                    self.clear_word_only();
                    self.last_dot_stem = None;
                    self.prev_context = if is_sentence_boundary(ch) {
                        LanguageContext::None
                    } else {
                        LanguageContext::Vietnamese
                    };
                    if ch == ' ' {
                        for part in adapted.split(' ').filter(|p| !p.is_empty()) {
                            let part_str = part.to_string();
                            self.ring_buffer.push(CommittedWordEntry {
                                raw_keys: part_str.clone(),
                                visible_word: part_str,
                                trailing_delimiters: " ".to_string(),
                                prior_context,
                                committed_context: self.prev_context,
                                was_auto_restored: false,
                                user_override_vn: false,
                            });
                        }
                    } else {
                        self.ring_buffer.clear();
                    }
                    self.last_transition = Some(TransitionKind::Commit);
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, adapted, ReplacementReason::Macro));
                }
            }

            let raw = self.raw_buffer.clone();
            let user_override = self.override_keep_vietnamese;
            let prior_context = self.prev_context;
            let had_pasted_prefix = !self.pasted_prefix.is_empty();
            self.clear_word_only();
            self.last_transition = Some(TransitionKind::Commit);

            if prev_len > 0 {
                let should_restore = self.auto_restore
                    && !user_override
                    && !self.user_dict.is_keep_vn(&raw)
                    && crate::lm::decide_should_restore(
                        english_dict(),
                        &previous,
                        &raw,
                        prior_context,
                        self.ring_buffer.last_three(),
                        Some((prev_boundary, ch)),
                    );

                let autocorrected = if self.autocorrect
                    && ch == ' '
                    && !had_pasted_prefix
                    && !user_override
                    && !self.user_dict.is_keep_raw(&raw)
                    && !matches!(prev_boundary, Some('_' | '/' | '\\' | '.'))
                {
                    let (prev_buf, prev_len_ctx) = self.ring_buffer.prev_words_buf();
                    crate::autocorrect::suggest_correction_with_context(
                        english_dict(),
                        &self.user_dict,
                        &previous,
                        &raw,
                        self.method,
                        self.keyboard_layout,
                        prior_context,
                        &prev_buf[..prev_len_ctx],
                        |r| self.compose_candidate(r),
                    )
                } else {
                    None
                };
                let should_restore = should_restore && autocorrected.is_none();

                let final_word = if should_restore {
                    raw.clone()
                } else if let Some(ref fixed) = autocorrected {
                    fixed.clone()
                } else {
                    previous.clone()
                };
                let committed_context = if is_sentence_boundary(ch) {
                    LanguageContext::None
                } else if should_restore {
                    LanguageContext::English
                } else {
                    english_dict().classify_committed_word_with_prior(
                        &final_word,
                        &raw,
                        prior_context,
                    )
                };

                // Retroactive dot-filename restore (`test.py`, `list.txt`, `host.conf`, `test.tar.gz`)
                if self.auto_restore {
                    if let Some((total_delete, insert, updated_ctx)) = try_dot_filename_restore(
                        &mut self.last_dot_stem,
                        english_dict(),
                        prev_boundary,
                        ch,
                        &raw,
                        &previous,
                        &final_word,
                        prev_len,
                        user_override,
                        had_pasted_prefix,
                    ) {
                        self.prev_context = updated_ctx;
                        self.ring_buffer.clear();
                        self.previous_screen_len = Some(total_delete);
                        return Some(self.replacement(
                            total_delete,
                            insert,
                            ReplacementReason::AutoRestore,
                        ));
                    }
                } else {
                    self.last_dot_stem = None;
                }

                // Gboard Retroactive 2-Word Correction: check if `(prev_word, final_word)`
                // resolves an ambiguous sentence-initial collision (`"Lots of"`, `"Lát nữa"`,
                // `"Mã số"`, `"Rút tiền"`, `"Thí nghiệm"`, `"Too much"`).
                if self.auto_restore && !had_pasted_prefix && !user_override {
                    if let Some(prev_entry) = self.ring_buffer.last().cloned() {
                        if let Some((corrected_w1, w1_restored)) = evaluate_retroactive_first_word(
                            &prev_entry,
                            &final_word,
                            &raw,
                            |w1_raw| self.compose_candidate(w1_raw),
                        ) {
                            let orig_pair = format!(
                                "{}{}{}",
                                prev_entry.visible_word, prev_entry.trailing_delimiters, final_word
                            );
                            let orig_w1 = prev_entry.clone();
                            let can_undo_pair = ch == ' ' && !final_word.contains(' ');
                            let (w2_raw, w2_was_ascii) = resolve_retroactive_undo_w2(
                                &raw,
                                &final_word,
                                should_restore,
                                autocorrected.is_some(),
                                self.method,
                                |s| self.compose_candidate(s),
                            );
                            let (total_delete, combined_insert, updated_ctx) =
                                commit_retroactive_pair(
                                    &mut self.ring_buffer,
                                    prev_entry,
                                    corrected_w1,
                                    w1_restored,
                                    raw,
                                    final_word,
                                    autocorrected,
                                    prev_len,
                                    ch,
                                    should_restore,
                                    user_override,
                                );
                            if can_undo_pair {
                                self.pending_restore_undo =
                                    Some(AutoRestoreUndo::RetroactivePair {
                                        rewritten_pair: combined_insert.clone(),
                                        original_pair: orig_pair,
                                        original_w1_entry: orig_w1,
                                        w2_raw_keys: w2_raw,
                                        w2_was_ascii,
                                        boundary_char: ch,
                                    });
                            }
                            self.prev_context = updated_ctx;
                            self.previous_screen_len = Some(total_delete);
                            return Some(self.replacement(
                                total_delete,
                                combined_insert,
                                ReplacementReason::AutoRestore,
                            ));
                        }
                    }
                }

                self.prev_context = committed_context;
                if ch == ' ' && !had_pasted_prefix {
                    if final_word.contains(' ') {
                        let mut prev_vis = self.ring_buffer.last().map(|e| e.visible_word.clone());
                        for part in final_word.split(' ').filter(|p| !p.is_empty()) {
                            let part_str = part.to_string();
                            if self.learning_enabled {
                                if let Some(ref prev_word) = prev_vis {
                                    self.user_dict.record_bigram_usage(prev_word, &part_str);
                                }
                            }
                            self.ring_buffer.push(CommittedWordEntry {
                                raw_keys: part_str.clone(),
                                visible_word: part_str.clone(),
                                trailing_delimiters: " ".to_string(),
                                prior_context,
                                committed_context,
                                was_auto_restored: should_restore,
                                user_override_vn: user_override,
                            });
                            prev_vis = Some(part_str);
                        }
                    } else {
                        if self.learning_enabled {
                            self.user_dict
                                .maybe_learn_committed(&final_word, &raw, english_dict());
                            if let Some(prev_entry) = self.ring_buffer.last() {
                                self.user_dict
                                    .record_bigram_usage(&prev_entry.visible_word, &final_word);
                            }
                        }
                        self.ring_buffer.push(CommittedWordEntry {
                            raw_keys: raw.clone(),
                            visible_word: final_word,
                            trailing_delimiters: " ".to_string(),
                            prior_context,
                            committed_context,
                            was_auto_restored: should_restore,
                            user_override_vn: user_override,
                        });
                    }
                } else {
                    self.ring_buffer.clear();
                }

                if should_restore {
                    let was_valid_vn = !previous.is_ascii()
                        && crate::spelling::is_valid_vietnamese_syllable(&previous);
                    if ch == ' ' && was_valid_vn {
                        self.pending_restore_undo = Some(AutoRestoreUndo::SingleWord {
                            restored_ascii: raw.clone(),
                            original_vn: previous,
                            raw_keys: raw.clone(),
                            boundary_char: ch,
                            is_autocorrect: false,
                        });
                    }
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, raw, ReplacementReason::AutoRestore));
                }
                if let Some(fixed) = autocorrected {
                    self.pending_restore_undo = Some(AutoRestoreUndo::SingleWord {
                        restored_ascii: fixed.clone(),
                        original_vn: raw.clone(),
                        raw_keys: raw,
                        boundary_char: ch,
                        is_autocorrect: true,
                    });
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, fixed, ReplacementReason::AutoRestore));
                }
            }
            return None;
        }

        if self.rehydrated_ascii_only {
            self.raw_buffer.push(ch);
            self.boundary = BoundaryState::Composing;
            self.previous_screen_len = None;
            return None;
        }
        if self.rehydrated_word {
            self.rehydrated_word = false;
            if !std::mem::replace(&mut self.rehydrated_edited, false) {
                let current = self.composed_tail();
                match crate::engine_helpers::evaluate_guarded_rehydration(
                    &self.raw_buffer,
                    &current,
                    ch,
                    self.method,
                    |r| self.compose_candidate(r),
                ) {
                    crate::engine_helpers::RehydrationDecision::Keep => {}
                    crate::engine_helpers::RehydrationDecision::ReplaceRaw(resolved) => {
                        self.raw_buffer = resolved;
                        self.replay_bamboo_from_raw();
                    }
                    crate::engine_helpers::RehydrationDecision::ClearWord => {
                        self.clear_word_only();
                    }
                }
            }
        }

        if self.method == InputMethod::Telex
            && ch.is_ascii_digit()
            && self.raw_buffer.is_empty()
            && self.pasted_prefix.is_empty()
            && !self.macros.keys().any(|k| k.starts_with(&ch.to_string()))
        {
            self.auto_cap.reset();
            // Preserve active English clause momentum across digits (`for 2 days`,
            // `top 10 lists`) while allowing numbered Vietnamese lists (`số 1 test`)
            // to restore English-dominant labels cleanly.
            if self.prev_context != LanguageContext::English {
                self.prev_context = LanguageContext::None;
            }
            self.ring_buffer.clear();
            self.last_dot_stem = None;
            self.last_boundary_char = None;
            return None;
        }

        if self.saturated_until_flush {
            self.previous_screen_len = None;
            return None;
        }

        if self.raw_buffer.chars().count() >= 64 {
            self.clear_word_only();
            self.ring_buffer.clear();
            self.saturated_until_flush = true;
            return None;
        }

        let previous = self.composed_tail();
        let had_recovered_screen_len = self.recovered_screen_len.is_some();
        let prev_len = self
            .recovered_screen_len
            .take()
            .unwrap_or_else(|| previous.chars().count());

        if let Some(coda_canon) = self.pending_coda_edit_canon.take() {
            let normal_cand = self.compose_candidate(&format!("{}{}", self.raw_buffer, ch));
            let normal_valid = !normal_cand.is_ascii()
                && crate::spelling::is_valid_vietnamese_prefix(&normal_cand);
            if !normal_valid {
                if let Some(resolved) = resolve_coda_edit_canon(&coda_canon, ch, self.method, |r| {
                    self.compose_candidate(r)
                }) {
                    self.raw_buffer = resolved;
                    self.replay_bamboo_from_raw();
                }
            }
        }

        let use_recomp = self.uses_recomposition_backend();
        let was_telex_undone = use_recomp && crate::telex::resolve_with_undo(&self.raw_buffer).1;
        if !use_recomp
            && crate::engine_helpers::apply_bamboo_telex_undo(
                &mut self.bamboo,
                &mut self.raw_buffer,
                ch,
            )
        {
            self.active_transform = None;
            self.boundary = BoundaryState::Composing;
            self.last_transition = Some(TransitionKind::UndoActive);
            let output = self.composed_tail();
            self.previous_screen_len = Some(prev_len);
            return Some(self.replacement(prev_len, output, ReplacementReason::UndoTransform));
        }

        let cap_override = self.auto_cap.maybe_capitalize(
            ch,
            self.raw_buffer.is_empty() && self.pasted_prefix.is_empty(),
        );
        let ch = cap_override.unwrap_or(ch);

        self.raw_buffer.push(ch);
        if matches!(
            self.boundary,
            BoundaryState::Committed | BoundaryState::Invalidated
        ) {
            self.boundary = BoundaryState::Composing;
        }

        if self.bamboo.process_key(ch).is_some() {
            let expected = format!("{}{}", previous, ch);
            let (cased, telex_undo_triggered) = if use_recomp {
                let (rendered, now_undone) = crate::telex::resolve_with_undo(&self.raw_buffer);
                let c = match_casing(&self.raw_buffer, &rendered);
                (c, !was_telex_undone && now_undone)
            } else {
                (self.composed_tail(), false)
            };
            if cased != expected
                || had_recovered_screen_len
                || (self.active_transform.is_some() && cased != previous)
            {
                if telex_undo_triggered
                    || (self.method == InputMethod::Vni
                        && cased != expected
                        && matches!(ch, '1'..='9')
                        && cased.ends_with(ch)
                        && self.active_transform.is_some())
                {
                    self.active_transform = None;
                    self.boundary = BoundaryState::Composing;
                    self.last_transition = Some(TransitionKind::UndoActive);
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(
                        prev_len,
                        cased,
                        ReplacementReason::UndoTransform,
                    ));
                }
                if cased != previous {
                    if let Some((kind, trigger, policy)) =
                        crate::engine_helpers::active_transform_for_method(
                            self.method,
                            &self.raw_buffer,
                            ch,
                            &cased,
                        )
                    {
                        let transition = if self.active_transform.is_some() {
                            TransitionKind::ReplaceActive
                        } else {
                            TransitionKind::Apply
                        };
                        self.generation += 1;
                        self.active_transform = Some(TransformIdentity {
                            kind,
                            trigger,
                            generation: self.generation,
                            undo_policy: policy,
                        });
                        self.last_transition = Some(transition);
                    }
                    if crate::engine_helpers::is_tone_strip_key(self.method, ch, &previous, &cased)
                    {
                        self.override_keep_vietnamese = true;
                    }
                    self.boundary = BoundaryState::Composing;
                    let reason = match self.last_transition {
                        Some(TransitionKind::ReplaceActive) => ReplacementReason::ReplaceTransform,
                        _ => ReplacementReason::ApplyTransform,
                    };
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, cased, reason));
                }
                self.boundary = BoundaryState::Composing;
                self.previous_screen_len = Some(prev_len);
                return Some(self.replacement(
                    prev_len,
                    cased,
                    ReplacementReason::ReplaceTransform,
                ));
            }
        }

        if cap_override.is_some() {
            self.previous_screen_len = Some(prev_len);
            return Some(self.replacement(
                prev_len,
                self.composed_tail(),
                ReplacementReason::ApplyTransform,
            ));
        }

        self.previous_screen_len = None;
        None
    }

    pub fn buffer(&self) -> String {
        self.visible_output()
    }

    pub fn raw_buffer(&self) -> &str {
        &self.raw_buffer
    }

    pub fn has_pasted_prefix(&self) -> bool {
        !self.pasted_prefix.is_empty()
    }

    pub fn state(&self) -> CompositionState {
        build_composition_state(
            format!("{}{}", self.pasted_prefix, self.raw_buffer),
            self.visible_output(),
            self.active_transform.clone(),
            self.boundary,
            self.generation,
            self.last_transition,
        )
    }
}
