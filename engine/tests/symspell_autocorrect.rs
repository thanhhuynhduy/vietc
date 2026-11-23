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
    assert!(engine.alternatives_for_word("a", None).is_empty());
    assert!(engine.alternatives_for_word("foo_bar", None).is_empty());
}
