// SPDX-License-Identifier: MIT
use crate::bamboo::{BambooEngine, TelexUndo};
use crate::bigram::evaluate_retroactive_first_word;
use crate::engine_helpers::{
    active_telex_transform, adapt_macro_casing, build_composition_state,
    build_surrounding_prev_entry, is_flush_char, is_seedable_surrounding_word,
    is_sentence_boundary, match_casing, resolve_coda_edit_canon,
};
use crate::english::{EnglishDict, LanguageContext};
use crate::input_method::InputMethod;
use crate::ring_buffer::{
    visible_to_canonical_raw, BackspaceAcrossBoundary, CommittedRingBuffer, CommittedWordEntry,
};
use crate::state::{
    BoundaryState, CompositionState, PhysicalEvent, Replacement, ReplacementReason,
    TransformIdentity, TransitionKind,
};
use crate::user_dict::{AutoCapState, AutoRestoreUndo, UserDict};
use std::collections::HashMap;
use std::sync::OnceLock;

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
    auto_cap: AutoCapState,
    user_dict: UserDict,
    prev_context: LanguageContext,
    ring_buffer: CommittedRingBuffer,
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
            auto_cap: AutoCapState::default(),
            user_dict: UserDict::new(),
            prev_context: LanguageContext::None,
            ring_buffer: CommittedRingBuffer::new(),
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
        }
    }

    pub fn set_auto_restore(&mut self, enabled: bool) {
        self.auto_restore = enabled;
    }

    pub fn set_autocorrect(&mut self, enabled: bool) {
        self.autocorrect = enabled;
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

    fn uses_recomposition(&self) -> bool {
        self.method == InputMethod::Telex
            && self.telex_backend == TelexBackend::Recomposition
            && self.boundary != BoundaryState::Invalidated
    }

    fn compose_candidate(&self, raw: &str) -> String {
        if raw.is_empty() {
            return String::new();
        }
        if self.uses_recomposition() {
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
        self.pending_restore_undo = None;
        self.last_boundary_char = None;
        self.auto_cap.reset();
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
        if !(self.raw_buffer.is_empty() && self.pasted_prefix.is_empty()) {
            return false;
        }
        let mut seeded = false;
        if after_sentence_boundary {
            self.auto_cap.arm();
            self.prev_context = LanguageContext::None;
            seeded = true;
        } else if let Some(pw) = prev_word {
            let (entry, ctx) = build_surrounding_prev_entry(pw, english_dict());
            self.prev_context = ctx;
            self.ring_buffer.push(entry);
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
        self.pending_restore_undo = None;
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

    pub fn reduce(&mut self, event: PhysicalEvent) -> Option<EngineEvent> {
        match event {
            PhysicalEvent::Printable(ch) => self.process_key(ch),
            PhysicalEvent::Backspace => self.process_key('\x08'),
            PhysicalEvent::WordBoundary(ch) => {
                if is_flush_char(ch) {
                    self.process_key(ch)
                } else {
                    None
                }
            }
            PhysicalEvent::Delete
            | PhysicalEvent::Navigate
            | PhysicalEvent::SelectionChanged
            | PhysicalEvent::ExternalEdit
            | PhysicalEvent::FocusChanged => {
                self.invalidate_composition();
                None
            }
            PhysicalEvent::Paste(text) => {
                self.invalidate_composition();
                self.pasted_prefix.push_str(&text);
                None
            }
            PhysicalEvent::ReplacementFailed => {
                self.recover_composition();
                None
            }
        }
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

    fn handle_backspace(&mut self) -> Option<EngineEvent> {
        self.auto_cap.reset();
        if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
            if let Some(undo) = self.pending_restore_undo.take() {
                let _ = self.ring_buffer.pop_last();
                if undo.is_autocorrect {
                    self.user_dict.record_keep_raw(&undo.raw_keys);
                } else {
                    self.user_dict.record_keep_vn(&undo.raw_keys);
                }
                let delete_len =
                    undo.restored_ascii.chars().count() + undo.boundary_char.len_utf8().min(1);
                self.raw_buffer = undo.raw_keys;
                if !undo.is_autocorrect {
                    self.replay_bamboo_from_raw();
                } else {
                    self.bamboo.reset();
                }
                self.override_keep_vietnamese = true;
                self.rehydrated_word = undo.is_autocorrect;
                self.rehydrated_ascii_only = undo.is_autocorrect;
                self.active_transform = None;
                self.boundary = BoundaryState::Composing;
                self.last_transition = Some(TransitionKind::Edit);
                self.previous_screen_len = Some(delete_len);
                return Some(self.replacement(
                    delete_len,
                    undo.original_vn,
                    ReplacementReason::Edit,
                ));
            }
        }

        self.pending_restore_undo = None;
        if self.raw_buffer.is_empty() {
            if !self.pasted_prefix.is_empty() {
                let previous = self.visible_output();
                let prev_len = self
                    .recovered_screen_len
                    .take()
                    .unwrap_or_else(|| previous.chars().count());
                self.previous_screen_len = Some(prev_len);
                self.pasted_prefix.pop();
                self.active_transform = None;
                self.boundary = if self.pasted_prefix.is_empty() {
                    BoundaryState::Committed
                } else {
                    BoundaryState::Composing
                };
                self.last_transition = Some(TransitionKind::Edit);
                return Some(self.replacement(
                    prev_len,
                    self.visible_output(),
                    ReplacementReason::Edit,
                ));
            }

            if let BackspaceAcrossBoundary::Rehydrated(entry) =
                self.ring_buffer.on_backspace_when_idle()
            {
                self.prev_context = entry.prior_context;
                self.override_keep_vietnamese = entry.user_override_vn;
                let method = self.method;
                let canonical = (!entry.was_auto_restored)
                    .then(|| visible_to_canonical_raw(&entry.visible_word, method))
                    .flatten()
                    .filter(|cand| {
                        crate::spelling::is_valid_vietnamese_syllable(&entry.visible_word)
                            && self.compose_candidate(cand) == entry.visible_word
                    });
                if let Some(canon_raw) = canonical {
                    self.raw_buffer = canon_raw;
                    self.replay_bamboo_from_raw();
                    self.rehydrated_word = true;
                    self.rehydrated_edited = false;
                    self.rehydrated_ascii_only = false;
                } else {
                    self.raw_buffer = entry.visible_word;
                    self.bamboo.reset();
                    self.rehydrated_word = true;
                    self.rehydrated_edited = false;
                    self.rehydrated_ascii_only = true;
                }
                self.active_transform = None;
                self.boundary = BoundaryState::Composing;
                self.last_transition = Some(TransitionKind::Edit);
                return None;
            }

            self.active_transform = None;
            self.boundary = BoundaryState::Committed;
            self.last_transition = Some(TransitionKind::Edit);
            return None;
        }

        // Character-granular Backspace for words rehydrated across Space:
        // deleting 1 visible char on screen takes exactly 1 native Backspace (`None`),
        // while keeping the remaining prefix live for Vietnamese tone/mark additions.
        if self.rehydrated_word && self.pasted_prefix.is_empty() {
            self.rehydrated_edited = true;
            if self.rehydrated_ascii_only {
                let _ = self.raw_buffer.pop();
                if self.raw_buffer.is_empty() {
                    self.clear_word_only();
                } else {
                    self.boundary = BoundaryState::Composing;
                }
                self.last_transition = Some(TransitionKind::Edit);
                return None;
            }
            let mut truncated = self.composed_tail();
            let _ = truncated.pop();
            if truncated.is_empty() {
                self.clear_word_only();
                self.last_transition = Some(TransitionKind::Edit);
                return None;
            }
            if let Some(canon) = visible_to_canonical_raw(&truncated, self.method) {
                if self.compose_candidate(&canon) == truncated {
                    self.raw_buffer = canon;
                    self.replay_bamboo_from_raw();
                    self.active_transform = None;
                    self.boundary = BoundaryState::Composing;
                    self.last_transition = Some(TransitionKind::Edit);
                    return None;
                }
            }
            self.raw_buffer = truncated;
            self.bamboo.reset();
            self.rehydrated_ascii_only = true;
            self.active_transform = None;
            self.boundary = BoundaryState::Composing;
            self.last_transition = Some(TransitionKind::Edit);
            return None;
        }

        let previous = self.composed_tail();
        let prev_len = self
            .recovered_screen_len
            .take()
            .unwrap_or_else(|| previous.chars().count());
        self.previous_screen_len = Some(prev_len);
        self.override_keep_vietnamese = false;

        let base_visible = self
            .pending_coda_edit_canon
            .as_deref()
            .map(|c| self.compose_candidate(c))
            .unwrap_or_else(|| previous.clone());

        let _ = self.raw_buffer.pop();
        self.replay_bamboo_from_raw();
        let popped_visible = self.composed_tail();

        let popped_tail_is_coda_consonant = base_visible.chars().last().is_some_and(|c| {
            c.is_ascii_alphabetic()
                && !matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
        });
        if !base_visible.is_ascii()
            && self.pasted_prefix.is_empty()
            && (popped_visible.chars().count() == prev_len
                || (self.pending_coda_edit_canon.is_some() && popped_tail_is_coda_consonant))
        {
            let mut truncated = base_visible;
            let _ = truncated.pop();
            self.pending_coda_edit_canon = (!truncated.is_empty() && !truncated.is_ascii())
                .then(|| visible_to_canonical_raw(&truncated, self.method))
                .flatten();
        } else {
            self.pending_coda_edit_canon = None;
        }

        self.active_transform = None;
        self.boundary = if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
            BoundaryState::Committed
        } else {
            BoundaryState::Composing
        };
        self.last_transition = Some(TransitionKind::Edit);
        Some(self.replacement(prev_len, popped_visible, ReplacementReason::Edit))
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
            let prev_boundary = self.last_boundary_char;
            self.last_boundary_char = Some(ch);
            self.auto_cap.on_flush_char(ch);
            if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
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
                let committed_context = if is_sentence_boundary(ch) {
                    LanguageContext::None
                } else {
                    english_dict().classify_committed_word_with_prior(&word, &word, prior_context)
                };
                self.prev_context = committed_context;
                self.last_transition = Some(TransitionKind::Commit);
                if ch == ' ' {
                    self.ring_buffer.push(CommittedWordEntry {
                        raw_keys: word.clone(),
                        visible_word: word,
                        trailing_delimiters: " ".to_string(),
                        prior_context,
                        committed_context,
                        was_auto_restored: true,
                        user_override_vn: false,
                    });
                } else {
                    self.ring_buffer.clear();
                }
                return None;
            }

            let previous = self.composed_tail();
            let prev_len = self
                .recovered_screen_len
                .take()
                .unwrap_or_else(|| previous.chars().count());

            if let Some(expansion) = self.macros.get(&self.raw_buffer.to_lowercase()).cloned() {
                let adapted = adapt_macro_casing(&self.raw_buffer, &expansion);
                self.clear_word_only();
                self.ring_buffer.clear();
                self.prev_context = if is_sentence_boundary(ch) {
                    LanguageContext::None
                } else {
                    LanguageContext::Vietnamese
                };
                self.last_transition = Some(TransitionKind::Commit);
                self.previous_screen_len = Some(prev_len);
                return Some(self.replacement(prev_len, adapted, ReplacementReason::Macro));
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
                        self.ring_buffer.last_two(),
                        Some((prev_boundary, ch)),
                    );

                let autocorrected = if self.autocorrect
                    && ch == ' '
                    && !had_pasted_prefix
                    && !user_override
                    && !self.user_dict.is_keep_raw(&raw)
                    && !matches!(prev_boundary, Some('_' | '/' | '\\' | '.'))
                {
                    crate::autocorrect::suggest_correction(
                        english_dict(),
                        &self.user_dict,
                        &previous,
                        &raw,
                        self.method,
                        prior_context,
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

                // Gboard Retroactive 2-Word Correction: check if `(prev_word, final_word)`
                // resolves an ambiguous sentence-initial collision (`"Lots of"`, `"Lát nữa"`,
                // `"Mã số"`, `"Rút tiền"`, `"Thí nghiệm"`, `"Too much"`).
                if self.auto_restore && !had_pasted_prefix {
                    if let Some(prev_entry) = self.ring_buffer.last().cloned() {
                        if let Some((corrected_w1, w1_restored)) = evaluate_retroactive_first_word(
                            &prev_entry,
                            &final_word,
                            &raw,
                            |w1_raw| self.compose_candidate(w1_raw),
                        ) {
                            let _ = self.ring_buffer.pop_last();
                            let effective_w2 = if w1_restored && !user_override {
                                autocorrected.unwrap_or_else(|| raw.clone())
                            } else {
                                final_word
                            };
                            let total_delete = prev_entry.visible_word.chars().count()
                                + prev_entry.trailing_delimiters.chars().count()
                                + prev_len;
                            let combined_insert = format!(
                                "{}{}{}",
                                corrected_w1, prev_entry.trailing_delimiters, effective_w2
                            );
                            let updated_ctx = if is_sentence_boundary(ch) {
                                LanguageContext::None
                            } else if w1_restored {
                                LanguageContext::English
                            } else {
                                LanguageContext::Vietnamese
                            };
                            self.prev_context = updated_ctx;
                            if ch == ' ' {
                                let w1_ctx = if w1_restored {
                                    LanguageContext::English
                                } else {
                                    LanguageContext::Vietnamese
                                };
                                self.ring_buffer.push(CommittedWordEntry {
                                    visible_word: corrected_w1,
                                    was_auto_restored: w1_restored,
                                    committed_context: w1_ctx,
                                    ..prev_entry
                                });
                                self.ring_buffer.push(CommittedWordEntry {
                                    raw_keys: raw,
                                    visible_word: effective_w2,
                                    trailing_delimiters: " ".to_string(),
                                    prior_context: w1_ctx,
                                    committed_context: updated_ctx,
                                    was_auto_restored: should_restore || w1_restored,
                                    user_override_vn: user_override,
                                });
                            } else {
                                self.ring_buffer.clear();
                            }
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
                    self.user_dict
                        .maybe_learn_committed(&final_word, &raw, english_dict());
                    self.ring_buffer.push(CommittedWordEntry {
                        raw_keys: raw.clone(),
                        visible_word: final_word,
                        trailing_delimiters: " ".to_string(),
                        prior_context,
                        committed_context,
                        was_auto_restored: should_restore,
                        user_override_vn: user_override,
                    });
                } else {
                    self.ring_buffer.clear();
                }

                if should_restore {
                    let was_valid_vn = !previous.is_ascii()
                        && crate::spelling::is_valid_vietnamese_syllable(&previous);
                    if ch == ' ' && was_valid_vn {
                        self.pending_restore_undo = Some(AutoRestoreUndo {
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
                    self.pending_restore_undo = Some(AutoRestoreUndo {
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
            self.prev_context = LanguageContext::None;
            self.ring_buffer.clear();
            return None;
        }

        if self.raw_buffer.chars().count() >= 64 {
            self.clear_word_only();
            self.ring_buffer.clear();
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

        if let Some(undo) = self.bamboo.undo_telex_transform(ch, &self.raw_buffer) {
            let consumed_stroke_literal = if undo == TelexUndo::ConsumeTrigger {
                self.raw_buffer.chars().last()
            } else {
                None
            };
            self.raw_buffer.push(ch);
            match undo {
                TelexUndo::AppendTrigger => self.bamboo.append_literal(ch),
                TelexUndo::ConsumeTrigger => {
                    if let Some(literal) = consumed_stroke_literal {
                        self.bamboo.append_literal(literal);
                    }
                }
            }
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
            let cased = self.composed_tail();
            if cased != expected
                || had_recovered_screen_len
                || (self.active_transform.is_some() && cased != previous)
            {
                if cased != previous {
                    if let Some((kind, trigger, policy)) =
                        active_telex_transform(&self.raw_buffer, ch, &cased)
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
                    if self.method == InputMethod::Telex
                        && ch.eq_ignore_ascii_case(&'z')
                        && !cased.is_ascii()
                        && cased.chars().count() == previous.chars().count()
                        && !cased.ends_with(['z', 'Z'])
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
