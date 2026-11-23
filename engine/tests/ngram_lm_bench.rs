// SPDX-License-Identifier: MIT
//! Verification & performance benchmark for Task C1:
//! Quantized Kneser-Ney 3-gram/4-gram Language Model & Next-Word Prediction.

use std::time::Instant;

use dboard_engine::{ngram_lm, Engine, InputMethod};

fn type_str(engine: &mut Engine, raw: &str) {
    for ch in raw.chars() {
        let _ = engine.process_key(ch);
    }
}

#[test]
fn ngram_lm_integrity_size_and_latency_gate() {
    assert!(
        ngram_lm::verify_binary_integrity(),
        "ngram_lm.bin failed FNV-1a checksum / header integrity check"
    );
    let size = ngram_lm::binary_size_bytes();
    assert!(
        size > 512 * 1024 && size <= 8 * 1024 * 1024,
        "ngram_lm.bin size out of budget (0.5MB..=8MB): {size} bytes"
    );

    // Latency gate: 10,000 3-gram/4-gram lookups + next-word predictions must
    // stay well below the 300us Smart-Path budget per call.
    let start = Instant::now();
    let mut acc: i64 = 0;
    for _ in 0..5_000 {
        acc += i64::from(ngram_lm::score_context(
            &["tôi", "đang", "chạy"],
            "tét",
            "test",
        ));
        acc += i64::from(ngram_lm::score_context(
            &["in", "the", "past"],
            "pát",
            "past",
        ));
        let preds = ngram_lm::predict_next_words(&["cảm", "ơn"], 3);
        acc += preds.len() as i64;
    }
    let elapsed = start.elapsed();
    let avg_us = elapsed.as_secs_f64() * 1_000_000.0 / 15_000.0;
    eprintln!(
        "[ngram_lm_bench] size={:.2}MB avg_call={avg_us:.3}us total_15k={elapsed:?} acc={acc}",
        size as f64 / (1024.0 * 1024.0)
    );
    assert!(
        avg_us < 300.0,
        "average ngram_lm call latency {avg_us:.2}us exceeded 300us Smart-Path budget"
    );
}

#[test]
fn engine_next_word_prediction_lifecycle_and_hit_rate() {
    let mut engine = Engine::new(InputMethod::Telex);

    // Mid-word: must return empty
    type_str(&mut engine, "xin");
    assert!(engine.predict_next_words(3).is_empty());

    // Immediately after Space: predicts Vietnamese next words
    let _ = engine.process_key(' ');
    let after_xin = engine.predict_next_words(3);
    assert!(
        after_xin.iter().any(|w| w == "chào"),
        "expected 'chào' after 'xin ', got {after_xin:?}"
    );

    // After sentence-ending period: ring_buffer cleared, must return empty
    type_str(&mut engine, "chaof.");
    assert!(engine.predict_next_words(3).is_empty());

    // Disabled engine: must return empty
    engine.reset();
    type_str(&mut engine, "xin ");
    engine.set_enabled(false);
    assert!(engine.predict_next_words(3).is_empty());

    // Top-3 Next-Word Prediction hit-rate across everyday VN office/chat & EN phrases
    let prompts: &[(&[&str], &str)] = &[
        (&["xin"], "chào"),
        (&["xin", "chào"], "bạn"),
        (&["cảm"], "ơn"),
        (&["cảm", "ơn"], "bạn"),
        (&["hẹn", "gặp"], "lại"),
        (&["không", "có"], "gì"),
        (&["không", "thể"], "nào"),
        (&["chúc", "bạn"], "một"),
        (&["hôm"], "nay"),
        (&["bây"], "giờ"),
        (&["công"], "việc"),
        (&["tiếng"], "việt"),
        (&["mọi"], "người"),
        (&["chúng"], "ta"),
        (&["tạm"], "biệt"),
        (&["thank"], "you"),
        (&["thank", "you"], "for"),
        (&["good"], "morning"),
        (&["let", "me"], "know"),
        (&["việt"], "nam"),
    ];
    let mut hits = 0usize;
    for &(ctx, expected) in prompts {
        let preds = ngram_lm::predict_next_words(ctx, 3);
        if preds.iter().any(|w| w == expected) {
            hits += 1;
        }
    }
    let hit_rate = (hits as f64) * 100.0 / (prompts.len() as f64);
    eprintln!(
        "[next_word_bench] top3_hits={hits}/{} ({hit_rate:.1}%)",
        prompts.len()
    );
    assert!(
        hit_rate >= 90.0,
        "next-word prediction top-3 hit rate too low: {hit_rate:.1}%"
    );
}

#[test]
fn personal_bigram_learning_adaptive_ranking_and_privacy_guards() {
    use dboard_engine::{EngineEvent, UserDict};

    // 1. Learn a personal Vietnamese name bigram after >= 2 commits
    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);

    type_str(&mut engine, "nguyeenx minh ");
    // Only seen once -> not yet promoted
    assert!(engine.take_user_dict_for_save().is_none());

    type_str(&mut engine, "nguyeenx minh ");
    // Seen twice -> promoted and persisted as `bigram:nguyễn|minh:2`
    let saved = engine
        .take_user_dict_for_save()
        .expect("personal bigram should be dirty after 2nd commit");
    assert!(saved.contains("bigram:nguyễn|minh:2"), "got:\n{saved}");

    // Reload into a fresh Engine -> `"nguyễn "` predicts `"minh"` at slot #1!
    let mut fresh = Engine::new(InputMethod::Telex);
    fresh.set_user_dict(UserDict::load_from_str(&saved));
    type_str(&mut fresh, "nguyeenx ");
    let preds = fresh.predict_next_words(3);
    assert_eq!(
        preds.first().map(String::as_str),
        Some("minh"),
        "expected personal bigram 'minh' at slot #1, got {preds:?}"
    );

    // 2. Adaptive SymSpell ranking: teach personal bigram `"món cháo"` twice,
    //    then ambiguous `"chaod"` after `"món "` resolves decisively to `"cháo"`!
    let mut adaptive = Engine::new(InputMethod::Telex);
    adaptive.set_auto_restore(true);
    adaptive.set_autocorrect(true);
    type_str(&mut adaptive, "mons chaos mons chaos ");
    let mut screen = String::new();
    for ch in "mons chaod ".chars() {
        match adaptive.process_key(ch) {
            Some(EngineEvent::Replace(rep)) => {
                for _ in 0..rep.delete_before_cursor {
                    screen.pop();
                }
                screen.push_str(&rep.insert);
                if dboard_engine::is_flush_char(ch) {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::AutoRestore(w)) => {
                screen.push_str(&w);
                if dboard_engine::is_flush_char(ch) {
                    screen.push(ch);
                }
            }
            Some(EngineEvent::Insert(t)) | Some(EngineEvent::Flush(t)) => screen.push_str(&t),
            Some(EngineEvent::UndoTones {
                backspaces,
                restored,
            }) => {
                for _ in 0..backspaces {
                    screen.pop();
                }
                screen.push_str(&restored);
            }
            Some(EngineEvent::Paste(t)) => screen.push_str(&t),
            None => screen.push(ch),
        }
    }
    assert_eq!(screen, "món cháo ");

    // 3. Privacy & Security guards: disabled learning & digits/symbols NEVER learned
    let mut guarded = Engine::new(InputMethod::Telex);
    guarded.set_learning_enabled(false);
    type_str(&mut guarded, "nguyeenx minh nguyeenx minh ");
    assert!(guarded.take_user_dict_for_save().is_none());

    let mut dict = UserDict::new();
    for _ in 0..3 {
        dict.record_bigram_usage("pass123", "secret");
        dict.record_bigram_usage("user@host", "token");
        dict.record_bigram_usage("bad|key", "val:1");
    }
    assert_eq!(dict.promoted_bigram_count(), 0);
    assert!(!dict.serialize().contains("bigram:"));
}
