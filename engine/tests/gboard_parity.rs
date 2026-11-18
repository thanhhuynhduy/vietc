// SPDX-License-Identifier: MIT
use vietc_engine::{Engine, EngineEvent, InputMethod};

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
