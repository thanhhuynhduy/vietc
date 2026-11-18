// SPDX-License-Identifier: MIT
use super::*;
use crate::telex::resolve_and_render;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
struct ShadowMismatch {
    raw_input: &'static str,
    legacy_output: String,
    recomposed_output: String,
    category: &'static str,
    approved: bool,
}

const APPROVED_SHADOW_DIFFERENCES: &[&str] = &[];

fn process(method: InputMethod, input: &str) -> String {
    let mut engine = BambooEngine::new(method);
    let mut output = String::new();
    for ch in input.chars() {
        if let Some(o) = engine.process_key(ch) {
            output = o;
        }
    }
    output
}

#[test]
fn phase5_shadow_recomposition_matches_phase1_golden_cases() {
    let cases = [
        ("mark", "aa"),
        ("mark", "aw"),
        ("mark", "ee"),
        ("mark", "oo"),
        ("mark", "ow"),
        ("mark", "uw"),
        ("stroke", "dd"),
        ("tone", "as"),
        ("tone", "af"),
        ("tone", "ar"),
        ("tone", "ax"),
        ("tone", "aj"),
        ("nucleus-coda", "tieengs"),
        ("nucleus-coda", "dduowngf"),
        ("free-order", "dduojwc"),
        ("free-order", "neues"),
        ("free-order", "dduocwj"),
        ("free-order", "neesu"),
        ("english-fallback", "teele"),
        ("english-fallback", "fixx"),
        ("english-fallback", "addd"),
        ("english-fallback", "aww"),
        ("literal-fallback", "readme"),
        ("literal-fallback", "type"),
        ("casing", "TIEENGS"),
        ("casing", "ATXX"),
    ];

    let mismatches = cases
        .into_iter()
        .filter_map(|(category, raw_input)| {
            let legacy_output = crate::Engine::replay_keystrokes(
                InputMethod::Telex,
                &HashMap::new(),
                &raw_input.chars().collect::<Vec<_>>(),
            )
            .0;
            let recomposed_output = resolve_and_render(raw_input);
            (legacy_output != recomposed_output).then(|| ShadowMismatch {
                raw_input,
                legacy_output,
                recomposed_output,
                category,
                approved: APPROVED_SHADOW_DIFFERENCES.contains(&raw_input),
            })
        })
        .collect::<Vec<_>>();

    let unapproved = mismatches
        .iter()
        .filter(|mismatch| !mismatch.approved)
        .collect::<Vec<_>>();

    assert!(
        unapproved.is_empty(),
        "unapproved shadow mismatches: {unapproved:#?}; all mismatches: {mismatches:#?}"
    );
    assert!(
        mismatches
            .iter()
            .all(|mismatch| APPROVED_SHADOW_DIFFERENCES.contains(&mismatch.raw_input)),
        "shadow allowlist contains no cases beyond approved differences: {mismatches:#?}"
    );
}

#[test]
fn test_telex_tone() {
    assert_eq!(process(InputMethod::Telex, "tieengs"), "tiếng");
    assert_eq!(process(InputMethod::Telex, "dduwowngf"), "đường");
    assert_eq!(process(InputMethod::Telex, "thuw"), "thư");
}

#[test]
fn test_telex_marks() {
    assert_eq!(process(InputMethod::Telex, "aa"), "â");
    assert_eq!(process(InputMethod::Telex, "ee"), "ê");
    assert_eq!(process(InputMethod::Telex, "oo"), "ô");
    assert_eq!(process(InputMethod::Telex, "aw"), "ă");
    assert_eq!(process(InputMethod::Telex, "ow"), "ơ");
    assert_eq!(process(InputMethod::Telex, "uw"), "ư");
    assert_eq!(process(InputMethod::Telex, "dd"), "đ");
}

#[test]
fn test_vni_tone() {
    assert_eq!(process(InputMethod::Vni, "d9"), "đ");
    assert_eq!(process(InputMethod::Vni, "u7"), "ư");
    assert_eq!(process(InputMethod::Vni, "o7"), "ơ");
    assert_eq!(process(InputMethod::Vni, "d9u7o7ng2"), "đường");
    assert_eq!(process(InputMethod::Vni, "tie6ng1"), "tiếng");
    assert_eq!(process(InputMethod::Vni, "thu3"), "thủ");
    assert_eq!(process(InputMethod::Vni, "xa4"), "xã");
    assert_eq!(process(InputMethod::Vni, "na85ng5"), "nặng");
}

#[test]
fn test_vni_marks() {
    assert_eq!(process(InputMethod::Vni, "a6"), "â");
    assert_eq!(process(InputMethod::Vni, "e6"), "ê");
    assert_eq!(process(InputMethod::Vni, "o6"), "ô");
    assert_eq!(process(InputMethod::Vni, "o7"), "ơ");
    assert_eq!(process(InputMethod::Vni, "u7"), "ư");
    assert_eq!(process(InputMethod::Vni, "a8"), "ă");
    assert_eq!(process(InputMethod::Vni, "d9"), "đ");
}

#[test]
fn test_tone_placement() {
    assert_eq!(process(InputMethod::Telex, "hoas"), "hóa");
    assert_eq!(process(InputMethod::Vni, "hoa1"), "hóa");
    assert_eq!(process(InputMethod::Vni, "hoan2"), "hoàn");
    assert_eq!(process(InputMethod::Telex, "thuoos"), "thuố");
}

#[test]
fn test_reset() {
    let mut engine = BambooEngine::new(InputMethod::Telex);
    engine.process_key('t');
    engine.reset();
    assert!(engine.get_output().is_empty());
}

#[test]
fn test_uppercase_preservation() {
    let mut engine = BambooEngine::new(InputMethod::Telex);
    engine.process_key('T');
    engine.process_key('i');
    engine.process_key('e');
    engine.process_key('e');
    engine.process_key('n');
    engine.process_key('g');
    engine.process_key('s');
    assert_eq!(engine.get_output(), "Tiếng");
}

#[test]
fn test_simple_words() {
    assert_eq!(process(InputMethod::Telex, "chafo"), "chào");
    assert_eq!(process(InputMethod::Vni, "chao2"), "chào");
}

#[test]
fn test_telex_tuaan() {
    let mut e = crate::bamboo::BambooEngine::new(crate::input_method::InputMethod::Telex);
    let mut out = String::new();
    for ch in "Tuaans".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "Tuấn", "Expected Tuấn, got {}", out);
}

#[test]
fn test_telex_nguyeenx() {
    let mut e = crate::bamboo::BambooEngine::new(crate::input_method::InputMethod::Telex);
    let mut out = String::new();
    for ch in "nguyeenx".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "nguyễn", "Expected nguyễn, got {}", out);
}

#[test]
fn test_telex_gios() {
    let mut e = crate::bamboo::BambooEngine::new(crate::input_method::InputMethod::Telex);
    let mut out = String::new();
    for ch in "gios".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "gió", "Expected gió, got {}", out);
}

#[test]
fn test_telex_ua_horn() {
    assert_eq!(process(InputMethod::Telex, "xuaw"), "xưa");
    assert_eq!(process(InputMethod::Telex, "chuaw"), "chưa");
    assert_eq!(process(InputMethod::Telex, "muaw"), "mưa");
    assert_eq!(process(InputMethod::Telex, "Xuaw"), "Xưa");
    assert_eq!(process(InputMethod::Telex, "luawr"), "lửa");
    assert_eq!(process(InputMethod::Telex, "quawng"), "quăng");
    assert_eq!(process(InputMethod::Vni, "xua7"), "xưa");
    assert_eq!(process(InputMethod::Vni, "qua8ng"), "quăng");
}

#[test]
fn test_telex_r_as_normal_char() {
    let mut e = BambooEngine::new(InputMethod::Telex);
    let mut out = String::new();
    for ch in "tr".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "tr");
    out.clear();
    e.reset();
    for ch in "traf".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "trà");
    out.clear();
    e.reset();
    for ch in "tar".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "tả");
    out.clear();
    e.reset();
    for ch in "tramr".chars() {
        if let Some(o) = e.process_key(ch) {
            out = o;
        }
    }
    assert_eq!(out, "trảm");
}
