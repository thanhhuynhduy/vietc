// SPDX-License-Identifier: MIT
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use super::inject::{InjectResult, KeyInjector};

fn socket_path_from_runtime_dir(runtime_dir: &std::path::Path) -> PathBuf {
    runtime_dir.join("vietc").join("uinput.sock")
}

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|runtime_dir| socket_path_from_runtime_dir(&runtime_dir))
}

pub struct UinputClient;

impl UinputClient {
    fn send_command(cmd: &str) -> InjectResult {
        let Some(socket_path) = socket_path() else {
            return InjectResult::Failed;
        };

        Self::send_command_to(&socket_path, cmd)
    }

    fn send_command_to(socket_path: &std::path::Path, cmd: &str) -> InjectResult {
        if cmd.contains(['\n', '\r']) {
            return InjectResult::NotSupported;
        }
        match UnixStream::connect(socket_path) {
            Ok(mut stream) => {
                let timeout = Some(std::time::Duration::from_secs(5));
                if stream.set_read_timeout(timeout).is_err()
                    || stream.set_write_timeout(timeout).is_err()
                {
                    return InjectResult::Failed;
                }
                if writeln!(stream, "{}", cmd).is_err() {
                    return InjectResult::Failed;
                }
                let mut reader = BufReader::new(&stream);
                let mut response = String::new();
                if reader.read_line(&mut response).is_err() {
                    return InjectResult::Failed;
                }
                if response.trim() == "OK" {
                    InjectResult::Success
                } else {
                    InjectResult::Failed
                }
            }
            Err(_) => InjectResult::Failed,
        }
    }

    pub fn is_available() -> bool {
        socket_path()
            .and_then(|path| UnixStream::connect(path).ok())
            .is_some()
    }
}

impl KeyInjector for UinputClient {
    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        if keycode > 0x1ff || !(0..=2).contains(&value) {
            return InjectResult::NotSupported;
        }
        Self::send_command(&format!("KEY:{keycode}:{value}"))
    }

    fn send_backspace(&self) -> InjectResult {
        Self::send_command("BACKSPACE:1")
    }

    fn send_char(&self, ch: char) -> InjectResult {
        self.send_string(&ch.to_string())
    }

    fn send_string(&self, s: &str) -> InjectResult {
        Self::send_command(&format!("TYPE:{}", s))
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        Self::send_command(&format!("REPLACE:{}:{}", backspaces, text))
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }

    fn update_pasted_text(&self, _text: &str) -> InjectResult {
        InjectResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_key_operations_must_not_claim_success() {
        // A raw key must either be delivered or explicitly unsupported.
        assert_ne!(UinputClient.send_key_event(30, -1), InjectResult::Success);
    }

    #[test]
    fn socket_request_requires_server_acknowledgement() {
        use std::os::unix::net::UnixListener;
        let path = std::env::temp_dir().join(format!("vietc-client-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            for (command, response) in [
                ("KEY:30:1", "OK"),
                ("BACKSPACE:1", "ERR write failed"),
                ("TYPE:á", "OK"),
                ("TYPE: ", "OK"),
                ("TYPE:\t", "OK"),
                ("REPLACE:2: a \t", "OK"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                assert_eq!(request, format!("{command}\n"));
                writeln!(stream, "{response}").unwrap();
            }
        });
        assert_eq!(
            UinputClient::send_command_to(&path, "KEY:30:1"),
            InjectResult::Success
        );
        assert_eq!(
            UinputClient::send_command_to(&path, "BACKSPACE:1"),
            InjectResult::Failed
        );
        assert_eq!(
            UinputClient::send_command_to(&path, "TYPE:á"),
            InjectResult::Success
        );
        for command in ["TYPE: ", "TYPE:\t", "REPLACE:2: a \t"] {
            assert_eq!(
                UinputClient::send_command_to(&path, command),
                InjectResult::Success
            );
        }
        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn newline_framing_is_rejected_before_connecting() {
        let path = std::path::Path::new("");
        for command in [
            "TYPE:\n",
            "TYPE:\r",
            "TYPE:a\nKEY:30:1",
            "REPLACE:1:a\r\nBACKSPACE:1",
        ] {
            assert_eq!(
                UinputClient::send_command_to(path, command),
                InjectResult::NotSupported
            );
        }
    }

    #[test]
    fn socket_path_matches_uinputd_runtime_contract() {
        let runtime_dir = std::path::Path::new("/run/user/1000");

        assert_eq!(
            socket_path_from_runtime_dir(runtime_dir),
            runtime_dir.join("vietc").join("uinput.sock")
        );
    }
}
