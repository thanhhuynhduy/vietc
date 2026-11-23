// SPDX-License-Identifier: MIT
//! Honest, independent >= 500-sentence bilingual VN/EN accuracy benchmark (Task B1 / M3 gate).
//!
//! Loads 510 distinct, realistic human sentences from `testdata/bilingual_bench_500.tsv`
//! across 5 domains (office, software engineering/Linux, pure Vietnamese, pure English,
//! and mid-sentence code-switching/homograph traps) with zero Cartesian-product padding.

use std::collections::{BTreeMap, HashSet};

use dboard_engine::{Engine, EngineEvent, InputMethod};

const BENCH_TSV: &str = include_str!("testdata/bilingual_bench_500.tsv");

fn simulate_telex(raw: &str) -> String {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    let mut screen = String::new();
    for ch in raw.chars() {
        match engine.process_key(ch) {
            Some(EngineEvent::Replace(rep)) => {
                for _ in 0..rep.delete_before_cursor {
                    screen.pop();
                }
                screen.push_str(&rep.insert);
                if dboard_engine::is_flush_char(ch) {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::AutoRestore(word)) => {
                screen.push_str(&word);
                if dboard_engine::is_flush_char(ch) {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::Insert(text)) => screen.push_str(&text),
            Some(EngineEvent::Flush(text)) => screen.push_str(&text),
            Some(EngineEvent::UndoTones {
                backspaces,
                restored,
            }) => {
                for _ in 0..backspaces {
                    screen.pop();
                }
                screen.push_str(&restored);
            }
            Some(EngineEvent::Paste(text)) => screen.push_str(&text),
            None => screen.push(ch),
        }
    }
    screen
}

#[test]
fn bilingual_accuracy_benchmark_meets_m3_gate() {
    // 1. Validate binary size & integrity
    assert!(
        dboard_engine::stat_lm::verify_binary_integrity(),
        "bilingual_lm.bin failed integrity check"
    );
    let size = dboard_engine::stat_lm::binary_size_bytes();
    assert!(
        size < 350 * 1024,
        "bilingual_lm.bin exceeds 350KB budget: {size} bytes"
    );

    // 2. Parse honest independent benchmark TSV
    let mut cases: Vec<(&str, &str, &str)> = Vec::new();
    let mut unique_expected: HashSet<&str> = HashSet::new();
    for line in BENCH_TSV.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let cat = parts.next().expect("category");
        let raw = parts.next().expect("raw_telex");
        let expected = parts.next().expect("expected_output");
        assert!(
            raw.is_ascii(),
            "benchmark raw Telex must be pure ASCII: {raw:?}"
        );
        assert!(
            unique_expected.insert(expected),
            "duplicate sentence in benchmark: {expected:?}"
        );
        cases.push((cat, raw, expected));
    }

    assert!(
        cases.len() >= 500,
        "M3 gate requires >= 500 unique sentences, got {}",
        cases.len()
    );

    let mut passed = 0usize;
    let mut total_words = 0usize;
    let mut passed_words = 0usize;
    let mut per_cat: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut failures: Vec<(&str, &str, &str, String)> = Vec::new();

    for &(cat, raw, expected) in &cases {
        let actual = simulate_telex(raw);
        let entry = per_cat.entry(cat).or_insert((0, 0));
        entry.1 += 1;

        let exp_words: Vec<&str> = expected.split_whitespace().collect();
        let act_words: Vec<&str> = actual.split_whitespace().collect();
        total_words += exp_words.len();
        for (e, a) in exp_words.iter().zip(act_words.iter()) {
            if e == a {
                passed_words += 1;
            }
        }

        if actual == expected {
            passed += 1;
            entry.0 += 1;
        } else {
            failures.push((cat, raw, expected, actual));
        }
    }

    let sentence_acc = (passed as f64) * 100.0 / (cases.len() as f64);
    let word_acc = (passed_words as f64) * 100.0 / (total_words as f64);
    eprintln!(
        "[bilingual_bench] sentences={}/{} ({:.2}%), words={}/{} ({:.2}%), lm_bytes={}",
        passed,
        cases.len(),
        sentence_acc,
        passed_words,
        total_words,
        word_acc,
        size
    );
    for (cat, (ok, tot)) in &per_cat {
        eprintln!(
            "  [{cat}] {ok}/{tot} ({:.2}%)",
            (*ok as f64) * 100.0 / (*tot as f64)
        );
    }
    if std::env::var_os("DBOARD_BENCH_VERBOSE").is_some() {
        for (cat, raw, exp, act) in &failures {
            eprintln!("  FAIL [{cat}] raw={raw:?}\n    exp={exp:?}\n    act={act:?}");
        }
    }

    assert!(
        sentence_acc >= 97.0,
        "bilingual sentence accuracy {sentence_acc:.2}% ({passed}/{}) below 97.0% gate! Sample failures: {:#?}",
        cases.len(),
        &failures[..failures.len().min(30)]
    );
}
