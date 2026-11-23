// SPDX-License-Identifier: MIT
use crate::ring_buffer::{visible_to_canonical_raw, BackspaceAcrossBoundary};
use crate::state::{BoundaryState, ReplacementReason, TransitionKind};

use super::{Engine, EngineEvent};

impl Engine {
    pub(super) fn handle_backspace(&mut self) -> Option<EngineEvent> {
        self.auto_cap.reset();
        self.last_dot_stem = None;
        if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
            if let Some(undo) = self.pending_restore_undo.take() {
                let (delete_len, restore_text) = crate::engine_helpers::apply_pending_restore_undo(
                    undo,
                    self.learning_enabled,
                    &mut self.ring_buffer,
                    &mut self.user_dict,
                    &mut self.bamboo,
                    &mut self.raw_buffer,
                    &mut self.prev_context,
                    &mut self.override_keep_vietnamese,
                    &mut self.rehydrated_word,
                    &mut self.rehydrated_ascii_only,
                );
                self.active_transform = None;
                self.boundary = BoundaryState::Composing;
                self.last_transition = Some(TransitionKind::Edit);
                self.previous_screen_len = Some(delete_len);
                return Some(self.replacement(delete_len, restore_text, ReplacementReason::Edit));
            }
        }

        self.pending_restore_undo = None;
        if self.raw_buffer.is_empty() {
            if !self.pasted_prefix.is_empty() {
                let recovered = self.recovered_screen_len.take();
                let prev_len = recovered.unwrap_or_else(|| self.pasted_prefix.chars().count());
                self.previous_screen_len = Some(prev_len);
                self.pasted_prefix.pop();
                self.active_transform = None;
                self.boundary = if self.pasted_prefix.is_empty() {
                    BoundaryState::Committed
                } else {
                    BoundaryState::Composing
                };
                self.last_transition = Some(TransitionKind::Edit);
                let (del, ins) = match recovered {
                    Some(n) => (n, self.visible_output()),
                    None => (1, String::new()),
                };
                return Some(self.replacement(del, ins, ReplacementReason::Edit));
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
}
