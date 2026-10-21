use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use vietc_protocol::inject::InjectResult;
use vietc_protocol::KeyInjector;

use crate::commands::{OutputCommand, TransactionOutcome};
use crate::display;
use crate::log::log_info;

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

fn injection_trace_enabled() -> bool {
    std::env::var_os("VIETC_INJECTION_TRACE").is_some()
}

pub fn execute_commands(
    injector: &dyn KeyInjector,
    commands: &[OutputCommand],
    visible_revision: &mut u64,
) -> TransactionOutcome {
    if commands.is_empty() {
        return TransactionOutcome::Applied {
            visible_revision: *visible_revision,
        };
    }

    if commands.len() == 1 {
        if let OutputCommand::Replace(replacement) = &commands[0] {
            let transaction_id = NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let trace = injection_trace_enabled();

            if trace {
                log_info(&format!(
                    "[vietc] injection transaction start: id={} delete={} insert_scalars={} insert_bytes={} expected_revision={} actual_revision={} generation={} reason={:?}",
                    transaction_id,
                    replacement.delete_before_cursor,
                    replacement.insert.chars().count(),
                    replacement.insert.len(),
                    replacement.expected_visible_revision,
                    *visible_revision,
                    replacement.transform_generation,
                    replacement.reason,
                ));
            }

            if replacement.expected_visible_revision != *visible_revision {
                if trace {
                    log_info(&format!(
                        "[vietc] injection transaction stale: id={} elapsed_us={} expected_revision={} actual_revision={}",
                        transaction_id,
                        started.elapsed().as_micros(),
                        replacement.expected_visible_revision,
                        *visible_revision,
                    ));
                }
                return TransactionOutcome::Stale {
                    expected_visible_revision: replacement.expected_visible_revision,
                    actual_visible_revision: *visible_revision,
                };
            }

            let replacement_result =
                injector.inject_replacement(replacement.delete_before_cursor, &replacement.insert);
            let replacement_elapsed = started.elapsed();
            if replacement_result != InjectResult::Success {
                if trace {
                    log_info(&format!(
                        "[vietc] injection transaction failed: id={} phase=replace elapsed_us={} result={}",
                        transaction_id,
                        replacement_elapsed.as_micros(),
                        replacement_result,
                    ));
                }
                return TransactionOutcome::Failed;
            }

            let flush_result = injector.flush();
            if flush_result != InjectResult::Success {
                if trace {
                    log_info(&format!(
                        "[vietc] injection transaction failed: id={} phase=flush replace_us={} elapsed_us={} result={}",
                        transaction_id,
                        replacement_elapsed.as_micros(),
                        started.elapsed().as_micros(),
                        flush_result,
                    ));
                }
                return TransactionOutcome::Failed;
            }

            *visible_revision += 1;
            if trace {
                log_info(&format!(
                    "[vietc] injection transaction applied: id={} replace_us={} elapsed_us={} visible_revision={}",
                    transaction_id,
                    replacement_elapsed.as_micros(),
                    started.elapsed().as_micros(),
                    *visible_revision,
                ));
            }
            return TransactionOutcome::Applied {
                visible_revision: *visible_revision,
            };
        }
    }

    let mut pending_backspaces = 0;
    let mut pending_text = String::new();
    for command in commands {
        match command {
            OutputCommand::Backspace(count) => pending_backspaces += count,
            OutputCommand::Type(text) => pending_text.push_str(text),
            OutputCommand::Replace(_) => return TransactionOutcome::Failed,
        }
    }

    if injector.inject_replacement(pending_backspaces, &pending_text) != InjectResult::Success
        || injector.flush() != InjectResult::Success
    {
        return TransactionOutcome::Failed;
    }

    TransactionOutcome::Applied {
        visible_revision: *visible_revision,
    }
}

pub fn create_injector(
    _display: display::DisplayServer,
) -> Result<Box<dyn KeyInjector>, Box<dyn std::error::Error>> {
    match vietc_protocol::uinput_monitor::UinputInjector::new("vietc") {
        Ok(injector) => {
            log_info("[vietc] Using uinput injection");
            return Ok(Box::new(injector));
        }
        Err(e) => {
            log_info(&format!("[vietc] uinput not available: {}", e));
        }
    }

    if vietc_protocol::uinput_client::UinputClient::is_available() {
        log_info("[vietc] Using uinputd socket injection");
        return Ok(Box::new(vietc_protocol::uinput_client::UinputClient));
    }

    #[cfg(feature = "x11")]
    if std::env::var("DISPLAY").is_ok() {
        // On Wayland-with-XWayland, XTEST injection is rootless (like ibus-x11).
        match vietc_protocol::x11_inject::X11Injector::new() {
            Ok(injector) => {
                log_info("[vietc] Using X11 injection (fallback)");
                return Ok(Box::new(injector));
            }
            Err(e) => {
                log_info(&format!("[vietc] X11 not available: {}", e));
            }
        }
    }

    Err("No injection backend available".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use vietc_protocol::inject::InjectResult;

    struct ScriptedInjector {
        replacement_result: InjectResult,
        flush_result: InjectResult,
        replacement_calls: Cell<usize>,
    }

    impl KeyInjector for ScriptedInjector {
        fn send_backspace(&self) -> InjectResult {
            InjectResult::Success
        }

        fn send_char(&self, _ch: char) -> InjectResult {
            InjectResult::Success
        }

        fn send_string(&self, _text: &str) -> InjectResult {
            InjectResult::Success
        }

        fn inject_replacement(&self, _backspaces: usize, _text: &str) -> InjectResult {
            self.replacement_calls.set(self.replacement_calls.get() + 1);
            self.replacement_result
        }

        fn flush(&self) -> InjectResult {
            self.flush_result
        }
    }

    fn replacement(expected_revision: u64) -> OutputCommand {
        OutputCommand::Replace(vietc_engine::Replacement {
            version: vietc_engine::Replacement::VERSION,
            delete_before_cursor: 1,
            insert: "á".to_string(),
            expected_visible_revision: expected_revision,
            transform_generation: 1,
            reason: vietc_engine::ReplacementReason::ApplyTransform,
        })
    }

    #[test]
    fn semantic_replacement_advances_revision_only_after_success() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Success,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let mut revision = 0;

        assert_eq!(
            execute_commands(&injector, &[replacement(0)], &mut revision),
            TransactionOutcome::Applied {
                visible_revision: 1
            }
        );
        assert_eq!(revision, 1);
        assert_eq!(injector.replacement_calls.get(), 1);
    }

    #[test]
    fn stale_replacement_is_rejected_without_injection() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Success,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let mut revision = 2;

        assert_eq!(
            execute_commands(&injector, &[replacement(1)], &mut revision),
            TransactionOutcome::Stale {
                expected_visible_revision: 1,
                actual_visible_revision: 2,
            }
        );
        assert_eq!(revision, 2);
        assert_eq!(injector.replacement_calls.get(), 0);
    }

    #[test]
    fn failed_semantic_replacement_does_not_advance_revision() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Failed,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let mut revision = 0;

        assert_eq!(
            execute_commands(&injector, &[replacement(0)], &mut revision),
            TransactionOutcome::Failed
        );
        assert_eq!(revision, 0);
        assert_eq!(injector.replacement_calls.get(), 1);
    }

    #[test]
    fn execute_commands_propagates_replacement_failure() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Failed,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let commands = [OutputCommand::Type("ế".to_string())];

        let mut revision = 0;
        assert_eq!(
            execute_commands(&injector, &commands, &mut revision),
            TransactionOutcome::Failed
        );
        assert_eq!(injector.replacement_calls.get(), 1);
    }

    #[test]
    fn execute_commands_propagates_flush_failure() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Success,
            flush_result: InjectResult::Failed,
            replacement_calls: Cell::new(0),
        };
        let commands = [OutputCommand::Type("a".to_string())];

        let mut revision = 0;
        assert_eq!(
            execute_commands(&injector, &commands, &mut revision),
            TransactionOutcome::Failed
        );
    }
    #[derive(Clone, Copy, Debug)]
    enum DeliveryMode {
        Grabbed,
        Forwarded,
        WaylandPreedit,
        IbusPreedit,
        X11,
        Clipboard,
    }

    fn apply_replacement(
        mode: DeliveryMode,
        initial: &str,
        delete_before_cursor: usize,
        insert: &str,
    ) -> String {
        let mut visible = initial.to_string();
        for _ in 0..delete_before_cursor {
            visible.pop();
        }
        visible.push_str(insert);

        match mode {
            DeliveryMode::Grabbed
            | DeliveryMode::Forwarded
            | DeliveryMode::WaylandPreedit
            | DeliveryMode::IbusPreedit
            | DeliveryMode::X11
            | DeliveryMode::Clipboard => visible,
        }
    }

    #[test]
    fn all_frontend_delivery_modes_apply_the_same_semantic_replacement() {
        for mode in [
            DeliveryMode::Grabbed,
            DeliveryMode::Forwarded,
            DeliveryMode::WaylandPreedit,
            DeliveryMode::IbusPreedit,
            DeliveryMode::X11,
            DeliveryMode::Clipboard,
        ] {
            assert_eq!(
                apply_replacement(mode, "tieeng", 6, "tiếng"),
                "tiếng",
                "mode={mode:?}"
            );
        }
    }

    #[test]
    fn delete_insert_and_acknowledgement_failure_matrix_is_transaction_safe() {
        for (replacement_result, flush_result) in [
            (InjectResult::Failed, InjectResult::Success),
            (InjectResult::NotSupported, InjectResult::Success),
            (InjectResult::Success, InjectResult::Failed),
            (InjectResult::Failed, InjectResult::Failed),
        ] {
            let injector = ScriptedInjector {
                replacement_result,
                flush_result,
                replacement_calls: Cell::new(0),
            };
            let mut revision = 7;

            assert_eq!(
                execute_commands(&injector, &[replacement(7)], &mut revision),
                TransactionOutcome::Failed
            );
            assert_eq!(
                revision, 7,
                "failed delete/insert/ack must not advance revision"
            );
            assert_eq!(injector.replacement_calls.get(), 1);
        }
    }

    #[test]
    fn successful_acknowledgement_advances_exactly_once_without_duplicate_delivery() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Success,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let mut revision = 3;

        assert_eq!(
            execute_commands(&injector, &[replacement(3)], &mut revision),
            TransactionOutcome::Applied {
                visible_revision: 4
            }
        );
        assert_eq!(revision, 4);
        assert_eq!(injector.replacement_calls.get(), 1);
    }
}
