// SPDX-License-Identifier: MIT
//! Exhaustive verification suite for Task C2:
//! Weighted Spatial SymSpell Autocorrect, Context Ranking & Compound Splitting.

use dboard_engine::{Engine, EngineEvent, InputMethod};

fn simulate_autocorrect(raw: &str) -> String {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);
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
            Some(EngineEvent::Insert(text)) | Some(EngineEvent::Flush(text)) => {
                screen.push_str(&text);
            }
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
            None => {
                if ch == '\x08' {
                    screen.pop();
                } else {
                    screen.push(ch);
                }
            }
        }
    }
    screen
}

#[test]
fn fixes_missing_extra_and_context_ranked_typos() {
    // Missing 1 character (Deletion)
    assert_eq!(simulate_autocorrect("khoog "), "không ");
    assert_eq!(simulate_autocorrect("recive "), "receive ");
    assert_eq!(simulate_autocorrect("progam "), "program ");
    assert_eq!(simulate_autocorrect("funtion "), "function ");

    // Extra 1 character adjacent to key (Insertion)
    assert_eq!(simulate_autocorrect("khoonhg "), "không ");
    assert_eq!(simulate_autocorrect("tieenghs "), "tiếng ");

    // Context-ranked disambiguation:
    // Standalone `chaod` is ambiguous (`d` borders `f` -> `chào` and `s` -> `cháo`) -> must NOT guess
    assert_ne!(simulate_autocorrect("chaod "), "chào ");
    // With Vietnamese context `"xin"`, n-gram LM decisively selects `"chào"`
    assert_eq!(simulate_autocorrect("xin chaod "), "xin chào ");
    // Standalone `tieengd` also stays untouched
    assert_ne!(simulate_autocorrect("tieengd "), "tiếng ");
}

#[test]
fn splits_missing_space_vietnamese_compounds_with_one_tap_undo() {
    assert_eq!(simulate_autocorrect("khoongbieets "), "không biết ");
    assert_eq!(simulate_autocorrect("xinchaof "), "xin chào ");
    assert_eq!(simulate_autocorrect("dduowcjrooif "), "được rồi ");

    // One-Tap Undo via Backspace right after Space restores exact raw input
    assert_eq!(simulate_autocorrect("khoongbieets \x08"), "khoongbieets");
    // Subsequent commit of the same undone raw token stays untouched (`keep_raw`)
    assert_eq!(
        simulate_autocorrect("khoongbieets \x08 khoongbieets "),
        "khoongbieets khoongbieets "
    );
}

#[test]
fn zero_false_positives_on_code_and_dev_identifiers() {
    let dev_tokens = [
        "kubectl",
        "systemd",
        "hashmap",
        "serde",
        "tokio",
        "localhost",
        "filename",
        "username",
        "makefile",
        "wayland",
        "println",
        "unwrap",
        "grpc",
        "jsonrpc",
        "evdev",
        "uinput",
    ];
    for tok in dev_tokens {
        let input = format!("{tok} ");
        let actual = simulate_autocorrect(&input);
        assert_eq!(
            actual, input,
            "dev identifier `{tok}` was corrupted into `{actual}`"
        );
    }
}

#[test]
fn mid_sentence_caret_reselection_alternatives_swap_cleanly() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_autocorrect(true);

    // Accented Vietnamese <-> English twin (preserves casing)
    assert_eq!(
        engine.alternatives_for_word("tét", None),
        vec!["test", "tét"]
    );
    assert_eq!(
        engine.alternatives_for_word("Tét", None),
        vec!["Test", "Tét"]
    );
    assert_eq!(
        engine.alternatives_for_word("lát", None),
        vec!["last", "lát"]
    );
    assert_eq!(
        engine.alternatives_for_word("hót", None),
        vec!["host", "hót"]
    );
    assert_eq!(
        engine.alternatives_for_word("bên", None),
        vec!["been", "bên"]
    );

    // English / ASCII -> Vietnamese twin
    assert_eq!(
        engine.alternatives_for_word("test", None),
        vec!["tét", "test"]
    );
    assert_eq!(
        engine.alternatives_for_word("last", None),
        vec!["lát", "last"]
    );

    // Typo under caret -> SymSpell fix
    assert_eq!(
        engine.alternatives_for_word("khoog", None),
        vec!["không", "khoog"]
    );
    assert_eq!(
        engine.alternatives_for_word("recive", None),
        vec!["receive", "recive"]
    );

    // Unambiguous normal words & punctuation-glued tokens -> empty (no popup spam)
    assert!(engine.alternatives_for_word("hôm", None).is_empty());
    assert!(engine.alternatives_for_word("hello", None).is_empty());
    // Non-ASCII Vietnamese typo sitting on screen (e.g. "khôg" from `khoog` when
    // autocorrect was off or undone) must also yield SymSpell re-selection candidates.
    let alts_non_ascii = engine.alternatives_for_word("khôg", None);
    assert_eq!(
        alts_non_ascii.first().map(String::as_str),
        Some("không"),
        "non-ASCII Vietnamese typo 'khôg' should suggest 'không', got {:?}",
        alts_non_ascii
    );
    assert!(alts_non_ascii.iter().any(|c| c == "khôg"));

    assert!(engine.alternatives_for_word("a", None).is_empty());
    assert!(engine.alternatives_for_word("foo_bar", None).is_empty());
}

#[test]
fn audit_regression_fixes_split_ring_buffer_circumflex_filename_and_autocap() {
    // 1. Missing-space split pushes individual words into ring_buffer so
    //    next-word prediction works and One-Tap Undo pops both entries cleanly.
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);
    for ch in "khoongbieets ".chars() {
        let _ = engine.process_key(ch);
    }
    let preds = engine.predict_next_words(3);
    assert!(
        !preds.is_empty(),
        "split 'không biết ' must push separate ring_buffer words for next-word prediction"
    );
    // One-Tap Undo immediately after split must restore raw keys cleanly
    let undo_ev = engine.process_key('\x08');
    assert!(matches!(undo_ev, Some(EngineEvent::Replace(_))));
    assert!(
        engine.predict_next_words(3).is_empty(),
        "One-Tap Undo of split pair must pop both ring_buffer entries"
    );

    // 2. Tone-before-circumflex across Vietnamese coda (`khongso`, `tiengse`, `vietje`)
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Telex, "khongso tiengse vietje pasta "),
        "khống tiếng việt pasta "
    );

    // 3. Office & modern dev filename extensions
    assert_eq!(
        simulate_autocorrect_with_method(
            InputMethod::Telex,
            "mowr test.docx xem list.xlsx suwar main.lua "
        ),
        "mở test.docx xem list.xlsx sửa main.lua "
    );

    // 4. Smart AutoCap across opening quotes & brackets after sentence boundary
    let mut cap_engine = Engine::new(InputMethod::Telex);
    cap_engine.set_auto_restore(true);
    cap_engine.set_auto_capitalize(true);
    let type_cap = |eng: &mut Engine, s: &str| -> String {
        let mut out = String::new();
        for ch in s.chars() {
            match eng.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    for _ in 0..rep.delete_before_cursor {
                        out.pop();
                    }
                    out.push_str(&rep.insert);
                    if dboard_engine::is_flush_char(ch) {
                        out.push(ch);
                    }
                }
                Some(EngineEvent::AutoRestore(w)) => {
                    out.push_str(&w);
                    if dboard_engine::is_flush_char(ch) {
                        out.push(ch);
                    }
                }
                Some(EngineEvent::Insert(t)) | Some(EngineEvent::Flush(t)) => out.push_str(&t),
                Some(EngineEvent::UndoTones {
                    backspaces,
                    restored,
                }) => {
                    for _ in 0..backspaces {
                        out.pop();
                    }
                    out.push_str(&restored);
                }
                Some(EngineEvent::Paste(t)) => out.push_str(&t),
                None => out.push(ch),
            }
        }
        out
    };
    assert_eq!(
        type_cap(&mut cap_engine, "Chaof. \"hoom nay\" "),
        "Chào. \"Hôm nay\" "
    );
    cap_engine.reset();
    assert_eq!(
        type_cap(&mut cap_engine, "Chaof. \u{201C}hoom nay\u{201D} "),
        "Chào. \u{201C}Hôm nay\u{201D} "
    );
    cap_engine.reset();
    assert_eq!(
        type_cap(&mut cap_engine, "Chaof (hoom nay) "),
        "Chào (hôm nay) "
    );
}

fn simulate_autocorrect_with_method(method: InputMethod, raw: &str) -> String {
    let mut engine = Engine::new(method);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);
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
            Some(EngineEvent::Insert(text)) | Some(EngineEvent::Flush(text)) => {
                screen.push_str(&text);
            }
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
            None => {
                if ch == '\x08' {
                    screen.pop();
                } else {
                    screen.push(ch);
                }
            }
        }
    }
    screen
}

#[test]
fn vni_symspell_and_extended_english_corrections_preserve_safety() {
    // 1. VNI SymSpell: missing char, extra char, and missing-space split
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "khog6 "),
        "không "
    );
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "khonhg6 "),
        "không "
    );
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "khong6biet61 "),
        "không biết "
    );
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "xinchao2 "),
        "xin chào "
    );
    // Tech alphanumeric tokens in VNI must NEVER be corrupted by SymSpell
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "u32 "),
        "u32 "
    );
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "sha256 "),
        "sha256 "
    );
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "h264 "),
        "h264 "
    );

    // 2. Extended English SymSpell (beyond the 1.4k core list)
    assert_eq!(simulate_autocorrect("algoritm "), "algorithm ");
    assert_eq!(simulate_autocorrect("tomorow "), "tomorrow ");
    assert_eq!(simulate_autocorrect("necesary "), "necessary ");

    // 3. VNI end-marked spatial typo (`tieng71` -> `tiếng`) must not be dropped by early seen.insert
    assert_eq!(
        simulate_autocorrect_with_method(InputMethod::Vni, "tieng71 "),
        "tiếng "
    );

    // 4. OOV English compounds must NEVER be falsely split into Vietnamese pairs
    assert_eq!(simulate_autocorrect("xem hostlist "), "xem hostlist ");
    assert_eq!(simulate_autocorrect("xem testlist "), "xem testlist ");
    assert_eq!(simulate_autocorrect("xem lastpost "), "xem lastpost ");
}

#[test]
fn one_tap_undo_respects_learning_disabled_privacy_guard() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_autocorrect(true);
    engine.set_learning_enabled(false);
    for ch in "khoog ".chars() {
        let _ = engine.process_key(ch);
    }
    // One-Tap Undo with learning disabled must NOT mutate UserDict
    let _ = engine.process_key('\x08');
    assert!(
        engine.take_user_dict_for_save().is_none(),
        "One-Tap Undo must not mark UserDict dirty when learning_enabled is false"
    );
}

#[test]
fn retroactive_pair_with_autocorrected_w2_one_tap_undo_never_corrupts_screen() {
    fn run_seq(input: &str) -> String {
        let mut engine = Engine::new(InputMethod::Telex);
        engine.set_auto_restore(true);
        engine.set_autocorrect(true);
        let mut screen = String::new();
        for ch in input.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        for _ in 0..rep.delete_before_cursor {
                            screen.pop();
                        }
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        screen.pop();
                    }
                    other => panic!("unexpected backspace event: {:?}", other),
                }
                continue;
            }
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
                Some(EngineEvent::Insert(t))
                | Some(EngineEvent::Flush(t))
                | Some(EngineEvent::Paste(t)) => screen.push_str(&t),
                Some(EngineEvent::UndoTones {
                    backspaces,
                    restored,
                }) => {
                    for _ in 0..backspaces {
                        screen.pop();
                    }
                    screen.push_str(&restored);
                }
                None => screen.push(ch),
            }
        }
        screen
    }

    // Direction-A: "nói " + "has " (kept as "há ") + "recive " (SymSpell -> "receive" + retroactive -> "has receive ")
    // Backspace #1 (One-Tap Undo) -> "nói há receive", Backspace #2 -> "nói há receiv" (never "nói há rreciv"!)
    assert_eq!(run_seq("nois has recive "), "nói has receive ");
    assert_eq!(run_seq("nois has recive \x08"), "nói há receive");
    assert_eq!(run_seq("nois has recive \x08\x08"), "nói há receiv");
}

#[test]
fn commit_selected_word_disarms_autocap_updates_boundary_and_splits_multiword_candidates() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_capitalize(true);
    // Arm AutoCap via Enter, then commit a selected word at start of line.
    let _ = engine.process_key('\n');
    engine.commit_selected_word("Xin", true);
    // Word #2 must NOT be auto-capitalized.
    let _ = engine.process_key('c');
    assert_eq!(
        engine.buffer(),
        "c",
        "commit_selected_word must disarm sentence-start AutoCap"
    );

    // Committing without trailing space must clear last_boundary_char so next-word
    // prediction does not fire without a space at caret.
    engine.commit_selected_word("test", false);
    assert!(
        engine.predict_next_words(3).is_empty(),
        "trailing_space=false must not leave stale Space boundary for next-word prediction"
    );

    // Committing a split-word candidate must push separate ring_buffer words so
    // n-gram next-word prediction still fires cleanly.
    engine.commit_selected_word("không biết", true);
    assert!(
        !engine.predict_next_words(3).is_empty(),
        "split-word candidate must push separate tokens for next-word prediction"
    );
}

#[test]
fn symspell_protects_prefixed_brand_names_and_candidate_bar_uses_context() {
    // Leading single-letter brand/tool names must NOT be truncated (e.g. gboard -> board).
    assert_eq!(simulate_autocorrect("gboard "), "gboard ");
    assert_eq!(simulate_autocorrect("dboard "), "dboard ");

    // Candidate bar suggestion must use sentence context (ring_buffer) just like Space autocorrect.
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_autocorrect(true);
    for ch in "xin chaod".chars() {
        let _ = engine.process_key(ch);
    }
    assert_eq!(
        engine.candidate_suggestion().as_deref(),
        Some("chào"),
        "candidate_suggestion must disambiguate 'xin chaod' -> 'chào' using ring_buffer context"
    );

    // Unicode ellipsis '…' must arm sentence-start auto-capitalization.
    let mut cap = Engine::new(InputMethod::Telex);
    cap.set_auto_capitalize(true);
    for ch in "xin… ".chars() {
        let _ = cap.process_key(ch);
    }
    let _ = cap.process_key('c');
    assert_eq!(cap.buffer(), "C");
}

#[test]
fn invalidate_clears_last_boundary_char_and_seed_sets_space_boundary() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);

    // Type a dot boundary, then invalidate (cursor move / focus change).
    for ch in "done.".chars() {
        let _ = engine.process_key(ch);
    }
    engine.invalidate_composition();

    // Autocorrect must still work on the next word instead of seeing stale `.` boundary.
    let mut screen = String::new();
    for ch in "teh ".chars() {
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
            None => screen.push(ch),
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(
        screen, "the ",
        "invalidate_composition must clear stale '.' last_boundary_char"
    );

    // Seeding a preceding word without an active word_at_caret must arm Space boundary
    // so next-word prediction fires immediately.
    engine.reset();
    assert!(engine.seed_surrounding_context(None, Some("xin"), false));
    assert!(
        !engine.predict_next_words(3).is_empty(),
        "seed_surrounding_context(prev_word) must set Space boundary for predict_next_words"
    );

    // commit_selected_word(..., false) followed by Backspace must rehydrate and
    // delete the last visible character so subsequent tone edits stay in sync.
    engine.reset();
    let mut screen = String::from("chào");
    engine.commit_selected_word("chào", false);
    assert!(engine.process_key('\x08').is_none());
    screen.pop(); // native BS deletes 'o' -> "chà"
    assert_eq!(screen, "chà");
    match engine.process_key('s') {
        Some(EngineEvent::Replace(rep)) => {
            for _ in 0..rep.delete_before_cursor {
                screen.pop();
            }
            screen.push_str(&rep.insert);
        }
        other => panic!("expected tone replacement after rehydrated BS, got {other:?}"),
    }
    assert_eq!(screen, "chá");
}
