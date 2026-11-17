// SPDX-License-Identifier: MIT
//! Ordering of clipboard-based key dispatch, payload transfer and cleanup.
//! Transfer completion is not acknowledgment of the target's document mutation.
use std::io;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PasteCompletion {
    PayloadServed {
        cleanup_ok: bool,
    },
    /// Some keys may already have been delivered. Never retry or restore here.
    Indeterminate,
}

pub(crate) fn paste_with_transfer(
    dispatch: impl FnOnce() -> io::Result<()>,
    wait_for_transfer: impl FnOnce() -> io::Result<()>,
    restore: impl FnOnce() -> bool,
) -> PasteCompletion {
    if dispatch().and_then(|()| wait_for_transfer()).is_err() {
        return PasteCompletion::Indeterminate;
    }
    PasteCompletion::PayloadServed {
        cleanup_ok: restore(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn delayed_reader_receives_vietnamese_before_filename_is_restored() {
        // Virtual time: this tests orchestration, not Wayland or an actual editor.
        for delay_ms in [21, 50, 200, 749] {
            let clipboard = RefCell::new("tự".to_owned());
            let observed = RefCell::new(String::new());
            let elapsed = Cell::new(0);
            let dispatches = Cell::new(0);
            let result = paste_with_transfer(
                || {
                    dispatches.set(dispatches.get() + 1);
                    Ok(())
                },
                || {
                    // Unlike a self-read, only the simulated recipient request
                    // supplies the transfer-completion event.
                    elapsed.set(delay_ms);
                    *observed.borrow_mut() = clipboard.borrow().clone();
                    Ok(())
                },
                || {
                    assert_eq!(elapsed.get(), delay_ms);
                    *clipboard.borrow_mut() = "COPIED_VIDEO_1080p.mp4".into();
                    true
                },
            );
            assert_eq!(result, PasteCompletion::PayloadServed { cleanup_ok: true });
            assert_eq!(&*observed.borrow(), "tự");
            assert_eq!(&*clipboard.borrow(), "COPIED_VIDEO_1080p.mp4");
            assert_eq!(dispatches.get(), 1);
        }
    }

    #[test]
    fn no_transfer_does_not_restore_filename_or_retry_paste() {
        let restored = Cell::new(false);
        let dispatches = Cell::new(0);
        let result = paste_with_transfer(
            || {
                dispatches.set(dispatches.get() + 1);
                Ok(())
            },
            || Err(io::ErrorKind::TimedOut.into()),
            || {
                restored.set(true);
                true
            },
        );
        assert_eq!(result, PasteCompletion::Indeterminate);
        assert!(!restored.get());
        assert_eq!(dispatches.get(), 1);
    }

    #[test]
    fn partial_dispatch_failure_does_not_wait_restore_or_retry() {
        let result = paste_with_transfer(
            || Err(io::ErrorKind::BrokenPipe.into()),
            || panic!("must not claim a failed dispatch was transferred"),
            || panic!("pending paste could still read the selection"),
        );
        assert_eq!(result, PasteCompletion::Indeterminate);
    }

    #[test]
    fn cleanup_failure_does_not_turn_a_served_payload_into_retriable_edit_failure() {
        assert_eq!(
            paste_with_transfer(|| Ok(()), || Ok(()), || false),
            PasteCompletion::PayloadServed { cleanup_ok: false }
        );
    }

    #[test]
    fn focus_loss_or_cancelled_source_never_restores_over_an_external_copy() {
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::BrokenPipe] {
            assert_eq!(
                paste_with_transfer(
                    || Ok(()),
                    || Err(kind.into()),
                    || panic!("must not restore")
                ),
                PasteCompletion::Indeterminate
            );
        }
    }
}
