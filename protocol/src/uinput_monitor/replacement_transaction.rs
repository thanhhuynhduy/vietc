// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_CLIPBOARD_SEQUENCE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplacementPhase {
    Prepared,
    ClipboardWritten,
    PasteDispatched,
    TargetObserved,
    Applied,
    ClipboardRestored,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClipboardAction {
    WriteInjected(String),
    Restore(String),
    SkipRestore,
}

struct ClipboardTransaction {
    id: u64,
    focus_generation: u64,
    saved_clipboard: Option<String>,
    injected_clipboard: String,
    phase: ReplacementPhase,
    started: Instant,
}

#[derive(Default)]
pub(super) struct ClipboardBurstState {
    transaction: Option<ClipboardTransaction>,
    last_unrestored_injected: Option<String>,
}

impl ClipboardBurstState {
    pub(super) fn begin_injection(
        &mut self,
        current_clipboard: Option<String>,
        injected_text: &str,
        focus_generation: u64,
    ) -> ClipboardAction {
        let filtered_clipboard = current_clipboard
            .filter(|clip| self.last_unrestored_injected.as_deref() != Some(clip.as_str()));
        let saved_clipboard = self
            .transaction
            .as_ref()
            .and_then(|transaction| transaction.saved_clipboard.clone())
            .or(filtered_clipboard);
        self.last_unrestored_injected = None;
        self.transaction = Some(ClipboardTransaction {
            id: NEXT_CLIPBOARD_SEQUENCE_ID.fetch_add(1, Ordering::Relaxed),
            focus_generation,
            saved_clipboard,
            injected_clipboard: injected_text.to_string(),
            phase: ReplacementPhase::Prepared,
            started: Instant::now(),
        });
        ClipboardAction::WriteInjected(injected_text.to_string())
    }

    pub(super) fn transition(&mut self, transaction_id: u64, phase: ReplacementPhase) -> bool {
        let Some(transaction) = self.transaction.as_mut() else {
            return false;
        };
        if transaction.id != transaction_id {
            return false;
        }

        let valid = matches!(
            (transaction.phase, phase),
            (
                ReplacementPhase::Prepared,
                ReplacementPhase::ClipboardWritten
            ) | (
                ReplacementPhase::ClipboardWritten,
                ReplacementPhase::PasteDispatched
            ) | (
                ReplacementPhase::PasteDispatched,
                ReplacementPhase::TargetObserved
            ) | (ReplacementPhase::TargetObserved, ReplacementPhase::Applied)
                | (
                    ReplacementPhase::Applied,
                    ReplacementPhase::ClipboardRestored
                )
                | (_, ReplacementPhase::Failed)
        );
        if valid {
            transaction.phase = phase;
        }
        valid
    }

    pub(super) fn mark_failed(&mut self, transaction_id: u64) {
        let _ = self.transition(transaction_id, ReplacementPhase::Failed);
    }

    pub(super) fn finish_restore(&mut self, current_clipboard: Option<&str>) -> ClipboardAction {
        let Some(transaction) = self.transaction.as_ref() else {
            return ClipboardAction::SkipRestore;
        };
        if transaction.phase != ReplacementPhase::Applied {
            return ClipboardAction::SkipRestore;
        }

        let still_contains_injected =
            current_clipboard == Some(transaction.injected_clipboard.as_str());
        if !still_contains_injected {
            self.reset();
            return ClipboardAction::SkipRestore;
        }

        match transaction.saved_clipboard.clone() {
            Some(saved) => ClipboardAction::Restore(saved),
            None => {
                let injected = transaction.injected_clipboard.clone();
                self.reset();
                self.last_unrestored_injected = Some(injected);
                ClipboardAction::SkipRestore
            }
        }
    }

    pub(super) fn forget_unrestored(&mut self) {
        self.last_unrestored_injected = None;
    }

    pub(super) fn complete_restore(&mut self, transaction_id: u64) -> bool {
        if !self.transition(transaction_id, ReplacementPhase::ClipboardRestored) {
            return false;
        }
        self.reset();
        self.last_unrestored_injected = None;
        true
    }

    pub(super) fn is_active(&self) -> bool {
        self.transaction.is_some()
    }

    pub(super) fn transaction_id(&self) -> Option<u64> {
        self.transaction.as_ref().map(|transaction| transaction.id)
    }

    pub(super) fn matches(&self, transaction_id: u64, focus_generation: u64) -> bool {
        self.transaction.as_ref().is_some_and(|transaction| {
            transaction.id == transaction_id && transaction.focus_generation == focus_generation
        })
    }

    pub(super) fn cancel_for_focus_change(&mut self, focus_generation: u64) -> bool {
        let should_cancel = self.transaction.as_ref().is_some_and(|transaction| {
            transaction.focus_generation != focus_generation
                && transaction.phase != ReplacementPhase::Applied
                && transaction.phase != ReplacementPhase::ClipboardRestored
        });
        if should_cancel {
            self.transaction = None;
        }
        should_cancel
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> Option<ReplacementPhase> {
        self.transaction
            .as_ref()
            .map(|transaction| transaction.phase)
    }

    pub(super) fn elapsed(&self) -> Option<Duration> {
        self.transaction
            .as_ref()
            .map(|transaction| transaction.started.elapsed())
    }

    pub(super) fn reset(&mut self) {
        self.transaction = None;
    }
}

#[cfg(test)]
mod clipboard_burst_tests {
    use super::*;

    #[test]
    fn restored_burst_resets_snapshot_for_next_user_clipboard() {
        let mut state = ClipboardBurstState::default();

        assert_eq!(
            state.begin_injection(Some("clipboard A".to_string()), "vietc A", 1),
            ClipboardAction::WriteInjected("vietc A".to_string())
        );
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("vietc A")),
            ClipboardAction::Restore("clipboard A".to_string())
        );
        assert!(state.complete_restore(id));

        assert_eq!(
            state.begin_injection(Some("clipboard B".to_string()), "vietc B", 1),
            ClipboardAction::WriteInjected("vietc B".to_string())
        );
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("vietc B")),
            ClipboardAction::Restore("clipboard B".to_string())
        );
    }

    #[test]
    fn user_clipboard_change_during_wait_is_not_overwritten() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(Some("clipboard A".to_string()), "vietc A", 1);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));

        assert_eq!(
            state.finish_restore(Some("clipboard B")),
            ClipboardAction::SkipRestore
        );
        assert!(!state.is_active());
    }

    #[test]
    fn clipboard_is_not_restored_before_transaction_is_applied() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 1);
        let id = state.transaction_id().unwrap();
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::SkipRestore
        );
        assert!(state.is_active());

        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::Restore("user clipboard".to_string())
        );
    }

    #[test]
    pub(super) fn transaction_ids_are_monotonic_and_stale_transitions_are_rejected() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(None, "đ", 1);
        let id = state.transaction_id().unwrap();
        let first = state.transaction_id().unwrap();
        assert!(!state.transition(id, ReplacementPhase::Applied));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        state.mark_failed(id);
        assert_eq!(state.phase(), Some(ReplacementPhase::Failed));
        state.reset();

        state.begin_injection(None, "đường", 1);
        let second = state.transaction_id().unwrap();
        assert!(second > first);
        assert!(!state.transition(first, ReplacementPhase::ClipboardWritten));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        assert!(!state.complete_restore(first));
        assert_eq!(state.transaction_id(), Some(second));
    }

    #[test]
    fn focus_change_cancels_waiting_transaction_and_rejects_stale_completion() {
        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 7);
        let stale_id = state.transaction_id().unwrap();
        assert!(state.transition(stale_id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(stale_id, ReplacementPhase::PasteDispatched));

        assert!(state.cancel_for_focus_change(8));
        assert!(!state.transition(stale_id, ReplacementPhase::TargetObserved));
        assert!(!state.complete_restore(stale_id));
        assert!(!state.is_active());

        state.begin_injection(Some("new clipboard".to_string()), "Việt", 8);
        let current_id = state.transaction_id().unwrap();
        assert!(current_id > stale_id);
        assert!(!state.transition(stale_id, ReplacementPhase::Applied));
        assert!(state.matches(current_id, 8));
        assert!(!state.matches(current_id, 7));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
    }

    #[test]
    fn applied_transaction_is_not_cancelled_by_late_focus_change() {
        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 11);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));

        assert!(!state.cancel_for_focus_change(12));
        assert!(state.matches(id, 11));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::Restore("user clipboard".to_string())
        );
    }

    #[test]
    fn each_unicode_command_finishes_its_clipboard_transaction_before_returning() {
        let mut state = ClipboardBurstState::default();

        for injected in ["đ", "đư", "đườ", "đường"] {
            state.begin_injection(Some("user clipboard".to_string()), injected, 1);
            let id = state.transaction_id().unwrap();
            assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
            assert!(state.transition(id, ReplacementPhase::PasteDispatched));
            assert!(state.transition(id, ReplacementPhase::TargetObserved));
            assert!(state.transition(id, ReplacementPhase::Applied));
            assert_eq!(
                state.finish_restore(Some(injected)),
                ClipboardAction::Restore("user clipboard".to_string())
            );
            assert!(state.complete_restore(id));
            assert!(!state.is_active());
        }
    }
}
