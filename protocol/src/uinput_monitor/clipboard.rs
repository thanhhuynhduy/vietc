// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::replacement_transaction::{ClipboardAction, ClipboardBurstState, ReplacementPhase};
use super::UinputInjector;

const CLIPBOARD_READY_TIMEOUT: Duration = Duration::from_millis(750);
const CLIPBOARD_READY_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PASTE_OBSERVATION_DELAY: Duration = Duration::from_millis(20);
const CLIPBOARD_TRACE_LIMIT: usize = 128;
static CLIPBOARD_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

pub(super) fn trace_clipboard(sequence_id: u64, phase: &str, detail: &str) {
    let trace_index = CLIPBOARD_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    if trace_index < CLIPBOARD_TRACE_LIMIT {
        eprintln!(
            "[vietc] clipboard trace={} sequence_id={} phase={} {}",
            trace_index, sequence_id, phase, detail
        );
    }
}

/// Largest non-text clipboard payload we are willing to back up and restore.
const MAX_BINARY_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;

/// Clipboard contents that are not plain text and therefore cannot go through
/// the text snapshot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NonTextClipboard {
    Empty,
    Data { mime: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardKind {
    Empty,
    Text,
    Other(String),
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/plain") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// Decide how a clipboard offering `types` must be backed up: plain text is
/// preserved as text; anything else (image, file list...) is preserved as raw
/// bytes of its first offered type; nothing offered means an empty clipboard.
fn classify_clipboard_types(types: &[String]) -> ClipboardKind {
    if types.is_empty() {
        ClipboardKind::Empty
    } else if types.iter().any(|mime| is_text_mime(mime)) {
        ClipboardKind::Text
    } else {
        ClipboardKind::Other(types[0].clone())
    }
}

/// Shared clipboard bookkeeping between the injection path and the background
/// restorer thread.
pub(super) struct ClipInner {
    pub(super) burst: ClipboardBurstState,
    /// When set, the restorer thread should rewrite the user's clipboard at
    /// this instant. `None` means no restore is pending.
    pub(super) restore_due: Option<Instant>,
    /// Set on shutdown so the restorer thread can exit.
    pub(super) shutdown: bool,
    /// The currently active window class (lowercase).
    pub(super) active_window: String,
}

pub(super) struct ClipState {
    pub(super) inner: Mutex<ClipInner>,
    pub(super) io_lock: Mutex<()>,
    pub(super) cv: Condvar,
    pub(super) focus_generation: Mutex<Option<Arc<AtomicU64>>>,
}

pub(super) struct ClipboardRestorer {
    state: Arc<ClipState>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ClipboardRestorer {
    pub(super) fn spawn(
        state: Arc<ClipState>,
        worker: impl FnOnce(Arc<ClipState>) + Send + 'static,
    ) -> Self {
        let worker_state = Arc::clone(&state);
        let handle = std::thread::spawn(move || worker(worker_state));
        Self {
            state,
            handle: Some(handle),
        }
    }

    pub(super) fn shutdown_and_join(&mut self) -> std::thread::Result<()> {
        {
            let mut inner = self.state.inner.lock().unwrap();
            inner.shutdown = true;
            inner.restore_due = None;
            inner.burst.reset();
        }
        self.state.cv.notify_all();

        if let Some(handle) = self.handle.take() {
            handle.join()
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    pub(super) fn is_joined(&self) -> bool {
        self.handle.is_none()
    }
}

#[cfg(feature = "wayland-protocol")]
#[derive(Clone)]
pub(super) enum WaylandClipboardBackup {
    Text(String),
    NonText(NonTextClipboard),
}

#[cfg(feature = "wayland-protocol")]
pub(super) struct RetainedWaylandPaste {
    offer: crate::wayland_clipboard::ClipboardOffer,
    backup: WaylandClipboardBackup,
}

impl UinputInjector {
    fn run_cmd_with_timeout(
        mut cmd: std::process::Command,
        timeout: Duration,
    ) -> Option<std::process::Output> {
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().ok()?;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return child.wait_with_output().ok(),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return None,
            }
        }
    }

    fn read_clipboard() -> Option<String> {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let (prog, args): (&str, &[&str]) = if is_wayland {
            ("wl-paste", &["-n"])
        } else {
            ("xclip", &["-selection", "clipboard", "-o"])
        };
        let mut cmd = Self::user_cmd(prog).ok()?;
        cmd.args(args);
        let output = Self::run_cmd_with_timeout(cmd, Duration::from_millis(250))?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn clipboard_types() -> Option<Vec<String>> {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        if !is_wayland {
            return None;
        }
        let mut cmd = Self::user_cmd("wl-paste").ok()?;
        cmd.args(["-l"]);
        let output = Self::run_cmd_with_timeout(cmd, Duration::from_millis(250))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return stderr.contains("Nothing is copied").then(Vec::new);
        }
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }

    fn backup_non_text_clipboard() -> Option<NonTextClipboard> {
        let types = Self::clipboard_types()?;
        match classify_clipboard_types(&types) {
            ClipboardKind::Text => None,
            ClipboardKind::Empty => Some(NonTextClipboard::Empty),
            ClipboardKind::Other(mime) => {
                let mut cmd = Self::user_cmd("wl-paste").ok()?;
                cmd.args(["--type", &mime]);
                let output = Self::run_cmd_with_timeout(cmd, Duration::from_millis(250))?;
                if !output.status.success() || output.stdout.len() > MAX_BINARY_CLIPBOARD_BYTES {
                    return None;
                }
                Some(NonTextClipboard::Data {
                    mime,
                    bytes: output.stdout,
                })
            }
        }
    }

    fn restore_non_text_clipboard(backup: &NonTextClipboard) -> bool {
        let Ok(mut cmd) = Self::user_cmd("wl-copy") else {
            return false;
        };
        match backup {
            NonTextClipboard::Empty => {
                cmd.arg("--clear");
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                cmd.status().map(|status| status.success()).unwrap_or(false)
            }
            NonTextClipboard::Data { mime, bytes } => {
                use std::io::Write;
                cmd.args(["--type", mime]);
                cmd.stdin(std::process::Stdio::piped());
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                let Ok(mut child) = cmd.spawn() else {
                    return false;
                };
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(bytes);
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                true
            }
        }
    }

    fn read_clipboard_expecting(expected: &str) -> Option<String> {
        let mut last = None;
        for attempt in 0..4 {
            last = Self::read_clipboard();
            if last.as_deref() == Some(expected) {
                return last;
            }
            if attempt < 3 {
                std::thread::sleep(CLIPBOARD_READY_POLL_INTERVAL * 2);
            }
        }
        last
    }

    pub(super) fn paste_via_clipboard(
        &self,
        text: &str,
        backspaces: usize,
        focus_generation: u64,
    ) -> bool {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            #[cfg(feature = "wayland-protocol")]
            return self.paste_via_wayland_clipboard(text, backspaces, focus_generation);
            #[cfg(not(feature = "wayland-protocol"))]
            {
                eprintln!("[vietc] Wayland Unicode injection requires the wayland-protocol feature; no keys sent");
                return false;
            }
        }
        let t_total = std::time::Instant::now();
        let _clipboard_io = self.clip.io_lock.lock().unwrap();

        let needs_snapshot = {
            let mut st = self.clip.inner.lock().unwrap();
            st.restore_due = None;
            !st.burst.is_active()
        };
        let non_text_backup = if needs_snapshot {
            Self::backup_non_text_clipboard()
        } else {
            None
        };
        let current_clipboard = (needs_snapshot && non_text_backup.is_none())
            .then(Self::read_clipboard)
            .flatten();

        let sequence_id = {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst
                .begin_injection(current_clipboard.clone(), text, focus_generation);
            st.burst.transaction_id().unwrap()
        };
        trace_clipboard(
            sequence_id,
            "prepared",
            if current_clipboard.is_some() {
                "clipboard=present"
            } else {
                "clipboard=empty-or-unavailable"
            },
        );

        trace_clipboard(sequence_id, "prepared", "state=writing");
        if !Self::copy_to_clipboard(text) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=clipboard-write");
            st.burst.reset();
            return false;
        }

        if !Self::wait_for_clipboard_value(text) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            let elapsed_ms = st.burst.elapsed().map_or(0, |elapsed| elapsed.as_millis());
            trace_clipboard(
                sequence_id,
                "failed",
                &format!("phase=clipboard-read-back elapsed_ms={elapsed_ms}"),
            );
            st.burst.reset();
            return false;
        }
        {
            let mut st = self.clip.inner.lock().unwrap();
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::ClipboardWritten);
            debug_assert!(transitioned);
        }
        trace_clipboard(sequence_id, "clipboard_written", "state=verified");

        if backspaces > 0 {
            if self.send_backspaces(backspaces).is_err() {
                let mut st = self.clip.inner.lock().unwrap();
                st.burst.mark_failed(sequence_id);
                trace_clipboard(sequence_id, "failed", "phase=backspace-dispatch");
                st.burst.reset();
                return false;
            }
            // Let the focused application consume the complete delete phase
            // before Ctrl+V starts the insert phase. Without this boundary,
            // Chromium-family clients can process the paste after only part of
            // the Backspace batch, leaving the first raw character duplicated.
            std::thread::sleep(Duration::from_millis(10));
        }

        if self.send_ctrl_v().is_err() {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=paste-dispatch");
            st.burst.reset();
            return false;
        }
        {
            let mut st = self.clip.inner.lock().unwrap();
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::PasteDispatched);
            debug_assert!(transitioned);
            st.restore_due = Some(Instant::now() + CLIPBOARD_READY_TIMEOUT);
        }
        trace_clipboard(sequence_id, "paste_dispatched", "state=sent");

        if !Self::wait_for_target_consumption(&self.clip, sequence_id, text, focus_generation) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            st.restore_due = None;
            trace_clipboard(sequence_id, "failed", "phase=target-observation");
            st.burst.reset();
            return false;
        }
        trace_clipboard(sequence_id, "target_observed", "state=clipboard-consumed");
        trace_clipboard(sequence_id, "applied", "state=ready-to-restore");

        let current_clipboard = Self::read_clipboard_expecting(text);
        let action = {
            let mut st = self.clip.inner.lock().unwrap();
            let action = st.burst.finish_restore(current_clipboard.as_deref());
            if non_text_backup.is_some() && current_clipboard.as_deref() == Some(text) {
                // The user's clipboard was empty/non-text: there is no text to
                // restore through the burst machinery, so forget the "unrestored
                // injected text" marker and restore the original data below.
                st.burst.forget_unrestored();
            }
            action
        };
        if let (Some(backup), true) = (
            non_text_backup.as_ref(),
            current_clipboard.as_deref() == Some(text),
        ) {
            trace_clipboard(sequence_id, "restore", "state=writing-non-text");
            let restored = Self::restore_non_text_clipboard(backup);
            trace_clipboard(
                sequence_id,
                "restore",
                if restored {
                    "state=non-text-restored"
                } else {
                    "state=non-text-restore-failed"
                },
            );
            self.clip.inner.lock().unwrap().burst.reset();
            let elapsed = t_total.elapsed().as_millis();
            if elapsed > 20 {
                eprintln!(
                    "[vietc] clipboard sequence_id={} elapsed_ms={}",
                    sequence_id, elapsed
                );
            }
            return restored;
        }

        let success = match action {
            ClipboardAction::Restore(saved) => {
                trace_clipboard(sequence_id, "restore", "state=writing");
                if !Self::copy_to_clipboard(&saved) {
                    self.clip.inner.lock().unwrap().burst.reset();
                    trace_clipboard(sequence_id, "restore", "state=write-failed");
                    false
                } else if !Self::wait_for_clipboard_value(&saved) {
                    self.clip.inner.lock().unwrap().burst.reset();
                    trace_clipboard(sequence_id, "verified-read-back", "state=failed");
                    false
                } else {
                    self.clip
                        .inner
                        .lock()
                        .unwrap()
                        .burst
                        .complete_restore(sequence_id);
                    trace_clipboard(sequence_id, "verified-read-back", "state=restored");
                    true
                }
            }
            ClipboardAction::SkipRestore => {
                trace_clipboard(
                    sequence_id,
                    "restore",
                    "state=skipped-user-clipboard-changed",
                );
                true
            }
            ClipboardAction::WriteInjected(_) => false,
        };

        let elapsed = t_total.elapsed().as_millis();
        if elapsed > 20 {
            eprintln!(
                "[vietc] clipboard sequence_id={} elapsed_ms={}",
                sequence_id, elapsed
            );
        }
        success
    }

    #[cfg(feature = "wayland-protocol")]
    pub(super) fn wayland_paste_is_uncertain(&self) -> bool {
        self.uncertain_focus.lock().unwrap().is_some()
    }

    #[cfg(feature = "wayland-protocol")]
    fn snapshot_wayland_clipboard() -> Option<WaylandClipboardBackup> {
        let types = Self::clipboard_types()?;
        match classify_clipboard_types(&types) {
            ClipboardKind::Empty => Some(WaylandClipboardBackup::NonText(NonTextClipboard::Empty)),
            ClipboardKind::Text => Self::read_clipboard().map(WaylandClipboardBackup::Text),
            ClipboardKind::Other(_) => {
                Self::backup_non_text_clipboard().map(WaylandClipboardBackup::NonText)
            }
        }
    }

    #[cfg(feature = "wayland-protocol")]
    fn restore_wayland_clipboard(backup: &WaylandClipboardBackup) -> bool {
        use crate::clipboard_command::run_copy;
        let Ok(mut command) = Self::user_cmd("wl-copy") else {
            return false;
        };
        match backup {
            WaylandClipboardBackup::Text(text) => {
                command.args(["--type", "text/plain;charset=utf-8"]);
                run_copy(command, text.as_bytes(), CLIPBOARD_READY_TIMEOUT)
                    && Self::wait_for_clipboard_value(text)
            }
            WaylandClipboardBackup::NonText(NonTextClipboard::Empty) => {
                command.arg("--clear");
                run_copy(command, &[], CLIPBOARD_READY_TIMEOUT)
                    && Self::clipboard_types().is_some_and(|types| types.is_empty())
            }
            WaylandClipboardBackup::NonText(NonTextClipboard::Data { mime, bytes }) => {
                command.args(["--type", mime]);
                if !run_copy(command, bytes, CLIPBOARD_READY_TIMEOUT) {
                    return false;
                }
                let Ok(mut verify) = Self::user_cmd("wl-paste") else {
                    return false;
                };
                verify.args(["--no-newline", "--type", mime]);
                Self::run_cmd_with_timeout(verify, CLIPBOARD_READY_TIMEOUT)
                    .is_some_and(|output| output.status.success() && output.stdout == *bytes)
            }
        }
    }

    #[cfg(feature = "wayland-protocol")]
    fn paste_via_wayland_clipboard(
        &self,
        text: &str,
        backspaces: usize,
        focus_generation: u64,
    ) -> bool {
        use crate::clipboard_transaction::{paste_with_transfer, PasteCompletion};
        use crate::wayland_clipboard::{ClipboardOffer, Ownership};

        let _clipboard_io = self.clip.io_lock.lock().unwrap();
        let mut retained = self.wayland_paste.lock().unwrap();
        let mut retired = self.retired_wayland_offers.lock().unwrap();
        // Reap only workers that already finished. A stalled recipient must not
        // make the grabbed-keyboard capture thread join a five-second drain.
        retired.retain(|offer| !offer.is_finished());
        if retired.len() + 2 > 8 {
            *self.uncertain_focus.lock().unwrap() = Some(focus_generation);
            eprintln!("[vietc] Clipboard transfer backlog; native input only until focus changes");
            return false;
        }
        let mut retire = |offer: ClipboardOffer| {
            offer.request_retire();
            retired.push(offer);
        };
        if self.wayland_paste_is_uncertain() {
            eprintln!("[vietc] Unicode replacement suspended after uncertain paste; change input field before resuming");
            return false;
        }
        let backup = match retained.as_ref() {
            // Keep the previous source and backup alive until publication of a
            // successor succeeds. Preparation failures must not discard them.
            Some(previous) if previous.offer.ownership() == Ownership::Owned => {
                previous.backup.clone()
            }
            Some(previous) if previous.offer.ownership() == Ownership::Unavailable => {
                // Unknown ownership is not evidence of an external copy. Keep
                // the only saved snapshot and do not overwrite a possible new owner.
                *self.uncertain_focus.lock().unwrap() = Some(focus_generation);
                eprintln!("[vietc] Clipboard transport unavailable; saved snapshot retained, no keys sent");
                return false;
            }
            _ => {
                let Some(backup) = Self::snapshot_wayland_clipboard() else {
                    eprintln!("[vietc] Clipboard snapshot unavailable; no replacement keys sent");
                    return false;
                };
                backup
            }
        };
        if self.current_focus_generation() != focus_generation {
            return false;
        }
        let offer = match ClipboardOffer::publish(text, CLIPBOARD_READY_TIMEOUT) {
            Ok(offer) => offer,
            Err(error) => {
                eprintln!(
                    "[vietc] Wayland clipboard publication failed before key dispatch: {error}"
                );
                return false;
            }
        };
        let publication = offer.wait_until_published(CLIPBOARD_READY_TIMEOUT);
        if let Some(previous) = retained.take() {
            retire(previous.offer);
        }
        if let Err(error) = publication {
            eprintln!(
                "[vietc] Clipboard publication uncertain; snapshot retained, no keys sent: {error}"
            );
            *self.uncertain_focus.lock().unwrap() = Some(focus_generation);
            *retained = Some(RetainedWaylandPaste { offer, backup });
            return false;
        }
        if self.current_focus_generation() != focus_generation || offer.arm().is_err() {
            let cleanup_ok = match offer.ownership() {
                Ownership::Owned => Self::restore_wayland_clipboard(&backup),
                Ownership::Replaced => true,
                Ownership::Unavailable => false,
            };
            if cleanup_ok {
                retire(offer);
            } else {
                *retained = Some(RetainedWaylandPaste { offer, backup });
            }
            return false;
        }
        let result = paste_with_transfer(
            || {
                if self.current_focus_generation() != focus_generation || !offer.owns_selection() {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                if backspaces > 0 {
                    self.send_backspaces(backspaces)?;
                    std::thread::sleep(Duration::from_millis(10));
                }
                if self.current_focus_generation() != focus_generation || !offer.owns_selection() {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.send_ctrl_v()
            },
            || {
                offer.wait_for_transfer(CLIPBOARD_READY_TIMEOUT, || {
                    self.current_focus_generation() == focus_generation
                })?;
                // A clipboard manager may request the payload before Chromium/Electron.
                // Keep the injected offer active briefly so the actual target cannot
                // observe the restored user clipboard when it handles Ctrl+V later.
                std::thread::sleep(Duration::from_millis(100));
                if self.current_focus_generation() != focus_generation || !offer.owns_selection() {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                Ok(())
            },
            || match offer.ownership() {
                Ownership::Owned => Self::restore_wayland_clipboard(&backup),
                Ownership::Replaced => true,
                Ownership::Unavailable => false,
            },
        );
        match result {
            PasteCompletion::PayloadServed { cleanup_ok } => {
                // This is transport evidence only. Neither a clipboard manager
                // nor the target app exposes insertion acknowledgment here.
                eprintln!("[vietc] Wayland clipboard payload served; cleanup_ok={cleanup_ok}; target insertion unconfirmed");
                if !cleanup_ok {
                    *retained = Some(RetainedWaylandPaste { offer, backup });
                } else {
                    retire(offer);
                }
                true
            }
            PasteCompletion::Indeterminate => {
                eprintln!("[vietc] Wayland paste indeterminate; keeping payload available, no clipboard restore or retry");
                *self.uncertain_focus.lock().unwrap() = Some(focus_generation);
                *retained = Some(RetainedWaylandPaste { offer, backup });
                false
            }
        }
    }

    fn wait_for_clipboard_value(expected: &str) -> bool {
        let deadline = Instant::now() + CLIPBOARD_READY_TIMEOUT;
        loop {
            if Self::read_clipboard().as_deref() == Some(expected) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(CLIPBOARD_READY_POLL_INTERVAL);
        }
    }

    fn wait_for_target_consumption(
        state: &ClipState,
        transaction_id: u64,
        injected_text: &str,
        focus_generation: u64,
    ) -> bool {
        let observed = Self::poll_paste_observation_with(
            transaction_id,
            injected_text,
            || {
                let state = state.inner.lock().unwrap();
                state
                    .burst
                    .matches(transaction_id, focus_generation)
                    .then_some(transaction_id)
            },
            Self::read_clipboard,
            Instant::now,
            std::thread::sleep,
        );
        if !observed {
            return false;
        }

        let mut inner = state.inner.lock().unwrap();
        if !inner.burst.matches(transaction_id, focus_generation) {
            return false;
        }
        if inner
            .burst
            .transition(transaction_id, ReplacementPhase::TargetObserved)
            && inner
                .burst
                .transition(transaction_id, ReplacementPhase::Applied)
        {
            inner.restore_due = None;
            true
        } else {
            false
        }
    }

    fn poll_paste_observation_with(
        transaction_id: u64,
        injected_text: &str,
        mut active_transaction: impl FnMut() -> Option<u64>,
        mut read_clipboard: impl FnMut() -> Option<String>,
        mut now: impl FnMut() -> Instant,
        mut sleep: impl FnMut(Duration),
    ) -> bool {
        let started = now();
        let deadline = started + CLIPBOARD_READY_TIMEOUT;
        loop {
            if active_transaction() != Some(transaction_id) {
                return false;
            }
            if read_clipboard().as_deref() != Some(injected_text) {
                return false;
            }

            let current = now();
            if current.duration_since(started) >= PASTE_OBSERVATION_DELAY {
                return true;
            }
            if current >= deadline {
                return false;
            }
            sleep(CLIPBOARD_READY_POLL_INTERVAL.min(deadline - current));
        }
    }

    fn copy_to_clipboard(s: &str) -> bool {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let (prog, args): (&str, &[&str]) = if is_wayland {
            ("wl-copy", &[])
        } else {
            ("xclip", &["-selection", "clipboard", "-i"])
        };
        let Ok(mut cmd) = Self::user_cmd(prog) else {
            return false;
        };
        cmd.args(args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(mut child) => {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(s.as_bytes());
                }
                if is_wayland {
                    // wl-copy may retain clipboard ownership after its launcher
                    // exits. Reap asynchronously while readiness and restoration
                    // are verified independently by bounded read-back polling.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return true;
                }
                // X11: wait for xclip to finish writing
                child.wait().map(|s| s.success()).unwrap_or(false)
            }
            Err(e) => {
                eprintln!("[vietc] copy_to_clipboard: {} spawn failed: {}", prog, e);
                false
            }
        }
    }
}

pub(super) fn run_restorer(state: Arc<ClipState>) {
    run_restorer_with(
        state,
        UinputInjector::read_clipboard,
        UinputInjector::copy_to_clipboard,
    );
}

fn run_restorer_with(
    state: Arc<ClipState>,
    mut read_clipboard: impl FnMut() -> Option<String>,
    mut copy_to_clipboard: impl FnMut(&str) -> bool,
) {
    loop {
        let mut st = state.inner.lock().unwrap();
        loop {
            if st.shutdown {
                return;
            }
            match st.restore_due {
                None => {
                    st = state.cv.wait(st).unwrap();
                }
                Some(due) => {
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    let (guard, _) = state.cv.wait_timeout(st, due - now).unwrap();
                    st = guard;
                }
            }
        }
        st.restore_due = None;
        drop(st);

        let _clipboard_io = state.io_lock.lock().unwrap();
        let current_clipboard = read_clipboard();
        let action = {
            let mut st = state.inner.lock().unwrap();
            if st.shutdown {
                st.burst.reset();
                return;
            }
            st.burst.finish_restore(current_clipboard.as_deref())
        };

        if let ClipboardAction::Restore(restored) = action {
            let current_clipboard = read_clipboard();
            let should_restore = {
                let mut st = state.inner.lock().unwrap();
                if st.shutdown {
                    st.burst.reset();
                    return;
                }
                matches!(
                    st.burst.finish_restore(current_clipboard.as_deref()),
                    ClipboardAction::Restore(_)
                )
            };
            if !should_restore {
                continue;
            }

            let transaction_id = {
                let st = state.inner.lock().unwrap();
                st.burst.transaction_id()
            };
            let restored_ok = copy_to_clipboard(&restored);
            let mut st = state.inner.lock().unwrap();
            if st.shutdown {
                st.burst.reset();
                return;
            }
            if let Some(transaction_id) = transaction_id {
                if restored_ok {
                    let _ = st.burst.complete_restore(transaction_id);
                } else if st.burst.transaction_id() == Some(transaction_id) {
                    st.burst.reset();
                }
            }
        }
    }
}

#[cfg(test)]
mod clipboard_observation_tests {
    use super::*;
    use std::cell::Cell;

    fn observe_with_delay(delay: Duration) -> bool {
        let transaction_id = 42;
        let elapsed = Cell::new(Duration::ZERO);
        UinputInjector::poll_paste_observation_with(
            transaction_id,
            "injected",
            || Some(transaction_id),
            || Some("injected".to_string()),
            || Instant::now() + elapsed.get(),
            |duration| {
                let simulated_delay = duration.min(delay.saturating_sub(elapsed.get()));
                elapsed.set(elapsed.get() + simulated_delay.max(duration));
            },
        )
    }

    #[test]
    fn clipboard_stability_does_not_prove_a_delayed_target_read() {
        let origin = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let reported_consumed = UinputInjector::poll_paste_observation_with(
            42,
            "tự",
            || Some(42),
            || Some("tự".to_string()),
            || origin + elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
        );
        // The target has not requested data at all. Restoring the saved filename
        // now lets a later Ctrl+V read that filename instead of the replacement.
        // Characterize the old heuristic explicitly: true here means only
        // clipboard stability. Wayland must never use it as transfer evidence.
        assert!(reported_consumed);
        assert_eq!(elapsed.get(), PASTE_OBSERVATION_DELAY);
    }

    #[test]
    fn bounded_polling_accepts_supported_clipboard_delays() {
        for delay_ms in [20, 50, 100, 250] {
            assert!(observe_with_delay(Duration::from_millis(delay_ms)));
        }
    }

    #[test]
    fn clipboard_unavailable_fails_without_retrying_dispatch() {
        let reads = Cell::new(0);
        let result = UinputInjector::poll_paste_observation_with(
            7,
            "injected",
            || Some(7),
            || {
                reads.set(reads.get() + 1);
                None
            },
            Instant::now,
            |_| panic!("unavailable clipboard must fail immediately"),
        );

        assert!(!result);
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn stale_transaction_aborts_before_observation_completion() {
        let active = Cell::new(Some(10));
        let elapsed = Cell::new(Duration::ZERO);
        let result = UinputInjector::poll_paste_observation_with(
            10,
            "injected",
            || active.get(),
            || Some("injected".to_string()),
            || Instant::now() + elapsed.get(),
            |duration| {
                elapsed.set(elapsed.get() + duration);
                active.set(Some(11));
            },
        );

        assert!(!result);
    }

    #[test]
    fn slow_paste_is_observed_once_without_duplicate_completion() {
        let elapsed = Cell::new(Duration::ZERO);
        let reads = Cell::new(0);
        let result = UinputInjector::poll_paste_observation_with(
            12,
            "injected",
            || Some(12),
            || {
                reads.set(reads.get() + 1);
                Some("injected".to_string())
            },
            || Instant::now() + elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
        );

        assert!(result);
        assert!(reads.get() > 1);

        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user".to_string()), "injected", 1);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert!(state.complete_restore(id));
        assert!(!state.complete_restore(id));
    }
}

#[cfg(test)]
mod restorer_tests {
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
                    worker_restored.lock().unwrap().push(text.to_string());
                    true
                },
            );
        });

        restorer.shutdown_and_join().unwrap();
        let restore_count_after_join = restored.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(20));

        assert_eq!(restored.lock().unwrap().len(), restore_count_after_join);
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
                    burst.begin_injection(
                        Some("old user clipboard".to_string()),
                        "injected text",
                        1,
                    );
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
                || worker_reads.lock().unwrap().pop_front().flatten(),
                |text| {
                    worker_restored.lock().unwrap().push(text.to_string());
                    true
                },
            );
        });

        for _ in 0..100 {
            if reads.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        {
            let mut inner = state.inner.lock().unwrap();
            inner.shutdown = true;
        }
        state.cv.notify_all();
        handle.join().unwrap();

        assert!(restored.lock().unwrap().is_empty());
    }
}

#[cfg(test)]
mod clipboard_backup_tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn empty_clipboard_is_backed_up_as_empty() {
        assert_eq!(classify_clipboard_types(&[]), ClipboardKind::Empty);
    }

    #[test]
    fn any_plain_text_offer_uses_the_text_path() {
        assert_eq!(
            classify_clipboard_types(&types(&["text/html", "text/plain;charset=utf-8"])),
            ClipboardKind::Text
        );
        assert_eq!(
            classify_clipboard_types(&types(&["UTF8_STRING"])),
            ClipboardKind::Text
        );
    }

    #[test]
    fn image_only_clipboard_keeps_its_own_type_instead_of_becoming_text() {
        assert_eq!(
            classify_clipboard_types(&types(&["image/png"])),
            ClipboardKind::Other("image/png".to_string())
        );
        assert_eq!(
            classify_clipboard_types(&types(&["text/uri-list", "x-special/gnome-copied-files"])),
            ClipboardKind::Other("text/uri-list".to_string())
        );
    }
}
