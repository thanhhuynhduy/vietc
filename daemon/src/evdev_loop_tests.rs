// SPDX-License-Identifier: MIT
use super::*;
use crate::commands::OutputCommand;

struct UncertainInjector {
    events: std::cell::RefCell<Vec<(u16, i32)>>,
    uncertain: std::cell::Cell<bool>,
}

impl Default for UncertainInjector {
    fn default() -> Self {
        Self {
            events: Default::default(),
            uncertain: std::cell::Cell::new(true),
        }
    }
}

impl dboard_protocol::KeyInjector for UncertainInjector {
    fn send_backspace(&self) -> dboard_protocol::inject::InjectResult {
        panic!("uncertain edits must not perform automatic deletion")
    }
    fn send_char(&self, _: char) -> dboard_protocol::inject::InjectResult {
        panic!("uncertain edits must bypass composition")
    }
    fn send_string(&self, _: &str) -> dboard_protocol::inject::InjectResult {
        panic!("uncertain edits must not retry insertion")
    }
    fn send_key_event(&self, code: u16, value: i32) -> dboard_protocol::inject::InjectResult {
        self.events.borrow_mut().push((code, value));
        dboard_protocol::inject::InjectResult::Success
    }
    fn has_uncertain_edit(&self) -> bool {
        self.uncertain.get()
    }
    fn flush(&self) -> dboard_protocol::inject::InjectResult {
        dboard_protocol::inject::InjectResult::Success
    }
}

#[test]
fn uncertain_paste_forwards_new_keys_but_keeps_original_trigger_release_consumed() {
    let injector = UncertainInjector::default();
    // Include the stale Backspace marker produced by older daemon builds.
    let mut consumed = HashSet::from([31, 14]);
    let mut forwarded = HashSet::new();
    for (code, value) in [
        (31, 2),
        (31, 0),
        (14, 1),
        (14, 0),
        (29, 1),
        (46, 1),
        (46, 0),
        (29, 0),
    ] {
        assert!(bypass_uncertain_edit(
            &injector,
            true,
            code,
            value,
            &mut consumed,
            &mut forwarded,
        ));
    }
    assert!(consumed.is_empty());
    assert_eq!(
        &*injector.events.borrow(),
        &[(14, 1), (14, 0), (29, 1), (46, 1), (46, 0), (29, 0)]
    );
}

#[test]
fn uncertain_ungrabbed_input_is_not_forwarded_twice() {
    let injector = UncertainInjector::default();
    let mut consumed = HashSet::new();
    let mut forwarded = HashSet::new();
    assert!(bypass_uncertain_edit(
        &injector,
        false,
        30,
        1,
        &mut consumed,
        &mut forwarded,
    ));
    assert!(injector.events.borrow().is_empty());
}

#[test]
fn uncertain_backspace_release_survives_focus_recovery_mid_stroke() {
    let injector = UncertainInjector::default();
    let mut consumed = HashSet::new();
    let mut forwarded = HashSet::new();
    assert!(bypass_uncertain_edit(
        &injector,
        true,
        14,
        1,
        &mut consumed,
        &mut forwarded
    ));
    // Another physical key observes a new field and clears the latch.
    injector.uncertain.set(false);
    assert!(finish_forwarded_stroke(&injector, 14, 2, &mut forwarded));
    assert!(finish_forwarded_stroke(&injector, 14, 0, &mut forwarded));
    assert!(forwarded.is_empty());
    assert_eq!(&*injector.events.borrow(), &[(14, 1), (14, 2), (14, 0)]);
}

#[derive(Clone, Copy)]
enum TestEvent {
    Press(char),
    Repeat(char),
    Release(char),
}

fn keycode(ch: char) -> u16 {
    ch.to_ascii_lowercase() as u16
}

fn apply(screen: &mut String, commands: &[OutputCommand]) {
    for command in commands {
        match command {
            OutputCommand::Backspace(count) => {
                let keep = screen.chars().count().saturating_sub(*count);
                *screen = screen.chars().take(keep).collect();
            }
            OutputCommand::Type(text) => screen.push_str(text),
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

fn simulate_grabbed_telex(events: &[TestEvent]) -> String {
    let config = crate::config::Config {
        input_method: "telex".to_string(),
        start_enabled: true,
        ..Default::default()
    };
    let mut daemon = Daemon::new(
        config,
        std::path::PathBuf::from("/tmp/dboard-grabbed-stream-test.toml"),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );

    let mut screen = String::new();
    let mut consumed_keys = HashSet::new();

    for event in events {
        match *event {
            TestEvent::Press(ch) => {
                let code = keycode(ch);
                let commands = daemon.process_key(ch);
                if commands.is_empty() {
                    screen.push(ch);
                } else {
                    consumed_keys.insert(code);
                    apply(&mut screen, &commands);
                }
            }
            TestEvent::Repeat(ch) => {
                if !consumed_keys.contains(&keycode(ch)) {
                    screen.push(ch);
                }
            }
            TestEvent::Release(ch) => {
                consumed_keys.remove(&keycode(ch));
            }
        }
    }

    screen
}

fn physical_stream(text: &str) -> Vec<TestEvent> {
    text.chars()
        .flat_map(|ch| [TestEvent::Press(ch), TestEvent::Release(ch)])
        .collect()
}

fn translated_key(
    key_state: &evdev::AttributeSet<evdev::KeyCode>,
    caps_lock: bool,
    key: evdev::KeyCode,
) -> char {
    key_to_char_with_case(key, is_modifier_held_shift(key_state), caps_lock)
        .expect("test key must translate")
}

#[test]
fn backspace_replacement_releases_and_restores_held_shift() {
    use dboard_protocol::inject::InjectResult;
    use std::cell::RefCell;
    struct Recorder(RefCell<Vec<(u16, i32)>>, bool);
    impl dboard_protocol::KeyInjector for Recorder {
        fn send_key_event(&self, code: u16, value: i32) -> InjectResult {
            self.0.borrow_mut().push((code, value));
            if value == 1 && self.1 {
                InjectResult::Failed
            } else {
                InjectResult::Success
            }
        }
        fn send_char(&self, _: char) -> InjectResult {
            InjectResult::NotSupported
        }
        fn send_string(&self, _: &str) -> InjectResult {
            InjectResult::NotSupported
        }
        fn send_backspace(&self) -> InjectResult {
            InjectResult::NotSupported
        }
        fn flush(&self) -> InjectResult {
            InjectResult::Success
        }
        fn inject_replacement(&self, _: usize, _: &str) -> InjectResult {
            assert_eq!(*self.0.borrow(), vec![(42, 0), (54, 0)]);
            InjectResult::Success
        }
    }
    let config = crate::config::Config {
        start_enabled: true,
        input_method: "telex".into(),
        ..Default::default()
    };
    let mut daemon = Daemon::new(
        config,
        std::path::PathBuf::from("/tmp/dboard-shift-test.toml"),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    daemon.process_key('a');
    daemon.process_key('b');
    let commands = daemon.process_key('\x08');
    let mut keys = evdev::AttributeSet::new();
    keys.insert(evdev::KeyCode::KEY_LEFTSHIFT);
    keys.insert(evdev::KeyCode::KEY_RIGHTSHIFT);
    for restore_fails in [false, true] {
        let recorder = Recorder(RefCell::new(Vec::new()), restore_fails);
        let mut revision = 0;
        let outcome = execute_with_shift_released(&recorder, &commands, &mut revision, 0, &keys);
        assert!(matches!(
            outcome,
            crate::commands::TransactionOutcome::Applied {
                visible_revision: 1
            }
        ));
        assert_eq!(revision, 1);
        assert_eq!(
            *recorder.0.borrow(),
            vec![(42, 0), (54, 0), (42, 1), (54, 1)]
        );
    }
}

#[test]
fn mixed_grab_results_and_hotplug_keep_per_keyboard_state() {
    let mut states = vec![
        KeyboardState::new(false, true),
        KeyboardState::new(false, false),
    ];
    states[0].consumed_keys.insert(30);
    assert!(states[0].grabbed);
    assert!(!states[1].grabbed);
    assert!(!states[1].consumed_keys.contains(&30));
    states.push(KeyboardState::new(false, true));
    states.remove(0);
    assert!(!states[0].grabbed);
    assert!(states[1].grabbed);
}

#[test]
fn production_evdev_state_tracks_caps_shift_and_lock_toggles() {
    let mut key_state = evdev::AttributeSet::new();
    let mut caps_lock = false;
    let mut num_lock = false;

    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        1,
    );
    assert!(caps_lock);
    assert!(!num_lock);
    assert_eq!(
        translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
        'A'
    );

    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_NUMLOCK,
        1,
    );
    assert!(num_lock);
    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_NUMLOCK,
        0,
    );
    assert!(num_lock);

    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_LEFTSHIFT,
        1,
    );
    assert_eq!(
        translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
        'a'
    );
    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_LEFTSHIFT,
        0,
    );

    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        0,
    );
    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        1,
    );
    assert!(!caps_lock);
    assert!(num_lock);
    assert_eq!(
        translated_key(&key_state, caps_lock, evdev::KeyCode::KEY_A),
        'a'
    );
}

#[test]
fn production_evdev_casing_composes_multiple_words_without_mixed_case() {
    let cases = [
        ("TIEENGS", "TIẾNG"),
        ("DDUOWNGF", "ĐƯỜNG"),
        ("DDUOJWC", "ĐƯỢC"),
        ("NEESU", "NẾU"),
        ("TIEENGS DDUOWNGF DDUOJWC NEESU", "TIẾNG ĐƯỜNG ĐƯỢC NẾU"),
    ];

    for (raw, expected) in cases {
        assert_eq!(
            simulate_grabbed_telex(&physical_stream(raw)),
            expected,
            "raw={raw}"
        );
    }
}

#[test]
fn production_evdev_caps_toggle_between_words_does_not_mix_case() {
    let mut key_state = evdev::AttributeSet::new();
    let mut caps_lock = false;
    let mut num_lock = false;
    let mut translated = String::new();

    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        1,
    );
    for key in [
        evdev::KeyCode::KEY_T,
        evdev::KeyCode::KEY_I,
        evdev::KeyCode::KEY_E,
        evdev::KeyCode::KEY_E,
        evdev::KeyCode::KEY_N,
        evdev::KeyCode::KEY_G,
        evdev::KeyCode::KEY_S,
    ] {
        translated.push(translated_key(&key_state, caps_lock, key));
    }
    translated.push(' ');
    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        0,
    );
    update_keyboard_state(
        &mut key_state,
        &mut caps_lock,
        &mut num_lock,
        evdev::KeyCode::KEY_CAPSLOCK,
        1,
    );
    for key in [
        evdev::KeyCode::KEY_N,
        evdev::KeyCode::KEY_E,
        evdev::KeyCode::KEY_E,
        evdev::KeyCode::KEY_S,
        evdev::KeyCode::KEY_U,
    ] {
        translated.push(translated_key(&key_state, caps_lock, key));
    }

    assert_eq!(translated, "TIEENGS neesu");
    assert_eq!(
        simulate_grabbed_telex(&physical_stream(&translated)),
        "TIẾNG nếu"
    );
}

#[test]
fn grabbed_stream_composes_telex_without_duplicate_prefixes() {
    assert_eq!(simulate_grabbed_telex(&physical_stream("hieenj")), "hiện");
    assert_eq!(simulate_grabbed_telex(&physical_stream("dd")), "đ");
    assert_eq!(simulate_grabbed_telex(&physical_stream("hoaf")), "hòa");
}

#[test]
fn grabbed_stream_second_tone_key_removes_the_tone() {
    assert_eq!(simulate_grabbed_telex(&physical_stream("hoaff")), "hoaf");
}

#[test]
fn grabbed_stream_preserves_word_boundaries_and_replacement_lengths() {
    let events = physical_stream("hieenj taij tao ddang gox tieengs Vieetj");
    assert_eq!(
        simulate_grabbed_telex(&events),
        "hiện tại tao đang gõ tiếng Việt"
    );
}

#[test]
fn consumed_press_suppresses_repeat_and_release_feedback() {
    let events = [
        TestEvent::Press('d'),
        TestEvent::Release('d'),
        TestEvent::Press('d'),
        TestEvent::Repeat('d'),
        TestEvent::Release('d'),
    ];

    assert_eq!(simulate_grabbed_telex(&events), "đ");
}
#[test]
fn disconnect_cleanup_releases_all_unconsumed_held_and_forwarded_keys() {
    let injector = UncertainInjector::default();
    let mut state = KeyboardState::new(false, true);
    state.keys.insert(evdev::KeyCode::KEY_LEFTCTRL);
    state.keys.insert(evdev::KeyCode::KEY_LEFTSHIFT);
    state.keys.insert(evdev::KeyCode::KEY_D);
    state.consumed_keys.insert(evdev::KeyCode::KEY_D.code());
    state.forwarded_keys.insert(evdev::KeyCode::KEY_C.code());
    release_disconnected_device_keys(&injector, &state, &[]);
    let mut released: Vec<u16> = injector
        .events
        .borrow()
        .iter()
        .map(|&(c, v)| {
            assert_eq!(v, 0);
            c
        })
        .collect();
    released.sort_unstable();
    assert_eq!(
        released,
        vec![
            evdev::KeyCode::KEY_LEFTCTRL.code(),
            evdev::KeyCode::KEY_LEFTSHIFT.code(),
            evdev::KeyCode::KEY_C.code(),
        ]
    );
}

#[test]
fn repeated_spaces_keep_ring_buffer_synced_on_backspace() {
    let config = crate::config::Config {
        start_enabled: true,
        input_method: "telex".into(),
        ..Default::default()
    };
    let mut daemon = Daemon::new(
        config,
        std::path::PathBuf::from("/tmp/dboard-space-repeat-test.toml"),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    for ch in "chao".chars() {
        let _ = daemon.process_key(ch);
    }
    // Initial Space press + 3 Space repeats (as dispatched by evdev_loop value==2)
    for _ in 0..4 {
        assert!(daemon.process_key(' ').is_empty());
    }
    // Deleting 3 of the 4 spaces must NOT rehydrate "chao" prematurely
    for _ in 0..3 {
        assert!(daemon.process_key('\x08').is_empty());
        assert!(daemon.engine.buffer().is_empty());
    }
    // Deleting the final space rehydrates "chao", allowing tone addition -> "chào"
    assert!(daemon.process_key('\x08').is_empty());
    assert_eq!(daemon.engine.buffer(), "chao");
    let cmds = daemon.process_key('f');
    assert_eq!(daemon.engine.buffer(), "chào");
    assert!(!cmds.is_empty());
}

#[test]
fn popup_click_pointer_absorption_guards_trailing_tap_without_hiding_real_clicks() {
    let base = std::time::Instant::now();
    let mut last_ptr = 10u64;
    let mut last_popup_click = Some(base);

    // 1. Single trailing +1 increment within 80ms is absorbed once
    assert!(absorb_trailing_popup_pointer_activity(
        11,
        &mut last_ptr,
        &mut last_popup_click,
        base + std::time::Duration::from_millis(25),
    ));
    assert_eq!(last_ptr, 11);
    assert!(last_popup_click.is_none());

    // 2. Subsequent click immediately after absorption MUST reset composition
    assert!(!absorb_trailing_popup_pointer_activity(
        12,
        &mut last_ptr,
        &mut last_popup_click,
        base + std::time::Duration::from_millis(30),
    ));
    assert_eq!(last_ptr, 12);

    // 3. Increment >= 80ms after popup click MUST reset composition
    let mut late_popup_click = Some(base);
    assert!(!absorb_trailing_popup_pointer_activity(
        13,
        &mut last_ptr,
        &mut late_popup_click,
        base + std::time::Duration::from_millis(80),
    ));
    assert_eq!(last_ptr, 13);

    // 4. Multi-click (+2) even within 80ms MUST reset composition
    let mut multi_popup_click = Some(base);
    assert!(!absorb_trailing_popup_pointer_activity(
        15,
        &mut last_ptr,
        &mut multi_popup_click,
        base + std::time::Duration::from_millis(10),
    ));
    assert_eq!(last_ptr, 15);
}
