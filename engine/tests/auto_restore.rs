//! Tests for Gboard-style Bilingual (Vietnamese + English) Auto-Restore & Bigram Context:
//! - English words and double-letter stems restore cleanly without dropping letters.
//! - Ambiguous collisions (`test`/`tét`, `own`/`ơn`, `been`/`bên`, `car`/`cả`,
//!   `six`/`sĩ`, `tree`/`trê`, `list`/`lít`, `host`/`hót`) disambiguate using
//!   unigram frequency + preceding-word Bigram context!

use std::collections::HashMap;
use vietc_engine::{Engine, EngineEvent, InputMethod};

fn telex(keys: &str) -> String {
    Engine::replay_keystrokes(
        InputMethod::Telex,
        &HashMap::new(),
        &keys.chars().collect::<Vec<_>>(),
    )
    .0
}

fn committed(keys: &str) -> String {
    let composed = telex(keys);
    let raw: String = keys.chars().collect();
    if Engine::should_restore_word(&composed, &raw) {
        raw
    } else {
        composed
    }
}

fn type_sentence(method: InputMethod, input: &str) -> String {
    let mut engine = Engine::new(method);
    engine.set_enabled(true);
    engine.set_auto_restore(true);
    let mut screen = String::new();
    for ch in input.chars() {
        let is_boundary = matches!(
            ch,
            ' ' | '\t' | '\n' | '.' | ',' | '!' | '?' | ';' | ':' | '—' | '–' | '…'
        );
        match engine.process_key(ch) {
            Some(EngineEvent::Replace(rep)) => {
                for _ in 0..rep.delete_before_cursor {
                    screen.pop();
                }
                screen.push_str(&rep.insert);
                if is_boundary {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::Insert(s)) => screen.push_str(&s),
            None if ch == '\x08' => {
                screen.pop();
            }
            None => screen.push(ch),
            other => panic!("unexpected event: {other:?}"),
        }
    }
    screen
}

#[test]
fn english_words_and_double_letter_stems_are_restored() {
    let cases = [
        ("fix", "fix"),
        ("cargo", "cargo"),
        ("status", "status"),
        ("world", "world"),
        ("english", "english"),
        ("sweet", "sweet"),
        ("more", "more"),
        ("core", "core"),
        ("lose", "lose"),
        ("rose", "rose"),
        ("nose", "nose"),
        ("chose", "chose"),
        ("those", "those"),
        ("these", "these"),
        ("here", "here"),
        ("there", "there"),
        ("where", "where"),
        ("were", "were"),
        ("care", "care"),
        ("share", "share"),
        ("base", "base"),
        ("case", "case"),
        ("use", "use"),
        ("pure", "pure"),
        ("cure", "cure"),
        ("hire", "hire"),
        ("fire", "fire"),
        ("house", "house"),
        ("mouse", "mouse"),
        ("pause", "pause"),
        ("cause", "cause"),
        ("order", "order"),
        ("border", "border"),
        ("server", "server"),
        ("sister", "sister"),
        ("user", "user"),
        ("laser", "laser"),
        ("nasa", "nasa"),
        ("para", "para"),
        ("sara", "sara"),
        ("exe", "exe"),
        ("awkward", "awkward"),
        ("gin", "gin"),
        ("win", "win"),
        ("weed", "weed"),
        // Dominant English words that previously turned into garbled Vietnamese
        ("test", "test"),
        ("rust", "rust"),
        ("this", "this"),
        ("has", "has"),
        ("his", "his"),
        ("how", "how"),
        ("now", "now"),
        ("low", "low"),
        ("see", "see"),
        ("most", "most"),
        ("turn", "turn"),
        ("own", "own"),
        ("down", "down"),
        ("list", "list"),
        ("host", "host"),
        ("cost", "cost"),
        ("must", "must"),
        ("bus", "bus"),
        ("six", "six"),
        ("box", "box"),
        ("max", "max"),
        ("room", "room"),
        ("DDoS", "DDoS"),
    ];
    for (keys, want) in cases {
        assert_eq!(
            committed(keys),
            want,
            "expected {keys} to restore to {want}"
        );
    }

    // Full sentence typing with natural English double-letter words & VNI tech tokens
    assert_eq!(
        type_sentence(
            InputMethod::Telex,
            "loose cheese needle keeper stuff staff business address IEEE "
        ),
        "loose cheese needle keeper stuff staff business address IEEE "
    );
    assert_eq!(
        type_sentence(InputMethod::Vni, "u32 i64 f32 mp4 sha256 "),
        "u32 i64 f32 mp4 sha256 "
    );
}

#[test]
fn core_vietnamese_words_and_shorthand_are_kept() {
    let cases = [
        ("tieengs", "tiếng"),
        ("vieejt", "việt"),
        ("quar", "quả"),
        ("gif", "gì"),
        ("khoong", "không"),
        ("tooi", "tôi"),
        ("banhf", "bành"),
        ("ddi", "đi"),
        ("been", "bên"),
        ("too", "tô"),
        ("car", "cả"),
        ("teen", "tên"),
        ("ddos", "đó"),
        ("its", "ít"),
        ("sown", "sơn"),
        ("gieengs", "giếng"),
        ("ddc", "đc"),
        ("ddk", "đk"),
    ];
    for (keys, want) in cases {
        assert_eq!(committed(keys), want, "expected {keys} to stay {want}");
    }
}

#[test]
fn gboard_bigram_context_disambiguates_bilingual_sentences() {
    // Vietnamese context keeps Vietnamese meaning for collisions:
    assert_eq!(
        type_sentence(InputMethod::Telex, "banhs test "),
        "bánh tét "
    );
    assert_eq!(type_sentence(InputMethod::Telex, "camr own "), "cảm ơn ");
    assert_eq!(type_sentence(InputMethod::Telex, "bacs six "), "bác sĩ ");
    assert_eq!(type_sentence(InputMethod::Telex, "cas tree "), "cá trê ");
    assert_eq!(type_sentence(InputMethod::Telex, "hai list "), "hai lít ");
    assert_eq!(type_sentence(InputMethod::Telex, "chim host "), "chim hót ");
    assert_eq!(type_sentence(InputMethod::Telex, "hai been "), "hai bên ");
    assert_eq!(type_sentence(InputMethod::Telex, "taats car "), "tất cả ");

    // English context switches collisions to English:
    assert_eq!(
        type_sentence(InputMethod::Telex, "cargo test "),
        "cargo test "
    );
    assert_eq!(type_sentence(InputMethod::Telex, "my own "), "my own ");
    assert_eq!(type_sentence(InputMethod::Telex, "six pack "), "six pack ");
    assert_eq!(
        type_sentence(InputMethod::Telex, "binary tree "),
        "binary tree "
    );
    assert_eq!(
        type_sentence(InputMethod::Telex, "array list "),
        "array list "
    );
    assert_eq!(
        type_sentence(InputMethod::Telex, "local host "),
        "local host "
    );
    assert_eq!(
        type_sentence(InputMethod::Telex, "have been "),
        "have been "
    );
    assert_eq!(type_sentence(InputMethod::Telex, "my car "), "my car ");
    assert_eq!(type_sentence(InputMethod::Telex, "me too "), "me too ");
    assert_eq!(type_sentence(InputMethod::Telex, "in its "), "in its ");
}

#[test]
fn untransformed_english_passes_through() {
    for keys in ["type", "code", "hello", "the", "and"] {
        assert_eq!(committed(keys), keys);
        assert!(!Engine::should_restore_word(&telex(keys), keys));
    }
}

#[test]
fn process_key_restores_on_flush() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_enabled(true);
    for ch in "cargo".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "cảgo");
    let event = engine.process_key(' ');
    match event {
        Some(EngineEvent::Replace(replacement)) => {
            assert_eq!(replacement.insert, "cargo");
        }
        other => panic!("expected Replace to 'cargo', got {other:?}"),
    }
}

#[test]
fn auto_restore_can_be_disabled() {
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_enabled(true);
    engine.set_auto_restore(false);
    for ch in "cargo".chars() {
        engine.process_key(ch);
    }
    assert_eq!(engine.buffer(), "cảgo");
    let event = engine.process_key(' ');
    assert!(
        event.is_none(),
        "with auto-restore off the composed VN word stays untouched on flush, got {event:?}"
    );
}
