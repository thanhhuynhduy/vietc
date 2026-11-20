// SPDX-License-Identifier: MIT
//! Pure casing, boundary, and Telex transform classification helpers for `Engine`.

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

    let mark = if lower.ends_with("dd") {
        Some((TransformKind::StrokeDd, UndoPolicy::ConsumeTrigger))
    } else if lower.ends_with("aa") {
        Some((TransformKind::Mark("aa"), UndoPolicy::AppendTrigger))
    } else if lower.ends_with("ee") {
        Some((TransformKind::Mark("ee"), UndoPolicy::AppendTrigger))
    } else if lower.ends_with("oo") {
        Some((TransformKind::Mark("oo"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w' && has_u_horn && has_o_horn {
        Some((TransformKind::Mark("uo+w"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w' && has_u_horn && lower.contains("ua") {
        Some((TransformKind::Mark("ua+w"), UndoPolicy::AppendTrigger))
    } else if trigger == 'w' {
        let kind = if has_a_breve || lower.contains("aw") {
            "aw"
        } else if has_o_horn || lower.contains("ow") {
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

pub(crate) fn is_sentence_boundary(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ':' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '\n' | '\r' | '\t' | '…'
    )
}

pub(crate) fn is_flush_char(ch: char) -> bool {
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
    if (!composed.contains('Đ') && !composed.contains('đ')) || raw_lower == "ddos" {
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
        && alpha.len() >= 3
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
}
