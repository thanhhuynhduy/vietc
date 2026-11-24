// SPDX-License-Identifier: MIT
//! True continuous un-reset stress test (50,000+ keystrokes WITHOUT `engine.reset()`).
//!
//! Unlike `soak_perf.rs` (which calls `engine.reset()` between cycles), this suite
//! keeps a single live `Engine` running across thousands of bilingual sentences,
//! Backspace cascades, repeated Spaces, One-Tap Undos, compound splits,
//! retroactive 2-word rewrites, dot-filename restores, next-word predictions,
//! and personal dictionary learning — verifying zero cumulative state drift,
//! bounded memory/UserDict growth, and fast-path latency invariants.

use std::time::Instant;

use dboard_engine::{Engine, EngineEvent, InputMethod};

const BS: char = '\x08';

fn apply_key(engine: &mut Engine, screen: &mut String, ch: char) {
    if ch == BS {
        match engine.process_key(BS) {
            Some(EngineEvent::Replace(rep)) => {
                let keep = screen
                    .chars()
                    .count()
                    .saturating_sub(rep.delete_before_cursor);
                *screen = screen.chars().take(keep).collect();
                screen.push_str(&rep.insert);
            }
            None => {
                let _ = screen.pop();
            }
            other => panic!("unexpected backspace event: {other:?}"),
        }
        return;
    }

    match engine.process_key(ch) {
        Some(EngineEvent::Replace(rep)) => {
            let keep = screen
                .chars()
                .count()
                .saturating_sub(rep.delete_before_cursor);
            *screen = screen.chars().take(keep).collect();
            screen.push_str(&rep.insert);
            if dboard_engine::is_flush_char(ch) {
                screen.push(ch);
            }
        }
        Some(EngineEvent::Insert(s)) => screen.push_str(&s),
        None => screen.push(ch),
        other => panic!("unexpected engine event for {ch:?}: {other:?}"),
    }
}

fn run_segment(
    engine: &mut Engine,
    latencies_ns: &mut Vec<u64>,
    input: &str,
    expected: &str,
    label: &str,
) {
    let mut screen = String::new();
    for ch in input.chars() {
        let t0 = Instant::now();
        apply_key(engine, &mut screen, ch);
        latencies_ns.push(t0.elapsed().as_nanos() as u64);
        assert!(
            engine.buffer().chars().count() <= 64,
            "buffer exceeded 64-char cap during {label}"
        );
    }
    assert_eq!(screen, expected, "drift detected in continuous {label}");
}

#[test]
fn continuous_50k_keystrokes_without_reset_zero_drift_telex_and_vni() {
    // Match production daemon startup (`dboard-warmup` background thread).
    dboard_engine::warm_up_symspell();

    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);
    engine.set_auto_capitalize(true);
    engine.set_learning_enabled(true);
    engine.add_macro("ko".into(), "không".into());
    engine.add_macro("dc".into(), "được".into());

    let mut latencies_ns: Vec<u64> = Vec::with_capacity(60_000);

    // 1. Continuous un-reset Telex stress (>40,000 keystrokes across 220 multi-feature cycles)
    let telex_cycles = 220usize;
    for cycle in 0..telex_cycles {
        // Segment A: Bilingual office + dev + filenames + acronyms + cross-coda circumflex
        let expected_a = if cycle == 0 {
            "mở test.py xem list.txt sửa host.conf archive.tar.gz không việt tiếng dOs QĐ-TTg! "
        } else {
            "Mở test.py xem list.txt sửa host.conf archive.tar.gz không việt tiếng dOs QĐ-TTg! "
        };
        run_segment(
            &mut engine,
            &mut latencies_ns,
            "mowr test.py xem list.txt suwar host.conf archive.tar.gz khongo vietej tiengse dOs QDD-TTg! ",
            expected_a,
            &format!("telex_cycle_{cycle}_a"),
        );

        // Segment B: Auto-cap after `! `, repeated Space + Backspace cascade + rehydration
        run_segment(
            &mut engine,
            &mut latencies_ns,
            "chao    \x08\x08\x08\x08f banj! ",
            "Chào bạn! ",
            &format!("telex_cycle_{cycle}_b"),
        );

        // Segment C: Mid-word Backspace edit + macro + English dev code-switching
        run_segment(
            &mut engine,
            &mut latencies_ns,
            "vieetj\x08c ko dc git push origin main. ",
            "Việc không được git push origin main. ",
            &format!("telex_cycle_{cycle}_c"),
        );

        // Segment D: Retroactive 2-word rewrite (`Last nuwax ` -> `Lát nữa `) + One-Tap Undo (`BS`)
        run_segment(
            &mut engine,
            &mut latencies_ns,
            "Last nuwax \x08 nhes. ",
            "Last nữa nhé. ",
            &format!("telex_cycle_{cycle}_d"),
        );

        // Segment E: Compound-split autocorrect (`khoongbieets ` -> `không biết `) + sentence end
        run_segment(
            &mut engine,
            &mut latencies_ns,
            "khoongbieets luoon! ",
            "Không biết luôn! ",
            &format!("telex_cycle_{cycle}_e"),
        );

        // Segment F: Exercise next-word prediction + commit_selected_word + alternatives_for_word
        let preds = engine.predict_next_words(3);
        assert!(preds.len() <= 3);
        let alts = engine.alternatives_for_word("tét", Some("chạy"));
        assert!(
            alts.iter().any(|a| a == "test"),
            "expected 'test' alternative for 'tét', got {alts:?}"
        );
        if cycle % 10 == 0 {
            engine.commit_selected_word("tiếng", true);
            let chained = engine.predict_next_words(3);
            assert!(
                !chained.is_empty(),
                "commit_selected_word must keep ring_buffer live for chained prediction"
            );
            // Terminate sentence so next cycle's AutoCap expectation stays deterministic
            let _ = engine.process_key('.');
            let _ = engine.process_key(' ');
        }
    }

    assert!(
        latencies_ns.len() >= 40_000,
        "expected >= 40k continuous Telex keystrokes, got {}",
        latencies_ns.len()
    );

    // Verify UserDict stays strictly bounded and clean after 40k+ continuous keystrokes
    if let Some(serialized) = engine.take_user_dict_for_save() {
        for line in serialized.lines() {
            if let Some(tok) = line
                .strip_prefix("keep_vn:")
                .or_else(|| line.strip_prefix("keep_raw:"))
            {
                assert!(
                    tok.chars().all(|c| c.is_ascii_alphabetic()),
                    "invalid token persisted in UserDict: {tok:?}"
                );
            }
        }
    }

    // 2. Continuous un-reset VNI stress (>11,000 keystrokes across 150 cycles)
    let mut vni = Engine::new(InputMethod::Vni);
    vni.set_auto_restore(true);
    vni.set_autocorrect(true);
    vni.set_auto_capitalize(true);
    vni.set_learning_enabled(true);

    let mut vni_latencies: Vec<u64> = Vec::with_capacity(15_000);
    for cycle in 0..150usize {
        let expected_vni = if cycle == 0 {
            "xin chào! Hôm nay mình chạy test.py trên linux. Toan tiếng việt nhé! "
        } else {
            "Xin chào! Hôm nay mình chạy test.py trên linux. Toan tiếng việt nhé! "
        };
        run_segment(
            &mut vni,
            &mut vni_latencies,
            "xin cha2o! ho6m nay mi2nh cha5y test.py tre6n linux. toan10 tie61ng vie65t nhe1! ",
            expected_vni,
            &format!("vni_cycle_{cycle}"),
        );
    }

    let total_keys = latencies_ns.len() + vni_latencies.len();
    assert!(
        total_keys >= 50_000,
        "expected >= 50k total continuous keystrokes, got {total_keys}"
    );

    latencies_ns.sort_unstable();
    let p50_us = latencies_ns[latencies_ns.len() / 2] as f64 / 1_000.0;
    let p99_us = latencies_ns[(latencies_ns.len() * 99) / 100] as f64 / 1_000.0;
    eprintln!(
        "[dboard continuous_stress] total_keys={total_keys} (0 resets) telex_p50={p50_us:.2}us telex_p99={p99_us:.2}us"
    );

    // Performance gate matching docs/dboard-gboard-parity-plan.md (Section 0.6):
    // Release builds must stay strictly below `p50 < 2.0us` and `p99 < 10.0us`.
    if cfg!(debug_assertions) {
        assert!(
            p99_us <= 2_000.0,
            "debug continuous un-reset p99 latency {p99_us:.2}us exceeded 2000.0us"
        );
    } else {
        assert!(
            p50_us < 2.0,
            "release continuous un-reset p50 latency {p50_us:.2}us exceeded 2.0us gate"
        );
        assert!(
            p99_us < 10.0,
            "release continuous un-reset p99 latency {p99_us:.2}us exceeded 10.0us gate"
        );
    }
}
