// SPDX-License-Identifier: MIT
use super::*;
use dboard_engine::InputEvent;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const BS: char = '\x08';

fn apply(screen: &mut String, cmds: &[OutputCommand]) {
    for cmd in cmds {
        match cmd {
            OutputCommand::Backspace(n) => {
                let len = screen.chars().count();
                let keep = len.saturating_sub(*n);
                *screen = screen.chars().take(keep).collect();
            }
            OutputCommand::Type(s) => screen.push_str(s),
            OutputCommand::Replace(replacement) => {
                let keep = screen
                    .chars()
                    .count()
                    .saturating_sub(replacement.delete_before_cursor);
                *screen = screen.chars().take(keep).collect();
                screen.push_str(&replacement.insert);
            }
        }
    }
}

#[test]
fn forwarded_keys_preserve_prefix_after_backspace_and_transforms() {
    let config = Config {
        input_method: "telex".into(),
        start_enabled: true,
        ..Default::default()
    };
    let mut daemon = Daemon::new(
        config,
        PathBuf::from("/tmp/dboard-forwarded-test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    let mut screen = String::from("X ");
    for key in "as\x08s\x08\x08".chars() {
        if key == BS {
            screen.pop();
        } else {
            screen.push(key);
        }
        let commands = daemon.process_forwarded_key(key);
        apply(&mut screen, &commands);
        assert_eq!(screen, format!("X {}", daemon.engine.buffer()));
    }
    assert_eq!(screen, "X ");
}

#[test]
fn focus_change_resets_engine_and_daemon_visible_revisions_together() {
    let mut daemon = Daemon::new(
        Config::default(),
        PathBuf::from("/tmp/dboard_focus_revision_test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);
    daemon.engine.set_method(InputMethod::Telex);

    let mut last_expected_revision = None;
    for ch in "hoor".chars() {
        for command in daemon.process_key(ch) {
            if let OutputCommand::Replace(replacement) = command {
                last_expected_revision = Some(replacement.expected_visible_revision);
            }
        }
    }
    let last_expected_revision =
        last_expected_revision.expect("target A must produce a replacement");
    daemon.visible_revision = last_expected_revision + 1;

    daemon.reset_composition_state();

    let mut replacement_after_focus_change = None;
    for ch in "chaof".chars() {
        for command in daemon.process_key(ch) {
            if let OutputCommand::Replace(replacement) = command {
                replacement_after_focus_change = Some(replacement);
            }
        }
    }

    let replacement = replacement_after_focus_change.expect("expected tone replacement");
    assert_eq!(replacement.expected_visible_revision, 0);
    assert_eq!(daemon.visible_revision, 0);
}

#[test]
fn two_target_focus_change_does_not_replay_stale_composition() {
    let mut daemon = Daemon::new(
        Config::default(),
        PathBuf::from("/tmp/dboard_two_target_test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);

    for ch in "hoor".chars() {
        daemon.process_key(ch);
    }
    assert_eq!(daemon.engine.buffer(), "hổ");

    daemon.event_store.push(InputEvent::KeyTyped('r'));
    daemon.screen_output = "hổ".to_string();
    daemon.visible_revision = 4;
    daemon.reset_composition_state();

    let mut target_b = String::new();
    for ch in "xin chao".chars() {
        let commands = daemon.process_key(ch);
        apply(&mut target_b, &commands);
    }

    assert!(!target_b.contains("hoor"));
    assert!(!target_b.contains("hổ"));
    assert_eq!(daemon.engine.buffer(), "chao");
    assert!(daemon.event_store.is_empty());
    assert!(daemon.screen_output.is_empty());
    assert_eq!(daemon.visible_revision, 0);
}

#[test]
fn focus_change_before_boundaries_does_not_commit_previous_target_composition() {
    for boundary in [' ', '.', '\n'] {
        let mut daemon = Daemon::new(
            Config::default(),
            PathBuf::from("focus-boundary-test.toml"),
            Arc::new(AtomicBool::new(true)),
        );
        daemon.engine.set_enabled(true);

        for ch in "tieeng".chars() {
            daemon.process_key(ch);
        }
        assert!(!daemon.engine.buffer().is_empty());

        daemon.reset_composition_state();
        let commands = daemon.process_key(boundary);
        let mut target_b = String::new();
        apply(&mut target_b, &commands);

        assert!(!target_b.contains("tieeng"));
        assert!(!target_b.contains("tiếng"));
        assert!(daemon.event_store.is_empty());
        assert!(daemon.screen_output.is_empty());
        assert_eq!(daemon.visible_revision, 0);
    }
}

#[test]
fn closed_target_during_composition_discards_raw_keys_before_next_target() {
    let mut daemon = Daemon::new(
        Config::default(),
        PathBuf::from("closed-target-compose-test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);

    for ch in "dduow".chars() {
        daemon.process_key(ch);
    }
    assert!(!daemon.engine.buffer().is_empty());

    daemon.reset_composition_state();

    let mut next_target = String::new();
    for ch in "moi".chars() {
        let commands = daemon.process_key(ch);
        apply(&mut next_target, &commands);
    }

    assert!(!next_target.contains("dduow"));
    assert!(!next_target.contains("đư"));
    assert_eq!(daemon.engine.buffer(), "moi");
    assert_eq!(daemon.visible_revision, 0);
}

#[test]
fn focus_change_reset_clears_all_composition_state() {
    let mut daemon = Daemon::new(
        Config::default(),
        PathBuf::from("/tmp/dboard_focus_reset_test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);
    daemon.process_key('h');
    daemon.process_key('o');
    daemon.process_key('r');
    daemon.process_key('r');
    daemon.event_store.push(InputEvent::KeyTyped('x'));
    daemon.screen_output = "stale".to_string();
    daemon.visible_revision = 7;

    daemon.reset_composition_state();

    assert!(daemon.engine.buffer().is_empty());
    assert!(daemon.event_store.is_empty());
    assert!(daemon.screen_output.is_empty());
    assert_eq!(daemon.visible_revision, 0);
}

#[test]
fn toggle_modes_supports_one_tap_vn_en_and_three_way_rotation() {
    let two_way = vec!["vn".into(), "en".into()];
    assert_eq!(
        next_toggle_state(&two_way, true, "telex"),
        (false, "telex".into())
    );
    assert_eq!(
        next_toggle_state(&two_way, false, "telex"),
        (true, "telex".into())
    );
    // Preserves VNI when toggling VN <-> EN
    assert_eq!(
        next_toggle_state(&two_way, true, "vni"),
        (false, "vni".into())
    );
    assert_eq!(
        next_toggle_state(&two_way, false, "vni"),
        (true, "vni".into())
    );

    let three_way = vec!["en".into(), "vni".into(), "telex".into()];
    assert_eq!(
        next_toggle_state(&three_way, false, "telex"),
        (true, "vni".into())
    );
    assert_eq!(
        next_toggle_state(&three_way, true, "vni"),
        (true, "telex".into())
    );
    assert_eq!(
        next_toggle_state(&three_way, true, "telex"),
        (false, "telex".into())
    );

    // Invalid/empty lists fall back safely to 2-way VN <-> EN
    assert_eq!(
        next_toggle_state(&[], true, "telex"),
        (false, "telex".into())
    );
    assert_eq!(
        next_toggle_state(&["bogus".into()], false, "vni"),
        (true, "vni".into())
    );
}

#[test]
fn select_candidate_supports_vn_ascii_and_third_autocorrect_slot() {
    let cfg = Config {
        candidate_bar: true,
        ..Config::default()
    };
    let mut daemon = Daemon::new(
        cfg,
        PathBuf::from("/tmp/dboard_cand_test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);
    daemon.engine.set_method(InputMethod::Telex);

    // Slot [1] (VN) vs Slot [2] (raw ASCII)
    let mut screen = String::new();
    for ch in "tieengs".chars() {
        let cmds = daemon.process_forwarded_key(ch);
        screen.push(ch);
        apply(&mut screen, &cmds);
    }
    assert_eq!(screen, "tiếng");
    let cmds = daemon.select_candidate(1);
    apply(&mut screen, &cmds);
    assert_eq!(screen, "tieengs");
    assert!(daemon.engine.buffer().is_empty());

    // Slot [3] (autocorrect suggestion for typo "tienegs" -> "tiếng")
    let mut screen2 = String::new();
    for ch in "tienegs".chars() {
        let cmds = daemon.process_forwarded_key(ch);
        screen2.push(ch);
        apply(&mut screen2, &cmds);
    }
    assert_eq!(
        daemon.engine.candidate_suggestion().as_deref(),
        Some("tiếng")
    );
    // Slots: [1]=composed ("tiếng" or "tiênegs"?), wait: "tienegs" composes to "tiênegs" or "tiếng"?
    // Regardless, pick the index matching "tiếng" (slot 2 when raw != visible, or slot 1 when raw == visible).
    let idx = if daemon.engine.raw_buffer() == daemon.engine.buffer() {
        1
    } else {
        2
    };
    let cmds = daemon.select_candidate(idx);
    apply(&mut screen2, &cmds);
    assert_eq!(screen2, "tiếng");
}

#[test]
fn candidate_selection_preserves_ring_buffer_and_avoids_idle_alt_hijack() {
    let cfg = Config {
        candidate_bar: true,
        ..Config::default()
    };
    let mut daemon = Daemon::new(
        cfg,
        PathBuf::from("/tmp/dboard_cand_chain_test.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);
    daemon.engine.set_method(InputMethod::Telex);

    // Missing slot [3] while composing "chaof" (only 2 slots: "chào", "chaof") returns None.
    for ch in "chaof".chars() {
        let _ = daemon.process_forwarded_key(ch);
    }
    assert!(daemon.try_select_candidate(2, false).is_none());

    // Commit "xin " -> next-word predictions exist, but keyboard Alt+1 (include_predictions=false)
    // must return None so browser/terminal Alt+1/2/3 tab shortcuts are never hijacked.
    daemon.reset_composition_state();
    for ch in "xin ".chars() {
        let _ = daemon.process_forwarded_key(ch);
    }
    assert!(!daemon.engine.predict_next_words(3).is_empty());
    assert!(daemon.try_select_candidate(0, false).is_none());

    // Mouse click on prediction (include_predictions=true) commits word and keeps ring_buffer
    // alive for chained next-word prediction.
    let cmds = daemon.try_select_candidate(0, true);
    assert!(cmds.is_some());
    assert!(
        !daemon.engine.predict_next_words(3).is_empty(),
        "selecting a next-word prediction must preserve ring_buffer for chained predictions"
    );

    // Pasted prefix must never be wiped by Alt+1/2/3 candidate selection
    daemon.reset_composition_state();
    daemon.engine.update_with_pasted_text("Hello world ");
    for ch in "chaof".chars() {
        let _ = daemon.process_key(ch);
    }
    assert!(daemon.engine.has_pasted_prefix());
    assert!(daemon.try_select_candidate(1, false).is_none());
    assert!(daemon.try_select_candidate(0, true).is_none());
}

#[test]
fn continuous_gboard_daemon_stress_without_reset_stays_exact() {
    let cfg = Config {
        start_enabled: true,
        input_method: "telex".into(),
        autocorrect: true,
        auto_capitalize: true,
        candidate_bar: true,
        ..Config::default()
    };
    let mut daemon = Daemon::new(
        cfg,
        PathBuf::from("/tmp/dboard_continuous_daemon_stress.toml"),
        Arc::new(AtomicBool::new(true)),
    );
    daemon.engine.set_enabled(true);
    daemon.engine.set_method(InputMethod::Telex);

    for cycle in 0..80usize {
        let mut screen = String::new();
        let input = "mowr test.py chao   \x08\x08\x08f git push! ";
        for ch in input.chars() {
            let cmds = daemon.process_key(ch);
            if ch == BS {
                if cmds.is_empty() {
                    let _ = screen.pop();
                } else {
                    apply(&mut screen, &cmds);
                }
            } else if cmds.is_empty() {
                screen.push(ch);
            } else {
                apply(&mut screen, &cmds);
                if dboard_engine::is_flush_char(ch) {
                    screen.push(ch);
                }
            }
        }
        let expected = if cycle == 0 {
            "mở test.py chào git push! "
        } else {
            "Mở test.py chào git push! "
        };
        assert_eq!(screen, expected, "daemon drift at continuous cycle {cycle}");
        // Idle suggestion sync + caret re-selection must not corrupt subsequent cycles
        daemon.sync_candidate_bar(true);
        daemon.sync_caret_reselection(Some("tét"), Some("chạy"));
        assert!(daemon.caret_reselection.is_some());
    }
}
