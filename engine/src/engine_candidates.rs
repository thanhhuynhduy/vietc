// SPDX-License-Identifier: MIT
use super::{english_dict, Engine};

impl Engine {
    /// Optional read-only autocorrect suggestion for the active composing word,
    /// used exclusively when the opt-in floating candidate strip is enabled.
    pub fn candidate_suggestion(&self) -> Option<String> {
        if self.raw_buffer.is_empty() || !self.pasted_prefix.is_empty() {
            return None;
        }
        let visible = self.composed_tail();
        let (prev_buf, prev_len_ctx) = self.ring_buffer.prev_words_buf();
        let cand = crate::autocorrect::suggest_correction_with_context(
            english_dict(),
            &self.user_dict,
            &visible,
            &self.raw_buffer,
            self.method,
            self.keyboard_layout,
            self.prev_context,
            &prev_buf[..prev_len_ctx],
            |r| self.compose_candidate(r),
        )?;
        (cand != visible && cand != self.raw_buffer).then_some(cand)
    }

    /// Predict up to `limit` (capped at 3) next words immediately after a Space
    /// boundary, prioritizing the user's promoted Personal Bigrams ahead of the
    /// quantized Kneser-Ney trigram/bigram corpus index.
    pub fn predict_next_words(&self, limit: usize) -> Vec<String> {
        let cap = limit.min(3);
        if cap == 0
            || !self.bamboo.is_enabled()
            || !self.raw_buffer.is_empty()
            || !self.pasted_prefix.is_empty()
            || self.last_boundary_char != Some(' ')
        {
            return Vec::new();
        }
        let (p2, p1) = self.ring_buffer.last_two();
        let Some(p1) = p1 else {
            return Vec::new();
        };
        let mut out = self.user_dict.predict_next_words(&p1.visible_word, cap);
        if out.len() < cap {
            let corpus = match p2 {
                Some(prev2) => crate::ngram_lm::predict_next_words(
                    &[prev2.visible_word.as_str(), p1.visible_word.as_str()],
                    cap,
                ),
                None => crate::ngram_lm::predict_next_words(&[p1.visible_word.as_str()], cap),
            };
            for w in corpus {
                if out.len() >= cap {
                    break;
                }
                if !out.iter().any(|existing| existing == &w) {
                    out.push(w);
                }
            }
        }
        out
    }

    /// Compute up to 3 alternative candidates for an already-committed word
    /// sitting immediately before the caret (e.g. `tét <-> test`, `lát <-> last`,
    /// or SymSpell typo fixes) for mid-sentence quick re-selection.
    pub fn alternatives_for_word(&self, word: &str, prev_word: Option<&str>) -> Vec<String> {
        crate::engine_helpers::compute_word_alternatives(
            word,
            prev_word,
            english_dict(),
            &self.user_dict,
            self.method,
            self.keyboard_layout,
            |r| self.compose_candidate(r),
        )
    }
}
