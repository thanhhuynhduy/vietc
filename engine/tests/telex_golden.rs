// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use vietc_engine::{telex::resolve_and_render, Engine, InputMethod};

#[derive(Debug)]
struct GoldenCase {
    category: &'static str,
    input: &'static str,
    expected: &'static str,
    contract: &'static str,
}

fn composed(keys: &str) -> String {
    Engine::replay_keystrokes(
        InputMethod::Telex,
        &HashMap::new(),
        &keys.chars().collect::<Vec<_>>(),
    )
    .0
}

#[test]
fn telex_golden_behavior_matrix() {
    let cases = [
        GoldenCase {
            category: "mark",
            input: "aa",
            expected: "â",
            contract: "baseline",
        },
        GoldenCase {
            category: "mark",
            input: "aw",
            expected: "ă",
            contract: "baseline",
        },
        GoldenCase {
            category: "mark",
            input: "ee",
            expected: "ê",
            contract: "baseline",
        },
        GoldenCase {
            category: "mark",
            input: "oo",
            expected: "ô",
            contract: "baseline",
        },
        GoldenCase {
            category: "mark",
            input: "ow",
            expected: "ơ",
            contract: "baseline",
        },
        GoldenCase {
            category: "mark",
            input: "uw",
            expected: "ư",
            contract: "baseline",
        },
        GoldenCase {
            category: "stroke",
            input: "dd",
            expected: "đ",
            contract: "baseline",
        },
        GoldenCase {
            category: "tone",
            input: "as",
            expected: "á",
            contract: "baseline",
        },
        GoldenCase {
            category: "tone",
            input: "af",
            expected: "à",
            contract: "baseline",
        },
        GoldenCase {
            category: "tone",
            input: "ar",
            expected: "ả",
            contract: "baseline",
        },
        GoldenCase {
            category: "tone",
            input: "ax",
            expected: "ã",
            contract: "baseline",
        },
        GoldenCase {
            category: "tone",
            input: "aj",
            expected: "ạ",
            contract: "baseline",
        },
        GoldenCase {
            category: "nucleus-coda",
            input: "tieengs",
            expected: "tiếng",
            contract: "baseline",
        },
        GoldenCase {
            category: "nucleus-coda",
            input: "dduowngf",
            expected: "đường",
            contract: "baseline",
        },
        GoldenCase {
            category: "free-order",
            input: "dduojwc",
            expected: "được",
            contract: "baseline",
        },
        GoldenCase {
            category: "free-order",
            input: "neues",
            expected: "nếu",
            contract: "baseline",
        },
        GoldenCase {
            category: "free-order",
            input: "dduocwj",
            expected: "được",
            contract: "approved-new",
        },
        GoldenCase {
            category: "free-order",
            input: "neesu",
            expected: "nếu",
            contract: "approved-new",
        },
        GoldenCase {
            category: "english-fallback",
            input: "teele",
            expected: "tele",
            contract: "baseline",
        },
        GoldenCase {
            category: "english-fallback",
            input: "fixx",
            expected: "fix",
            contract: "baseline",
        },
        GoldenCase {
            category: "english-fallback",
            input: "addd",
            expected: "add",
            contract: "baseline",
        },
        GoldenCase {
            category: "english-fallback",
            input: "aww",
            expected: "aw",
            contract: "baseline",
        },
        GoldenCase {
            category: "literal-fallback",
            input: "readme",
            expected: "readme",
            contract: "baseline",
        },
        GoldenCase {
            category: "literal-fallback",
            input: "type",
            expected: "type",
            contract: "baseline",
        },
        GoldenCase {
            category: "casing",
            input: "TIEENGS",
            expected: "TIẾNG",
            contract: "baseline",
        },
        GoldenCase {
            category: "casing",
            input: "ATXX",
            expected: "ATX",
            contract: "baseline",
        },
    ];

    for case in cases {
        assert_eq!(
            composed(case.input),
            case.expected,
            "Telex golden mismatch: category={}, contract={}, input={:?}",
            case.category,
            case.contract,
            case.input
        );
    }
}

#[test]
fn pure_phase3_pipeline_matches_approved_golden_corpus() {
    let cases = [
        ("mark", "aa", "â", "baseline"),
        ("mark", "aw", "ă", "baseline"),
        ("mark", "ee", "ê", "baseline"),
        ("mark", "oo", "ô", "baseline"),
        ("mark", "ow", "ơ", "baseline"),
        ("mark", "uw", "ư", "baseline"),
        ("stroke", "dd", "đ", "baseline"),
        ("tone", "as", "á", "baseline"),
        ("tone", "af", "à", "baseline"),
        ("tone", "ar", "ả", "baseline"),
        ("tone", "ax", "ã", "baseline"),
        ("tone", "aj", "ạ", "baseline"),
        ("nucleus-coda", "tieengs", "tiếng", "baseline"),
        ("nucleus-coda", "dduowngf", "đường", "baseline"),
        ("free-order", "dduojwc", "được", "baseline"),
        ("free-order", "neues", "nếu", "baseline"),
        ("free-order", "dduocwj", "được", "approved-new"),
        ("free-order", "neesu", "nếu", "approved-new"),
        ("literal-fallback", "readme", "readme", "fallback"),
        ("literal-fallback", "type", "type", "fallback"),
        ("casing", "TIEENGS", "TIẾNG", "baseline"),
    ];

    for (category, input, expected, classification) in cases {
        assert_eq!(
            resolve_and_render(input),
            expected,
            "pure Phase 3 mismatch: category={category}, classification={classification}, input={input:?}"
        );
    }
}

#[test]
fn telex_boundaries_commit_each_word_independently() {
    let cases = [
        ("tieengs", ' ', "tiếng "),
        ("dduojwc", '.', "được."),
        ("neues", ',', "nếu,"),
        ("readme", '\n', "readme\n"),
    ];

    for (input, boundary, expected) in cases {
        let actual = format!("{}{}", composed(input), boundary);
        assert_eq!(
            actual, expected,
            "Telex boundary mismatch: input={input:?}, boundary={boundary:?}"
        );
    }
}
