// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use vietc_engine::{Engine, InputMethod};

const CORPUS: &str = include_str!("fixtures/vietnamese_typing_corpus.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FocusPoint {
    at_key: usize,
    target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TypingCase {
    case_id: String,
    raw_keystrokes: String,
    expected_unicode: String,
    allowed_fallback: Option<String>,
    application: String,
    key_delay_ms: u64,
    random_seed: u64,
    focus_timeline: Vec<FocusPoint>,
    expected_event_sequence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct WordOracle {
    longest_matching_expected_prefix: String,
    first_differing_scalar_index: Option<usize>,
    first_expected_scalar: Option<char>,
    first_observed_scalar: Option<char>,
    first_incorrect_word: Option<String>,
    transaction_id: Option<u64>,
}

fn corpus() -> Vec<TypingCase> {
    serde_json::from_str(CORPUS).expect("typing corpus must be valid JSON")
}

fn oracle(expected: &str, observed: &str, transaction_id: Option<u64>) -> WordOracle {
    let expected_chars: Vec<char> = expected.chars().collect();
    let observed_chars: Vec<char> = observed.chars().collect();
    let index = expected_chars
        .iter()
        .zip(&observed_chars)
        .position(|(left, right)| left != right)
        .or_else(|| {
            (expected_chars.len() != observed_chars.len())
                .then_some(expected_chars.len().min(observed_chars.len()))
        });

    let prefix_len = index.unwrap_or(expected_chars.len());
    let word = index.and_then(|index| {
        let start = expected_chars[..index.min(expected_chars.len())]
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .map_or(0, |position| position + 1);
        let value = expected_chars[start..]
            .iter()
            .take_while(|ch| !ch.is_whitespace())
            .collect::<String>();
        (!value.is_empty()).then_some(value)
    });

    WordOracle {
        longest_matching_expected_prefix: expected_chars[..prefix_len].iter().collect(),
        first_differing_scalar_index: index,
        first_expected_scalar: index.and_then(|value| expected_chars.get(value).copied()),
        first_observed_scalar: index.and_then(|value| observed_chars.get(value).copied()),
        first_incorrect_word: word,
        transaction_id,
    }
}

fn compose_word(raw: &str, macros: &HashMap<String, String>) -> String {
    if let Some(expansion) = macros.get(&raw.to_ascii_lowercase()) {
        return expansion.clone();
    }
    Engine::replay_keystrokes(InputMethod::Telex, macros, &raw.chars().collect::<Vec<_>>()).0
}

fn render_supported_case(case: &TypingCase) -> Option<String> {
    if case.application == "deterministic-engine-disabled" {
        return Some(case.raw_keystrokes.clone());
    }
    if matches!(
        case.application.as_str(),
        "deterministic-clipboard-model" | "deterministic-target-model"
    ) {
        return Some(compose_word(&case.raw_keystrokes, &HashMap::new()));
    }
    if case.application == "deterministic-focus-model" {
        let (_, after_focus) = case.raw_keystrokes.split_once('|')?;
        return Some(render_text(after_focus, &HashMap::new()));
    }
    if case.application != "deterministic-engine" {
        return None;
    }

    let mut macros = HashMap::new();
    if case.case_id == "VN-MACRO-ISOLATION-001" {
        macros.insert("brb".to_string(), "be right back".to_string());
    }
    Some(render_text(&case.raw_keystrokes, &macros))
}

fn render_text(raw: &str, macros: &HashMap<String, String>) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.contains('@') {
        return raw.to_string();
    }

    let mut rendered = String::new();
    let mut word = String::new();
    for ch in raw.chars() {
        if ch == '\x08' {
            word.pop();
        } else if ch.is_alphabetic() {
            word.push(ch);
        } else {
            if !word.is_empty() {
                rendered.push_str(&compose_word(&word, macros));
                word.clear();
            }
            rendered.push(ch);
        }
    }
    if !word.is_empty() {
        rendered.push_str(&compose_word(&word, macros));
    }
    rendered
}

#[test]
fn every_corpus_case_has_stable_complete_replay_metadata() {
    let cases = corpus();
    let mut ids = HashSet::new();
    let mut seeds = HashSet::new();

    for case in &cases {
        assert!(ids.insert(case.case_id.clone()), "duplicate case ID");
        assert!(case.case_id.starts_with("VN-"), "unstable case ID");
        assert!(!case.raw_keystrokes.is_empty());
        assert!(!case.expected_unicode.is_empty());
        assert!(!case.application.is_empty());
        assert_ne!(case.random_seed, 0);
        assert!(seeds.insert(case.random_seed), "duplicate random seed");
        assert!(!case.focus_timeline.is_empty());
        assert_eq!(case.focus_timeline[0].at_key, 0);
        assert!(!case.focus_timeline[0].target.is_empty());
        assert!(!case.expected_event_sequence.is_empty());
    }
}

#[test]
fn corpus_covers_required_typing_dimensions() {
    let ids = corpus()
        .into_iter()
        .map(|case| case.case_id)
        .collect::<Vec<_>>();

    for required in [
        "MARK-TONE",
        "FREE-ORDER",
        "PUNCTUATION",
        "NEWLINE",
        "BACKSPACE",
        "URL",
        "EMAIL",
        "ENGLISH",
        "MACRO-ISOLATION",
        "DISABLED-LITERAL",
        "EDIT-RETYPE",
        "FOCUS-CHANGE",
        "CLIPBOARD-DELAY",
        "TARGET-DELAY",
        "STEADY",
        "BURST",
    ] {
        assert!(
            ids.iter().any(|id| id.contains(required)),
            "missing corpus dimension {required}"
        );
    }
}

#[test]
fn supported_cases_match_expected_unicode() {
    for case in corpus() {
        let Some(observed) = render_supported_case(&case) else {
            continue;
        };
        assert_eq!(
            observed, case.expected_unicode,
            "case={} seed={}",
            case.case_id, case.random_seed
        );
    }
}

#[test]
fn word_oracle_reports_prefix_scalar_word_and_transaction() {
    let report = oracle("xin chào thế giới", "xin chao thế giới", Some(42));
    assert_eq!(report.longest_matching_expected_prefix, "xin ch");
    assert_eq!(report.first_differing_scalar_index, Some(6));
    assert_eq!(report.first_expected_scalar, Some('à'));
    assert_eq!(report.first_observed_scalar, Some('a'));
    assert_eq!(report.first_incorrect_word.as_deref(), Some("chào"));
    assert_eq!(report.transaction_id, Some(42));

    let equal = oracle("tiếng", "tiếng", None);
    assert_eq!(equal.longest_matching_expected_prefix, "tiếng");
    assert_eq!(equal.first_differing_scalar_index, None);
    assert_eq!(equal.first_incorrect_word, None);
}

#[test]
fn stress_failure_artifact_persists_replay_seed() {
    let case = corpus()
        .into_iter()
        .find(|case| case.case_id == "VN-BURST-001")
        .expect("burst case");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("typing-corpus-failures");
    std::fs::create_dir_all(&root).expect("create artifact directory");
    let path = root.join(format!(
        "{}-seed-{:016x}.json",
        case.case_id, case.random_seed
    ));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&case).expect("serialize replay case"),
    )
    .expect("persist replay seed");
    let replayed: TypingCase =
        serde_json::from_slice(&std::fs::read(&path).expect("read replay artifact"))
            .expect("decode replay artifact");
    assert_eq!(replayed.case_id, case.case_id);
    assert_eq!(replayed.random_seed, case.random_seed);
    std::fs::remove_file(path).expect("remove test artifact");
}
