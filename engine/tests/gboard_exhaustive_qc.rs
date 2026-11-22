// SPDX-License-Identifier: MIT
//! Exhaustive Gboard-Parity QC & Adversarial Edge-Case Suite (Pillar 4).
//!
//! Tests every high-risk real-world Vietnamese + English typing scenario:
//! - URLs, emails, file paths, code identifiers, version numbers, hashtags
//! - Tricky Vietnamese orthography (`gi`/`qu`/`ươ`/`oă`/`uy`/double-`oo`/free-order/tone-overwrite)
//! - Punctuation & brackets around Vietnamese and English words
//! - Backspace torture, guarded rehydration, and One-Tap Undo
//! - Casing torture (ALL CAPS, Title Case, acronyms, accidental Shift)
//! - Full realistic bilingual tech/office paragraphs (Telex + VNI)
//! - Autocorrect zero-false-positive sweep across valid Vietnamese & English corpora

use dboard_engine::{Engine, EngineEvent, InputMethod};

fn type_keys(engine: &mut Engine, keys: &str) -> String {
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
                if matches!(
                    ch,
                    ' ' | ','
                        | '.'
                        | '!'
                        | '?'
                        | ';'
                        | ':'
                        | '\n'
                        | '\t'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '"'
                        | '\''
                        | '/'
                        | '\\'
                        | '-'
                        | '_'
                        | '='
                        | '+'
                        | '@'
                        | '#'
                        | '%'
                ) {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::Insert(s)) => screen.push_str(&s),
            None => screen.push(ch),
            other => panic!("unexpected event for {ch:?}: {other:?}"),
        }
    }
    screen
}

fn sim(method: InputMethod, keys: &str) -> String {
    let mut engine = Engine::new(method);
    type_keys(&mut engine, keys)
}

#[test]
fn trap_a_urls_emails_paths_code_identifiers_and_versions_stay_intact() {
    let cases = [
        "https://github.com/thanhhd/dboard ",
        "user.name@example.com ",
        "/home/thanhhd/Downloads/report_2026.pdf ",
        "my_variable_name ",
        "foo-bar-baz ",
        "std::collections::HashMap ",
        "v1.2.3 ",
        "100% ",
        "2026-11-22 ",
        "10:30 ",
        "#dboard ",
        "@thanhhd ",
        "config.toml ",
        "Cargo.lock ",
        "wayland-1 ",
        "zwp_virtual_keyboard_v1 ",
        "postgresql ",
        "localhost:8080/api/v1/users ",
    ];
    for case in cases {
        assert_eq!(
            sim(InputMethod::Telex, case),
            case,
            "Telex corrupted tech/URL/code token: {case:?}"
        );
        assert_eq!(
            sim(InputMethod::Vni, case),
            case,
            "VNI corrupted tech/URL/code token: {case:?}"
        );
    }
}

#[test]
fn trap_b_tricky_vietnamese_orthography_free_order_and_tone_overwrite() {
    // gi / qu / ươ / oă / uy / double-oo
    let telex_cases = [
        ("gif giuwx giuwowngf gieengs giuwax giaos giair gioir ", "gì giữ giường giếng giữa giáo giải giỏi "),
        ("quas quar quaanf quyeenf quyts queen quoocs quyr ", "quá quả quần quyền quýt quên quốc quỷ "),
        ("dduowcj dduwocj nguowif truwowngf truwongf thuwowngf nuwowcs buwowcs huwowngs muwowif cuwowif tuwoi ", "được được người trường trường thường nước bước hướng mười cười tươi "),
        ("hoawcj xoawn hoaf khoer thuyr quyeenr chuyeenj tuyeets nguyeenx khuyeen ", "hoặc xoăn hòa khỏe thủy quyển chuyện tuyết nguyễn khuyên "),
        ("xooong booong mooocs ", "xoong boong moóc "),
        // Tone overwrite & z tone removal + multi-tone tech token protection (`nextjs`, `nuxtjs`)
        ("tooisz chaofj chaofs nextjs nuxtjs ", "tôi chạo cháo nextjs nuxtjs "),
        // Double-key literal undo (English double-s/f/o/w words)
        ("css pass class less boss miss loss diff off staff buffer www google book look root boot loop pool tool cool good food wood door floor room zoom soon ", "css pass class less boss miss loss diff off staff buffer www google book look root boot loop pool tool cool good food wood door floor room zoom soon "),
    ];
    for (raw, expected) in telex_cases {
        assert_eq!(
            sim(InputMethod::Telex, raw),
            expected,
            "Telex orthography mismatch for {raw:?}"
        );
    }

    // VNI free-order & tone overwrite & tone removal (0)
    let vni_cases = [
        ("tieng61 tieng16 ", "tiếng tiếng "),
        ("duoc75 d9uoc75 ", "dược được "),
        ("toi61230 ", "tôi "),
        ("chao21345 ", "chạo "),
        ("nguoi72 truong72 thuong72 ", "người trường thường "),
    ];
    for (raw, expected) in vni_cases {
        assert_eq!(
            sim(InputMethod::Vni, raw),
            expected,
            "VNI orthography mismatch for {raw:?}"
        );
    }
}

#[test]
fn trap_c_punctuation_and_brackets_around_vietnamese_and_english_words() {
    let cases = [
        ("(tieengs vieetj) ", "(tiếng việt) "),
        ("\"xin chaof\" ", "\"xin chào\" "),
        ("'khoong cos gif' ", "'không có gì' "),
        ("[dduowcj rooif] ", "[được rồi] "),
        (
            "chaof, chaof. chaof! chaof? chaof: chaof; ",
            "chào, chào. chào! chào? chào: chào; ",
        ),
        (
            "test, test. rust! server? push: commit; ",
            "test, test. rust! server? push: commit; ",
        ),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            sim(InputMethod::Telex, raw),
            expected,
            "Punctuation handling mismatch for {raw:?}"
        );
    }
}

#[test]
fn trap_d_backspace_rehydration_and_guarded_word_boundary_torture() {
    // Mid-word backspace (pops raw keys `s`, `g`, `n`) + re-type `ns` -> `tiến`
    assert_eq!(sim(InputMethod::Telex, "tieengs\x08\x08\x08ns "), "tiến ");
    // Backspace across space into valid VN word + add tone
    assert_eq!(sim(InputMethod::Telex, "xin chao \x08f "), "xin chào ");
    // Guarded rehydration: Backspace across space + unrelated consonant must NOT corrupt previous word
    assert_eq!(sim(InputMethod::Telex, "caaus \x08h "), "cấuh ");
    // Delete entire previous word across space and type a brand-new word
    assert_eq!(
        sim(InputMethod::Telex, "xin chaof \x08\x08\x08\x08\x08banj "),
        "xin bạn "
    );
}

#[test]
fn trap_e_casing_acronyms_and_all_caps_vietnamese_torture() {
    assert_eq!(sim(InputMethod::Telex, "VIEETJ NAM "), "VIỆT NAM ");
    assert_eq!(
        sim(
            InputMethod::Telex,
            "COONGJ HOAF XAX HOOIJ CHUR NGHIAX VIEETJ NAM "
        ),
        "CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM "
    );
    assert_eq!(
        sim(InputMethod::Telex, "Nguyeenx Ddawng Khoa "),
        "Nguyễn Đăng Khoa "
    );
    // Tech acronyms must NEVER be mangled by Telex
    let acronyms = "HTTP HTTPS JSON YAML TOML SQL API SDK CLI CPU GPU RAM SSD USB HDMI WAYLAND COSMIC XKB ATX DDR5 ";
    assert_eq!(sim(InputMethod::Telex, acronyms), acronyms);
}

#[test]
fn trap_f_realistic_bilingual_vietnamese_english_office_and_dev_paragraphs() {
    let cases = [
        (
            "Hoom nay minhf deploy banr release mowis leen server production bawngf cargo build --release rooif test treen COSMIC Wayland. ",
            "Hôm nay mình deploy bản release mới lên server production bằng cargo build --release rồi test trên COSMIC Wayland. ",
        ),
        (
            "Team minhf ddang fix bug trong file config.toml vaf toois uwu performance cho database PostgreSQL. ",
            "Team mình đang fix bug trong file config.toml và tối ưu performance cho database PostgreSQL. ",
        ),
        (
            "Bacs six khuyeen awn banhs test vuwaf phair, conf anh dev thif chayj cargo test vowis rustc truwowcs khi git push. ",
            "Bác sĩ khuyên ăn bánh tét vừa phải, còn anh dev thì chạy cargo test với rustc trước khi git push. ",
        ),
        (
            "Last nuwax minhf sex review pull request vaf merge nhanhs feature/keyboard-layout vaof main. ",
            "Lát nữa mình sẽ review pull request và merge nhánh feature/keyboard-layout vào main. ",
        ),
        (
            "Last week we shipped the new architecture documentation and accessibility configuration. ",
            "Last week we shipped the new architecture documentation and accessibility configuration. ",
        ),
    ];

    for (raw, expected) in cases {
        assert_eq!(
            sim(InputMethod::Telex, raw),
            expected,
            "Bilingual paragraph mismatch for:\n  raw: {raw}"
        );
    }
}

#[test]
fn trap_g_autocorrect_zero_false_positives_on_valid_bilingual_corpus() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_autocorrect(true);

    let clean_sentence = "Hoom nay minhf deploy banr release mowis leen server production bawngf cargo build rooif test treen COSMIC Wayland. ";
    let expected = "Hôm nay mình deploy bản release mới lên server production bằng cargo build rồi test trên COSMIC Wayland. ";
    assert_eq!(type_keys(&mut engine, clean_sentence), expected);

    // Common valid Vietnamese words and English words must NEVER be altered by autocorrect
    engine.reset();
    let vn_clean = "xin chaof vieetj nam khoong cos gif dduowcj rooif nguowif banj thaan thuwowng ";
    assert_eq!(
        type_keys(&mut engine, vn_clean),
        "xin chào việt nam không có gì được rồi người bạn thân thương "
    );
}
