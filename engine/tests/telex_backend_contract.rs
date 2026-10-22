// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use vietc_engine::{
    BoundaryState, Engine, EngineEvent, EventStore, InputEvent, InputMethod, PhysicalEvent,
    ReplacementReason,
};

fn replay(input: &str) -> String {
    Engine::replay_keystrokes(
        InputMethod::Telex,
        &HashMap::new(),
        &input.chars().collect::<Vec<_>>(),
    )
    .0
}

#[test]
fn selected_backend_preserves_approved_telex_contract() {
    for (input, expected) in [
        ("tieengs", "tiếng"),
        ("dduowngf", "đường"),
        ("dduojwc", "được"),
        ("dduocwj", "được"),
        ("neues", "nếu"),
        ("neesu", "nếu"),
        ("readme", "readme"),
        ("teele", "tele"),
        ("TIEENGS", "TIẾNG"),
    ] {
        assert_eq!(replay(input), expected, "input={input}");
    }
}

#[test]
fn live_and_replay_are_equivalent() {
    for input in ["dduocwj", "neesu", "TIEENGS", "readme", "teele"] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.process_key(ch);
        }
        assert_eq!(engine.buffer(), replay(input), "input={input}");
    }
}

#[test]
fn replacement_delete_count_uses_unicode_scalars() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "tieeng".chars() {
        engine.process_key(ch);
    }

    let event = engine.process_key('s').expect("tone must replace output");
    let EngineEvent::Replace(replacement) = event else {
        panic!("expected replacement event");
    };

    assert_eq!(replacement.delete_before_cursor, "tiêng".chars().count());
    assert_eq!(replacement.insert, "tiếng");
}

#[test]
fn backspace_recomposes_from_raw_history() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "dduocwj".chars() {
        engine.process_key(ch);
    }

    engine.reduce(PhysicalEvent::Backspace);

    assert_eq!(engine.state().raw_keys, "dduocw");
    assert_eq!(engine.buffer(), replay("dduocw"));
}

#[test]
fn macros_boundaries_and_reset_keep_public_behavior() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.add_macro("vn".into(), "Việt Nam".into());
    engine.process_key('v');
    engine.process_key('n');

    let event = engine
        .process_key(' ')
        .expect("macro must replace shortcut");
    let EngineEvent::Replace(replacement) = event else {
        panic!("expected macro replacement");
    };
    assert_eq!(replacement.reason, ReplacementReason::Macro);
    assert_eq!(replacement.insert, "Việt Nam");
    assert!(engine.state().raw_keys.is_empty());
    assert_eq!(engine.state().boundary, BoundaryState::Committed);

    for ch in "tieengs".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.process_key('.'), None);
    assert!(engine.buffer().is_empty());

    engine.process_key('a');
    engine.reset();
    assert!(engine.buffer().is_empty());
}

#[test]
fn method_switch_enable_disable_and_vni_are_unchanged() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_enabled(false);
    assert_eq!(
        engine.process_key('a'),
        Some(EngineEvent::Insert("a".into()))
    );

    engine.set_enabled(true);
    engine.set_method(InputMethod::Vni);
    for ch in "tie6ng1".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "tiếng");
}

#[test]
fn undo_casing_fallback_and_paste_are_preserved() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "asff".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "af");

    engine.reset();
    for ch in "DDd".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "DD");

    engine.reset();
    for ch in "readme".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "readme");

    engine.reduce(PhysicalEvent::Paste("đã dán".into()));
    assert_eq!(engine.state().raw_keys, "đã dán");
    assert_eq!(engine.buffer(), "đã dán");
    assert_eq!(engine.state().boundary, BoundaryState::Invalidated);
}

#[test]
fn event_replay_matches_key_replay() {
    let mut events = EventStore::new();
    for ch in "dduocwj".chars() {
        events.push(InputEvent::KeyTyped(ch));
    }

    assert_eq!(
        Engine::replay_events(InputMethod::Telex, &HashMap::new(), &events),
        Engine::replay_keystrokes(
            InputMethod::Telex,
            &HashMap::new(),
            &"dduocwj".chars().collect::<Vec<_>>()
        )
    );
}
