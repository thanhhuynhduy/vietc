use crate::{Engine, EngineEvent, InputMethod};

fn process_input(engine: &mut Engine, input: &str) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    for ch in input.chars() {
        if let Some(event) = engine.process_key(ch) {
            let is_replace = matches!(&event, EngineEvent::Replace(_));
            let fl = is_flush_char(ch);
            events.push(event);
            if is_replace && fl {
                events.push(EngineEvent::Insert(ch.to_string()));
            }
        } else if engine.is_enabled() {
            events.push(EngineEvent::Insert(ch.to_string()));
        }
    }
    events
}

fn is_flush_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '.' | ',' | '!' | '?' | ';' | ':' | '\n')
}

fn get_display(events: &[EngineEvent]) -> String {
    let mut display = String::new();
    for ev in events {
        match ev {
            EngineEvent::Flush(text) | EngineEvent::Paste(text) => {
                display.push_str(text);
            }
            EngineEvent::Insert(text) => {
                display.push_str(text);
            }
            EngineEvent::Replace(replacement) => {
                for _ in 0..replacement.delete_before_cursor {
                    display.pop();
                }
                display.push_str(&replacement.insert);
            }
            EngineEvent::AutoRestore(word) => {
                for _ in 0..word.len() {
                    display.pop();
                }
                display.push_str(word);
            }
            EngineEvent::UndoTones {
                backspaces,
                restored,
            } => {
                for _ in 0..*backspaces {
                    display.pop();
                }
                display.push_str(restored);
            }
        }
    }
    display
}

// ================================================================
// Telex: Vowel combinations
// ================================================================

#[test]
fn telex_double_a() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aa")), "â");
}

#[test]
fn telex_double_e() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ee")), "ê");
}

#[test]
fn telex_double_o() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "oo")), "ô");
}

#[test]
fn telex_reverts_double_vowel_for_english_continuation() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "teele")), "tele");
}

#[test]
fn telex_reverts_tone_for_english_continuation() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "fixx")), "fix");
}

#[test]
fn regression_uppercase_atxx_restores_atx() {
    let mut engine = Engine::new(InputMethod::Telex);
    let events = process_input(&mut engine, "ATXX");

    assert_eq!(get_display(&events), "ATX");
}

#[test]
fn telex_reverts_double_d_for_english_continuation() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ddd")), "dd");
    e.reset();
    assert_eq!(get_display(&process_input(&mut e, "add")), "add");
    e.reset();
    assert_eq!(get_display(&process_input(&mut e, "addd")), "addd");
}

#[test]
fn telex_reverts_mark_modifier_for_english_continuation() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aww")), "aw");
}

#[test]
fn repeated_vowel_does_not_create_mark_across_another_vowel() {
    let mut engine = Engine::new(InputMethod::Telex);

    assert_eq!(get_display(&process_input(&mut engine, "readme")), "readme");
}

#[test]
fn telex_double_key_undo_preserves_literal_across_word_boundary() {
    let mut engine = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut engine, "tesst ")), "test ");
    assert_eq!(get_display(&process_input(&mut engine, "TESST ")), "TEST ");
    assert_eq!(get_display(&process_input(&mut engine, "Tesst ")), "Test ");
    assert_eq!(
        get_display(&process_input(&mut engine, "tesster ")),
        "tester "
    );
    assert_eq!(
        get_display(&process_input(&mut engine, "offfer ")),
        "offer "
    );
    assert_eq!(
        get_display(&process_input(&mut engine, "isssue ")),
        "issue "
    );
    assert_eq!(get_display(&process_input(&mut engine, "lawws ")), "laws ");
    assert_eq!(
        get_display(&process_input(&mut engine, "boooks ")),
        "books "
    );
}

#[test]
fn telex_auto_restore_general_stop_coda_phonology() {
    let mut engine = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut engine, "chart ")), "chart ");
    assert_eq!(get_display(&process_input(&mut engine, "text ")), "text ");
    assert_eq!(get_display(&process_input(&mut engine, "boot ")), "boot ");
    assert_eq!(get_display(&process_input(&mut engine, "keep ")), "keep ");
}

#[test]
fn telex_tone_key_can_be_typed_before_later_mark_key() {
    let mut engine = Engine::new(InputMethod::Telex);

    assert_eq!(get_display(&process_input(&mut engine, "dduojwc")), "được");
}

#[test]
fn telex_mark_key_can_be_typed_before_later_tone_key() {
    let mut engine = Engine::new(InputMethod::Telex);

    assert_eq!(get_display(&process_input(&mut engine, "neues")), "nếu");
}

#[test]
fn replay_keystrokes_reverts_tone_for_english_continuation() {
    let (output, did_flush) = Engine::replay_keystrokes(
        InputMethod::Telex,
        &std::collections::HashMap::new(),
        &['f', 'i', 'x', 'x'],
    );
    assert_eq!(output, "fix");
    assert!(!did_flush);
}

#[test]
fn telex_aw() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aw")), "ă");
}

#[test]
fn telex_ow() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ow")), "ơ");
}

#[test]
fn telex_uw() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "uw")), "ư");
}

// ================================================================
// Telex: Tones on all vowels
// ================================================================

#[test]
fn telex_tone_a_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "as")), "á");
}

#[test]
fn telex_tone_a_huyen() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "af")), "à");
}

#[test]
fn telex_tone_a_hoi() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ar")), "ả");
}

#[test]
fn telex_tone_a_nga() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ax")), "ã");
}

#[test]
fn telex_tone_a_nang() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aj")), "ạ");
}

#[test]
fn telex_tone_e_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "es")), "é");
}

#[test]
fn telex_tone_i_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "is")), "í");
}

// ================================================================
// Telex: Tones on modified vowels
// ================================================================

#[test]
fn telex_tone_aa_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aas")), "ấ");
}

#[test]
fn telex_tone_aw_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "aws")), "ắ");
}

#[test]
fn telex_tone_ee_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ees")), "ế");
}

#[test]
fn telex_tone_ow_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "ows")), "ớ");
}

#[test]
fn telex_tone_uw_sac() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "uws")), "ứ");
}

// ================================================================
// Telex: Compound vowels with tones
// ================================================================

#[test]
fn telex_oa_tone() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "oas")), "óa");
}

#[test]
fn telex_uy_tone() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "uys")), "úy");
}

// ================================================================
// Telex: Full Vietnamese words
// ================================================================

#[test]
fn telex_word_chao() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "chafo")), "chào");
}

#[test]
fn telex_word_duong() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "dduwowngf")), "đường");
}

#[test]
fn telex_word_cam_on() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "cams own")), "cám ơn");
}

// ================================================================
// VNI: Tones
// ================================================================

#[test]
fn vni_a_sac() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a1")), "á");
}

#[test]
fn vni_a_huyen() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a2")), "à");
}

#[test]
fn vni_a_hoi() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a3")), "ả");
}

#[test]
fn vni_a_nga() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a4")), "ã");
}

#[test]
fn vni_a_nang() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a5")), "ạ");
}

// ================================================================
// VNI: Vowel modifications
// ================================================================

#[test]
fn vni_a6_aa() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a6")), "â");
}

#[test]
fn vni_a8_aw() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a8")), "ă");
}

#[test]
fn vni_e6_ee() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "e6")), "ê");
}

#[test]
fn vni_o6_oo() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "o6")), "ô");
}

#[test]
fn vni_o7_ow() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "o7")), "ơ");
}

#[test]
fn vni_u7_uw() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "u7")), "ư");
}

// ================================================================
// VNI: Tone on modified vowel
// ================================================================

#[test]
fn vni_aa_sac() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a61")), "ấ");
}

#[test]
fn vni_aw_sac() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a81")), "ắ");
}

// ================================================================
// VNI: Full Vietnamese words
// ================================================================

#[test]
fn vni_word_tieng() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "tie6ng1")), "tiếng");
}

#[test]
fn vni_word_duong() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "d9u7o7ng2")), "đường");
}

// ================================================================
// Telex: dd
// ================================================================

#[test]
fn telex_dd() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "dd")), "đ");
}

#[test]
fn vni_d9() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "d9")), "đ");
}

// ================================================================
// Uppercase preservation
// ================================================================

#[test]
fn telex_uppercase_tieng() {
    let mut e = Engine::new(InputMethod::Telex);
    let events = process_input(&mut e, "Tieengs");
    let display = get_display(&events);
    assert_eq!(display, "Tiếng");
}

// ================================================================
// Macros
// ================================================================

#[test]
fn macro_ko() {
    let mut e = Engine::new(InputMethod::Telex);
    e.add_macro("ko".into(), "không".into());
    let events = process_input(&mut e, "ko ");
    assert_eq!(get_display(&events), "không ");
}

#[test]
fn macro_clear() {
    let mut e = Engine::new(InputMethod::Telex);
    e.add_macro("ko".into(), "không".into());
    e.clear_macros();
    let events = process_input(&mut e, "ko ");
    assert_eq!(get_display(&events), "ko ");
}

// ================================================================
// Toggle enabled/disabled
// ================================================================

#[test]
fn toggle_disabled() {
    let mut e = Engine::new(InputMethod::Telex);
    e.set_enabled(false);
    // When disabled, chars pass through as Insert events
    let events = process_input(&mut e, "aas");
    // a,a,s → "aas" via Insert events
    assert_eq!(get_display(&events), "aas");
}

#[test]
fn toggle_reenabled() {
    let mut e = Engine::new(InputMethod::Telex);
    e.set_enabled(false);
    e.set_enabled(true);
    assert_eq!(get_display(&process_input(&mut e, "aas")), "ấ");
}

// ================================================================
// Replay keystrokes
// ================================================================

#[test]
fn replay_telex_chao() {
    let macros = std::collections::HashMap::new();
    let (output, _) =
        Engine::replay_keystrokes(InputMethod::Telex, &macros, &['c', 'h', 'a', 'o', 'f']);
    assert_eq!(output, "chào");
}

#[test]
fn replay_vni_tieng() {
    let macros = std::collections::HashMap::new();
    let (output, _) = Engine::replay_keystrokes(
        InputMethod::Vni,
        &macros,
        &['t', 'i', 'e', '6', 'n', 'g', '1'],
    );
    assert_eq!(output, "tiếng");
}

// ================================================================
// Edge cases
// ================================================================

#[test]
fn empty_input() {
    let mut e = Engine::new(InputMethod::Telex);
    assert!(process_input(&mut e, "").is_empty());
}

#[test]
fn only_consonants() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "bcd")), "bcd");
}

#[test]
fn numbers_passthrough() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "123")), "123");
}

#[test]
fn tone_key_standalone() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "s")), "s");
}

#[test]
fn reset_clears() {
    let mut e = Engine::new(InputMethod::Telex);
    e.process_key('a');
    e.process_key('a');
    e.reset();
    assert_eq!(e.buffer(), "");
}

#[test]
fn method_switch() {
    let mut e = Engine::new(InputMethod::Telex);
    e.set_method(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "a1")), "á");
}

// ================================================================
// Spacing / flush behavior (regression)
// ================================================================

// A space after a finished word must NOT re-emit the word as a Replace
// (backspace + retype). Re-typing the already-on-screen word races with
// the separately-forwarded space in the daemon, eating spaces and merging
// words (e.g. "mất sự" -> "mấtsự"). The flush should produce no engine
// event so the space simply passes through.
#[test]
fn flush_after_word_emits_no_replace() {
    let mut e = Engine::new(InputMethod::Telex);
    // Compose "chào".
    for ch in "chaof".chars() {
        e.process_key(ch);
    }
    // Space finalizes the word — engine must return None.
    assert_eq!(e.process_key(' '), None);
}

// Punctuation flush chars behave the same as space.
#[test]
fn flush_punctuation_emits_no_replace() {
    let mut e = Engine::new(InputMethod::Telex);
    for ch in "chaof".chars() {
        e.process_key(ch);
    }
    assert_eq!(e.process_key('.'), None);
}

// Full multi-word sentence keeps every space and never concatenates words.
#[test]
fn multi_word_keeps_spacing() {
    let mut e = Engine::new(InputMethod::Telex);
    // "toio is" with telex: "tooi" -> "tôi"; "ddi" -> "đi"
    let events = process_input(&mut e, "tooi ddi hocj ");
    assert_eq!(get_display(&events), "tôi đi học ");
}

// A macro flush still expands (Replace) and keeps the trailing space.
#[test]
fn macro_flush_still_replaces() {
    let mut e = Engine::new(InputMethod::Telex);
    e.add_macro("vn".into(), "Việt Nam".into());
    let events = process_input(&mut e, "vn ");
    assert_eq!(get_display(&events), "Việt Nam ");
}

// ================================================================
// Comprehensive Vietnamese Names & Sentences
// ================================================================

#[test]
fn telex_nguyen_dang_khoa() {
    let mut e = Engine::new(InputMethod::Telex);
    let events = process_input(&mut e, "Nguyeenx DDawng Khoa");
    assert_eq!(get_display(&events), "Nguyễn Đăng Khoa");
}

#[test]
fn vni_nguyen_dang_khoa() {
    let mut e = Engine::new(InputMethod::Vni);
    let events = process_input(&mut e, "Nguyen64 D9ang8 Khoa");
    assert_eq!(get_display(&events), "Nguyễn Đăng Khoa");
}

#[test]
fn telex_full_vietnamese_sentence() {
    let mut e = Engine::new(InputMethod::Telex);
    let events = process_input(&mut e, "Coongj hoaf xax hooij chur nghiax Vieetj Nam");
    assert_eq!(get_display(&events), "Cộng hòa xã hội chủ nghĩa Việt Nam");
}

#[test]
fn vni_full_vietnamese_sentence() {
    let mut e = Engine::new(InputMethod::Vni);
    let events = process_input(&mut e, "Co6ng5 hoa2 xa4 ho6i5 chu3 nghia4 Vie6t5 Nam");
    assert_eq!(get_display(&events), "Cộng hòa xã hội chủ nghĩa Việt Nam");
}

#[test]
fn telex_standard_proverb() {
    let mut e = Engine::new(InputMethod::Telex);
    let events = process_input(&mut e, "Khoong cos gif quis hown ddoocj laapj tuwj do");
    assert_eq!(get_display(&events), "Không có gì quí hơn độc lập tự do");
}

#[test]
fn vni_standard_proverb() {
    let mut e = Engine::new(InputMethod::Vni);
    let events = process_input(&mut e, "Kho6ng co1 gi2 qui1 ho7n d9o65c la65p tu75 do");
    assert_eq!(get_display(&events), "Không có gì quí hơn độc lập tự do");
}

#[test]
fn telex_complex_words() {
    let mut e = Engine::new(InputMethod::Telex);
    assert_eq!(get_display(&process_input(&mut e, "nghieeng ")), "nghiêng ");
    assert_eq!(get_display(&process_input(&mut e, "quyeets ")), "quyết ");
    assert_eq!(get_display(&process_input(&mut e, "truwowngf ")), "trường ");
    assert_eq!(get_display(&process_input(&mut e, "thuwf ")), "thừ ");
    assert_eq!(get_display(&process_input(&mut e, "ddieefu ")), "điều ");
    assert_eq!(get_display(&process_input(&mut e, "khuaays ")), "khuấy ");
    assert_eq!(get_display(&process_input(&mut e, "chuyeern ")), "chuyển ");
    assert_eq!(get_display(&process_input(&mut e, "muooxng ")), "muỗng ");
    assert_eq!(get_display(&process_input(&mut e, "tuyeetj ")), "tuyệt ");
}

#[test]
fn vni_complex_words() {
    let mut e = Engine::new(InputMethod::Vni);
    assert_eq!(get_display(&process_input(&mut e, "nghie6ng ")), "nghiêng ");
    assert_eq!(get_display(&process_input(&mut e, "quye6t1 ")), "quyết ");
    assert_eq!(get_display(&process_input(&mut e, "tru7o7ng2 ")), "trường ");
    assert_eq!(get_display(&process_input(&mut e, "thu72 ")), "thừ ");
    assert_eq!(get_display(&process_input(&mut e, "d9ie6u2 ")), "điều ");
    assert_eq!(get_display(&process_input(&mut e, "khua6y1 ")), "khuấy ");
    assert_eq!(get_display(&process_input(&mut e, "chuye6n3 ")), "chuyển ");
    assert_eq!(get_display(&process_input(&mut e, "muo6ng4 ")), "muỗng ");
    assert_eq!(get_display(&process_input(&mut e, "tuye6t5 ")), "tuyệt ");
}

#[test]
fn phase4_replay_matches_live_processing_for_golden_inputs() {
    let macros = std::collections::HashMap::new();

    for input in ["dduojwc", "dduocwj", "neues", "neesu", "TIEENGS"] {
        let mut engine = Engine::new(InputMethod::Telex);
        let live = get_display(&process_input(&mut engine, input));
        let keys: Vec<char> = input.chars().collect();
        let (replayed, did_flush) = Engine::replay_keystrokes(InputMethod::Telex, &macros, &keys);

        assert_eq!(
            replayed, live,
            "input={input:?}, expected={live:?}, actual={replayed:?}, category=replay-live, case_id=phase4-engine-replay"
        );
        assert!(!did_flush, "input={input:?}, case_id=phase4-engine-replay");
    }
}

#[test]
fn phase4_backspace_removes_one_raw_key_and_restores_each_prior_state() {
    for input in ["dduojwc", "dduocwj", "neues", "neesu"] {
        let mut engine = Engine::new(InputMethod::Telex);
        for ch in input.chars() {
            engine.process_key(ch);
        }

        let prefixes: Vec<String> = (0..=input.chars().count())
            .map(|length| input.chars().take(length).collect())
            .collect();

        for expected_raw in prefixes.iter().rev().skip(1) {
            engine.reduce(crate::state::PhysicalEvent::Backspace);
            assert_eq!(
                engine.state().raw_keys,
                *expected_raw,
                "input={input:?}, expected={expected_raw:?}, actual={:?}, category=backspace-raw-key, case_id=phase4-engine-backspace",
                engine.state().raw_keys
            );
        }
    }

    // Closed syllable with free-order trailing tone (`vieetj` -> `việt`, `dduocwj` -> `được`):
    // Backspace followed by a new final consonant replaces the coda while preserving the vowel tone
    // (`vieetj` + BS + `c` -> `việc`, matching System Architecture Spec 7.3.4).
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "vieetj".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "việt");
    engine.reduce(crate::state::PhysicalEvent::Backspace);
    engine.process_key('c');
    assert_eq!(engine.buffer(), "việc");
}

#[test]
fn phase4_focus_change_invalidates_composition_and_prevents_trigger_crossing() {
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "ban".chars() {
        engine.reduce(crate::state::PhysicalEvent::Printable(ch));
    }

    engine.reduce(crate::state::PhysicalEvent::FocusChanged);
    assert!(engine.state().raw_keys.is_empty());
    assert_eq!(
        engine.state().boundary,
        crate::state::BoundaryState::Invalidated
    );

    engine.reduce(crate::state::PhysicalEvent::Printable('s'));
    assert_eq!(
        engine.buffer(),
        "s",
        "input=\"ban<focus>s\", expected=\"s\", actual={:?}, category=focus-boundary, case_id=phase4-engine-focus",
        engine.buffer()
    );
}

#[test]
fn office_audit_digraph_coda_acronyms_multitone_and_z_tone_removal() {
    // 1. Digraph coda backspace replacement (`tieengs` + BS + `t` -> `tiết`)
    let mut engine = Engine::new(InputMethod::Telex);
    for ch in "tieengs".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "tiếng");
    engine.process_key('\x08');
    engine.process_key('t');
    assert_eq!(engine.buffer(), "tiết");

    // 2. ALLCAPS acronyms, double-undo (`err`, `arr`, `OOO`), and multi-tone (`nextjs`, `nuxtjs`)
    for token in [
        "AWS", "DDOS", "ECS", "DOS", "ORM", "GIF", "TAR", "RAR", "EOF", "ERR", "ARR", "COO", "OOO",
        "POS", "EST", "TOS", "SOW", "ATS", "IR", "AR", "err", "arr", "nextjs", "nuxtjs", "var",
        "usr", "res", "ref", "pos", "eof",
    ] {
        let mut eng = Engine::new(InputMethod::Telex);
        let mut screen = String::new();
        for ch in format!("{token} ").chars() {
            match eng.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if ch == ' ' {
                        screen.push(' ');
                    }
                }
                None => screen.push(ch),
                _ => {}
            }
        }
        assert_eq!(screen, format!("{token} "), "token={token}");
    }

    // 3. Telex 'z' tone removal followed by Space must NOT auto-restore to raw `vieetjz `
    let mut eng = Engine::new(InputMethod::Telex);
    let mut screen = String::new();
    for ch in "vieetjz ".chars() {
        match eng.process_key(ch) {
            Some(EngineEvent::Replace(rep)) => {
                let keep = screen
                    .chars()
                    .count()
                    .saturating_sub(rep.delete_before_cursor);
                screen = screen.chars().take(keep).collect();
                screen.push_str(&rep.insert);
                if ch == ' ' {
                    screen.push(' ');
                }
            }
            None => screen.push(ch),
            _ => {}
        }
    }
    assert_eq!(screen, "viêt ");
}

#[test]
fn pasted_prefix_backspace_is_o1_single_char_delete() {
    let mut eng = Engine::new(InputMethod::Telex);
    let pasted = "a".repeat(2_000);
    eng.update_with_pasted_text(&pasted);
    match eng.process_key('\x08') {
        Some(EngineEvent::Replace(rep)) => {
            assert_eq!(rep.delete_before_cursor, 1);
            assert!(rep.insert.is_empty());
        }
        other => panic!("expected O(1) Replace(1, \"\"), got {other:?}"),
    }
    assert_eq!(eng.buffer().len(), 1_999);
}

#[test]
fn pasted_prefix_backspace_recovery_preserves_prefix_and_stays_o1() {
    let mut eng = Engine::new(InputMethod::Telex);
    eng.update_with_pasted_text("hello");
    let mut screen = String::from("hello");

    // First BS fails to apply on screen (`recover_composition`), so screen still has "hello".
    assert!(matches!(
        eng.process_key('\x08'),
        Some(EngineEvent::Replace(rep)) if rep.delete_before_cursor == 1 && rep.insert.is_empty()
    ));
    eng.recover_composition();

    // Second BS succeeds: deletes only the 2 removed chars ("lo") -> "hel", O(1).
    match eng.process_key('\x08') {
        Some(EngineEvent::Replace(rep)) => {
            assert_eq!(rep.delete_before_cursor, 2);
            assert!(rep.insert.is_empty());
            let keep = screen
                .chars()
                .count()
                .saturating_sub(rep.delete_before_cursor);
            screen = screen.chars().take(keep).collect();
        }
        other => panic!("expected Replace(2, \"\"), got {other:?}"),
    }
    assert_eq!(screen, "hel");

    // Another BS fails, then user types 'a' -> deletes only the 1 un-deleted char ('l') and inserts "a" -> "hea".
    assert!(eng.process_key('\x08').is_some());
    eng.recover_composition();
    match eng.process_key('a') {
        Some(EngineEvent::Replace(rep)) => {
            assert_eq!(rep.delete_before_cursor, 1);
            assert_eq!(rep.insert, "a");
            let keep = screen
                .chars()
                .count()
                .saturating_sub(rep.delete_before_cursor);
            screen = screen.chars().take(keep).collect();
            screen.push_str(&rep.insert);
        }
        other => panic!("expected Replace(1, \"a\"), got {other:?}"),
    }
    assert_eq!(screen, "hea");
    assert_eq!(eng.buffer(), "hea");
}
