// SPDX-License-Identifier: MIT
use std::collections::HashMap;
use vietc_engine::{
    BoundaryState, CaseShape, Engine, EngineEvent, InputMethod, PhysicalEvent, SpellingValidity,
    TransformKind, TransitionKind, UndoPolicy,
};

const SMOKE_SEEDS: [u64; 8] = [
    0x4154_5858,
    0x5445_4c45,
    0x4444_4457,
    0x544f_4e45,
    0x4341_5345,
    0x4544_4954,
    0x5245_504c,
    0x5354_414c,
];

fn property_budget() -> usize {
    std::env::var("VIETC_PROPERTY_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            if std::env::var("VIETC_PROPERTY_PROFILE").as_deref() == Ok("exhaustive") {
                4096
            } else {
                64
            }
        })
}

fn property_seeds() -> Vec<u64> {
    if std::env::var("VIETC_PROPERTY_PROFILE").as_deref() == Ok("exhaustive") {
        (0..256)
            .map(|index| {
                0x5649_4554_4300_0000_u64 ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            })
            .collect()
    } else {
        SMOKE_SEEDS.to_vec()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModelEvent {
    Printable(char),
    Backspace,
    Boundary(char),
    Delete,
    Navigate,
    Paste(String),
    Focus,
    ExternalEdit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ReferenceModel {
    raw: String,
    visible: String,
    boundary: BoundaryStateModel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BoundaryStateModel {
    Composing,
    #[default]
    Committed,
    Invalidated,
}

impl ReferenceModel {
    fn reduce(&mut self, event: &ModelEvent) {
        match event {
            ModelEvent::Printable(ch) => {
                self.raw.push(*ch);
                self.visible = reference_telex(&self.raw);
                self.boundary = BoundaryStateModel::Composing;
            }
            ModelEvent::Backspace => {
                self.raw.pop();
                self.visible = reference_telex(&self.raw);
                self.boundary = if self.raw.is_empty() {
                    BoundaryStateModel::Committed
                } else {
                    BoundaryStateModel::Composing
                };
            }
            ModelEvent::Boundary(_) => {
                self.raw.clear();
                self.visible.clear();
                self.boundary = BoundaryStateModel::Committed;
            }
            ModelEvent::Delete
            | ModelEvent::Navigate
            | ModelEvent::Focus
            | ModelEvent::ExternalEdit => {
                self.raw.clear();
                self.visible.clear();
                self.boundary = BoundaryStateModel::Invalidated;
            }
            ModelEvent::Paste(text) => {
                self.raw = text.clone();
                self.visible = text.clone();
                self.boundary = BoundaryStateModel::Invalidated;
            }
        }
    }
}

fn tone(base: char, trigger: char) -> Option<char> {
    const TABLE: &[(char, &str)] = &[
        ('a', "àáảãạ"),
        ('ă', "ằắẳẵặ"),
        ('â', "ầấẩẫậ"),
        ('e', "èéẻẽẹ"),
        ('ê', "ềếểễệ"),
        ('i', "ìíỉĩị"),
        ('o', "òóỏõọ"),
        ('ô', "ồốổỗộ"),
        ('ơ', "ờớởỡợ"),
        ('u', "ùúủũụ"),
        ('ư', "ừứửữự"),
        ('y', "ỳýỷỹỵ"),
    ];
    let index = match trigger {
        'f' => 0,
        's' => 1,
        'r' => 2,
        'x' => 3,
        'j' => 4,
        _ => return None,
    };
    TABLE
        .iter()
        .find(|(candidate, _)| *candidate == base)
        .and_then(|(_, values)| values.chars().nth(index))
}

fn plain_vowel(ch: char) -> char {
    match ch.to_ascii_lowercase() {
        'à' | 'á' | 'ả' | 'ã' | 'ạ' => 'a',
        'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' => 'ă',
        'ầ' | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'â',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' => 'e',
        'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'ê',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' => 'o',
        'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' => 'ô',
        'ờ' | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'ơ',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' => 'u',
        'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'ư',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        other => other,
    }
}

fn tone_position(chars: &[char]) -> Option<usize> {
    let mut vowels: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter_map(|(index, ch)| "aeiouyăâêôơư".contains(plain_vowel(*ch)).then_some(index))
        .collect();

    if vowels.len() >= 2 && vowels[0] == 1 {
        let onset = chars[0].to_ascii_lowercase();
        let glide = chars[1].to_ascii_lowercase();
        if (onset == 'q' && glide == 'u') || (onset == 'g' && glide == 'i') {
            vowels.remove(0);
        }
    }

    match vowels.as_slice() {
        [] => None,
        [only] => Some(*only),
        many => {
            let a = plain_vowel(chars[many[many.len() - 2]]);
            let b = plain_vowel(chars[many[many.len() - 1]]);
            if matches!(
                (a, b),
                ('o', 'a')
                    | ('o', 'e')
                    | ('u', 'y')
                    | ('i', 'ê')
                    | ('y', 'ê')
                    | ('u', 'ô')
                    | ('ư', 'ơ')
                    | ('i', 'o')
                    | ('u', 'â')
                    | ('u', 'ê')
                    | ('u', 'ơ')
            ) {
                Some(many[many.len() - 1])
            } else if many.len() >= 3 {
                Some(many[1])
            } else {
                Some(many[0])
            }
        }
    }
}

fn apply_case(source: char, replacement: char) -> char {
    if source.is_uppercase() {
        replacement.to_uppercase().next().unwrap_or(replacement)
    } else {
        replacement
    }
}

fn reference_telex(raw: &str) -> String {
    let mut out = Vec::new();
    let mut active_tone: Option<char> = None;
    let mut active_mark: Option<(char, usize, char)> = None;

    for input in raw.chars() {
        let lower = input.to_ascii_lowercase();

        if matches!(lower, 'f' | 's' | 'r' | 'x' | 'j') {
            if active_tone == Some(lower) {
                if let Some(position) = tone_position(&out) {
                    let source = out[position];
                    let plain = match source.to_ascii_lowercase() {
                        'à' | 'á' | 'ả' | 'ã' | 'ạ' => 'a',
                        'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' => 'ă',
                        'ầ' | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'â',
                        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' => 'e',
                        'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'ê',
                        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
                        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' => 'o',
                        'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' => 'ô',
                        'ờ' | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'ơ',
                        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' => 'u',
                        'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'ư',
                        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
                        other => other,
                    };
                    out[position] = apply_case(source, plain);
                    out.push(input);
                    active_tone = None;
                    continue;
                }
            }
            if let Some(position) = tone_position(&out) {
                let source = out[position];
                let plain = match source.to_ascii_lowercase() {
                    'à' | 'á' | 'ả' | 'ã' | 'ạ' => 'a',
                    'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' => 'ă',
                    'â' | 'ầ' | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'â',
                    'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' => 'e',
                    'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'ê',
                    'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
                    'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' => 'o',
                    'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' => 'ô',
                    'ơ' | 'ờ' | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'ơ',
                    'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' => 'u',
                    'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'ư',
                    'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
                    other => other,
                };
                if let Some(replacement) = tone(plain, lower) {
                    out[position] = apply_case(source, replacement);
                    active_tone = Some(lower);
                    continue;
                }
            }
        }

        if lower == 'd' {
            if let Some(('d', index, consumed_trigger)) = active_mark {
                let source = out[index];
                out[index] = apply_case(source, 'd');
                out.push(consumed_trigger);
                active_mark = None;
                continue;
            }
            if out.last().is_some_and(|ch| ch.eq_ignore_ascii_case(&'d')) {
                let index = out.len() - 1;
                let source = out[index];
                out[index] = apply_case(source, 'đ');
                active_mark = Some(('d', index, input));
                continue;
            }
        }

        if matches!(lower, 'a' | 'e' | 'o')
            && out.last().is_some_and(|ch| ch.eq_ignore_ascii_case(&lower))
        {
            let index = out.len() - 1;
            let source = out[index];
            let replacement = match lower {
                'a' => 'â',
                'e' => 'ê',
                'o' => 'ô',
                _ => unreachable!(),
            };
            out[index] = apply_case(source, replacement);
            active_mark = Some((lower, index, input));
            continue;
        }

        if lower == 'w' {
            let plain = out.iter().map(|ch| plain_vowel(*ch)).collect::<Vec<_>>();

            if plain.len() >= 2 {
                let o_index = plain.len() - 1;
                if plain[o_index] == 'o' && plain[o_index - 1] == 'u' {
                    let u_index = o_index - 1;
                    let u_source = out[u_index];
                    let o_source = out[o_index];
                    out[u_index] = apply_case(u_source, 'ư');
                    out[o_index] = apply_case(o_source, 'ơ');
                    active_mark = Some(('w', u_index, input));
                    continue;
                }

                let a_index = plain.len() - 1;
                if plain[a_index] == 'a' && plain[a_index - 1] == 'u' {
                    let u_index = a_index - 1;
                    let preceded_by_q = u_index > 0 && out[u_index - 1].eq_ignore_ascii_case(&'q');
                    if !preceded_by_q {
                        let source = out[u_index];
                        out[u_index] = apply_case(source, 'ư');
                        active_mark = Some(('w', u_index, input));
                        continue;
                    }
                }
            }

            if let Some(index) = out
                .iter()
                .rposition(|ch| matches!(plain_vowel(*ch), 'a' | 'o' | 'u'))
            {
                let source = out[index];
                let replacement = match plain_vowel(source) {
                    'a' => 'ă',
                    'o' => 'ơ',
                    'u' => 'ư',
                    _ => source,
                };
                out[index] = apply_case(source, replacement);
                active_mark = Some(('w', index, input));
                continue;
            }
        }

        out.push(input);
        active_tone = None;
    }

    out.into_iter().collect()
}

fn production_output(events: &[ModelEvent], auto_restore: bool) -> (String, BoundaryState) {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(auto_restore);
    for event in events {
        let physical = match event {
            ModelEvent::Printable(ch) => PhysicalEvent::Printable(*ch),
            ModelEvent::Backspace => PhysicalEvent::Backspace,
            ModelEvent::Boundary(ch) => PhysicalEvent::WordBoundary(*ch),
            ModelEvent::Delete => PhysicalEvent::Delete,
            ModelEvent::Navigate => PhysicalEvent::Navigate,
            ModelEvent::Paste(text) => PhysicalEvent::Paste(text.clone()),
            ModelEvent::Focus => PhysicalEvent::FocusChanged,
            ModelEvent::ExternalEdit => PhysicalEvent::ExternalEdit,
        };
        engine.reduce(physical);
    }
    (engine.buffer(), engine.state().boundary)
}

fn minimized_failure(
    events: &[ModelEvent],
    fails: impl Fn(&[ModelEvent]) -> bool,
) -> Vec<ModelEvent> {
    let mut minimized = events.to_vec();
    let mut index = 0;
    while index < minimized.len() {
        let mut candidate = minimized.clone();
        candidate.remove(index);
        if !candidate.is_empty() && fails(&candidate) {
            minimized = candidate;
        } else {
            index += 1;
        }
    }
    minimized
}

fn assert_trace(seed: u64, events: &[ModelEvent]) {
    let mut model = ReferenceModel::default();
    for event in events {
        model.reduce(event);
    }
    let actual = production_output(events, false);
    let expected_boundary = match model.boundary {
        BoundaryStateModel::Composing => BoundaryState::Composing,
        BoundaryStateModel::Committed => BoundaryState::Committed,
        BoundaryStateModel::Invalidated => BoundaryState::Invalidated,
    };
    if actual != (model.visible.clone(), expected_boundary) {
        let minimized = minimized_failure(events, |candidate| {
            let mut reference = ReferenceModel::default();
            for event in candidate {
                reference.reduce(event);
            }
            let boundary = match reference.boundary {
                BoundaryStateModel::Composing => BoundaryState::Composing,
                BoundaryStateModel::Committed => BoundaryState::Committed,
                BoundaryStateModel::Invalidated => BoundaryState::Invalidated,
            };
            production_output(candidate, false) != (reference.visible, boundary)
        });
        panic!(
            "property failure seed={seed:#x}\ntrace={events:?}\nminimized={minimized:?}\nexpected=({:?}, {:?}) actual={actual:?}",
            model.visible, expected_boundary
        );
    }
}

#[test]
fn generated_trigger_case_vowel_and_replacement_matrix_matches_reference() {
    let tones = ['f', 's', 'r', 'x', 'j'];
    let stems = ["a", "A", "aA", "tieeng", "nguyeen", "tram", "quang"];
    for (seed_index, seed) in property_seeds().into_iter().enumerate() {
        for stem in stems {
            for trigger in tones {
                let input = format!("{stem}{trigger}");
                let events = input.chars().map(ModelEvent::Printable).collect::<Vec<_>>();
                assert_trace(seed ^ seed_index as u64 ^ trigger as u64, &events);
            }
        }
    }

    for input in [
        "aw", "ow", "uw", "aa", "ee", "oo", "dd", "ddd", "DDd", "dDd", "asf", "tieengsf", "tramr",
        "quawng", "xuaw",
    ] {
        let events = input.chars().map(ModelEvent::Printable).collect::<Vec<_>>();
        assert_trace(0x434f_5250 ^ input.len() as u64, &events);
    }
}

#[test]
fn deterministic_seeded_traces_are_replay_equivalent() {
    for seed in property_seeds() {
        let mut state = seed;
        let alphabet = b"aeioudtnmfgwsrxj";
        let mut events = Vec::new();
        for _ in 0..property_budget() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let ch = alphabet[(state as usize) % alphabet.len()] as char;
            events.push(ModelEvent::Printable(ch));
            if state % 29 == 0 {
                events.push(ModelEvent::Backspace);
            }
        }

        let first = production_output(&events, false);
        let second = production_output(&events, false);
        assert_eq!(first, second, "seed={seed:#x}");

        let chars = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::Printable(ch) => Some(*ch),
                ModelEvent::Backspace => Some('\x08'),
                _ => None,
            })
            .collect::<Vec<_>>();
        let replay = Engine::replay_keystrokes(InputMethod::Telex, &HashMap::new(), &chars).0;
        assert_eq!(first.0, replay, "replay mismatch seed={seed:#x}");
    }
}

#[test]
fn edit_boundary_paste_focus_and_external_edit_transitions_match_reference() {
    for suffix in [
        ModelEvent::Backspace,
        ModelEvent::Delete,
        ModelEvent::Navigate,
        ModelEvent::Boundary(' '),
        ModelEvent::Paste("plain text".to_string()),
        ModelEvent::Focus,
        ModelEvent::ExternalEdit,
    ] {
        let events = vec![
            ModelEvent::Printable('a'),
            ModelEvent::Printable('s'),
            suffix,
        ];
        assert_trace(0x4544_4954, &events);
    }
}

#[test]
fn auto_restore_modes_validity_and_case_are_preserved() {
    for (input, expected, validity) in [
        ("tieengs", "tiếng", SpellingValidity::Valid),
        ("cargo", "cảgo", SpellingValidity::Invalid),
        ("DDUWOWNGF", "ĐƯỜNG", SpellingValidity::Valid),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }
        assert_eq!(engine.buffer(), expected, "input={input}");
        assert_eq!(engine.state().validity, validity, "input={input}");
    }

    for enabled in [false, true] {
        let mut engine = Engine::new(InputMethod::Telex);
        engine.set_auto_restore(enabled);
        for ch in "cargo".chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }
        let event = engine.reduce(PhysicalEvent::WordBoundary(' '));
        assert!(
            event.is_none(),
            "word boundaries must not restore raw keystrokes when auto_restore={enabled}, got {event:?}"
        );
    }

    for (input, shape) in [
        ("tieengs", CaseShape::Lower),
        ("TIEENGS", CaseShape::Upper),
        ("Tieengs", CaseShape::Mixed),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }
        assert_eq!(engine.state().case_shape, shape, "input={input}");
    }
}

#[test]
fn undo_policy_generation_and_stale_identity_invariants_hold() {
    for trigger in ['f', 's', 'r', 'x', 'j'] {
        assert_eq!(
            Engine::telex_undo_policy(TransformKind::Tone(trigger)),
            Some(UndoPolicy::AppendTrigger)
        );
    }
    for mark in ["aw", "ow", "uw", "uo+w", "ua+w", "aa", "ee", "oo"] {
        assert_eq!(
            Engine::telex_undo_policy(TransformKind::Mark(mark)),
            Some(UndoPolicy::AppendTrigger)
        );
    }
    assert_eq!(
        Engine::telex_undo_policy(TransformKind::StrokeDd),
        Some(UndoPolicy::ConsumeTrigger)
    );

    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "asff".chars() {
        engine.reduce(PhysicalEvent::Printable(ch));
    }
    assert_eq!(engine.buffer(), "af");
    assert_eq!(engine.state().active_transform, None);
    assert_eq!(
        engine.state().last_transition,
        Some(TransitionKind::UndoActive)
    );

    engine.reduce(PhysicalEvent::ExternalEdit);
    engine.reduce(PhysicalEvent::Printable('f'));
    assert_eq!(engine.buffer(), "f");
    assert_ne!(
        engine.state().last_transition,
        Some(TransitionKind::UndoActive),
        "invalidated transform identity must not be undone"
    );
}

#[test]
fn persisted_regression_corpus_is_reproducible() {
    for line in include_str!("testdata/telex-regressions.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let seed =
            u64::from_str_radix(fields.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        let input = fields.next().unwrap();
        let expected = fields.next().unwrap();
        let mut engine = Engine::new(InputMethod::Telex);
        let mut visible = String::new();
        for ch in input.chars() {
            let before = visible.chars().count();
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(replacement)) => {
                    for _ in 0..replacement.delete_before_cursor {
                        visible.pop();
                    }
                    visible.push_str(&replacement.insert);
                }
                Some(EngineEvent::Insert(text)) => visible.push_str(&text),
                None if engine.buffer().chars().count() > before => visible.push(ch),
                None => {}
                other => panic!("seed={seed:#x} unexpected event {other:?}"),
            }
        }
        assert_eq!(visible, expected, "seed={seed:#x} input={input}");
    }
}
