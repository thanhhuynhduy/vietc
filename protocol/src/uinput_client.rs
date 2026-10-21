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

        match UnixStream::connect(socket_path) {
            Ok(mut stream) => {
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
    fn send_key_event(&self, _keycode: u16, _value: i32) -> InjectResult {
        InjectResult::Success
    }

    fn send_backspace(&self) -> InjectResult {
        InjectResult::Success
    }

    fn send_char(&self, _ch: char) -> InjectResult {
        InjectResult::Success
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
    fn socket_path_matches_uinputd_runtime_contract() {
        let runtime_dir = std::path::Path::new("/run/user/1000");

        assert_eq!(
            socket_path_from_runtime_dir(runtime_dir),
            runtime_dir.join("vietc").join("uinput.sock")
        );
    }
}
