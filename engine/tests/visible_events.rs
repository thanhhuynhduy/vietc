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
    // A04: VNI "ho1a2" must move the tone cleanly ("hòa" in Old-Style) rather than producing two tones ("hóà")
    let mut engine = Engine::new(InputMethod::Vni);
    let mut screen = String::new();
    for key in "ho1a2".chars() {
        type_key(&mut engine, &mut screen, key);
    }
    assert_eq!(screen, "hòa");
    assert_eq!(engine.buffer(), "hòa");
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
    for (raw, expected) in [("ass", "ass"), ("class", "class"), ("password", "password")] {
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

#[test]
fn gboard_old_style_tone_coda_repositioning_and_free_order_regressions() {
    for (method, input, expected) in [
        // Old-style open diphthongs vs closed diphthongs with final consonants
        (InputMethod::Telex, "hoaf", "hòa"),
        (InputMethod::Telex, "hofa", "hòa"),
        (InputMethod::Telex, "hofan", "hoàn"),
        (InputMethod::Telex, "tosan", "toán"),
        (InputMethod::Telex, "hosang", "hoáng"),
        (InputMethod::Telex, "hufynh", "huỳnh"),
        (InputMethod::Telex, "tusy", "túy"),
        (InputMethod::Telex, "tusyt", "tuýt"),
        (InputMethod::Telex, "thuyr", "thủy"),
        (InputMethod::Telex, "khore", "khỏe"),
        (InputMethod::Telex, "hofe", "hòe"),
        (InputMethod::Telex, "xofe", "xòe"),
        // Free-order glides and -ươu triphthong
        (InputMethod::Telex, "gifa", "già"),
        (InputMethod::Telex, "gifan", "giàn"),
        (InputMethod::Telex, "qusa", "quá"),
        (InputMethod::Telex, "qusan", "quán"),
        (InputMethod::Telex, "huouw", "hươu"),
        (InputMethod::Telex, "ruouwj", "rượu"),
        (InputMethod::Telex, "buouws", "bướu"),
        // Telex 'z' & VNI '0' tone removal + shape overrides
        (InputMethod::Telex, "toansz", "toan"),
        (InputMethod::Telex, "size", "size"),
        (InputMethod::Telex, "aaw", "ă"),
        (InputMethod::Telex, "oow", "ơ"),
        (InputMethod::Vni, "toan10", "toan"),
        (InputMethod::Vni, "100", "100"),
        (InputMethod::Vni, "quo7", "quơ"),
        (InputMethod::Vni, "quo72", "quờ"),
        (InputMethod::Vni, "dan9", "đan"),
        (InputMethod::Vni, "a68", "ă"),
        (InputMethod::Vni, "o67", "ơ"),
        // Screen/buffer sync on non-adjacent repeated VNI keys
        (InputMethod::Vni, "a1n1", "án"),
        (InputMethod::Vni, "tie6n6", "tien6"),
    ] {
        let mut engine = Engine::new(method);
        let mut screen = String::new();
        for ch in input.chars() {
            type_key(&mut engine, &mut screen, ch);
            assert_eq!(
                screen,
                engine.buffer(),
                "screen/buffer desync on method={method:?} input={input}"
            );
        }
        assert_eq!(screen, expected, "method={method:?} input={input}");
    }
}

#[test]
fn gboard_backspace_after_space_undoes_auto_restore_and_locks_vietnamese() {
    let mut engine = Engine::new(InputMethod::Telex);
    let mut screen = String::new();
    for ch in "test".chars() {
        type_key(&mut engine, &mut screen, ch);
    }
    assert_eq!(screen, "tét");

    // Pressing Space auto-restores standalone "test" -> "test "
    if let Some(EngineEvent::Replace(rep)) = engine.process_key(' ') {
        for _ in 0..rep.delete_before_cursor {
            screen.pop();
        }
        screen.push_str(&rep.insert);
    }
    screen.push(' ');
    assert_eq!(screen, "test ");

    // Pressing Backspace immediately reverts "test " -> "tét"!
    type_key(&mut engine, &mut screen, '\x08');
    assert_eq!(screen, "tét");

    // Pressing Space again respects the user's explicit override and commits "tét "!
    assert_eq!(engine.process_key(' '), None);
    screen.push(' ');
    assert_eq!(screen, "tét ");
}

#[test]
fn qa_audit_tone_overwrite_undo_rehydrated_prefix_digit_space_and_buffer_bound() {
    // 1. Overwritten tone followed by double-key tone undo ("asnff" -> "anf", never resurrecting 's')
    let mut engine = Engine::new(InputMethod::Telex);
    let mut screen = String::new();
    for ch in "asnff".chars() {
        type_key(&mut engine, &mut screen, ch);
        assert_eq!(screen, engine.buffer());
    }
    assert_eq!(screen, "anf");

    // 2. Rehydrated word across Space supports open-prefix edits ("đường " + BS*2 + "sg" -> "đướng")
    // and intentional Telex 'z' tone removal ("tiếng " + BS + "z" -> "tiêng")
    let mut engine2 = Engine::new(InputMethod::Telex);
    let mut screen2 = String::new();
    for ch in "dduowngf ".chars() {
        type_key(&mut engine2, &mut screen2, ch);
    }
    assert_eq!(screen2, "đường ");
    type_key(&mut engine2, &mut screen2, '\x08');
    type_key(&mut engine2, &mut screen2, '\x08');
    assert_eq!(screen2, "đườn");
    type_key(&mut engine2, &mut screen2, 's');
    assert_eq!(screen2, "đướn");
    type_key(&mut engine2, &mut screen2, 'g');
    assert_eq!(screen2, "đướng");

    let mut engine3 = Engine::new(InputMethod::Telex);
    let mut screen3 = String::new();
    for ch in "tieengs ".chars() {
        type_key(&mut engine3, &mut screen3, ch);
    }
    assert_eq!(screen3, "tiếng ");
    type_key(&mut engine3, &mut screen3, '\x08');
    type_key(&mut engine3, &mut screen3, 'z');
    assert_eq!(screen3, "tiêng");

    // 3. Digit-prefixed Vietnamese words ("9nhuw ", "10ddoongf ") preserve Vietnamese on Space
    let mut engine4 = Engine::new(InputMethod::Telex);
    let mut screen4 = String::new();
    for ch in "9nhuw ".chars() {
        type_key(&mut engine4, &mut screen4, ch);
    }
    assert_eq!(screen4, "9như ");

    // 4. Literal unconsumed 'w' after accented 'é' ("esw" -> "éw") does not report false Mark("uw")
    let mut engine5 = Engine::new(InputMethod::Telex);
    for ch in "esw".chars() {
        engine5.process_key(ch);
    }
    assert_eq!(engine5.buffer(), "éw");
    assert_eq!(
        engine5.state().active_transform.map(|t| t.kind),
        Some(vietc_engine::TransformKind::Tone('s'))
    );

    // 5. Unbounded key hold (>64 chars) is bounded in O(1) without freeze or panic
    let mut engine6 = Engine::new(InputMethod::Telex);
    for _ in 0..200 {
        let _ = engine6.process_key('a');
    }
    assert!(engine6.buffer().chars().count() <= 64);
}
