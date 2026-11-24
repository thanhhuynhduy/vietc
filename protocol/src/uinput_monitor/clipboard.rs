// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::clipboard_backup::{
    classify_clipboard_types, ClipboardKind, NonTextClipboard, MAX_BINARY_CLIPBOARD_BYTES,
};
use super::replacement_transaction::{ClipboardAction, ClipboardBurstState, ReplacementPhase};
use super::UinputInjector;

const CLIPBOARD_READY_TIMEOUT: Duration = Duration::from_millis(350);
const CLIPBOARD_READY_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PASTE_OBSERVATION_DELAY: Duration = Duration::from_millis(20);
const CLIPBOARD_TRACE_LIMIT: usize = 128;
static CLIPBOARD_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "wayland-protocol")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreOutcome {
    Restored,
    Superseded,
    Failed,
}

pub(super) fn trace_clipboard(sequence_id: u64, phase: &str, detail: &str) {
    let trace_index = CLIPBOARD_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    if trace_index < CLIPBOARD_TRACE_LIMIT {
        eprintln!(
            "[dboard] clipboard trace={} sequence_id={} phase={} {}",
            trace_index, sequence_id, phase, detail
        );
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
            let mut inner = self
                .state
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        cmd: std::process::Command,
        timeout: Duration,
    ) -> Option<std::process::Output> {
        crate::clipboard_command::run_capture(cmd, timeout, MAX_BINARY_CLIPBOARD_BYTES)
    }

    fn read_clipboard() -> Option<String> {
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !is_wayland {
            return None;
        }
        let mut cmd = Self::user_cmd("wl-paste").ok()?;
        cmd.args(["-n"]);
        let output = Self::run_cmd_with_timeout(cmd, Duration::from_millis(250))?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn clipboard_types() -> Option<Vec<String>> {
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
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

    fn backup_non_text_mimes(mimes: Vec<String>) -> Option<NonTextClipboard> {
        let mut entries = Vec::with_capacity(mimes.len());
        let mut total_bytes = 0usize;
        for (idx, mime) in mimes.into_iter().enumerate() {
            let Ok(mut cmd) = Self::user_cmd("wl-paste") else {
                break;
            };
            // Always pass `--no-newline` so `wl-paste` never appends a spurious `\n`
            // to `text/html`, `text/plain`, or `text/uri-list` payloads.
            cmd.args(["--no-newline", "--type", &mime]);
            let Some(output) = Self::run_cmd_with_timeout(cmd, Duration::from_millis(500)) else {
                // If the primary rich MIME (e.g. large `image/png` or `text/html`)
                // cannot be backed up completely, abort the backup rather than
                // silently overwriting the clipboard with only a plain-text fallback.
                if idx == 0 {
                    return None;
                }
                continue;
            };
            if !output.status.success()
                || total_bytes.saturating_add(output.stdout.len()) > MAX_BINARY_CLIPBOARD_BYTES
            {
                if idx == 0 {
                    return None;
                }
                continue;
            }
            total_bytes += output.stdout.len();
            entries.push((mime, output.stdout));
        }
        (!entries.is_empty()).then_some(NonTextClipboard::MultiMime(entries))
    }

    fn backup_non_text_clipboard() -> Option<NonTextClipboard> {
        let types = Self::clipboard_types()?;
        match classify_clipboard_types(&types) {
            ClipboardKind::Text => None,
            ClipboardKind::Empty => Some(NonTextClipboard::Empty),
            ClipboardKind::Rich(mimes) => Self::backup_non_text_mimes(mimes),
        }
    }

    fn restore_non_text_clipboard(backup: &NonTextClipboard) -> bool {
        match backup {
            NonTextClipboard::Empty => Self::user_cmd("wl-copy").ok().is_some_and(|mut cmd| {
                cmd.arg("--clear");
                crate::clipboard_command::run_copy(cmd, &[], Duration::from_millis(200))
            }),
            NonTextClipboard::MultiMime(entries) => {
                if super::clipboard_backup::restore_multi_mime_native_first(
                    entries,
                    Duration::from_millis(250),
                ) {
                    return true;
                }
                let Some((mime, bytes)) = entries.first() else {
                    return false;
                };
                let Ok(mut cmd) = Self::user_cmd("wl-copy") else {
                    return false;
                };
                cmd.args(["--type", mime]);
                let timeout = if bytes.len() >= 1024 * 1024 {
                    Duration::from_millis(1500)
                } else {
                    Duration::from_millis(250)
                };
                crate::clipboard_command::run_copy(cmd, bytes, timeout)
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
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
            #[cfg(feature = "wayland-protocol")]
            return self.paste_via_wayland_clipboard(text, backspaces, focus_generation);
            #[cfg(not(feature = "wayland-protocol"))]
            {
                eprintln!("[dboard] Wayland Unicode injection requires the wayland-protocol feature; no keys sent");
                return false;
            }
        }
        let t_total = std::time::Instant::now();
        let _clipboard_io = self
            .clip
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let needs_snapshot = {
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=clipboard-write");
            st.burst.reset();
            return false;
        }

        if !Self::wait_for_clipboard_value(text) {
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::ClipboardWritten);
            debug_assert!(transitioned);
        }
        trace_clipboard(sequence_id, "clipboard_written", "state=verified");

        if backspaces > 0 {
            if self.send_backspaces(backspaces).is_err() {
                let mut st = self
                    .clip
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=paste-dispatch");
            st.burst.reset();
            return false;
        }
        {
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::PasteDispatched);
            debug_assert!(transitioned);
            st.restore_due = Some(Instant::now() + CLIPBOARD_READY_TIMEOUT);
        }
        trace_clipboard(sequence_id, "paste_dispatched", "state=sent");

        if !Self::wait_for_target_consumption(&self.clip, sequence_id, text, focus_generation) {
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let mut st = self
                .clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            self.clip
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .burst
                .reset();
            let elapsed = t_total.elapsed().as_millis();
            if elapsed > 20 {
                eprintln!(
                    "[dboard] clipboard sequence_id={} elapsed_ms={}",
                    sequence_id, elapsed
                );
            }
            return restored;
        }

        let success = match action {
            ClipboardAction::Restore(saved) => {
                trace_clipboard(sequence_id, "restore", "state=writing");
                if !Self::copy_to_clipboard(&saved) {
                    self.clip
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .burst
                        .reset();
                    trace_clipboard(sequence_id, "restore", "state=write-failed");
                    false
                } else if !Self::wait_for_clipboard_value(&saved) {
                    self.clip
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .burst
                        .reset();
                    trace_clipboard(sequence_id, "verified-read-back", "state=failed");
                    false
                } else {
                    self.clip
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
                "[dboard] clipboard sequence_id={} elapsed_ms={}",
                sequence_id, elapsed
            );
        }
        success
    }

    #[cfg(feature = "wayland-protocol")]
    pub(super) fn wayland_paste_is_uncertain(&self) -> bool {
        self.uncertain_focus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    #[cfg(feature = "wayland-protocol")]
    fn snapshot_wayland_clipboard() -> Option<WaylandClipboardBackup> {
        let types = Self::clipboard_types()?;
        match classify_clipboard_types(&types) {
            ClipboardKind::Empty => Some(WaylandClipboardBackup::NonText(NonTextClipboard::Empty)),
            ClipboardKind::Text => Self::read_clipboard().map(WaylandClipboardBackup::Text),
            ClipboardKind::Rich(mimes) => {
                Self::backup_non_text_mimes(mimes).map(WaylandClipboardBackup::NonText)
            }
        }
    }

    #[cfg(feature = "wayland-protocol")]
    fn restore_wayland_clipboard(backup: &WaylandClipboardBackup) -> bool {
        Self::restore_wayland_clipboard_guarded(backup, || true) == RestoreOutcome::Restored
    }

    #[cfg(feature = "wayland-protocol")]
    fn restore_wayland_clipboard_guarded(
        backup: &WaylandClipboardBackup,
        should_proceed: impl Fn() -> bool,
    ) -> RestoreOutcome {
        use crate::clipboard_command::run_copy;
        const RESTORE_STEP_TIMEOUT: Duration = Duration::from_millis(250);
        if !should_proceed() {
            return RestoreOutcome::Superseded;
        }
        let step_result = |ok: bool| {
            if ok {
                RestoreOutcome::Restored
            } else if !should_proceed() {
                RestoreOutcome::Superseded
            } else {
                RestoreOutcome::Failed
            }
        };
        match backup {
            WaylandClipboardBackup::Text(text) => {
                let Ok(mut command) = Self::user_cmd("wl-copy") else {
                    return RestoreOutcome::Failed;
                };
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                command.args(["--type", "text/plain;charset=utf-8"]);
                if !run_copy(command, text.as_bytes(), RESTORE_STEP_TIMEOUT) {
                    return step_result(false);
                }
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                step_result(Self::wait_for_clipboard_value(text))
            }
            WaylandClipboardBackup::NonText(NonTextClipboard::Empty) => {
                let Ok(mut command) = Self::user_cmd("wl-copy") else {
                    return RestoreOutcome::Failed;
                };
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                command.arg("--clear");
                if !run_copy(command, &[], RESTORE_STEP_TIMEOUT) {
                    return step_result(false);
                }
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                step_result(Self::clipboard_types().is_some_and(|types| types.is_empty()))
            }
            WaylandClipboardBackup::NonText(NonTextClipboard::MultiMime(entries)) => {
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                if crate::wayland_clipboard_restore::restore_multi_mime_native(
                    entries.clone(),
                    RESTORE_STEP_TIMEOUT,
                )
                .is_ok()
                {
                    return RestoreOutcome::Restored;
                }
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                let Some((mime, bytes)) = entries.first() else {
                    return RestoreOutcome::Failed;
                };
                let Ok(mut command) = Self::user_cmd("wl-copy") else {
                    return RestoreOutcome::Failed;
                };
                command.args(["--type", mime]);
                let step_timeout = if bytes.len() >= 1024 * 1024 {
                    Duration::from_millis(1500)
                } else {
                    RESTORE_STEP_TIMEOUT
                };
                if !run_copy(command, bytes, step_timeout) {
                    return step_result(false);
                }
                if !should_proceed() {
                    return RestoreOutcome::Superseded;
                }
                let Ok(mut verify) = Self::user_cmd("wl-paste") else {
                    return RestoreOutcome::Failed;
                };
                verify.args(["--no-newline", "--type", mime]);
                step_result(
                    Self::run_cmd_with_timeout(verify, step_timeout)
                        .is_some_and(|output| output.status.success() && output.stdout == *bytes),
                )
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

        // Cancel any pending background Wayland clipboard restore so a burst
        // paste reuses the retained user snapshot instead of racing with it.
        self.wayland_restore_gen.fetch_add(1, Ordering::AcqRel);
        let _clipboard_io = self
            .clip
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut retained = self
            .wayland_paste
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut retired = self
            .retired_wayland_offers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Reap only workers that already finished. A stalled recipient must not
        // make the grabbed-keyboard capture thread join a five-second drain.
        retired.retain(|offer| !offer.is_finished());
        if retired.len() + 2 > 8 {
            *self
                .uncertain_focus
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(focus_generation);
            eprintln!("[dboard] Clipboard transfer backlog; native input only until focus changes");
            return false;
        }
        let mut retire = |offer: ClipboardOffer| {
            offer.request_retire();
            retired.push(offer);
        };
        if self.wayland_paste_is_uncertain() {
            eprintln!("[dboard] Unicode replacement suspended after uncertain paste; change input field before resuming");
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
                *self
                    .uncertain_focus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(focus_generation);
                eprintln!("[dboard] Clipboard transport unavailable; saved snapshot retained, no keys sent");
                return false;
            }
            _ => {
                let Some(backup) = Self::snapshot_wayland_clipboard() else {
                    eprintln!("[dboard] Clipboard snapshot unavailable; no replacement keys sent");
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
                    "[dboard] Wayland clipboard publication failed before key dispatch: {error}"
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
                "[dboard] Clipboard publication uncertain; snapshot retained, no keys sent: {error}"
            );
            *self
                .uncertain_focus
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(focus_generation);
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
                if self.current_focus_generation() != focus_generation || !offer.owns_selection() {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                Ok(())
            },
            || true,
        );
        match result {
            PasteCompletion::PayloadServed { .. } => {
                // Keep the offer & user snapshot retained so a burst paste reuses it,
                // and run the 100ms clipboard-manager grace + restore off `evdev_loop`.
                let expected_gen = self.wayland_restore_gen.fetch_add(1, Ordering::AcqRel) + 1;
                *retained = Some(RetainedWaylandPaste { offer, backup });
                let clip = Arc::clone(&self.clip);
                let paste_slot = Arc::clone(&self.wayland_paste);
                let retired_slot = Arc::clone(&self.retired_wayland_offers);
                let restore_gen = Arc::clone(&self.wayland_restore_gen);
                let _ = std::thread::Builder::new()
                    .name("dboard-clip-restore".into())
                    .spawn(move || {
                        for delay_ms in [100u64, 250u64] {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                            if restore_gen.load(Ordering::Acquire) != expected_gen {
                                return;
                            }
                            let Ok(_io) = clip.io_lock.try_lock() else {
                                // A newer paste on `evdev_loop` is already active and superseded this restore.
                                return;
                            };
                            if restore_gen.load(Ordering::Acquire) != expected_gen {
                                return;
                            }
                            let entry = {
                                let mut slot = paste_slot.lock().unwrap_or_else(|p| p.into_inner());
                                let Some(entry) = slot.take() else { return };
                                entry
                            };
                            let should_proceed = || {
                                restore_gen.load(Ordering::Acquire) == expected_gen
                                    && entry.offer.ownership() == Ownership::Owned
                            };
                            let outcome = match entry.offer.ownership() {
                                Ownership::Owned => Self::restore_wayland_clipboard_guarded(
                                    &entry.backup,
                                    should_proceed,
                                ),
                                Ownership::Replaced => RestoreOutcome::Restored,
                                Ownership::Unavailable => RestoreOutcome::Failed,
                            };
                            match outcome {
                                RestoreOutcome::Restored => {
                                    entry.offer.request_retire();
                                    let mut retired =
                                        retired_slot.lock().unwrap_or_else(|p| p.into_inner());
                                    retired.retain(|offer| !offer.is_finished());
                                    retired.push(entry.offer);
                                    return;
                                }
                                RestoreOutcome::Superseded => {
                                    // A newer paste superseded this restore while we held `io_lock`:
                                    // put `entry` back so the newer paste reuses the original backup.
                                    let mut slot =
                                        paste_slot.lock().unwrap_or_else(|p| p.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(entry);
                                    }
                                    return;
                                }
                                RestoreOutcome::Failed => {
                                    let mut slot =
                                        paste_slot.lock().unwrap_or_else(|p| p.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(entry);
                                    } else {
                                        return;
                                    }
                                }
                            }
                        }
                    });
                true
            }
            PasteCompletion::Indeterminate => {
                eprintln!("[dboard] Wayland paste indeterminate; keeping payload available, no clipboard restore or retry");
                *self
                    .uncertain_focus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(focus_generation);
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
                let state = state
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
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

        let mut inner = state
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !is_wayland {
            return false;
        }
        let Ok(mut cmd) = Self::user_cmd("wl-copy") else {
            return false;
        };
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(mut child) => {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(s.as_bytes());
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                true
            }
            Err(e) => {
                eprintln!("[dboard] copy_to_clipboard: wl-copy spawn failed: {}", e);
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
        let mut st = state
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if st.shutdown {
                return;
            }
            match st.restore_due {
                None => {
                    st = state
                        .cv
                        .wait(st)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                Some(due) => {
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    let (guard, _) = state
                        .cv
                        .wait_timeout(st, due - now)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    st = guard;
                }
            }
        }
        st.restore_due = None;
        drop(st);

        let _clipboard_io = state
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current_clipboard = read_clipboard();
        let action = {
            let mut st = state
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if st.shutdown {
                st.burst.reset();
                return;
            }
            st.burst.finish_restore(current_clipboard.as_deref())
        };

        if let ClipboardAction::Restore(restored) = action {
            let current_clipboard = read_clipboard();
            let should_restore = {
                let mut st = state
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                let st = state
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                st.burst.transaction_id()
            };
            let restored_ok = copy_to_clipboard(&restored);
            let mut st = state
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
#[path = "clipboard_restorer_tests.rs"]
mod restorer_tests;
