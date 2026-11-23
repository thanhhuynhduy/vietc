// SPDX-License-Identifier: MIT
//! Pure casing, boundary, and Telex transform classification helpers for `Engine`.

use crate::english::{EnglishDict, LanguageContext};
use crate::ring_buffer::CommittedWordEntry;
use crate::state::{CaseShape, TransformKind, UndoPolicy};

pub(crate) fn case_shape(raw: &str) -> CaseShape {
    let has_lower = raw.chars().any(char::is_lowercase);
    let has_upper = raw.chars().any(char::is_uppercase);
    match (has_lower, has_upper) {
        (false, true) => CaseShape::Upper,
        (true, true) => CaseShape::Mixed,
        _ => CaseShape::Lower,
    }
}

pub(crate) fn active_telex_transform(
    raw: &str,
    ch: char,
    visible: &str,
) -> Option<(TransformKind, char, UndoPolicy)> {
    let trigger = ch.to_ascii_lowercase();
    if matches!(trigger, 'f' | 's' | 'r' | 'x' | 'j') {
        if visible
            .chars()
            .last()
            .is_some_and(|last| last.to_ascii_lowercase() == trigger)
        {
            return None;
        }
        return Some((
            TransformKind::Tone(trigger),
            trigger,
            UndoPolicy::AppendTrigger,
        ));
    }

    let lower = raw.to_ascii_lowercase();
    let vis_lower = visible.to_lowercase();
    let has_u_horn = vis_lower
        .chars()
        .any(|c| matches!(c, 'ư' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự'));
    let has_o_horn = vis_lower
        .chars()
        .any(|c| matches!(c, 'ơ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ'));
    let has_a_breve = vis_lower
        .chars()
        .any(|c| matches!(c, 'ă' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ'));

    let mark = if lower.ends_with("dd") && (vis_lower.contains('đ')) {
        Some((TransformKind::StrokeDd, UndoPolicy::ConsumeTrigger))
    } else if lower.ends_with("aa")
        && vis_lower
            .chars()
            .any(|c| matches!(c, 'â' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ'))
    {
        Some((TransformKind::Mark("aa"), UndoPolicy::AppendTrigger))
    } else if lower.ends_with("ee")
        && vis_lower
            .chars()
            .any(|c| matches!(c, 'ê' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ'))
    {
        Some((TransformKind::Mark("ee"), UndoPolicy::AppendTrigger))
    } else if lower.ends_with("oo")
        && vis_lower
            .chars()
            .any(|c| matches!(c, 'ô' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ'))
    {
        Some((TransformKind::Mark("oo"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w' && has_u_horn && has_o_horn {
        Some((TransformKind::Mark("uo+w"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w' && has_u_horn && lower.contains("ua") {
        Some((TransformKind::Mark("ua+w"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w'
        && (has_a_breve || has_o_horn || has_u_horn)
        && !vis_lower.ends_with('w')
    {
        let kind = if has_a_breve {
            "aw"
        } else if has_o_horn {
            "ow"
        } else {
            "uw"
        };
        Some((TransformKind::Mark(kind), UndoPolicy::AppendTrigger))
    } else {
        None
    };

    mark.map(|(kind, policy)| (kind, trigger, policy))
}

pub(crate) fn active_vni_transform(
    ch: char,
    visible: &str,
) -> Option<(TransformKind, char, UndoPolicy)> {
    if !matches!(ch, '1'..='9') || visible.ends_with(ch) {
        return None;
    }
    let kind = match ch {
        '1'..='5' => TransformKind::Tone(ch),
        '6' => TransformKind::Mark("6"),
        '7' => TransformKind::Mark("7"),
        '8' => TransformKind::Mark("8"),
        '9' => TransformKind::StrokeDd,
        _ => return None,
    };
    Some((kind, ch, UndoPolicy::AppendTrigger))
}

pub(crate) fn active_transform_for_method(
    method: crate::InputMethod,
    raw: &str,
    ch: char,
    visible: &str,
) -> Option<(TransformKind, char, UndoPolicy)> {
    match method {
        crate::InputMethod::Telex => active_telex_transform(raw, ch, visible),
        crate::InputMethod::Vni => active_vni_transform(ch, visible),
    }
}

pub(crate) fn is_tone_strip_key(
    method: crate::InputMethod,
    ch: char,
    previous: &str,
    cased: &str,
) -> bool {
    let stripped = (method == crate::InputMethod::Telex
        && ch.eq_ignore_ascii_case(&'z')
        && !cased.ends_with(['z', 'Z']))
        || (method == crate::InputMethod::Vni && ch == '0' && !cased.ends_with('0'));
    stripped && !previous.is_ascii() && cased.chars().count() == previous.chars().count()
}

pub(crate) fn build_surrounding_prev_entry(
    prev_word: &str,
    dict: &crate::english::EnglishDict,
) -> (
    crate::ring_buffer::CommittedWordEntry,
    crate::english::LanguageContext,
) {
    let ctx = dict.classify_committed_word_with_prior(
        prev_word,
        prev_word,
        crate::english::LanguageContext::None,
    );
    let entry = crate::ring_buffer::CommittedWordEntry {
        raw_keys: prev_word.to_string(),
        visible_word: prev_word.to_string(),
        trailing_delimiters: " ".to_string(),
        prior_context: crate::english::LanguageContext::None,
        committed_context: ctx,
        was_auto_restored: ctx == crate::english::LanguageContext::English,
        user_override_vn: false,
    };
    (entry, ctx)
}

pub(crate) fn build_composition_state(
    raw_keys: String,
    visible: String,
    active_transform: Option<crate::state::TransformIdentity>,
    boundary: crate::state::BoundaryState,
    generation: u64,
    last_transition: Option<crate::state::TransitionKind>,
) -> crate::state::CompositionState {
    crate::state::CompositionState {
        case_shape: case_shape(&raw_keys),
        raw_keys,
        validity: if visible.is_empty() {
            crate::state::SpellingValidity::Unknown
        } else if crate::spelling::is_valid_vietnamese_syllable(&visible) {
            crate::state::SpellingValidity::Valid
        } else {
            crate::state::SpellingValidity::Invalid
        },
        visible,
        active_transform,
        boundary,
        generation,
        last_transition,
    }
}

pub(crate) fn is_sentence_boundary(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ':' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '\n' | '\r' | '\t' | '…'
    )
}

pub(crate) fn replay_bamboo(bamboo: &mut crate::bamboo::BambooEngine, raw: &str) {
    bamboo.reset();
    let mut replayed = String::new();
    for rk in raw.chars() {
        if let Some(undo) = bamboo.undo_telex_transform(rk, &replayed) {
            let consumed_lit = if undo == crate::bamboo::TelexUndo::ConsumeTrigger {
                replayed.chars().last()
            } else {
                None
            };
            replayed.push(rk);
            match undo {
                crate::bamboo::TelexUndo::AppendTrigger => bamboo.append_literal(rk),
                crate::bamboo::TelexUndo::ConsumeTrigger => {
                    if let Some(lit) = consumed_lit {
                        bamboo.append_literal(lit);
                    }
                }
            }
        } else {
            replayed.push(rk);
            let _ = bamboo.process_key(rk);
        }
    }
}

pub(crate) fn compose_with_bamboo(method: crate::input_method::InputMethod, raw: &str) -> String {
    let mut tmp = crate::bamboo::BambooEngine::new(method);
    replay_bamboo(&mut tmp, raw);
    match_casing(raw, &tmp.get_output())
}

pub(crate) fn telex_undo_policy(kind: TransformKind) -> Option<UndoPolicy> {
    match kind {
        TransformKind::Tone('f' | 's' | 'r' | 'x' | 'j') => Some(UndoPolicy::AppendTrigger),
        TransformKind::Mark("aw" | "ow" | "uw" | "uo+w" | "ua+w" | "aa" | "ee" | "oo") => {
            Some(UndoPolicy::AppendTrigger)
        }
        TransformKind::StrokeDd => Some(UndoPolicy::ConsumeTrigger),
        _ => None,
    }
}

pub(crate) fn is_seedable_surrounding_word(visible_word: &str) -> bool {
    !visible_word.is_empty()
        && visible_word.chars().count() <= 24
        && visible_word.chars().all(char::is_alphabetic)
        && crate::spelling::is_valid_vietnamese_prefix(visible_word)
}

pub fn is_flush_char(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\u{00A0}'
            | '.'
            | ','
            | '!'
            | '?'
            | ';'
            | ':'
            | '/'
            | '\\'
            | '-'
            | '_'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '='
            | '+'
            | '*'
            | '&'
            | '%'
            | '$'
            | '#'
            | '@'
            | '~'
            | '`'
            | '"'
            | '\''
            | '|'
            | '^'
            | '—'
            | '–'
            | '…'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '«'
            | '»'
    )
}

pub(crate) fn adapt_macro_casing(raw_shortcut: &str, expansion: &str) -> String {
    if expansion.chars().any(|c| c.is_uppercase()) {
        return expansion.to_string();
    }
    let alpha: Vec<char> = raw_shortcut.chars().filter(|c| c.is_alphabetic()).collect();
    if alpha.is_empty() {
        return expansion.to_string();
    }
    if alpha.iter().all(|c| c.is_uppercase()) {
        expansion.to_uppercase()
    } else if alpha[0].is_uppercase() {
        let mut chars = expansion.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => expansion.to_string(),
        }
    } else {
        expansion.to_string()
    }
}

pub(crate) fn is_vn_d_abbreviation(composed: &str, raw_lower: &str) -> bool {
    if (!composed.contains('Đ') && !composed.contains('đ'))
        || raw_lower == "ddos"
        || raw_lower.bytes().any(|b| b.is_ascii_digit())
    {
        return false;
    }
    let alpha: Vec<char> = composed.chars().filter(|c| c.is_alphabetic()).collect();
    let all_upper = !alpha.is_empty() && alpha.iter().all(|c| c.is_uppercase());
    let all_consonants = alpha
        .iter()
        .all(|&c| !matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y'));
    all_upper || all_consonants
}

pub(crate) fn match_casing(raw: &str, processed: &str) -> String {
    if raw.is_empty() || processed.is_empty() {
        return processed.to_string();
    }

    let alpha: Vec<char> = raw.chars().filter(|c| c.is_alphabetic()).collect();
    if alpha.is_empty() {
        return processed.to_string();
    }

    let all_upper = alpha.iter().all(|c| c.is_uppercase());
    if all_upper {
        processed.to_uppercase()
    } else if alpha[0].is_lowercase()
        && !processed.is_ascii()
        && !raw.eq_ignore_ascii_case("ddos")
        && alpha.len() >= 5
        && alpha[1..].iter().filter(|c| c.is_uppercase()).count() == 1
        && processed.chars().skip(1).any(|c| c.is_uppercase())
        && crate::spelling::is_valid_vietnamese_syllable(processed)
        && crate::spelling::is_in_vietnamese_dictionary(processed)
    {
        processed.to_lowercase()
    } else if alpha[0].is_uppercase() {
        let mut chars = processed.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => processed.to_string(),
        }
    } else {
        processed.to_string()
    }
}

/// Resolve canonical raw buffer for replacing a 1-char or 2-char digraph coda (`ng`, `nh`, `ch`)
/// after Backspace on a closed accented syllable (`vieetj`+BS+`c` -> `việc`, `tieengs`+BS+`t` -> `tiết`).
pub(crate) fn resolve_coda_edit_canon(
    coda_canon: &str,
    ch: char,
    method: crate::InputMethod,
    compose: impl Fn(&str) -> String,
) -> Option<String> {
    let edit_cand = compose(&format!("{coda_canon}{ch}"));
    if crate::spelling::is_valid_vietnamese_prefix(&edit_cand) {
        return Some(coda_canon.to_string());
    }
    // Digraph coda (`ng`, `nh`, `ch`): after stripping the last char, `coda_vis` ends in `n` or `c`.
    // Strip that remaining digraph lead consonant so typing a new coda (`t`, `p`, `m`, `c`...) succeeds.
    let mut coda_vis = compose(coda_canon);
    let last_lo = coda_vis.chars().last().map(|c| c.to_ascii_lowercase());
    if matches!(last_lo, Some('n' | 'c')) {
        coda_vis.pop();
        if let Some(shorter_canon) = crate::ring_buffer::visible_to_canonical_raw(&coda_vis, method)
        {
            let shorter_cand = compose(&format!("{shorter_canon}{ch}"));
            if crate::spelling::is_valid_vietnamese_prefix(&shorter_cand) {
                return Some(shorter_canon);
            }
        }
    }
    None
}

pub(crate) enum RehydrationDecision {
    Keep,
    ReplaceRaw(String),
    ClearWord,
}

/// Evaluate Invariant 1 (Guarded Rehydration) when a character `ch` is typed
/// immediately after Backspace deleted the trailing Space of a committed word.
pub(crate) fn evaluate_guarded_rehydration(
    raw_buffer: &str,
    current_visible: &str,
    ch: char,
    method: crate::InputMethod,
    compose: impl Fn(&str) -> String,
) -> RehydrationDecision {
    let cand = compose(&format!("{raw_buffer}{ch}"));
    let is_tone_strip = (method == crate::InputMethod::Telex && ch.eq_ignore_ascii_case(&'z'))
        || (method == crate::InputMethod::Vni && ch == '0');
    let is_valid_mod = cand != current_visible
        && crate::spelling::is_valid_vietnamese_prefix(&cand)
        && (is_tone_strip || crate::spelling::is_in_vietnamese_dictionary_or_prefix(&cand));
    if is_valid_mod {
        return RehydrationDecision::Keep;
    }
    if let Some(resolved) = resolve_coda_edit_canon(raw_buffer, ch, method, &compose) {
        let edited = compose(&format!("{resolved}{ch}"));
        if crate::spelling::is_in_vietnamese_dictionary_or_prefix(&edited) {
            return RehydrationDecision::ReplaceRaw(resolved);
        }
    }
    RehydrationDecision::ClearWord
}

impl crate::Engine {
    pub fn replay_keystrokes(
        method: crate::InputMethod,
        macros: &std::collections::HashMap<String, String>,
        keystrokes: &[char],
    ) -> (String, bool) {
        let mut engine = crate::Engine::new(method);
        for (shortcut, expansion) in macros {
            engine.add_macro(shortcut.clone(), expansion.clone());
        }

        let mut last_output = String::new();
        let mut composing = String::new();

        for &ch in keystrokes {
            if ch == '\x08' {
                let _ = engine.process_key(ch);
                composing = engine.buffer();
                last_output = composing.clone();
                continue;
            }

            if is_flush_char(ch) {
                if !composing.is_empty() {
                    last_output = composing.clone();
                }
                composing.clear();
                engine.clear_word_only();
                continue;
            }

            let _ = engine.process_key(ch);
            composing = engine.buffer();
            last_output = composing.clone();
        }

        let output = engine.visible_output();
        if !output.is_empty() {
            last_output = output.clone();
        }

        let did_flush = output.is_empty() && composing.is_empty();
        (
            if did_flush {
                String::new()
            } else {
                last_output
            },
            did_flush,
        )
    }

    pub fn replay_events(
        method: crate::InputMethod,
        macros: &std::collections::HashMap<String, String>,
        events: &crate::event::EventStore,
    ) -> (String, bool) {
        let mut engine = crate::Engine::new(method);
        for (shortcut, expansion) in macros {
            engine.add_macro(shortcut.clone(), expansion.clone());
        }

        let mut last_output = String::new();
        let mut composing = String::new();

        for event in events.iter() {
            match event {
                crate::event::InputEvent::KeyTyped(ch) => {
                    let _ = engine.process_key(*ch);
                    composing = engine.buffer();
                    last_output = composing.clone();
                }
                crate::event::InputEvent::Backspace => {
                    let _ = engine.process_key('\x08');
                    composing = engine.buffer();
                    last_output = composing.clone();
                }
                crate::event::InputEvent::Flush(_) => {
                    if !composing.is_empty() {
                        last_output = composing.clone();
                    }
                    composing.clear();
                    engine.clear_word_only();
                }
                crate::event::InputEvent::Paste(text) => {
                    engine.reduce(crate::state::PhysicalEvent::Paste(text.clone()));
                    composing = engine.buffer();
                    last_output = composing.clone();
                }
            }
        }

        let output = engine.visible_output();
        let output_is_empty = output.is_empty();
        if !output.is_empty() {
            last_output = output;
        }

        let did_flush = output_is_empty && composing.is_empty();
        (
            if did_flush {
                String::new()
            } else {
                last_output
            },
            did_flush,
        )
    }

    pub fn replay_events_to_commands(
        method: crate::InputMethod,
        macros: &std::collections::HashMap<String, String>,
        events: &crate::event::EventStore,
        screen_output: &str,
    ) -> Vec<crate::event::Command> {
        let (new_output, _) = crate::Engine::replay_events(method, macros, events);

        let mut commands = Vec::new();
        if new_output != screen_output {
            let diff = crate::telex::replacement_diff(screen_output, &new_output);
            if diff.delete_before_cursor > 0 {
                commands.push(crate::event::Command::Backspace(diff.delete_before_cursor));
            }
            if !diff.insert.is_empty() {
                commands.push(crate::event::Command::Type(diff.insert));
            }
        }
        commands
    }

    pub fn seed_surrounding_word(&mut self, visible_word: &str) -> bool {
        self.seed_surrounding_context(Some(visible_word), None, false)
    }

    pub fn telex_undo_policy(
        kind: crate::state::TransformKind,
    ) -> Option<crate::state::UndoPolicy> {
        crate::engine_helpers::telex_undo_policy(kind)
    }

    pub fn should_restore_word(composed: &str, raw: &str) -> bool {
        Self::should_restore_word_with_context(composed, raw, crate::english::LanguageContext::None)
    }

    pub fn should_restore_word_with_context(
        composed: &str,
        raw: &str,
        prev_context: crate::english::LanguageContext,
    ) -> bool {
        crate::english::should_restore_word_decision(
            crate::engine::english_dict(),
            composed,
            raw,
            prev_context,
            None,
            None,
        )
    }

    pub fn reduce(&mut self, event: crate::state::PhysicalEvent) -> Option<crate::EngineEvent> {
        use crate::state::PhysicalEvent;
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
                self.update_with_pasted_text(&text);
                None
            }
            PhysicalEvent::ReplacementFailed => {
                self.recover_composition();
                None
            }
        }
    }
}

pub(crate) fn commit_rehydrated_ascii_word(
    ring_buffer: &mut crate::ring_buffer::CommittedRingBuffer,
    dict: &EnglishDict,
    word: String,
    prior_context: LanguageContext,
    ch: char,
) -> LanguageContext {
    let committed_context = if is_sentence_boundary(ch) {
        LanguageContext::None
    } else {
        dict.classify_committed_word_with_prior(&word, &word, prior_context)
    };
    if ch == ' ' {
        ring_buffer.push(CommittedWordEntry {
            raw_keys: word.clone(),
            visible_word: word,
            trailing_delimiters: " ".to_string(),
            prior_context,
            committed_context,
            was_auto_restored: true,
            user_override_vn: false,
        });
    } else {
        ring_buffer.clear();
    }
    committed_context
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_retroactive_pair(
    ring_buffer: &mut crate::ring_buffer::CommittedRingBuffer,
    prev_entry: CommittedWordEntry,
    corrected_w1: String,
    w1_restored: bool,
    raw: String,
    final_word: String,
    autocorrected: Option<String>,
    prev_len: usize,
    ch: char,
    should_restore: bool,
    user_override: bool,
) -> (usize, String, LanguageContext) {
    let _ = ring_buffer.pop_last();
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
    if ch == ' ' {
        let w1_ctx = if w1_restored {
            LanguageContext::English
        } else {
            LanguageContext::Vietnamese
        };
        ring_buffer.push(CommittedWordEntry {
            visible_word: corrected_w1,
            was_auto_restored: w1_restored,
            committed_context: w1_ctx,
            ..prev_entry
        });
        ring_buffer.push(CommittedWordEntry {
            raw_keys: raw,
            visible_word: effective_w2,
            trailing_delimiters: " ".to_string(),
            prior_context: w1_ctx,
            committed_context: updated_ctx,
            was_auto_restored: should_restore || w1_restored,
            user_override_vn: user_override,
        });
    } else {
        ring_buffer.clear();
    }
    (total_delete, combined_insert, updated_ctx)
}

pub(crate) fn apply_bamboo_telex_undo(
    bamboo: &mut crate::bamboo::BambooEngine,
    raw_buffer: &mut String,
    ch: char,
) -> bool {
    let Some(undo) = bamboo.undo_telex_transform(ch, raw_buffer) else {
        return false;
    };
    let consumed_stroke_literal = if undo == crate::bamboo::TelexUndo::ConsumeTrigger {
        raw_buffer.chars().last()
    } else {
        None
    };
    raw_buffer.push(ch);
    match undo {
        crate::bamboo::TelexUndo::AppendTrigger => bamboo.append_literal(ch),
        crate::bamboo::TelexUndo::ConsumeTrigger => {
            if let Some(literal) = consumed_stroke_literal {
                bamboo.append_literal(literal);
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_pending_restore_undo(
    undo: crate::user_dict::AutoRestoreUndo,
    ring_buffer: &mut crate::ring_buffer::CommittedRingBuffer,
    user_dict: &mut crate::user_dict::UserDict,
    bamboo: &mut crate::bamboo::BambooEngine,
    raw_buffer: &mut String,
    prev_context: &mut LanguageContext,
    override_keep_vietnamese: &mut bool,
    rehydrated_word: &mut bool,
    rehydrated_ascii_only: &mut bool,
) -> (usize, String) {
    use crate::user_dict::AutoRestoreUndo;
    match undo {
        AutoRestoreUndo::SingleWord {
            restored_ascii,
            original_vn,
            raw_keys,
            boundary_char,
            is_autocorrect,
        } => {
            let _ = ring_buffer.pop_last();
            if is_autocorrect {
                user_dict.record_keep_raw(&raw_keys);
            } else {
                user_dict.record_keep_vn(&raw_keys);
            }
            *raw_buffer = raw_keys;
            if !is_autocorrect {
                replay_bamboo(bamboo, raw_buffer);
            } else {
                bamboo.reset();
            }
            *override_keep_vietnamese = true;
            *rehydrated_word = is_autocorrect;
            *rehydrated_ascii_only = is_autocorrect;
            (
                restored_ascii.chars().count() + boundary_char.len_utf8().min(1),
                original_vn,
            )
        }
        AutoRestoreUndo::RetroactivePair {
            rewritten_pair,
            original_pair,
            original_w1_entry,
            w2_raw_keys,
            boundary_char,
        } => {
            let _ = ring_buffer.pop_last();
            let _ = ring_buffer.pop_last();
            *prev_context = original_w1_entry.committed_context;
            ring_buffer.push(original_w1_entry);
            *raw_buffer = w2_raw_keys;
            replay_bamboo(bamboo, raw_buffer);
            *override_keep_vietnamese = true;
            (
                rewritten_pair.chars().count() + boundary_char.len_utf8().min(1),
                original_pair,
            )
        }
    }
}
