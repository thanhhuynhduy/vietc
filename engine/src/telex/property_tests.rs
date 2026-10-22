use super::resolve_and_render;
use std::collections::BTreeSet;

const CASE_ID_PREFIX: &str = "phase4-permutation";

#[derive(Clone, Copy)]
struct PermutationCase {
    id: &'static str,
    category: &'static str,
    letters: &'static str,
    triggers: &'static str,
    expected: &'static str,
}

fn is_tone_trigger(ch: char) -> bool {
    matches!(ch, 's' | 'f' | 'r' | 'x' | 'j')
}

fn insert_triggers(letters: &str, triggers: &str) -> Vec<String> {
    let letters: Vec<char> = letters.chars().collect();
    let first_vowel = if letters.len() > 2
        && matches!(
            (
                letters[0].to_ascii_lowercase(),
                letters[1].to_ascii_lowercase()
            ),
            ('q', 'u') | ('g', 'i')
        )
        && matches!(
            letters[2].to_ascii_lowercase(),
            'a' | 'e' | 'i' | 'o' | 'u' | 'y'
        ) {
        2
    } else {
        letters
            .iter()
            .position(|ch| matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
            .expect("fixture contains a nucleus")
    };
    let nucleus_end = letters[first_vowel..]
        .iter()
        .position(|ch| !matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
        .map(|offset| first_vowel + offset)
        .unwrap_or(letters.len());
    let mut generated = BTreeSet::new();

    fn generate(
        letters: &[char],
        nucleus_end: usize,
        remaining: Vec<char>,
        placements: &mut Vec<(usize, char)>,
        output: &mut BTreeSet<String>,
    ) {
        if remaining.is_empty() {
            let mut rendered = String::new();
            for gap in 0..=letters.len() {
                for (_, trigger) in placements
                    .iter()
                    .filter(|(placed_gap, _)| *placed_gap == gap)
                {
                    rendered.push(*trigger);
                }
                if let Some(letter) = letters.get(gap) {
                    rendered.push(*letter);
                }
            }
            output.insert(rendered);
            return;
        }

        for trigger_index in 0..remaining.len() {
            let trigger = remaining[trigger_index];
            if is_tone_trigger(trigger)
                && remaining.iter().any(|candidate| {
                    !is_tone_trigger(*candidate) && matches!(*candidate, 'a' | 'e' | 'o')
                })
            {
                continue;
            }

            let nucleus_start = if letters.len() > 2
                && matches!(
                    (
                        letters[0].to_ascii_lowercase(),
                        letters[1].to_ascii_lowercase()
                    ),
                    ('q', 'u') | ('g', 'i')
                )
                && matches!(
                    letters[2].to_ascii_lowercase(),
                    'a' | 'e' | 'i' | 'o' | 'u' | 'y'
                ) {
                2
            } else {
                letters
                    .iter()
                    .position(|ch| {
                        matches!(ch.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
                    })
                    .expect("fixture contains a nucleus")
            };
            let first_allowed_gap = if is_tone_trigger(trigger) {
                nucleus_start + 1
            } else {
                nucleus_end
            };
            let minimum_gap = placements
                .last()
                .map(|(gap, _)| gap + 1)
                .unwrap_or(first_allowed_gap)
                .max(first_allowed_gap);
            let maximum_gap = if is_tone_trigger(trigger) {
                letters.len()
            } else {
                nucleus_end
            };

            let mut rest = remaining.clone();
            rest.remove(trigger_index);
            for gap in minimum_gap..=maximum_gap {
                placements.push((gap, trigger));
                generate(letters, nucleus_end, rest.clone(), placements, output);
                placements.pop();
            }
        }
    }

    generate(
        &letters,
        nucleus_end,
        triggers.chars().collect(),
        &mut Vec::new(),
        &mut generated,
    );
    generated.into_iter().collect()
}
#[test]
fn allowed_free_order_permutations_converge_for_all_required_classes() {
    let cases = [
        PermutationCase {
            id: "mark-breve",
            category: "mark-single",
            letters: "a",
            triggers: "w",
            expected: "ă",
        },
        PermutationCase {
            id: "mark-a-circumflex",
            category: "mark-single",
            letters: "a",
            triggers: "a",
            expected: "â",
        },
        PermutationCase {
            id: "mark-e-circumflex",
            category: "mark-single",
            letters: "e",
            triggers: "e",
            expected: "ê",
        },
        PermutationCase {
            id: "mark-o-circumflex",
            category: "mark-single",
            letters: "o",
            triggers: "o",
            expected: "ô",
        },
        PermutationCase {
            id: "mark-o-horn",
            category: "mark-single",
            letters: "o",
            triggers: "w",
            expected: "ơ",
        },
        PermutationCase {
            id: "mark-u-horn",
            category: "mark-single",
            letters: "u",
            triggers: "w",
            expected: "ư",
        },
        PermutationCase {
            id: "tone-acute",
            category: "tone",
            letters: "an",
            triggers: "s",
            expected: "án",
        },
        PermutationCase {
            id: "tone-grave",
            category: "tone",
            letters: "an",
            triggers: "f",
            expected: "àn",
        },
        PermutationCase {
            id: "tone-hook",
            category: "tone",
            letters: "an",
            triggers: "r",
            expected: "ản",
        },
        PermutationCase {
            id: "tone-tilde",
            category: "tone",
            letters: "an",
            triggers: "x",
            expected: "ãn",
        },
        PermutationCase {
            id: "tone-dot",
            category: "tone",
            letters: "an",
            triggers: "j",
            expected: "ạn",
        },
        PermutationCase {
            id: "double-oa",
            category: "nucleus-double",
            letters: "hoa",
            triggers: "s",
            expected: "hoá",
        },
        PermutationCase {
            id: "double-ie",
            category: "nucleus-double",
            letters: "tieng",
            triggers: "es",
            expected: "tiếng",
        },
        PermutationCase {
            id: "double-uo",
            category: "nucleus-double",
            letters: "dduong",
            triggers: "wf",
            expected: "đường",
        },
        PermutationCase {
            id: "triple-uye",
            category: "nucleus-triple",
            letters: "tuyen",
            triggers: "es",
            expected: "tuyến",
        },
        PermutationCase {
            id: "glide-qu",
            category: "glide",
            letters: "qua",
            triggers: "s",
            expected: "quá",
        },
        PermutationCase {
            id: "glide-gi",
            category: "glide",
            letters: "gia",
            triggers: "s",
            expected: "giá",
        },
    ];

    for case in cases {
        for (permutation_index, input) in insert_triggers(case.letters, case.triggers)
            .into_iter()
            .enumerate()
        {
            let actual = resolve_and_render(&input);
            assert_eq!(
                actual,
                case.expected,
                "input={input:?}, expected={:?}, actual={actual:?}, category={}, case_id={CASE_ID_PREFIX}-{}-{permutation_index}",
                case.expected,
                case.category,
                case.id
            );
        }
    }
}

#[test]
fn disallowed_trigger_orders_fallback_without_becoming_another_vietnamese_word() {
    let cases = [
        ("trigger-before-nucleus", "sban", "sban"),
        ("trigger-after-boundary", "ban s", "ban s"),
        ("mark-after-boundary", "duoc w", "duoc w"),
        ("tone-without-nucleus", "str", "str"),
    ];

    for (case_id, input, expected) in cases {
        let actual = resolve_and_render(input);
        assert_eq!(
            actual,
            expected,
            "input={input:?}, expected={expected:?}, actual={actual:?}, category=disallowed-policy, case_id={CASE_ID_PREFIX}-{case_id}"
        );
    }
}

#[test]
fn raw_key_deletion_restores_each_previous_pure_pipeline_state() {
    let cases = [
        ("backspace-duoc", "dduocwj"),
        ("backspace-neu", "neesu"),
        ("backspace-triple", "tuyeens"),
        ("backspace-glide", "quas"),
    ];

    for (case_id, input) in cases {
        let prefixes: Vec<String> = (0..=input.chars().count())
            .map(|length| input.chars().take(length).collect())
            .collect();
        let mut raw = input.to_string();

        for expected_raw in prefixes.iter().rev().skip(1) {
            raw.pop();
            let actual = resolve_and_render(&raw);
            let expected = resolve_and_render(expected_raw);
            assert_eq!(
                (raw.as_str(), actual.as_str()),
                (expected_raw.as_str(), expected.as_str()),
                "input={input:?}, expected={expected:?}, actual={actual:?}, category=raw-key-deletion, case_id={CASE_ID_PREFIX}-{case_id}"
            );
        }
    }
}

#[test]
fn replaying_prefixes_matches_direct_pure_pipeline_resolution() {
    let cases = [
        ("replay-dduojwc", "dduojwc"),
        ("replay-dduocwj", "dduocwj"),
        ("replay-neues", "neues"),
        ("replay-neesu", "neesu"),
        ("replay-casing", "TIEENGS"),
    ];

    for (case_id, input) in cases {
        let mut replay_raw = String::new();
        for key in input.chars() {
            replay_raw.push(key);
            let replayed = resolve_and_render(&replay_raw);
            let direct = resolve_and_render(
                &input
                    .chars()
                    .take(replay_raw.chars().count())
                    .collect::<String>(),
            );
            assert_eq!(
                replayed,
                direct,
                "input={input:?}, expected={direct:?}, actual={replayed:?}, category=replay-equivalence, case_id={CASE_ID_PREFIX}-{case_id}"
            );
        }
    }
}

#[test]
fn boundaries_prevent_triggers_from_crossing_syllables() {
    let cases = [
        ("space-tone", "ban s", "ban s"),
        ("comma-tone", "ban,s", "ban,s"),
        ("period-mark", "duoc.w", "duoc.w"),
        ("hyphen-mark", "neu-w", "neu-w"),
    ];

    for (case_id, input, expected) in cases {
        let actual = resolve_and_render(input);
        assert_eq!(
            actual,
            expected,
            "input={input:?}, expected={expected:?}, actual={actual:?}, category=boundary, case_id={CASE_ID_PREFIX}-{case_id}"
        );
    }
}

#[test]
fn replacement_delete_count_uses_visible_unicode_scalar_count() {
    let cases = [
        ("one-precomposed", "được", "đường"),
        ("uppercase-precomposed", "TIẾNG", "TỪ"),
        ("empty-next", "nếu", ""),
    ];

    for (case_id, previous, next) in cases {
        let diff = super::replacement_diff(previous, next);
        let expected = previous.chars().count() - diff.common_prefix_chars;
        assert_eq!(
            diff.delete_before_cursor,
            expected,
            "input={previous:?}, expected={expected:?}, actual={:?}, category=unicode-delete-count, case_id={CASE_ID_PREFIX}-{case_id}",
            diff.delete_before_cursor
        );
    }
}

#[test]
fn deterministic_bounded_generated_inputs_never_panic() {
    const SEED: u32 = 0x4f52_4445;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ ,.-";
    let mut state = SEED;

    for length in 0..=24 {
        for case_index in 0..256 {
            let input: String = (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ALPHABET[(state as usize) % ALPHABET.len()] as char
                })
                .collect();
            let first = resolve_and_render(&input);
            let second = resolve_and_render(&input);
            assert_eq!(
                first,
                second,
                "input={input:?}, expected={first:?}, actual={second:?}, category=determinism-no-panic, seed={SEED:#010x}, case_id={CASE_ID_PREFIX}-generated-{length}-{case_index}"
            );
        }
    }
}
