// SPDX-License-Identifier: MIT
//! 100,000-keystroke deterministic soak & latency gate for Dboard's Gboard-mode
//! engine (Telex + VNI, Trigram LM, Spatial Autocorrect, Auto-Capitalization,
//! Personal Dictionary, and Context-Aware Surrounding Word editing).

use std::time::Instant;

use dboard_engine::{Engine, EngineEvent, InputMethod};

const SOAK_SENTENCE_TELEX: &str =
    "xin chaof! hoom nay minhf chayj cosmic wayland treen linux. it has been awesome, khoong cos gif caanf lo, tienegs vieetj vaof google.com v1.0 nhes! ";
const EXPECTED_TELEX: &str =
    "xin chào! Hôm nay mình chạy cosmic wayland trên linux. It has been awesome, không có gì cần lo, tiếng việt vào google.com v1.0 nhé! ";

const SOAK_SENTENCE_VNI: &str =
    "xin cha2o! ho6m nay mi2nh cha5y cosmic wayland tre6n linux. teh tie61ng vie65t nhe1! ";
const EXPECTED_VNI: &str =
    "xin chào! Hôm nay mình chạy cosmic wayland trên linux. The tiếng việt nhé! ";

fn apply_key(engine: &mut Engine, screen: &mut String, ch: char) {
    if ch == '\x08' {
        match engine.process_key('\x08') {
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
            if matches!(ch, ' ' | '.' | ',' | '!' | '?' | '\n') {
                screen.push(ch);
            }
        }
        Some(EngineEvent::Insert(s)) => screen.push_str(&s),
        None => screen.push(ch),
        other => panic!("unexpected engine event for {ch:?}: {other:?}"),
    }
}

#[test]
fn soak_100k_keystrokes_bilingual_telex_and_vni_meets_latency_and_exactness_gate() {
    // Match production daemon startup (`dboard-warmup` background thread) so
    // latency percentiles measure steady-state typing rather than cold OnceLock init.
    dboard_engine::warm_up_symspell();

    let mut engine = Engine::new(InputMethod::Telex);
    engine.set_auto_restore(true);
    engine.set_autocorrect(true);
    engine.set_auto_capitalize(true);

    let keys_per_cycle = SOAK_SENTENCE_TELEX.chars().count();
    let cycles = (100_000 / keys_per_cycle) + 1;
    let total_keys = cycles * keys_per_cycle;
    let mut latencies_ns: Vec<u64> = Vec::with_capacity(total_keys);

    for _ in 0..cycles {
        engine.reset();
        let mut screen = String::new();
        for ch in SOAK_SENTENCE_TELEX.chars() {
            let t0 = Instant::now();
            apply_key(&mut engine, &mut screen, ch);
            latencies_ns.push(t0.elapsed().as_nanos() as u64);
            assert!(
                engine.buffer().chars().count() <= 64,
                "engine buffer exceeded 64-char safety bound"
            );
        }
        assert_eq!(screen, EXPECTED_TELEX);
    }

    // VNI soak (20,000+ keystrokes)
    let mut vni = Engine::new(InputMethod::Vni);
    vni.set_auto_restore(true);
    vni.set_autocorrect(true);
    vni.set_auto_capitalize(true);
    let vni_cycles = (20_000 / SOAK_SENTENCE_VNI.chars().count()) + 1;
    for _ in 0..vni_cycles {
        vni.reset();
        let mut screen = String::new();
        for ch in SOAK_SENTENCE_VNI.chars() {
            apply_key(&mut vni, &mut screen, ch);
        }
        assert_eq!(screen, EXPECTED_VNI);
    }

    latencies_ns.sort_unstable();
    let p50_us = latencies_ns[latencies_ns.len() / 2] as f64 / 1_000.0;
    let p95_us = latencies_ns[(latencies_ns.len() * 95) / 100] as f64 / 1_000.0;
    let p99_us = latencies_ns[(latencies_ns.len() * 99) / 100] as f64 / 1_000.0;
    let max_us = *latencies_ns.last().unwrap_or(&0) as f64 / 1_000.0;

    eprintln!(
        "[dboard soak] total_telex_keys={total_keys} p50={p50_us:.2}us p95={p95_us:.2}us p99={p99_us:.2}us max={max_us:.2}us"
    );

    // Performance gate matching docs/dboard-gboard-parity-plan.md (Section 0.6):
    // Release builds must stay strictly below `p50 < 2.0us` and `p99 < 10.0us`;
    // unoptimized debug builds keep a generous 2ms upper bound.
    if cfg!(debug_assertions) {
        assert!(
            p99_us <= 2_000.0,
            "debug p99 keystroke latency exceeded 2ms gate: {p99_us:.2}us"
        );
    } else {
        assert!(
            p50_us < 2.0,
            "release p50 keystroke latency {p50_us:.2}us exceeded 2.0us gate"
        );
        assert!(
            p99_us < 10.0,
            "release p99 keystroke latency {p99_us:.2}us exceeded 10.0us gate"
        );
    }
}
