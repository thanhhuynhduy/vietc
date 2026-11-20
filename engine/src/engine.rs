// SPDX-License-Identifier: MIT
use crate::bamboo::{BambooEngine, TelexUndo};
use crate::bigram::evaluate_retroactive_first_word;
use crate::engine_helpers::{
    active_telex_transform, adapt_macro_casing, case_shape, is_flush_char, is_sentence_boundary,
    match_casing, resolve_coda_edit_canon,
};
use crate::english::{should_restore_word_decision, EnglishDict, LanguageContext};
use crate::input_method::InputMethod;
use crate::ring_buffer::{
    visible_to_canonical_raw, BackspaceAcrossBoundary, CommittedRingBuffer, CommittedWordEntry,
};
use crate::state::{
    BoundaryState, CompositionState, PhysicalEvent, Replacement, ReplacementReason,
    SpellingValidity, TransformIdentity, TransformKind, TransitionKind, UndoPolicy,
};
use std::collections::HashMap;
use std::sync::OnceLock;

fn english_dict() -> &'static EnglishDict {
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

#[derive(Debug, Clone)]
struct AutoRestoreUndo {
    restored_ascii: String,
    original_vn: String,
    raw_keys: String,
    boundary_char: char,
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
    prev_context: LanguageContext,
    ring_buffer: CommittedRingBuffer,
    rehydrated_word: bool,
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
            prev_context: LanguageContext::None,
            ring_buffer: CommittedRingBuffer::new(),
            rehydrated_word: false,
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

    pub fn telex_undo_policy(kind: TransformKind) -> Option<UndoPolicy> {
        match kind {
            TransformKind::Tone('f' | 's' | 'r' | 'x' | 'j') => Some(UndoPolicy::AppendTrigger),
            TransformKind::Mark("aw" | "ow" | "uw" | "uo+w" | "ua+w" | "aa" | "ee" | "oo") => {
                Some(UndoPolicy::AppendTrigger)
            }
            TransformKind::StrokeDd => Some(UndoPolicy::ConsumeTrigger),
            _ => None,
        }
    }

    pub fn set_auto_restore(&mut self, enabled: bool) {
        self.auto_restore = enabled;
    }

    pub fn should_restore_word(composed: &str, raw: &str) -> bool {
        Self::should_restore_word_with_context(composed, raw, LanguageContext::None)
    }

    pub fn should_restore_word_with_context(
        composed: &str,
        raw: &str,
        prev_context: LanguageContext,
    ) -> bool {
        Self::should_restore_word_with_bigram(composed, raw, prev_context, None, None)
    }

    pub(crate) fn should_restore_word_with_bigram(
        composed: &str,
        raw: &str,
        prev_context: LanguageContext,
        prev_visible_word: Option<&str>,
        boundary_context: Option<(Option<char>, char)>,
    ) -> bool {
        should_restore_word_decision(
            english_dict(),
            composed,
            raw,
            prev_context,
            prev_visible_word,
            boundary_context,
        )
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
            let mut tmp = BambooEngine::new(self.method);
            let mut replayed = String::new();
            for rk in raw.chars() {
                if let Some(undo) = tmp.undo_telex_transform(rk, &replayed) {
                    let consumed_lit = if undo == TelexUndo::ConsumeTrigger {
                        replayed.chars().last()
                    } else {
                        None
                    };
                    replayed.push(rk);
                    match undo {
                        TelexUndo::AppendTrigger => tmp.append_literal(rk),
                        TelexUndo::ConsumeTrigger => {
                            if let Some(lit) = consumed_lit {
                                tmp.append_literal(lit);
                            }
                        }
                    }
                } else {
                    replayed.push(rk);
                    let _ = tmp.process_key(rk);
                }
            }
            match_casing(raw, &tmp.get_output())
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
    }

    pub fn reset_visible_revision(&mut self) {
        self.visible_revision = 0;
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
    }

    pub fn invalidate_composition(&mut self) {
        self.bamboo.reset();
        self.raw_buffer.clear();
        self.pasted_prefix.clear();
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
        self.prev_context = LanguageContext::None;
        self.ring_buffer.clear();
        self.rehydrated_word = false;
        self.rehydrated_ascii_only = false;
        self.pending_coda_edit_canon = None;
        self.pending_restore_undo = None;
        self.override_keep_vietnamese = false;
        self.active_transform = None;
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
        if self.paste_mode && !self.raw_buffer.is_empty() {
            let has_unicode = !self.raw_buffer.is_ascii();
            if has_unicode {
                let word = self.raw_buffer.clone();
                self.raw_buffer.clear();
                self.pasted_prefix.clear();
                self.bamboo.reset();
                self.paste_mode = false;
                return Some(EngineEvent::Flush(word));
            }
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
        self.bamboo.reset();
        let mut replayed_raw = String::new();
        for raw_key in self.raw_buffer.chars() {
            if let Some(undo) = self.bamboo.undo_telex_transform(raw_key, &replayed_raw) {
                let consumed_stroke_literal = if undo == TelexUndo::ConsumeTrigger {
                    replayed_raw.chars().last()
                } else {
                    None
                };
                replayed_raw.push(raw_key);
                match undo {
                    TelexUndo::AppendTrigger => self.bamboo.append_literal(raw_key),
                    TelexUndo::ConsumeTrigger => {
                        if let Some(literal) = consumed_stroke_literal {
                            self.bamboo.append_literal(literal);
                        }
                    }
                }
            } else {
                replayed_raw.push(raw_key);
                let _ = self.bamboo.process_key(raw_key);
            }
        }
    }

    fn handle_backspace(&mut self) -> Option<EngineEvent> {
        if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
            if let Some(undo) = self.pending_restore_undo.take() {
                let _ = self.ring_buffer.pop_last();
                let delete_len =
                    undo.restored_ascii.chars().count() + undo.boundary_char.len_utf8().min(1);
                self.raw_buffer = undo.raw_keys;
                self.replay_bamboo_from_raw();
                self.override_keep_vietnamese = true;
                self.rehydrated_word = false;
                self.rehydrated_ascii_only = false;
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
                    self.rehydrated_ascii_only = false;
                } else {
                    self.raw_buffer = entry.visible_word;
                    self.bamboo.reset();
                    self.rehydrated_word = true;
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

        if !base_visible.is_ascii()
            && self.pasted_prefix.is_empty()
            && (self.pending_coda_edit_canon.is_some()
                || popped_visible.chars().count() == prev_len)
        {
            let mut truncated = base_visible;
            let _ = truncated.pop();
            self.pending_coda_edit_canon = (!truncated.is_empty())
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

            let prev_boundary = self.last_boundary_char;
            self.last_boundary_char = Some(ch);

            if prev_len > 0 {
                let should_restore = self.auto_restore && !user_override && {
                    let prev_vis = self.ring_buffer.last().map(|e| e.visible_word.as_str());
                    Self::should_restore_word_with_bigram(
                        &previous,
                        &raw,
                        prior_context,
                        prev_vis,
                        Some((prev_boundary, ch)),
                    )
                };

                let final_word = if should_restore {
                    raw.clone()
                } else {
                    previous.clone()
                };
                let committed_context = if is_sentence_boundary(ch) {
                    LanguageContext::None
                } else if should_restore {
                    LanguageContext::English
                } else {
                    english_dict().classify_committed_word_with_prior(
                        &previous,
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
                                raw.clone()
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
                        && crate::spelling::is_valid_vietnamese_syllable(&previous)
                        && crate::spelling::is_in_vietnamese_dictionary(&previous);
                    if ch == ' ' && was_valid_vn {
                        self.pending_restore_undo = Some(AutoRestoreUndo {
                            restored_ascii: raw.clone(),
                            original_vn: previous,
                            raw_keys: raw.clone(),
                            boundary_char: ch,
                        });
                    }
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, raw, ReplacementReason::AutoRestore));
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
            let current = self.composed_tail();
            let cand = self.compose_candidate(&format!("{}{}", self.raw_buffer, ch));
            let is_tone_strip = (self.method == InputMethod::Telex
                && ch.eq_ignore_ascii_case(&'z'))
                || (self.method == InputMethod::Vni && ch == '0');
            let is_valid_mod = cand != current
                && crate::spelling::is_valid_vietnamese_prefix(&cand)
                && (is_tone_strip || crate::spelling::is_in_vietnamese_dictionary_or_prefix(&cand));
            if !is_valid_mod {
                if let Some(resolved) =
                    resolve_coda_edit_canon(&self.raw_buffer, ch, self.method, |r| {
                        self.compose_candidate(r)
                    })
                {
                    let edited = self.compose_candidate(&format!("{resolved}{ch}"));
                    if crate::spelling::is_in_vietnamese_dictionary_or_prefix(&edited) {
                        self.raw_buffer = resolved;
                        self.replay_bamboo_from_raw();
                    } else {
                        self.clear_word_only();
                    }
                } else {
                    self.clear_word_only();
                }
            }
        }

        if self.method == InputMethod::Telex
            && ch.is_ascii_digit()
            && self.raw_buffer.is_empty()
            && self.pasted_prefix.is_empty()
            && !self.macros.keys().any(|k| k.starts_with(&ch.to_string()))
        {
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
            return Some(self.replacement(
                prev_len,
                match_casing(&self.raw_buffer, &output),
                ReplacementReason::UndoTransform,
            ));
        }

        self.raw_buffer.push(ch);
        if matches!(
            self.boundary,
            BoundaryState::Committed | BoundaryState::Invalidated
        ) {
            self.boundary = BoundaryState::Composing;
        }

        if self.bamboo.process_key(ch).is_some() {
            let expected = format!("{}{}", previous, ch);
            let cased = match_casing(&self.raw_buffer, &self.composed_tail());
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

        self.previous_screen_len = None;
        None
    }

    pub fn buffer(&self) -> String {
        self.visible_output()
    }

    pub fn state(&self) -> CompositionState {
        let visible = self.visible_output();
        let raw_keys = format!("{}{}", self.pasted_prefix, self.raw_buffer);
        CompositionState {
            case_shape: case_shape(&raw_keys),
            raw_keys,
            validity: if visible.is_empty() {
                SpellingValidity::Unknown
            } else if crate::spelling::is_valid_vietnamese_syllable(&visible) {
                SpellingValidity::Valid
            } else {
                SpellingValidity::Invalid
            },
            visible,
            active_transform: self.active_transform.clone(),
            boundary: self.boundary,
            generation: self.generation,
            last_transition: self.last_transition,
        }
    }
}
