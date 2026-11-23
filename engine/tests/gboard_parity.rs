// SPDX-License-Identifier: MIT
use dboard_engine::{Engine, EngineEvent, InputMethod};

fn simulate_typing(method: InputMethod, keys: &str) -> String {
    let mut engine = Engine::new(method);
    let mut screen = String::new();
    for ch in keys.chars() {
        if ch == '\x08' {
            match engine.process_key('\x08') {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                }
                None => {
                    let _ = screen.pop();
                }
                other => panic!("unexpected backspace event: {other:?}"),
            }
            continue;
        }

        let buf_before = engine.buffer().chars().count();
        match engine.process_key(ch) {
            Some(EngineEvent::Replace(rep)) => {
                let keep = screen
                    .chars()
                    .count()
                    .saturating_sub(rep.delete_before_cursor);
                screen = screen.chars().take(keep).collect();
                screen.push_str(&rep.insert);
                if matches!(ch, ' ' | ',' | '.' | '!' | '?' | ';' | ':' | '\n') {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::Insert(s)) => screen.push_str(&s),
            None => {
                let is_vn_ctrl = match method {
                    InputMethod::Telex => matches!(
                        ch.to_ascii_lowercase(),
                        's' | 'f' | 'r' | 'x' | 'j' | 'w' | 'z'
                    ),
                    InputMethod::Vni => ch.is_ascii_digit(),
                };
                if is_vn_ctrl
                    && !matches!(ch, ' ' | ',' | '.' | '!' | '?' | ';' | ':' | '\n')
                    && engine.buffer().chars().count() <= buf_before
                {
                    // Consumed control key with no visible delta
                } else {
                    screen.push(ch);
                }
            }
            other => panic!("unexpected event for {ch:?}: {other:?}"),
        }
    }
    screen
}

#[test]
fn backspace_across_space_rehydrates_committed_word_with_zero_underline() {
    // 1. Add forgotten tone after Space: `"xin chao "` + BS + `"f "` -> `"xin chào "`
    assert_eq!(
        simulate_typing(InputMethod::Telex, "xin chao \x08f "),
        "xin chào "
    );

    // 2. Retarget existing tone after Space: `"xin chaof "` + BS + `"s "` -> `"xin cháo "`
    assert_eq!(
        simulate_typing(InputMethod::Telex, "xin chaof \x08s "),
        "xin cháo "
    );

    // 3. Old-style tone shifts when appending final consonant after backspacing Space:
    //    `"hoaf "` ("hòa ") + BS + `"n "` -> `"hoàn "`
    assert_eq!(simulate_typing(InputMethod::Telex, "hoaf \x08n "), "hoàn ");

    // 4. Multi-word character-granular Backspace across 4 committed words:
    //    `"xin chào bạn hiền "` -> 15 Backspaces (`"hiền "`=5 + `"bạn "`=4 + `"chào "`=5 + `' '`=1)
    //    leaves `"xin"`, then `"looix "`? Wait: after 15 BS caret is right at `"xin"`, so `" looix "` -> `"xin lỗi "`!
    let fifteen_bs = "\x08".repeat(15);
    let seq = format!("xin chaof banj hieenf {fifteen_bs} looix ");
    assert_eq!(simulate_typing(InputMethod::Telex, &seq), "xin lỗi ");

    // 5. English auto-restored word re-edited across Space stays literal ASCII (`w` not turned into `ư`)
    assert_eq!(
        simulate_typing(InputMethod::Telex, "open window \x08s "),
        "open windows "
    );

    // 6. VNI mode re-editing across Space: `"xin chao "` + BS + `"2 "` -> `"xin chào "`
    assert_eq!(
        simulate_typing(InputMethod::Vni, "xin chao \x082 "),
        "xin chào "
    );
}

#[test]
fn forward_word_bigrams_resolve_vietnamese_english_code_switching() {
    let cases = [
        ("lamf baif test ", "làm bài test "),
        ("chayj booj test ", "chạy bộ test "),
        ("awn banhs test ", "ăn bánh tét "),
        ("ddi xe bus ", "đi xe bus "),
        ("baatj beeps gas ", "bật bếp gas "),
        ("ddooir binhf gas ", "đổi bình gas "),
        ("vaof quans bar ", "vào quán bar "),
        ("check file ddos ", "check file đó "),
        ("mitigate ddos ", "mitigate ddos "),
        ("fix bug gif ", "fix bug gì "),
        ("animated gif ", "animated gif "),
        ("xem repo teen gif ", "xem repo tên gì "),
    ];
    for (typed, expected) in cases {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "failed forward bigram case: {typed:?}"
        );
    }
}

#[test]
fn retroactive_two_word_correction_resolves_sentence_initial_ambiguities() {
    let cases = [
        // Direction A: VN -> EN retroactive phrase correction at sentence start
        ("Lots of data ", "Lots of data "),
        ("Sets of rules ", "Sets of rules "),
        ("Too much work ", "Too much work "),
        ("Too phowr bof ", "Tô phở bò "),
        ("Its own value ", "Its own value "),
        ("Its nhaats ", "Ít nhất "),
        ("Been updated ", "Been updated "),
        ("Been canhj ", "Bên cạnh "),
        // Direction B: EN -> VN retroactive compound correction at sentence start
        ("Last nuwax gawpj nhes ", "Lát nữa gặp nhé "),
        ("Last week ", "Last week "),
        ("Nest ddepj truyeefn thoongs ", "Nét đẹp truyền thống "),
        ("Cast buij ", "Cát bụi "),
        ("Max soos baor maatj ", "Mã số bảo mật "),
        ("Max nguoonf mowr ", "Mã nguồn mở "),
        ("Max value ", "Max value "),
        ("Rust tieenf mawtj ", "Rút tiền mặt "),
        ("Rust laf ngoon nguwx ", "Rust là ngôn ngữ "),
        ("This nghieemj laam sangf ", "Thí nghiệm lâm sàng "),
        ("This sinh duwj thi ", "Thí sinh dự thi "),
        ("Six quan ", "Sĩ quan "),
        ("Six months ", "Six months "),
        ("Mix thuaatj ", "Mĩ thuật "),
        ("Own trowif ", "Ơn trời "),
        ("How luwar ", "Hơ lửa "),
        ("List xawng ", "Lít xăng "),
        ("Lawn banhs ", "Lăn bánh "),
        ("Noon nongs ", "Nôn nóng "),
    ];
    for (typed, expected) in cases {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "failed retroactive 2-word case: {typed:?}"
        );
    }
}

#[test]
fn morphological_oov_compounds_restore_while_telex_escapes_stay_escaped() {
    assert_eq!(
        simulate_typing(
            InputMethod::Telex,
            "serverless treesitter storybook glassware classroom "
        ),
        "serverless treesitter storybook glassware classroom "
    );
    assert_eq!(
        simulate_typing(InputMethod::Telex, "tesst boooks "),
        "test books "
    );
}

#[test]
fn qa_audit_homographs_contractions_uo_triphthongs_and_double_oo_regressions() {
    let cases = [
        // QA-06: English pronoun "I", short function words "to"/"a"/"an", and contractions
        ("I see ", "I see "),
        ("I own ", "I own "),
        ("to test ", "to test "),
        ("to see ", "to see "),
        ("I've been ", "I've been "),
        ("we've been ", "we've been "),
        ("buy a car ", "buy a car "),
        // QA-07: Bilingual unmarked ASCII homographs preserve Vietnamese context
        ("con car ", "con cả "),
        ("mays in nets ", "máy in nét "),
        ("awn me its ", "ăn me ít "),
        ("vair may lots ", "vải may lót "),
        // QA-08: Retroactive phrase guard never corrupts Vietnamese phrases
        ("Nuts in ", "Nút in "),
        ("Nets in ", "Nét in "),
        ("Been An ", "Bên An "),
        ("Too Long ", "Tô Long "),
        ("Car has ", "Cả há "),
        // QA-09: Open `-uơ` words (`thuở`, `huơ`, `khuơ`) + `oao`/`oeo` triphthongs (`ngoáo`, `ngoẻo`)
        ("thuowr xuaw ", "thuở xưa "),
        ("huow tay ", "huơ tay "),
        ("khuow dduax ", "khuơ đũa "),
        ("ngoaos oopj ", "ngoáo ộp "),
        ("ngoawnf ngoeof ", "ngoằn ngoèo "),
        // QA-10: Double-`oo` words (`xoong`, `moóc`) survive Backspace across Space
        ("xooong \x08 ", "xoong "),
        ("mooocs \x08 ", "moóc "),
    ];
    for (typed, expected) in cases {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "QA regression failed for {typed:?}"
        );
    }
}

#[test]
fn context_aware_surrounding_word_edits_mid_sentence_safely() {
    let apply_after_seed = |method: InputMethod, seeded: &str, keys: &str| -> String {
        let mut engine = Engine::new(method);
        let mut screen = seeded.to_string();
        let _ = engine.seed_surrounding_word(seeded);
        for ch in keys.chars() {
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                }
                None => screen.push(ch),
                other => panic!("unexpected event for {seeded:?} + {ch:?}: {other:?}"),
            }
        }
        screen
    };

    // Add forgotten tone after clicking behind `"chao"` mid-sentence
    assert_eq!(apply_after_seed(InputMethod::Telex, "chao", "f"), "chào");
    // Retarget existing tone (`"chào"` -> `"cháo"`)
    assert_eq!(apply_after_seed(InputMethod::Telex, "chào", "s"), "cháo");
    // Strip existing tone (`"chào"` -> `"chao"`)
    assert_eq!(apply_after_seed(InputMethod::Telex, "chào", "z"), "chao");
    // Old-style tone shifts when appending coda (`"hòa"` + `n` -> `"hoàn"`)
    assert_eq!(apply_after_seed(InputMethod::Telex, "hòa", "n"), "hoàn");
    // VNI mid-sentence tone addition & retargeting
    assert_eq!(apply_after_seed(InputMethod::Vni, "chao", "2"), "chào");
    assert_eq!(apply_after_seed(InputMethod::Vni, "chào", "1"), "cháo");
    // English / non-Vietnamese words are NEVER seeded or corrupted
    let mut eng = Engine::new(InputMethod::Telex);
    assert!(!eng.seed_surrounding_word("windows"));
    assert!(!eng.seed_surrounding_word("hello"));
    assert!(!eng.seed_surrounding_word("server"));
    assert_eq!(
        apply_after_seed(InputMethod::Telex, "windows", "s"),
        "windowss"
    );
}

#[test]
fn trigram_lm_expanded_tech_lexicon_and_spatial_autocorrect_with_one_tap_undo() {
    // 1. Trigram LM disambiguates 3-word English & Vietnamese idioms
    let trigram_cases = [
        ("it has been ", "it has been "),
        ("on its own ", "on its own "),
        ("how to test ", "how to test "),
        ("khoong cos gif ", "không có gì "),
        ("ddi awn too phowr ", "đi ăn tô phở "),
        ("awn banhs test ", "ăn bánh tét "),
        // Expanded Linux / COSMIC / Wayland / Dev lexicon
        ("cai dat cosmic wayland ", "cai dat cosmic wayland "),
        (
            "chayj docker kubernetes nginx ",
            "chạy docker kubernetes nginx ",
        ),
        (
            "check workflow dashboard feedback ",
            "check workflow dashboard feedback ",
        ),
    ];
    for (typed, expected) in trigram_cases {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "trigram/lexicon case failed for {typed:?}"
        );
    }

    // 2. Spatial autocorrect + One-Tap Undo (Gboard parity)
    let simulate_ac = |method: InputMethod, keys: &str| -> String {
        let mut engine = Engine::new(method);
        engine.set_autocorrect(true);
        let mut screen = String::new();
        for ch in keys.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected backspace event: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
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
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    assert_eq!(
        simulate_ac(InputMethod::Telex, "tienegs vieetj "),
        "tiếng việt "
    );
    assert_eq!(
        simulate_ac(InputMethod::Telex, "khonog cos gif "),
        "không có gì "
    );
    assert_eq!(
        simulate_ac(InputMethod::Telex, "teh cosimc wayalnd "),
        "the cosmic wayland "
    );
    // Regression: autocorrected English word (`sercer` -> `server`) must classify as English context
    // so subsequent bilingual collisions (`car` vs `cả`) stay English!
    assert_eq!(
        simulate_ac(InputMethod::Telex, "sercer car "),
        "server car "
    );
    // One-Tap Undo: Backspace immediately after autocorrect restores user's original text
    assert_eq!(simulate_ac(InputMethod::Telex, "tienegs \x08 "), "tienegs ");
    assert_eq!(simulate_ac(InputMethod::Telex, "teh \x08 "), "teh ");
    // VNI parity with autocorrect + One-Tap Undo
    assert_eq!(simulate_ac(InputMethod::Vni, "teh cosimc "), "the cosmic ");
    assert_eq!(simulate_ac(InputMethod::Vni, "teh \x08 "), "teh ");
}

#[test]
fn personal_dictionary_learns_from_one_tap_undo_and_auto_capitalize_respects_urls() {
    let type_with = |engine: &mut Engine, keys: &str| -> String {
        let mut screen = String::new();
        for ch in keys.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected backspace event: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if matches!(ch, ' ' | '.' | '!' | '?' | '\n') {
                        screen.push(ch);
                    }
                }
                None => screen.push(ch),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // 1. Personal dictionary learns from One-Tap Undo:
    //    First time: `lets ` auto-restores to `lets `; user hits Backspace -> `lét` and learns `keep_vn:lets`!
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_autocorrect(true);
    assert_eq!(type_with(&mut engine, "lets \x08 "), "lét ");
    let saved = engine
        .take_user_dict_for_save()
        .expect("undo must mark user_dict dirty");
    assert!(saved.contains("keep_vn:lets"));

    // Next time (even on a fresh engine loaded from disk), `lets ` stays `lét `!
    let mut reloaded = Engine::new(InputMethod::Telex);
    reloaded.set_autocorrect(true);
    reloaded.set_user_dict(dboard_engine::UserDict::load_from_str(&saved));
    assert_eq!(type_with(&mut reloaded, "lets "), "lét ");

    // Undoing autocorrect on `teh` teaches `keep_raw:teh` so it never autocorrects again
    assert_eq!(type_with(&mut reloaded, "teh \x08 "), "teh ");
    assert_eq!(type_with(&mut reloaded, "teh "), "teh ");

    // 2. Smart Auto-Capitalization after `. `, `! `, `? `, `\n` without touching URLs/versions
    let mut cap_engine = Engine::new(InputMethod::Telex);
    cap_engine.set_auto_capitalize(true);
    assert_eq!(
        type_with(
            &mut cap_engine,
            "xin chaof. hoom nay! thaatj tuyeetj? ddungs "
        ),
        "xin chào. Hôm nay! Thật tuyệt? Đúng "
    );
    cap_engine.reset();
    assert_eq!(
        type_with(&mut cap_engine, "xem google.com v1.0 nhes "),
        "xem google.com v1.0 nhé "
    );
}

#[test]
fn pillar1_lexicon_spatial_autocorrect_bigrams_and_adaptive_user_dict() {
    // 1. 35k extended English lexicon words auto-restore cleanly in Telex mode
    for word in [
        "beautiful",
        "architecture",
        "configuration",
        "vulnerability",
        "accessibility",
        "spreadsheet",
        "presentation",
        "optimization",
        "infrastructure",
        "documentation",
    ] {
        assert_eq!(
            simulate_typing(InputMethod::Telex, &format!("{word} ")),
            format!("{word} "),
            "extended English word {word:?} should auto-restore cleanly"
        );
    }

    // 2. Pruned Vietnamese Telex words must NEVER be corrupted by the extended English lexicon
    for (raw, expected) in [
        ("chaos ", "cháo "),
        ("chair ", "chải "),
        ("char ", "chả "),
        ("chef ", "chè "),
        ("barn ", "bản "),
        ("ghee ", "ghê "),
        ("giro ", "giỏ "),
        ("bust ", "bút "),
    ] {
        assert_eq!(
            simulate_typing(InputMethod::Telex, raw),
            expected,
            "Vietnamese Telex word {raw:?} must stay {expected:?}"
        );
    }

    // 3. Vietnamese compound bigrams override English-dominant collisions cleanly
    assert_eq!(
        simulate_typing(InputMethod::Telex, "banhs test "),
        "bánh tét "
    );
    assert_eq!(simulate_typing(InputMethod::Telex, "hoof sow "), "hồ sơ ");
    assert_eq!(simulate_typing(InputMethod::Telex, "bacs six "), "bác sĩ ");
    assert_eq!(simulate_typing(InputMethod::Telex, "camr own "), "cảm ơn ");
    assert_eq!(simulate_typing(InputMethod::Telex, "cas tree "), "cá trê ");
    assert_eq!(simulate_typing(InputMethod::Telex, "taats car "), "tất cả ");

    // 4. All 4 Spatial & Transposition Autocorrect classes end-to-end + One-Tap Undo
    let type_ac = |engine: &mut Engine, keys: &str| -> String {
        let mut screen = String::new();
        for ch in keys.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected backspace event: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
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
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    let mut ac = Engine::new(InputMethod::Telex);
    ac.set_autocorrect(true);
    assert_eq!(
        type_ac(&mut ac, "tieemgs vieetk khoonh dduowch "),
        "tiếng việt không được "
    );
    ac.reset();
    assert_eq!(
        type_ac(&mut ac, "khoongg vieettj proejct "),
        "không việt project "
    );
    ac.reset();
    // One-Tap Undo reverts spatial autocorrect back to original raw keys
    assert_eq!(type_ac(&mut ac, "khoonh \x08 "), "khoonh ");

    // 5. Adaptive Personal Dictionary learns repeated OOV ASCII words and persists them
    let mut learner = Engine::new(InputMethod::Telex);
    learner.set_autocorrect(true);
    assert_eq!(type_ac(&mut learner, "thanhhd "), "thanhhd ");
    assert!(
        learner.take_user_dict_for_save().is_none(),
        "single occurrence should not promote a personal word yet"
    );
    assert_eq!(type_ac(&mut learner, "thanhhd "), "thanhhd ");
    let serialized = learner
        .take_user_dict_for_save()
        .expect("second deliberate commit must promote and persist personal word");
    assert!(serialized.contains("learned:thanhhd:2"));
    let reloaded = dboard_engine::UserDict::load_from_str(&serialized);
    assert!(reloaded.is_known_personal_word("thanhhd"));
}

#[test]
fn pillar3_surrounding_context_bigrams_autocap_and_vni_parity() {
    let type_with = |engine: &mut Engine, keys: &str| -> String {
        let mut screen = String::new();
        for ch in keys.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected backspace event: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if matches!(ch, ' ' | '.' | '!' | '?' | '\n') {
                        screen.push(ch);
                    }
                }
                None => screen.push(ch),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // 1. Clicking after a space mid-sentence seeds prev_word bigram context:
    //    After "bánh |", typing "test " keeps Vietnamese compound "tét "
    let mut vn_ctx = Engine::new(InputMethod::Telex);
    assert!(vn_ctx.seed_surrounding_context(None, Some("bánh"), false));
    assert_eq!(type_with(&mut vn_ctx, "test "), "tét ");

    //    After "cargo |", typing "test " stays English "test "
    let mut en_ctx = Engine::new(InputMethod::Telex);
    assert!(en_ctx.seed_surrounding_context(None, Some("cargo"), false));
    assert_eq!(type_with(&mut en_ctx, "test "), "test ");

    // 2. Clicking after sentence punctuation + space arms Smart Auto-Capitalization
    let mut sent_ctx = Engine::new(InputMethod::Telex);
    sent_ctx.set_auto_capitalize(true);
    assert!(sent_ctx.seed_surrounding_context(None, None, true));
    assert_eq!(type_with(&mut sent_ctx, "hoom "), "Hôm ");

    // 3. Full VNI Parity: surrounding-word editing + tone strip
    let mut vni_edit = Engine::new(InputMethod::Vni);
    assert!(vni_edit.seed_surrounding_context(Some("chao"), Some("xin"), false));
    let mut screen = "chao".to_string();
    if let Some(EngineEvent::Replace(rep)) = vni_edit.process_key('2') {
        let keep = screen
            .chars()
            .count()
            .saturating_sub(rep.delete_before_cursor);
        screen = screen.chars().take(keep).collect();
        screen.push_str(&rep.insert);
    }
    assert_eq!(screen, "chào");

    let mut vni_strip = Engine::new(InputMethod::Vni);
    assert!(vni_strip.seed_surrounding_word("chào"));
    let mut strip_screen = "chào".to_string();
    if let Some(EngineEvent::Replace(rep)) = vni_strip.process_key('0') {
        let keep = strip_screen
            .chars()
            .count()
            .saturating_sub(rep.delete_before_cursor);
        strip_screen = strip_screen.chars().take(keep).collect();
        strip_screen.push_str(&rep.insert);
    }
    assert_eq!(strip_screen, "chao");

    // 4. VNI Backspace-across-space rehydration
    assert_eq!(
        simulate_typing(InputMethod::Vni, "xin chao \x082 "),
        "xin chào "
    );

    // 5. VNI Spatial + Duplicate-Key Autocorrect + One-Tap Undo + double-digit undo preserved
    let mut vni_ac = Engine::new(InputMethod::Vni);
    vni_ac.set_autocorrect(true);
    assert_eq!(
        type_with(&mut vni_ac, "tiemg61 khonh6 khoong6 "),
        "tiếng không không "
    );
    vni_ac.reset();
    assert_eq!(type_with(&mut vni_ac, "khonh6 \x08 "), "khonh6 ");
    vni_ac.reset();
    // VNI double-digit undo (`a66 -> a6`) must never be corrupted by duplicate-key autocorrect
    assert_eq!(type_with(&mut vni_ac, "a66 "), "a6 ");

    // 6. VNI Smart Auto-Capitalization + Adaptive UserDict
    let mut vni_cap = Engine::new(InputMethod::Vni);
    vni_cap.set_auto_capitalize(true);
    assert_eq!(
        type_with(&mut vni_cap, "xin chao2. hom6 nay "),
        "xin chào. Hôm nay "
    );
    let mut vni_learn = Engine::new(InputMethod::Vni);
    assert_eq!(
        type_with(&mut vni_learn, "thanhhd thanhhd "),
        "thanhhd thanhhd "
    );
    let saved = vni_learn
        .take_user_dict_for_save()
        .expect("VNI must also learn repeated OOV personal words");
    assert!(saved.contains("learned:thanhhd:2"));
}

#[test]
fn qa_p0_sentence_boundary_slang_and_multi_backspace_regressions() {
    // 1. Period + Space must NOT leak '.' boundary context into the next sentence's first word
    let sentence_cases = [
        ("Xin chaof. Car nhaf ", "Xin chào. Cả nhà "),
        ("Xin chaof. Been trong ", "Xin chào. Bên trong "),
        ("Xin chaof. Teen tooi ", "Xin chào. Tên tôi "),
        ("Xin chaof. Gif vaayj ", "Xin chào. Gì vậy "),
        ("Xin chaof. Too phowr ", "Xin chào. Tô phở "),
        ("Xin chaof. Mats me ", "Xin chào. Mát me "),
        // True dot-separated extensions/domains without space MUST still auto-restore
        ("app.conf ", "app.conf "),
        ("file.tar ", "file.tar "),
    ];
    for (typed, expected) in sentence_cases {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "sentence boundary regression failed for {typed:?}"
        );
    }

    // 2. Valid non-dictionary Vietnamese syllables (slang/dialect/names) must NOT be forced to ASCII
    for (typed, expected) in [("quaoj ", "quạo "), ("gheej ", "ghệ "), ("deoj ", "dẹo ")] {
        assert_eq!(
            simulate_typing(InputMethod::Telex, typed),
            expected,
            "valid non-dict VN syllable failed for {typed:?}"
        );
    }
    // English collision ("torn") restores to English, and One-Tap Backspace Undo recovers "tỏn"
    assert_eq!(simulate_typing(InputMethod::Telex, "torn "), "torn ");
    assert_eq!(simulate_typing(InputMethod::Telex, "torn \x08 "), "tỏn ");

    // 3. Multi-backspace on accented closed syllable must not drop vowel on ASCII continuation
    assert_eq!(
        simulate_typing(InputMethod::Telex, "toans\x08\x08n"),
        "toan"
    );
    assert_eq!(
        simulate_typing(InputMethod::Telex, "toans\x08\x08i"),
        "toai"
    );
    // Single-backspace coda edit must still work (`vieetj` + BS + `c` -> `việc`)
    assert_eq!(simulate_typing(InputMethod::Telex, "vieetj\x08c"), "việc");
}

#[test]
fn qa_p1_vni_uo_undo_autocorrect_context_and_lm_regressions() {
    // 1. VNI `ươ` compound promotion and double-key `77` undo
    assert_eq!(simulate_typing(InputMethod::Vni, "d9u7oc5 "), "được ");
    assert_eq!(simulate_typing(InputMethod::Vni, "tuo77 "), "tuo7 ");
    assert_eq!(simulate_typing(InputMethod::Vni, "duoc77 "), "duoc7 ");

    // 2. LM must NOT emit redundant AutoRestore on unchanged ASCII trigrams
    let mut eng = Engine::new(InputMethod::Telex);
    for ch in "one of the".chars() {
        let _ = eng.process_key(ch);
    }
    assert_eq!(
        eng.process_key(' '),
        None,
        "unchanged ASCII trigram 'one of the ' must not emit redundant AutoRestore"
    );
    // LM VN compound pair must NOT bypass ALL-CAPS / MixedCase acronym guards,
    // while ALL-CAPS Vietnamese sentences (`CAIS GIF `) still stay Vietnamese!
    assert_eq!(simulate_typing(InputMethod::Telex, "cais GIF "), "cái GIF ");
    assert_eq!(
        simulate_typing(InputMethod::Telex, "cais DDoS "),
        "cái DDoS "
    );
    assert_eq!(simulate_typing(InputMethod::Telex, "CAIS GIF "), "CÁI GÌ ");

    // 3. Autocorrect respects English LanguageContext & One-Tap Undo keeps screen in sync
    let mut ac = Engine::new(InputMethod::Telex);
    ac.set_autocorrect(true);
    let type_ac = |engine: &mut Engine, keys: &str| -> String {
        let mut screen = String::new();
        for ch in keys.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected bs: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
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
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };
    assert_eq!(type_ac(&mut ac, "run tmux "), "run tmux ");
    ac.reset();
    // Autocorrect undo followed by a second Backspace must delete 1 char cleanly (`tmux` -> `tmu`),
    // and erasing the entire undone word must NOT leave the next Vietnamese word stuck in ASCII.
    assert_eq!(type_ac(&mut ac, "tmux \x08\x08"), "tmu");
    ac.reset();
    assert_eq!(
        type_ac(&mut ac, "tmux \x08\x08\x08\x08\x08vieetj "),
        "việt "
    );
    ac.reset();
    // Retroactive 2-word correction preserves Word 2 autocorrect (`Too mnay ` -> `Too many `)
    assert_eq!(type_ac(&mut ac, "Too mnay "), "Too many ");
}

#[test]
fn p0_guarded_rehydration_preserves_prefix_after_backspacing_into_word() {
    let type_str = |input: &str| -> String {
        let mut engine = Engine::new(InputMethod::Telex);
        let mut screen = String::new();
        for ch in input.chars() {
            if ch == '\x08' {
                match engine.process_key('\x08') {
                    Some(EngineEvent::Replace(rep)) => {
                        let keep = screen
                            .chars()
                            .count()
                            .saturating_sub(rep.delete_before_cursor);
                        screen = screen.chars().take(keep).collect();
                        screen.push_str(&rep.insert);
                    }
                    None => {
                        let _ = screen.pop();
                    }
                    other => panic!("unexpected backspace event: {other:?}"),
                }
                continue;
            }
            match engine.process_key(ch) {
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
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // Backspacing into a rehydrated word ("quán " -> BSx4 -> "q") and retyping "uas"
    // must compose "quá", NOT drop 'q' and produce "qúa".
    assert_eq!(type_str("quans \x08\x08\x08\x08uas "), "quá ");
    // Backspacing into "người " -> "ng" and typing "hiax" must compose "nghĩa".
    assert_eq!(type_str("nguwowif \x08\x08\x08\x08hiax "), "nghĩa ");
    // Invariant 1 still holds when ONLY Space is deleted: "cấu " + BS + "h" splits cleanly.
    assert_eq!(type_str("caaus \x08hoaf "), "cấuhòa ");
}

#[test]
fn p2_sentence_initial_vietnamese_compounds_and_code_autocorrect_guards() {
    let type_str = |autocorrect: bool, input: &str| -> String {
        let mut engine = Engine::new(InputMethod::Telex);
        engine.set_autocorrect(autocorrect);
        let mut screen = String::new();
        for ch in input.chars() {
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if matches!(ch, ' ' | '_' | '/' | '.') {
                        screen.push(ch);
                    }
                }
                None => screen.push(ch),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // Sentence-initial Vietnamese lexical compounds retroactively recover from English
    assert_eq!(type_str(false, "Rets ddaamj "), "Rét đậm ");
    assert_eq!(type_str(false, "As haauj "), "Á hậu ");
    assert_eq!(type_str(false, "Low mow "), "Lơ mơ ");
    assert_eq!(type_str(false, "Now ddor "), "Nơ đỏ ");
    assert_eq!(type_str(false, "Bus suwax "), "Bú sữa ");
    assert_eq!(type_str(false, "Us oaf "), "Ú òa ");
    assert_eq!(type_str(false, "If achj "), "Ì ạch ");
    assert_eq!(type_str(false, "Rust khoir "), "Rút khỏi ");

    // Autocorrect must never corrupt snake_case, paths, or Vietnamese "adn"
    assert_eq!(type_str(true, "foo_teh /usr/adn "), "foo_teh /usr/adn ");
    assert_eq!(type_str(true, "xets nghieemj adn "), "xét nghiệm adn ");
}

#[test]
fn short_camelcase_preserved_while_early_shift_vietnamese_works() {
    let type_line = |input: &str| -> String {
        let mut engine = Engine::new(InputMethod::Telex);
        engine.set_enabled(true);
        let mut screen = String::new();
        for ch in input.chars() {
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if ch == ' ' {
                        screen.push(ch);
                    }
                }
                None => screen.push(ch),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // Short 3-4 char camelCase tech tokens stay intact
    assert_eq!(
        type_line("dOs cAs tOs iOs mAc dDos "),
        "dOs cAs tOs iOs mAc dDos "
    );
    // Early-Shift trailing modifier & long-word Vietnamese syllables still compose cleanly
    assert_eq!(
        type_line("chaoF VieetJ cHAOF vieeTj nguowIf dduowNgf "),
        "chào Việt cHÀO việt người đường "
    );
    // English mid-word `dd` words never flash `đ` and stay English
    assert_eq!(
        type_line("add reddit hidden middle wedding "),
        "add reddit hidden middle wedding "
    );
}

#[test]
fn task_a4_telex_vni_parity_and_circumflex_across_coda() {
    use dboard_engine::ReplacementReason;

    let type_with = |method: InputMethod, input: &str| -> String {
        let mut engine = Engine::new(method);
        engine.set_enabled(true);
        let mut screen = String::new();
        for ch in input.chars() {
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if dboard_engine::is_flush_char(ch) {
                        screen.push(ch);
                    }
                }
                None => screen.push(ch),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // 1. `ddsd` / `ddsf`: no bamboo-vs-telex undo desync (never emits false UndoTransform)
    let mut ddsd = Engine::new(InputMethod::Telex);
    assert!(ddsd.process_key('d').is_none());
    assert!(matches!(
        ddsd.process_key('d'),
        Some(EngineEvent::Replace(r)) if r.insert == "đ"
    ));
    for ch in ['s', 'd'] {
        if let Some(EngineEvent::Replace(r)) = ddsd.process_key(ch) {
            assert_ne!(r.reason, ReplacementReason::UndoTransform);
        }
    }
    assert_eq!(ddsd.buffer(), "đsd");

    // 2. `seed_surrounding_context` immediately after `invalidate_composition` uses Recomposition
    let mut inv = Engine::new(InputMethod::Telex);
    inv.invalidate_composition();
    assert!(inv.seed_surrounding_context(Some("thương"), Some("yêu"), false));
    assert_eq!(inv.buffer(), "thương");

    // 3. VNI active_transform parity: `a1` -> `a2` emits ReplaceTransform; `a11` emits UndoTransform
    let mut vni = Engine::new(InputMethod::Vni);
    assert!(vni.process_key('a').is_none());
    assert!(matches!(
        vni.process_key('1'),
        Some(EngineEvent::Replace(r)) if r.insert == "á" && r.reason == ReplacementReason::ApplyTransform
    ));
    assert!(matches!(
        vni.process_key('2'),
        Some(EngineEvent::Replace(r)) if r.insert == "à" && r.reason == ReplacementReason::ReplaceTransform
    ));
    assert!(matches!(
        vni.process_key('2'),
        Some(EngineEvent::Replace(r)) if r.insert == "a2" && r.reason == ReplacementReason::UndoTransform
    ));

    // 4. VNI `'0'` and Telex `'z'` tone-strip set `override_keep_vietnamese` even when result is ASCII
    assert_eq!(type_with(InputMethod::Vni, "toan10 "), "toan ");
    assert_eq!(type_with(InputMethod::Telex, "toansz "), "toan ");

    // 5. Telex circumflex across Vietnamese coda (`khongo`, `vietej`, `tienges`, `tana`)
    assert_eq!(
        type_with(InputMethod::Telex, "khongo vietej tienges tana "),
        "không việt tiếng tân "
    );
    // English words must remain untouched
    assert_eq!(
        type_with(
            InputMethod::Telex,
            "banana canada delete choose mono mama meme benefit manage tenet "
        ),
        "banana canada delete choose mono mama meme benefit manage tenet "
    );
}

#[test]
fn task_b2_and_b3_vni_autocorrect_retroactive_undo_and_smart_autocap() {
    let type_sim = |method: InputMethod, autocorr: bool, autocap: bool, input: &str| -> String {
        let mut engine = Engine::new(method);
        engine.set_enabled(true);
        engine.set_autocorrect(autocorr);
        engine.set_auto_capitalize(autocap);
        let mut screen = String::new();
        for ch in input.chars() {
            match engine.process_key(ch) {
                Some(EngineEvent::Replace(rep)) => {
                    let keep = screen
                        .chars()
                        .count()
                        .saturating_sub(rep.delete_before_cursor);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(&rep.insert);
                    if dboard_engine::is_flush_char(ch) {
                        screen.push(ch);
                    }
                }
                None => {
                    if ch == '\x08' {
                        screen.pop();
                    } else {
                        screen.push(ch);
                    }
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        screen
    };

    // 1. Task B2.1: VNI digit spatial autocorrect (`tieng71` -> `tiếng` via adjacent `7 -> 6`,
    //    `d9uoc65` -> `được` via `6 -> 7`) + One-Tap Undo on Backspace
    assert_eq!(
        type_sim(InputMethod::Vni, true, false, "tieng71 "),
        "tiếng "
    );
    assert_eq!(
        type_sim(InputMethod::Vni, true, false, "tieng71 \x08"),
        "tieng71"
    );
    assert_eq!(type_sim(InputMethod::Vni, true, false, "d9uoc65 "), "được ");
    // Cross-row safety: top-row letters never mutate into digits
    assert_eq!(
        type_sim(InputMethod::Vni, true, false, "qwerty "),
        "qwerty "
    );

    // 2. Task B2.2: Retroactive 2-word One-Tap Undo (`Last nuwax ` -> `Lát nữa `, BS -> `Last nữa`)
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "Last nuwax "),
        "Lát nữa "
    );
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "Last nuwax \x08"),
        "Last nữa"
    );
    // Pressing Space + typing a 3rd word after RetroactivePair One-Tap Undo must NOT re-trigger `Lát nữa`
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "Last nuwax \x08 nha "),
        "Last nữa nha "
    );

    // Cross-coda circumflex double-key undo (`khongoo` -> `khongo`, `tanaa` -> `tana`, `vietee` -> `viete`)
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "khongoo"),
        "khongo"
    );
    assert_eq!(type_sim(InputMethod::Telex, false, false, "tanaa"), "tana");
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "vietee"),
        "viete"
    );

    // Leading digit after dot must clear dot-filename stem and never corrupt screen text
    assert_eq!(
        type_sim(InputMethod::Telex, false, false, "awn basnh test.1py "),
        "ăn bánh tét.1py "
    );

    // 3. Task B3: Smart Auto-Capitalization closing quotes/brackets & abbreviation guard
    assert_eq!(
        type_sim(InputMethod::Telex, false, true, "\"Chaof.\" hoom "),
        "\"Chào.\" Hôm "
    );
    assert_eq!(
        type_sim(InputMethod::Telex, false, true, "“Chaof.” hoom "),
        "“Chào.” Hôm "
    );
    assert_eq!(
        type_sim(InputMethod::Telex, false, true, "(Xong rooif!) tieesp "),
        "(Xong rồi!) Tiếp "
    );
    assert_eq!(
        type_sim(
            InputMethod::Telex,
            false,
            true,
            "TP. hcm Mr. nam TS. nguyeenx "
        ),
        "TP. hcm Mr. nam TS. nguyễn "
    );
}
