// SPDX-License-Identifier: MIT
use crate::bamboo::{BambooEngine, TelexUndo};
use crate::english::EnglishDict;
use crate::event::{Command, EventStore};
use crate::input_method::InputMethod;
use crate::state::{
    BoundaryState, CaseShape, CompositionState, PhysicalEvent, Replacement, ReplacementReason,
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
    active_transform: Option<TransformIdentity>,
    generation: u64,
    visible_revision: u64,
    boundary: BoundaryState,
    last_transition: Option<TransitionKind>,
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
            active_transform: None,
            generation: 0,
            visible_revision: 0,
            boundary: BoundaryState::Committed,
            last_transition: None,
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

    /// Decide whether a committed word should be reverted to the raw keystrokes
    /// the user typed instead of the Vietnamese transformation. Returns true for
    /// words that are clearly English / non-Vietnamese: a known English word, a
    /// result that isn't a phonologically valid Vietnamese syllable, or one that
    /// contains letters foreign to Vietnamese. `composed` is the transformed
    /// output; `raw` is the literal keystrokes typed.
    pub fn should_restore_word(composed: &str, raw: &str) -> bool {
        // No transformation happened — English already passed through untouched.
        if composed == raw {
            return false;
        }

        // If the visible output is already pure ASCII, a double-key Telex undo
        // has already stripped all Vietnamese diacritics (e.g. "tesst" -> "test",
        // "tesster" -> "tester", "offfer" -> "offer", "isssue" -> "issue",
        // "lawws" -> "laws", "boooks" -> "books", "fixx" -> "fix", "ATXX" -> "ATX").
        // Keep the undone ASCII word on screen UNLESS `raw` is a natural English
        // word whose double letter (e.g. "issue", "offer", "error", "press",
        // "ass", "class", "password") was consumed by the undo, leaving a
        // truncated word like "isue", "as", "clas", or "pasword" on screen.
        if composed.is_ascii() {
            let dict = english_dict();
            let raw_lower = raw.to_lowercase();
            return dict.is_english_word(&raw_lower) || has_natural_english_double(&raw_lower);
        }

        // When the composed word carries Vietnamese diacritics, keep it when it
        // is a valid Vietnamese syllable present in the Vietnamese dictionary
        // (or a single-character vowel/stroke like "á", "ă", "đ").
        if !crate::spelling::is_valid_vietnamese_syllable(composed) {
            return true;
        }
        if composed.chars().count() <= 1 || composed.contains('đ') || composed.contains('Đ') {
            return !crate::spelling::is_in_vietnamese_dictionary(composed)
                && composed.chars().any(|c| crate::spelling::is_in_vietnamese_dictionary(&c.to_string()));
        }
        !crate::spelling::is_in_vietnamese_dictionary(composed)
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

    fn composed_tail(&self) -> String {
        if self.raw_buffer.is_empty() {
            return String::new();
        }
        if self.uses_recomposition() {
            crate::telex::resolve_and_render(&self.raw_buffer)
        } else {
            match_casing(&self.raw_buffer, &self.bamboo.get_output())
        }
    }

    fn visible_output(&self) -> String {
        if self.pasted_prefix.is_empty() {
            if self.uses_recomposition() {
                crate::telex::resolve_and_render(&self.raw_buffer)
            } else {
                self.bamboo.get_output()
            }
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
        self.raw_buffer.clear();
        self.pasted_prefix.clear();
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
        let event = EngineEvent::Paste(text.to_string());
        self.raw_buffer.push_str(text);
        event
    }

    pub fn replay_keystrokes(
        method: InputMethod,
        macros: &HashMap<String, String>,
        keystrokes: &[char],
    ) -> (String, bool) {
        let mut engine = Engine::new(method);
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
                engine.reset();
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

    /// Replay events through a fresh engine, returning (expected_output, did_flush).
    /// This is the Event Sourcing equivalent of replay_keystrokes.
    pub fn replay_events(
        method: InputMethod,
        macros: &HashMap<String, String>,
        events: &EventStore,
    ) -> (String, bool) {
        let mut engine = Engine::new(method);
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
                    engine.reset();
                }
                crate::event::InputEvent::Paste(text) => {
                    engine.reduce(PhysicalEvent::Paste(text.clone()));
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

    /// Event Sourcing + Command Pattern: replay events and return diff commands.
    /// Compares expected output against screen_output and generates backspace/type commands.
    pub fn replay_events_to_commands(
        method: InputMethod,
        macros: &HashMap<String, String>,
        events: &EventStore,
        screen_output: &str,
    ) -> Vec<Command> {
        let (new_output, _) = Engine::replay_events(method, macros, events);

        let mut commands = Vec::new();
        if new_output != screen_output {
            let diff = crate::telex::replacement_diff(screen_output, &new_output);
            if diff.delete_before_cursor > 0 {
                commands.push(Command::Backspace(diff.delete_before_cursor));
            }
            if !diff.insert.is_empty() {
                commands.push(Command::Type(diff.insert));
            }
        }
        commands
    }

    pub fn update_with_pasted_text(&mut self, text: &str) {
        self.raw_buffer.clear();
        self.pasted_prefix.clear();
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
        self.raw_buffer.push_str(text);
    }

    pub fn reset(&mut self) {
        self.bamboo.reset();
        self.raw_buffer.clear();
        self.pasted_prefix.clear();
        self.previous_screen_len = None;
        self.recovered_screen_len = None;
        self.active_transform = None;
        self.boundary = BoundaryState::Committed;
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
        self.active_transform = None;
        self.boundary = BoundaryState::Invalidated;
        self.last_transition = Some(TransitionKind::Invalidate);
    }

    pub fn recover_composition(&mut self) {
        self.visible_revision = self.visible_revision.saturating_sub(1);
        if let Some(prev_len) = self.previous_screen_len.take() {
            self.recovered_screen_len = Some(prev_len);
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
                self.paste_mode = false;
                return Some(EngineEvent::Flush(word));
            }
        }

        None
    }

    pub fn add_macro(&mut self, shortcut: String, expansion: String) {
        self.macros.insert(shortcut.clone(), expansion.clone());
        self.bamboo.add_macro(shortcut, expansion);
    }

    pub fn clear_macros(&mut self) {
        self.macros.clear();
        self.bamboo.clear_macros();
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

    pub fn process_key(&mut self, ch: char) -> Option<EngineEvent> {
        if !self.bamboo.is_enabled() {
            return Some(EngineEvent::Insert(ch.to_string()));
        }

        if ch == '\x08' {
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
                self.active_transform = None;
                self.boundary = BoundaryState::Committed;
                self.last_transition = Some(TransitionKind::Edit);
                return None;
            }
            let previous = self.composed_tail();
            let prev_len = self
                .recovered_screen_len
                .take()
                .unwrap_or_else(|| previous.chars().count());
            self.previous_screen_len = Some(prev_len);
            let _ = self.raw_buffer.pop();
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
            self.active_transform = None;
            self.boundary = if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
                BoundaryState::Committed
            } else {
                BoundaryState::Composing
            };
            self.last_transition = Some(TransitionKind::Edit);
            return Some(self.replacement(
                prev_len,
                self.composed_tail(),
                ReplacementReason::Edit,
            ));
        }

        if is_flush_char(ch) {
            if self.raw_buffer.is_empty() && self.pasted_prefix.is_empty() {
                return None;
            }

            let previous = self.composed_tail();
            let prev_len = self
                .recovered_screen_len
                .take()
                .unwrap_or_else(|| previous.chars().count());

            // Check for macro
            let macro_expansion = self.macros.get(&self.raw_buffer.to_lowercase()).cloned();
            if let Some(expansion) = macro_expansion {
                self.reset();
                self.previous_screen_len = Some(prev_len);
                return Some(self.replacement(prev_len, expansion, ReplacementReason::Macro));
            }

            let raw = self.raw_buffer.clone();
            self.reset();
            self.last_transition = Some(TransitionKind::Commit);
            if prev_len > 0 {
                // Auto-restore: if the committed word is English / not valid
                // Vietnamese, revert to the raw keystrokes the user typed.
                if self.auto_restore && Engine::should_restore_word(&previous, &raw) {
                    self.previous_screen_len = Some(prev_len);
                    return Some(self.replacement(prev_len, raw, ReplacementReason::AutoRestore));
                }
                // Normal case: the composed word is already correctly on screen.
                // Finalize and let the flush character through unchanged.
            }
            return None;
        }

        if self.method == InputMethod::Telex
            && ch.is_ascii_digit()
            && self.raw_buffer.is_empty()
            && self.pasted_prefix.is_empty()
            && !self
                .macros
                .keys()
                .any(|k| k.starts_with(&ch.to_string()))
        {
            return None;
        }

        let previous = self.composed_tail();
        let prev_len = self
            .recovered_screen_len
            .take()
            .unwrap_or_else(|| previous.chars().count());

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
        if self.boundary == BoundaryState::Committed {
            self.boundary = BoundaryState::Composing;
        }

        if self.bamboo.process_key(ch).is_some() {
            // Only emit Replace when Vietnamese processing CHANGED the output
            // (tone/mark keys). For simple appends, let the raw key go through.
            let expected = format!("{}{}", previous, ch);
            let cased = match_casing(&self.raw_buffer, &self.composed_tail());
            if cased != expected && cased != previous {
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
                self.boundary = BoundaryState::Composing;
                let reason = match self.last_transition {
                    Some(TransitionKind::ReplaceActive) => ReplacementReason::ReplaceTransform,
                    _ => ReplacementReason::ApplyTransform,
                };
                self.previous_screen_len = Some(prev_len);
                return Some(self.replacement(prev_len, cased, reason));
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

fn case_shape(raw: &str) -> CaseShape {
    let has_lower = raw.chars().any(char::is_lowercase);
    let has_upper = raw.chars().any(char::is_uppercase);
    match (has_lower, has_upper) {
        (false, true) => CaseShape::Upper,
        (true, true) => CaseShape::Mixed,
        _ => CaseShape::Lower,
    }
}

fn active_telex_transform(
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

fn is_flush_char(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
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
    )
}

fn match_casing(raw: &str, processed: &str) -> String {
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

fn has_natural_english_double(raw_lower: &str) -> bool {
    let chars: Vec<char> = raw_lower.chars().collect();
    if chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2]) {
        return false;
    }
    if raw_lower.ends_with("ss")
        && matches!(
            raw_lower,
            "ass"
                | "bass"
                | "boss"
                | "brass"
                | "bypass"
                | "chess"
                | "class"
                | "compass"
                | "cross"
                | "css"
                | "discuss"
                | "dress"
                | "express"
                | "fitness"
                | "floss"
                | "glass"
                | "gloss"
                | "grass"
                | "gross"
                | "guess"
                | "harass"
                | "headless"
                | "illness"
                | "impress"
                | "kiss"
                | "lass"
                | "less"
                | "loss"
                | "mass"
                | "mess"
                | "miss"
                | "moss"
                | "ness"
                | "pass"
                | "possess"
                | "press"
                | "process"
                | "progress"
                | "rss"
                | "serverless"
                | "stateless"
                | "stress"
                | "subclass"
                | "success"
                | "suppress"
                | "swiss"
                | "toss"
                | "truss"
                | "unless"
                | "useless"
                | "wellness"
                | "wireless"
                | "witness"
        )
    {
        return true;
    }
    if raw_lower.starts_with("passw")
        || raw_lower.starts_with("class")
        || raw_lower.starts_with("cross")
    {
        return true;
    }
    for i in 0..chars.len().saturating_sub(2) {
        let c0 = chars[i];
        let c1 = chars[i + 1];
        let c2 = chars[i + 2];
        if c0 == c1
            && matches!(c0, 's' | 'f' | 'r')
            && matches!(c2, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
        {
            return true;
        }
    }
    false
}
