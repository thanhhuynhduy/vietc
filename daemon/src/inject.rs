use vietc_protocol::inject::InjectResult;
use vietc_protocol::KeyInjector;

use crate::commands::OutputCommand;
use crate::display;
use crate::log::log_info;

pub fn execute_commands(injector: &dyn KeyInjector, commands: &[OutputCommand]) -> InjectResult {
    let mut pending_backspaces: usize = 0;
    let mut pending_text = String::new();

    for cmd in commands {
        match cmd {
            OutputCommand::Backspace(count) => {
                pending_backspaces += count;
            }
            OutputCommand::Type(text) => {
                pending_text.push_str(text);
            }
        }
    }

    if !commands.is_empty() {
        let result = injector.inject_replacement(pending_backspaces, &pending_text);
        if result != InjectResult::Success {
            return result;
        }
    }

    injector.flush()
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

    #[test]
    fn execute_commands_propagates_replacement_failure() {
        let injector = ScriptedInjector {
            replacement_result: InjectResult::Failed,
            flush_result: InjectResult::Success,
            replacement_calls: Cell::new(0),
        };
        let commands = [OutputCommand::Type("ế".to_string())];

        assert_eq!(execute_commands(&injector, &commands), InjectResult::Failed);
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

        assert_eq!(execute_commands(&injector, &commands), InjectResult::Failed);
    }
}
