use vietc_engine::{Engine, EngineEvent, InputMethod};

fn type_key(engine: &mut Engine, screen: &mut String, key: char) {
    match engine.process_key(key) {
        Some(EngineEvent::Replace(replacement)) => {
            for _ in 0..replacement.delete_before_cursor {
                assert!(screen.pop().is_some(), "replacement exceeds visible text");
            }
            screen.push_str(&replacement.insert);
        }
        Some(EngineEvent::Insert(text)) => screen.push_str(&text),
        None if key == '\x08' => {
            screen.pop();
        }
        None => screen.push(key),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn emitted_screen_matches_selected_backend_after_every_key() {
    for input in [
        "hosa", "hoaf", "dduocwj", "neesu", "TIEENGS", "teele", "asff", "DDd",
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        let mut screen = String::from("X ");
        for key in input.chars() {
            type_key(&mut engine, &mut screen, key);
            assert_eq!(
                screen,
                format!("X {}", engine.buffer()),
                "input={input} key={key}"
            );
        }
    }
}

#[test]
fn backspace_keeps_screen_and_composition_in_sync_without_deleting_prefix() {
    for (method, input) in [(InputMethod::Telex, "as"), (InputMethod::Vni, "a1")] {
        let mut engine = Engine::new(method);
        let mut screen = String::from("X ");
        for key in input.chars().chain(['\x08']) {
            type_key(&mut engine, &mut screen, key);
            assert_eq!(screen, format!("X {}", engine.buffer()));
        }
        type_key(&mut engine, &mut screen, input.chars().last().unwrap());
        assert_eq!(screen, "X á");
        type_key(&mut engine, &mut screen, '\x08');
        type_key(&mut engine, &mut screen, '\x08');
        assert_eq!(screen, "X ");
        type_key(&mut engine, &mut screen, '\x08');
        assert_eq!(screen, "X"); // No composition: native Backspace is allowed.
    }
}

#[test]
fn telex_and_vni_screen_buffer_consistency_regressions() {
    // A01: Telex "awa", "owo", "ww" and VNI "a11", "a112" must never diverge between screen and buffer()
    for (method, input) in [
        (InputMethod::Telex, "awa"),
        (InputMethod::Telex, "owo"),
        (InputMethod::Telex, "ww"),
        (InputMethod::Telex, "www"),
        (InputMethod::Vni, "a11"),
        (InputMethod::Vni, "a112"),
    ] {
        let mut engine = Engine::new(method);
        let mut screen = String::from("PRE|");
        for key in input.chars() {
            type_key(&mut engine, &mut screen, key);
            assert_eq!(
                screen,
                format!("PRE|{}", engine.buffer()),
                "method={method:?} input={input} key={key}"
            );
        }
    }
}

#[test]
fn vni_macro_expansion_deletes_shorthand_before_inserting_expansion() {
    // A03: VNI macro expansion must delete the shorthand instead of leaving it prefixed
    let mut engine = Engine::new(InputMethod::Vni);
    engine.add_macro("a".to_string(), "ALPHA".to_string());
    let mut screen = String::from("PRE|");
    type_key(&mut engine, &mut screen, 'a');
    assert_eq!(screen, "PRE|a");
    match engine.process_key(' ') {
        Some(EngineEvent::Replace(replacement)) => {
            assert_eq!(replacement.delete_before_cursor, 1);
            assert_eq!(replacement.insert, "ALPHA");
            for _ in 0..replacement.delete_before_cursor {
                screen.pop();
            }
            screen.push_str(&replacement.insert);
            screen.push(' ');
        }
        other => panic!("expected macro replacement, got {other:?}"),
    }
    assert_eq!(screen, "PRE|ALPHA ");
}

#[test]
fn vni_tone_retargeting_does_not_create_double_tone_vowels() {
    // A04: VNI "ho1a2" must move the tone to the new vowel ("hoà") rather than producing two tones ("hóà")
    let mut engine = Engine::new(InputMethod::Vni);
    let mut screen = String::new();
    for key in "ho1a2".chars() {
        type_key(&mut engine, &mut screen, key);
    }
    assert_eq!(screen, "hoà");
    assert_eq!(engine.buffer(), "hoà");
    assert!(!vietc_engine::spelling::is_valid_vietnamese_syllable("hóà"));
}

#[test]
fn paste_followed_by_backspace_or_tone_key_does_not_reinterpret_paste_as_telex() {
    // A02: PhysicalEvent::Paste must not be re-parsed as Telex raw keys on Backspace or follow-up keys
    use vietc_engine::PhysicalEvent;
    let mut engine = Engine::new(InputMethod::Telex);
    assert_eq!(engine.reduce(PhysicalEvent::Paste("as".to_string())), None);
    let _ = engine.reduce(PhysicalEvent::Backspace);
    assert_eq!(engine.state().visible, "a");
    assert_eq!(engine.state().raw_keys, "a");

    let mut engine2 = Engine::new(InputMethod::Telex);
    let _ = engine2.reduce(PhysicalEvent::Paste("as".to_string()));
    let _ = engine2.reduce(PhysicalEvent::Printable('s'));
    assert_eq!(engine2.state().visible, "ass");
}

#[test]
fn replacement_failure_recovery_protects_document_prefix_on_backspace() {
    // A11: After ReplacementFailed when replacing "a" -> "á" (1 char on screen, 2 raw keys "as"),
    // Backspace must only delete the 1 on-screen char and never eat into the document prefix.
    use vietc_engine::PhysicalEvent;
    let mut engine = Engine::new(InputMethod::Telex);
    let mut doc = String::from("PRE|");

    // Type 'a'
    match engine.reduce(PhysicalEvent::Printable('a')) {
        None => doc.push('a'),
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(doc, "PRE|a");

    // Type 's' -> Replacement("á", delete 1), but simulate replacement failure on screen
    match engine.reduce(PhysicalEvent::Printable('s')) {
        Some(EngineEvent::Replace(replacement)) => {
            assert_eq!(replacement.delete_before_cursor, 1);
            assert_eq!(replacement.insert, "á");
            // Simulate failed injection: doc still has "PRE|a"
            let _ = engine.reduce(PhysicalEvent::ReplacementFailed);
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(doc, "PRE|a");

    // Now user presses Backspace -> must delete at most 1 char from doc, preserving "PRE|"
    match engine.reduce(PhysicalEvent::Backspace) {
        Some(EngineEvent::Replace(replacement)) => {
            for _ in 0..replacement.delete_before_cursor {
                doc.pop();
            }
            doc.push_str(&replacement.insert);
        }
        None => {
            doc.pop();
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(
        doc.starts_with("PRE|"),
        "document prefix corrupted after ReplacementFailed + Backspace: {doc}"
    );
}

#[test]
fn metadata_boundary_and_active_transform_invariants_hold() {
    // A05: BoundaryState transitions to Composing after Committed + printable key,
    // and active_transform reports Mark("aw") for "asw" and Mark("uo+w") for "UOW".
    use vietc_engine::{BoundaryState, TransformKind};

    let mut engine = Engine::new(InputMethod::Telex);
    engine.process_key('a');
    engine.process_key(' ');
    assert_eq!(engine.state().boundary, BoundaryState::Committed);
    engine.process_key('b');
    assert_eq!(engine.state().boundary, BoundaryState::Composing);

    let mut engine2 = Engine::new(InputMethod::Telex);
    for ch in "asw".chars() {
        engine2.process_key(ch);
    }
    assert_eq!(
        engine2.state().active_transform.map(|t| t.kind),
        Some(TransformKind::Mark("aw"))
    );

    let mut engine3 = Engine::new(InputMethod::Telex);
    for ch in "UOW".chars() {
        engine3.process_key(ch);
    }
    assert_eq!(
        engine3.state().active_transform.map(|t| t.kind),
        Some(TransformKind::Mark("uo+w"))
    );
}

#[test]
fn telex_parser_recognizes_qu_and_gi_glides_as_valid() {
    // A06: Words with onset glides "qu" and "gi" must parse as ParseStatus::Complete
    use vietc_engine::telex::{parse, ParseStatus};
    for word in ["qua", "gia", "quyen", "gieng"] {
        let parsed = parse(word);
        assert_eq!(parsed.status(), ParseStatus::Complete, "word={word}");
    }
}

#[test]
fn english_double_s_words_auto_restore_on_space() {
    // U10: English words with double 's' ("ass", "class", "password") auto-restore on space
    for (raw, expected) in [
        ("ass", "ass"),
        ("class", "class"),
        ("password", "password"),
    ] {
        let mut engine = Engine::new(InputMethod::Telex);
        let mut screen = String::new();
        for ch in raw.chars() {
            type_key(&mut engine, &mut screen, ch);
        }
        // Flush with space: if the word was transformed mid-word, Replace restores raw English
        if let Some(EngineEvent::Replace(replacement)) = engine.process_key(' ') {
            for _ in 0..replacement.delete_before_cursor {
                screen.pop();
            }
            screen.push_str(&replacement.insert);
        }
        assert_eq!(screen, expected, "raw={raw}");
    }
}

#[test]
fn parenthesized_and_digit_prefixed_telex_words_compose_and_backspace_cleanly() {
    let mut engine = Engine::new(InputMethod::Telex);
    let mut screen = String::new();
    for ch in "(nhuw)".chars() {
        type_key(&mut engine, &mut screen, ch);
    }
    assert_eq!(screen, "(như)");

    let mut engine2 = Engine::new(InputMethod::Telex);
    let mut screen2 = String::new();
    for ch in "(nhuw".chars() {
        type_key(&mut engine2, &mut screen2, ch);
    }
    assert_eq!(screen2, "(như");
    type_key(&mut engine2, &mut screen2, '\x08');
    assert_eq!(screen2, "(nhu");

    let mut engine3 = Engine::new(InputMethod::Telex);
    let mut screen3 = String::new();
    for ch in "9nhuw".chars() {
        type_key(&mut engine3, &mut screen3, ch);
    }
    assert_eq!(screen3, "9như");
    type_key(&mut engine3, &mut screen3, '\x08');
    assert_eq!(screen3, "9nhu");
}


