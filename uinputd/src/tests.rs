use super::*;

#[test]
fn unicode_injection_restores_user_clipboard_before_returning() {
    let mut state = ClipboardInjectionState::default();

    state.begin(Some("copied text".to_string()), "tiếng Việt");
    let action = state.finish(Some("tiếng Việt"));

    assert_eq!(
        action,
        ClipboardInjectionAction::Restore("copied text".to_string())
    );
    assert!(!state.is_active());
}

#[test]
fn raw_key_payload_accepts_press_release_repeat_only() {
    assert_eq!(parse_key_event("30:1"), Some((30, 1)));
    assert_eq!(parse_key_event("30:0"), Some((30, 0)));
    assert_eq!(parse_key_event("30:2"), Some((30, 2)));
    for invalid in ["30:-1", "512:1", "30:3", "30:1:0", "a:1"] {
        assert_eq!(parse_key_event(invalid), None);
    }
}

#[test]
fn raw_key_write_failure_is_not_acknowledged_as_success() {
    let device = UinputDevice {
        fd: -1,
        clip: Arc::new(ClipState {
            inner: Mutex::new(ClipboardInjectionState::default()),
        }),
        transaction_lock: Mutex::new(()),
    };
    assert!(!device.send_key(30, 1));
    assert!(!device.backspace_n(1));
}

fn test_device(fd: i32) -> UinputDevice {
    UinputDevice {
        fd,
        clip: Arc::new(ClipState {
            inner: Mutex::new(ClipboardInjectionState::default()),
        }),
        transaction_lock: Mutex::new(()),
    }
}

fn run_test_commands(device: &UinputDevice, commands: &str) -> String {
    use std::io::Read;
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    client.write_all(commands.as_bytes()).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    handle_client(server, unsafe { libc::geteuid() }, device).unwrap();
    let mut responses = String::new();
    client.read_to_string(&mut responses).unwrap();
    responses
}

#[test]
fn oversized_request_is_rejected_without_dispatching() {
    let device = test_device(-1);
    let request = format!("TYPE:{}\n", "a".repeat(MAX_REQUEST_BYTES));
    assert_eq!(
        run_test_commands(&device, &request),
        "ERR request too large\n"
    );
}

#[test]
fn text_and_key_count_limits_are_enforced() {
    let device = test_device(-1);
    let commands = format!(
        "TYPE:{}\nPASTE:{}\nREPLACE:{}:a\nBACKSPACE:{}\n",
        "a".repeat(MAX_TEXT_BYTES + 1),
        "a".repeat(MAX_TEXT_BYTES + 1),
        MAX_KEY_COUNT + 1,
        MAX_KEY_COUNT + 1,
    );
    assert_eq!(
        run_test_commands(&device, &commands),
        "ERR text too large\nERR text too large\nERR count too large\nERR count too large\n"
    );
}

#[test]
fn request_at_size_limit_is_parsed() {
    let prefix = "TYPE:";
    let payload = "a".repeat(MAX_REQUEST_BYTES - prefix.len() - 1);
    let request = format!("{prefix}{payload}\n");
    let mut reader = BufReader::new(request.as_bytes());
    let line = read_bounded_line(&mut reader).unwrap().unwrap();
    assert_eq!(line.len(), MAX_REQUEST_BYTES - 1);
}

#[test]
fn ascii_commands_do_not_acknowledge_failed_writes() {
    let device = test_device(-1);
    let responses = run_test_commands(&device, "TYPE:a\nTYPE:A\nREPLACE:0:a\nREPLACE:1:\n");
    assert_eq!(responses.lines().count(), 4);
    assert!(responses.lines().all(|line| line.starts_with("ERR ")));
}

#[test]
fn ascii_and_paste_shortcut_fail_without_a_writable_device() {
    let device = test_device(-1);
    assert!(!device.type_ascii("a"));
    assert!(!device.type_ascii("A"));
    assert!(!device.paste_shortcut());
    assert!(!device.replace(1, "a"));
    assert!(!device.replace(0, "a"));
}

#[test]
fn partial_key_sequences_release_keys_and_modifiers() {
    for sequence in [
        [(42, 1), (30, 1), (30, 0), (42, 0)],
        [(29, 1), (47, 1), (47, 0), (29, 0)],
    ] {
        for fail_at in 0..sequence.len() {
            let mut calls = Vec::new();
            let mut held = Vec::new();
            assert!(!send_key_sequence_with(&sequence, |code, value| {
                let index = calls.len();
                calls.push((code, value));
                // Model a write that reached the device but whose SYN failed.
                if value == 1 {
                    held.push(code);
                } else {
                    held.retain(|&held_code| held_code != code);
                }
                index != fail_at
            }));
            assert!(held.is_empty(), "sequence={sequence:?} fail_at={fail_at}");
            assert_eq!(&calls[..=fail_at], &sequence[..=fail_at]);
            assert!(calls[fail_at + 1..].iter().all(|&(_, value)| value == 0));
        }
        let mut calls = Vec::new();
        assert!(send_key_sequence_with(&sequence, |code, value| {
            calls.push((code, value));
            true
        }));
        assert_eq!(calls, sequence);
    }
}

#[test]
fn cleanup_still_releases_modifier_if_key_release_fails() {
    let mut calls = Vec::new();
    assert!(!send_key_sequence_with(
        &[(29, 1), (47, 1), (47, 0), (29, 0)],
        |code, value| {
            calls.push((code, value));
            code != 47
        }
    ));
    assert_eq!(calls, [(29, 1), (47, 1), (47, 0), (29, 0)]);
}

#[test]
fn socket_commands_preserve_payload_spaces() {
    use std::io::Read;
    use std::os::fd::IntoRawFd;
    // Capture input_event bytes in a socket, never a real input device.
    let (writer, mut reader) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let device = test_device(writer.into_raw_fd());
    assert_eq!(
        run_test_commands(&device, "TYPE: \nREPLACE:0: a \n"),
        "OK\nOK\n"
    );
    drop(device);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    let presses: Vec<_> = bytes
        .chunks_exact(std::mem::size_of::<input_event>())
        .map(|bytes| unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<input_event>()) })
        .filter(|event| event.type_ == EV_KEY && event.value == 1)
        .map(|event| event.code)
        .collect();
    assert_eq!(presses, [57, 57, 30, 57]);
}

#[test]
fn unicode_command_ack_requires_verified_clipboard_restore() {
    let mut state = ClipboardInjectionState::default();
    state.begin(Some("user clipboard".to_string()), "đ");
    let action = state.finish(Some("đ"));

    assert_eq!(
        action,
        ClipboardInjectionAction::Restore("user clipboard".to_string())
    );

    let clipboard = std::cell::RefCell::new("đ".to_string());
    let restore_verified = restore_clipboard_verified_with(
        "user clipboard",
        |text| {
            *clipboard.borrow_mut() = text.to_string();
            true
        },
        || Some(clipboard.borrow().clone()),
        |_| {},
        || std::time::Duration::ZERO,
    );
    assert!(restore_verified);
    assert_eq!(*clipboard.borrow(), "user clipboard");
}

#[test]
fn clipboard_appears_after_multiple_polls() {
    let reads = std::cell::Cell::new(0);
    let elapsed = std::cell::Cell::new(std::time::Duration::ZERO);

    assert!(poll_clipboard_until(
        "tiếng Việt",
        std::time::Duration::from_millis(50),
        std::time::Duration::from_millis(10),
        || {
            let attempt = reads.get();
            reads.set(attempt + 1);
            (attempt >= 2).then(|| "tiếng Việt".to_string())
        },
        |duration| elapsed.set(elapsed.get() + duration),
        || elapsed.get(),
    ));
    assert_eq!(reads.get(), 3);
}

#[test]
fn clipboard_never_appears_before_timeout() {
    let elapsed = std::cell::Cell::new(std::time::Duration::ZERO);

    assert!(!poll_clipboard_until(
        "missing",
        std::time::Duration::from_millis(25),
        std::time::Duration::from_millis(10),
        || None,
        |duration| elapsed.set(elapsed.get() + duration),
        || elapsed.get(),
    ));
    assert_eq!(elapsed.get(), std::time::Duration::from_millis(25));
}

#[test]
fn restore_accepts_delayed_read_back() {
    let reads = std::cell::Cell::new(0);
    let elapsed = std::cell::Cell::new(std::time::Duration::ZERO);

    assert!(restore_clipboard_verified_with(
        "user clipboard",
        |_| true,
        || {
            let attempt = reads.get();
            reads.set(attempt + 1);
            (attempt >= 2).then(|| "user clipboard".to_string())
        },
        |duration| elapsed.set(elapsed.get() + duration),
        || elapsed.get(),
    ));
    assert_eq!(reads.get(), 3);
}

#[test]
fn user_clipboard_change_between_transaction_steps_skips_restore() {
    let mut state = ClipboardInjectionState::default();
    state.begin(Some("old user copy".to_string()), "temporary text");

    assert_eq!(
        state.finish(Some("new user copy")),
        ClipboardInjectionAction::SkipRestore
    );
    assert!(!state.is_active());
}

#[test]
fn consecutive_unicode_replacements_preserve_original_clipboard() {
    let mut state = ClipboardInjectionState::default();

    state.begin(Some("user copy".to_string()), "tiếng");
    assert_eq!(
        state.finish(Some("tiếng")),
        ClipboardInjectionAction::Restore("user copy".to_string())
    );

    state.begin(Some("user copy".to_string()), "Việt");
    assert_eq!(
        state.finish(Some("Việt")),
        ClipboardInjectionAction::Restore("user copy".to_string())
    );
    assert!(!state.is_active());
}

#[test]
fn user_clipboard_change_during_injection_is_not_overwritten() {
    let mut state = ClipboardInjectionState::default();

    state.begin(Some("old copy".to_string()), "tiếng");
    assert_eq!(
        state.finish(Some("new user copy")),
        ClipboardInjectionAction::SkipRestore
    );
    assert!(!state.is_active());
}

#[test]
fn failed_or_timed_out_injection_cleans_transaction_state() {
    let mut state = ClipboardInjectionState::default();

    state.begin(Some("user copy".to_string()), "tiếng");
    assert_eq!(state.finish(None), ClipboardInjectionAction::SkipRestore);
    assert!(!state.is_active());

    state.begin(Some("next copy".to_string()), "Việt");
    assert_eq!(
        state.finish(Some("Việt")),
        ClipboardInjectionAction::Restore("next copy".to_string())
    );
}

#[test]
fn overlapping_replacement_does_not_replace_original_snapshot() {
    let mut state = ClipboardInjectionState::default();

    state.begin(Some("user copy".to_string()), "tiếng");
    state.begin(Some("tiếng".to_string()), "Việt");

    assert_eq!(
        state.finish(Some("Việt")),
        ClipboardInjectionAction::Restore("user copy".to_string())
    );
    assert!(!state.is_active());
}

#[test]
fn authorized_peer_reaches_dispatcher() {
    let mut dispatched = false;

    let result = authorize_and_dispatch(1000, 1000, || {
        dispatched = true;
    });

    assert!(result.is_ok());
    assert!(dispatched);
}

#[test]
fn unauthorized_peer_is_rejected_before_dispatch() {
    let mut dispatched = false;

    let result = authorize_and_dispatch(1001, 1000, || {
        dispatched = true;
    });

    assert!(result.is_err());
    assert!(!dispatched);
}

#[test]
fn socket_permissions_are_applied_after_bind() {
    use std::os::unix::fs::PermissionsExt;

    let test_dir =
        std::env::temp_dir().join(format!("vietc-uinputd-phase1-{}", std::process::id()));
    let socket_path = test_dir.join("uinput.sock");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir(&test_dir).unwrap();

    let listener = bind_socket(&socket_path, 0o600).unwrap();
    let mode = fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode, 0o600);

    drop(listener);
    fs::remove_dir_all(&test_dir).unwrap();
}

#[test]
fn bind_failure_does_not_remove_preexisting_regular_file() {
    let test_dir = std::env::temp_dir().join(format!(
        "vietc-uinputd-phase1-existing-{}",
        std::process::id()
    ));
    let socket_path = test_dir.join("uinput.sock");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir(&test_dir).unwrap();
    fs::write(&socket_path, b"keep me").unwrap();

    let result = bind_socket(&socket_path, 0o600);

    assert!(result.is_err());
    assert_eq!(fs::read(&socket_path).unwrap(), b"keep me");

    fs::remove_dir_all(&test_dir).unwrap();
}

#[test]
fn runtime_socket_path_is_private_and_canonical() {
    let runtime_root = Path::new("/run/user/1000");

    let socket_path = runtime_socket_path(runtime_root);

    assert_eq!(socket_path, runtime_root.join("vietc").join("uinput.sock"));
}

#[test]
fn authorized_unix_peer_reaches_dispatcher() {
    let test_dir =
        std::env::temp_dir().join(format!("vietc-uinputd-phase2-peer-{}", std::process::id()));
    let socket_path = test_dir.join("uinput.sock");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir(&test_dir).unwrap();

    let listener = bind_socket(&socket_path, 0o600).unwrap();
    let client = UnixStream::connect(&socket_path).unwrap();
    let (server, _) = listener.accept().unwrap();
    let expected_uid = unsafe { libc::getuid() };
    let mut dispatched = false;

    authorize_stream_and_dispatch(&server, expected_uid, || {
        dispatched = true;
    })
    .unwrap();

    assert!(dispatched);

    drop(client);
    drop(server);
    drop(listener);
    fs::remove_dir_all(&test_dir).unwrap();
}

#[test]
fn unix_peer_is_rejected_before_dispatch_when_uid_is_not_allowed() {
    let test_dir = std::env::temp_dir().join(format!(
        "vietc-uinputd-phase2-reject-{}",
        std::process::id()
    ));
    let socket_path = test_dir.join("uinput.sock");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir(&test_dir).unwrap();

    let listener = bind_socket(&socket_path, 0o600).unwrap();
    let client = UnixStream::connect(&socket_path).unwrap();
    let (server, _) = listener.accept().unwrap();
    let current_uid = unsafe { libc::getuid() };
    let denied_uid = current_uid.saturating_add(1);
    let mut dispatched = false;

    let result = authorize_stream_and_dispatch(&server, denied_uid, || {
        dispatched = true;
    });

    assert!(result.is_err());
    assert!(!dispatched);

    drop(client);
    drop(server);
    drop(listener);
    fs::remove_dir_all(&test_dir).unwrap();
}
