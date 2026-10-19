use vietc_engine::{
    BoundaryState, CaseShape, Engine, EngineEvent, InputMethod, PhysicalEvent, SpellingValidity,
    TransformIdentity, TransformKind, TransitionKind, UndoPolicy,
};

#[test]
fn initial_state_exposes_explicit_empty_composition() {
    let engine = Engine::new(InputMethod::Telex);
    let state = engine.state();

    assert!(state.raw_keys.is_empty());
    assert!(state.visible.is_empty());
    assert_eq!(state.active_transform, None::<TransformIdentity>);
    assert_eq!(state.case_shape, CaseShape::Lower);
    assert_eq!(state.validity, SpellingValidity::Unknown);
    assert_eq!(state.boundary, BoundaryState::Committed);
    assert_eq!(state.generation, 0);
}

#[test]
fn applying_tone_tracks_raw_visible_and_active_identity() {
    let mut engine = Engine::new(InputMethod::Telex);

    engine.process_key('a');
    engine.process_key('s');
    let state = engine.state();

    assert_eq!(state.raw_keys, "as");
    assert_eq!(state.visible, "á");
    assert_eq!(state.case_shape, CaseShape::Lower);
    assert_eq!(state.validity, SpellingValidity::Valid);
    assert_eq!(state.boundary, BoundaryState::Composing);
    assert_eq!(
        state.active_transform,
        Some(TransformIdentity {
            kind: TransformKind::Tone('s'),
            trigger: 's',
            generation: 1,
            undo_policy: UndoPolicy::AppendTrigger,
        })
    );
    assert_eq!(state.last_transition, Some(TransitionKind::Apply));
}

#[test]
fn replacing_active_transform_closes_the_old_generation() {
    let mut engine = Engine::new(InputMethod::Telex);

    for key in "asf".chars() {
        engine.process_key(key);
    }
    let state = engine.state();

    assert_eq!(state.raw_keys, "asf");
    assert_eq!(state.visible, "à");
    assert_eq!(state.generation, 2);
    assert_eq!(
        state.active_transform,
        Some(TransformIdentity {
            kind: TransformKind::Tone('f'),
            trigger: 'f',
            generation: 2,
            undo_policy: UndoPolicy::AppendTrigger,
        })
    );
    assert_eq!(state.last_transition, Some(TransitionKind::ReplaceActive));
}

#[test]
fn repeated_tone_undoes_only_the_current_identity() {
    let mut engine = Engine::new(InputMethod::Telex);

    for key in "asff".chars() {
        engine.process_key(key);
    }
    let state = engine.state();

    assert_eq!(state.visible, "af");
    assert_eq!(state.active_transform, None);
    assert_eq!(state.last_transition, Some(TransitionKind::UndoActive));
}

#[test]
fn telex_policy_table_is_locked_for_tones_marks_and_stroke() {
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
}

#[test]
fn dd_consume_trigger_outputs_are_defined_for_case_shapes() {
    for (input, expected, case_shape) in [
        ("ddd", "dd", CaseShape::Lower),
        ("DDd", "DD", CaseShape::Mixed),
        ("dDd", "dD", CaseShape::Mixed),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for key in input.chars() {
            engine.process_key(key);
        }

        assert_eq!(engine.buffer(), expected, "input={input}");
        assert_eq!(engine.state().case_shape, case_shape, "input={input}");
        assert_eq!(
            engine.state().last_transition,
            Some(TransitionKind::UndoActive),
            "input={input}"
        );
    }
}

#[test]
fn edit_invalidate_commit_and_recover_are_explicit_transitions() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.process_key('a');
    engine.process_key('\x08');
    assert_eq!(engine.state().last_transition, Some(TransitionKind::Edit));

    engine.invalidate_composition();
    assert_eq!(engine.state().boundary, BoundaryState::Invalidated);
    assert_eq!(
        engine.state().last_transition,
        Some(TransitionKind::Invalidate)
    );

    engine.process_key('a');
    engine.process_key(' ');
    assert_eq!(engine.state().boundary, BoundaryState::Committed);
    assert_eq!(engine.state().last_transition, Some(TransitionKind::Commit));

    engine.recover_composition();
    assert_eq!(
        engine.state().last_transition,
        Some(TransitionKind::Recover)
    );
}

#[test]
fn backspace_reduces_raw_history_and_replays_visible_composition() {
    let mut engine = Engine::new(InputMethod::Telex);
    for key in "as".chars() {
        engine.process_key(key);
    }
    assert_eq!(engine.buffer(), "á");

    engine.process_key('\x08');
    let state = engine.state();

    assert_eq!(state.raw_keys, "a");
    assert_eq!(state.visible, "a");
    assert_eq!(state.active_transform, None);
    assert_eq!(state.boundary, BoundaryState::Composing);
    assert_eq!(state.last_transition, Some(TransitionKind::Edit));
}

#[test]
fn physical_event_reducer_preserves_history_across_transform_replacement() {
    let mut engine = Engine::new(InputMethod::Telex);

    for event in [
        PhysicalEvent::Printable('t'),
        PhysicalEvent::Printable('i'),
        PhysicalEvent::Printable('e'),
        PhysicalEvent::Printable('e'),
        PhysicalEvent::Printable('n'),
        PhysicalEvent::Printable('g'),
        PhysicalEvent::Printable('s'),
        PhysicalEvent::Printable('f'),
    ] {
        engine.reduce(event);
    }

    let state = engine.state();
    assert_eq!(state.raw_keys, "tieengsf");
    assert_eq!(state.visible, "tiềng");
    assert_eq!(state.generation, 3);
    assert_eq!(state.last_transition, Some(TransitionKind::ReplaceActive));
}

#[test]
fn invalidating_physical_events_close_the_active_composition() {
    for event in [
        PhysicalEvent::Delete,
        PhysicalEvent::Navigate,
        PhysicalEvent::SelectionChanged,
        PhysicalEvent::ExternalEdit,
        PhysicalEvent::FocusChanged,
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        engine.reduce(PhysicalEvent::Printable('a'));
        engine.reduce(PhysicalEvent::Printable('s'));

        engine.reduce(event);

        let state = engine.state();
        assert!(state.raw_keys.is_empty());
        assert!(state.visible.is_empty());
        assert_eq!(state.active_transform, None);
        assert_eq!(state.boundary, BoundaryState::Invalidated);
        assert_eq!(state.last_transition, Some(TransitionKind::Invalidate));
    }
}

#[test]
fn paste_invalidates_composition_and_starts_from_pasted_text() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.reduce(PhysicalEvent::Printable('a'));
    engine.reduce(PhysicalEvent::Printable('s'));

    engine.reduce(PhysicalEvent::Paste("plain text".to_string()));

    let state = engine.state();
    assert_eq!(state.raw_keys, "plain text");
    assert_eq!(state.visible, "plain text");
    assert_eq!(state.active_transform, None);
    assert_eq!(state.boundary, BoundaryState::Invalidated);
    assert_eq!(state.last_transition, Some(TransitionKind::Invalidate));
}

#[test]
fn replacement_failure_keeps_composition_recoverable() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.reduce(PhysicalEvent::Printable('a'));
    engine.reduce(PhysicalEvent::Printable('s'));
    let before_failure = engine.state();

    engine.reduce(PhysicalEvent::ReplacementFailed);

    let recovered = engine.state();
    assert_eq!(recovered.raw_keys, before_failure.raw_keys);
    assert_eq!(recovered.visible, before_failure.visible);
    assert_eq!(recovered.active_transform, before_failure.active_transform);
    assert_eq!(recovered.boundary, BoundaryState::Composing);
    assert_eq!(recovered.last_transition, Some(TransitionKind::Recover));
}

#[test]
fn physical_event_reducer_preserves_case_and_mark_history() {
    for (input, expected, case_shape) in [
        ("dduwowngf", "đường", CaseShape::Lower),
        ("DDUWOWNGF", "ĐƯỜNG", CaseShape::Upper),
        ("Dduwowngf", "Đường", CaseShape::Mixed),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }

        let state = engine.state();
        assert_eq!(state.raw_keys, input, "input={input}");
        assert_eq!(state.visible, expected, "input={input}");
        assert_eq!(state.case_shape, case_shape, "input={input}");
    }
}

#[test]
fn physical_event_reducer_handles_multi_vowel_and_non_adjacent_transforms() {
    for (input, expected) in [
        ("tieengs", "tiếng"),
        ("nguyeenx", "nguyễn"),
        ("dduwowngf", "đường"),
        ("tramr", "trảm"),
        ("quawng", "quăng"),
        ("xuaw", "xưa"),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }

        let state = engine.state();
        assert_eq!(state.raw_keys, input, "input={input}");
        assert_eq!(state.visible, expected, "input={input}");
        assert_eq!(state.boundary, BoundaryState::Composing, "input={input}");
    }
}

#[test]
fn physical_event_reducer_reports_valid_and_invalid_spelling() {
    for (input, expected, validity) in [
        ("tieengs", "tiếng", SpellingValidity::Valid),
        ("cargo", "cảgo", SpellingValidity::Invalid),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.reduce(PhysicalEvent::Printable(ch));
        }

        let state = engine.state();
        assert_eq!(state.raw_keys, input, "input={input}");
        assert_eq!(state.visible, expected, "input={input}");
        assert_eq!(state.validity, validity, "input={input}");
    }
}

#[test]
fn backspace_at_empty_end_of_buffer_is_a_deterministic_edit() {
    let mut engine = Engine::new(InputMethod::Telex);

    let event = engine.reduce(PhysicalEvent::Backspace);

    let state = engine.state();
    assert_eq!(event, None);
    assert!(state.raw_keys.is_empty());
    assert!(state.visible.is_empty());
    assert_eq!(state.active_transform, None);
    assert_eq!(state.boundary, BoundaryState::Committed);
    assert_eq!(state.last_transition, Some(TransitionKind::Edit));
}

#[test]
fn word_boundary_commits_and_auto_restores_invalid_composition() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "cargo".chars() {
        engine.reduce(PhysicalEvent::Printable(ch));
    }

    let event = engine.reduce(PhysicalEvent::WordBoundary(' '));

    assert_eq!(
        event,
        Some(EngineEvent::Replace {
            backspaces: 4,
            insert: "cargo".to_string(),
        })
    );
    let state = engine.state();
    assert!(state.raw_keys.is_empty());
    assert!(state.visible.is_empty());
    assert_eq!(state.active_transform, None);
    assert_eq!(state.boundary, BoundaryState::Committed);
    assert_eq!(state.last_transition, Some(TransitionKind::Commit));
}

#[test]
fn word_boundary_commits_valid_composition_without_replacement() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "tieengs".chars() {
        engine.reduce(PhysicalEvent::Printable(ch));
    }

    let event = engine.reduce(PhysicalEvent::WordBoundary(' '));

    assert_eq!(event, None);
    let state = engine.state();
    assert!(state.raw_keys.is_empty());
    assert!(state.visible.is_empty());
    assert_eq!(state.active_transform, None);
    assert_eq!(state.boundary, BoundaryState::Committed);
    assert_eq!(state.last_transition, Some(TransitionKind::Commit));
}

#[test]
fn empty_word_boundary_does_not_create_a_commit_transition() {
    let mut engine = Engine::new(InputMethod::Telex);

    let event = engine.reduce(PhysicalEvent::WordBoundary(' '));

    assert_eq!(event, None);
    let state = engine.state();
    assert_eq!(state.boundary, BoundaryState::Committed);
    assert_eq!(state.last_transition, None);
}
