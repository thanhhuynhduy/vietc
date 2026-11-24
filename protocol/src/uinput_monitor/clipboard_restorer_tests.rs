// SPDX-License-Identifier: MIT
use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

#[test]
fn shutdown_wakes_and_joins_restorer_deterministically() {
    let exited = Arc::new(AtomicBool::new(false));
    let worker_exited = Arc::clone(&exited);
    let state = Arc::new(ClipState {
        inner: Mutex::new(ClipInner {
            burst: ClipboardBurstState::default(),
            restore_due: None,
            shutdown: false,
            active_window: String::new(),
        }),
        io_lock: Mutex::new(()),
        cv: Condvar::new(),
        focus_generation: Mutex::new(None),
    });
    let (started_tx, started_rx) = mpsc::channel();

    let mut restorer = ClipboardRestorer::spawn(Arc::clone(&state), move |state| {
        started_tx.send(()).unwrap();
        run_restorer_with(state, || None, |_| true);
        worker_exited.store(true, Ordering::SeqCst);
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    restorer.shutdown_and_join().unwrap();

    assert!(exited.load(Ordering::SeqCst));
    assert!(restorer.is_joined());
}

#[test]
fn shutdown_prevents_restore_after_join() {
    let restored = Arc::new(Mutex::new(Vec::new()));
    let worker_restored = Arc::clone(&restored);
    let state = Arc::new(ClipState {
        inner: Mutex::new(ClipInner {
            burst: {
                let mut burst = ClipboardBurstState::default();
                burst.begin_injection(Some("user clipboard".to_string()), "injected text", 1);
                burst
            },
            restore_due: Some(Instant::now()),
            shutdown: false,
            active_window: String::new(),
        }),
        io_lock: Mutex::new(()),
        cv: Condvar::new(),
        focus_generation: Mutex::new(None),
    });

    let mut restorer = ClipboardRestorer::spawn(Arc::clone(&state), move |state| {
        run_restorer_with(
            state,
            || Some("injected text".to_string()),
            |text| {
                worker_restored
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_string());
                true
            },
        );
    });

    restorer.shutdown_and_join().unwrap();
    let restore_count_after_join = restored
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len();
    std::thread::sleep(Duration::from_millis(20));

    assert_eq!(
        restored
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len(),
        restore_count_after_join
    );
}

#[test]
fn user_clipboard_change_during_restore_io_is_not_overwritten() {
    let reads = Arc::new(Mutex::new(VecDeque::from([
        Some("injected text".to_string()),
        Some("new user clipboard".to_string()),
    ])));
    let worker_reads = Arc::clone(&reads);
    let restored = Arc::new(Mutex::new(Vec::new()));
    let worker_restored = Arc::clone(&restored);
    let state = Arc::new(ClipState {
        inner: Mutex::new(ClipInner {
            burst: {
                let mut burst = ClipboardBurstState::default();
                burst.begin_injection(Some("old user clipboard".to_string()), "injected text", 1);
                burst
            },
            restore_due: Some(Instant::now()),
            shutdown: false,
            active_window: String::new(),
        }),
        io_lock: Mutex::new(()),
        cv: Condvar::new(),
        focus_generation: Mutex::new(None),
    });
    let worker_state = Arc::clone(&state);

    let handle = std::thread::spawn(move || {
        run_restorer_with(
            worker_state,
            || {
                worker_reads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .pop_front()
                    .flatten()
            },
            |text| {
                worker_restored
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(text.to_string());
                true
            },
        );
    });

    for _ in 0..100 {
        if reads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    {
        let mut inner = state
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.shutdown = true;
    }
    state.cv.notify_all();
    handle.join().unwrap();

    assert!(restored
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty());
}

#[cfg(feature = "wayland-protocol")]
#[test]
fn superseded_wayland_clipboard_restore_returns_superseded_not_restored() {
    let backups = [
        WaylandClipboardBackup::Text("original user text".to_string()),
        WaylandClipboardBackup::NonText(NonTextClipboard::Empty),
        WaylandClipboardBackup::NonText(NonTextClipboard::MultiMime(vec![(
            "text/html".to_string(),
            b"<b>hello</b>".to_vec(),
        )])),
    ];
    for backup in &backups {
        let outcome = UinputInjector::restore_wayland_clipboard_guarded(backup, || false);
        assert_eq!(
            outcome,
            RestoreOutcome::Superseded,
            "superseded restore must never report Restored and drop original backup"
        );
    }
}
