// SPDX-License-Identifier: MIT
//! Standalone optional floating candidate strip receiver (`dboard-popup`).
//!
//! Architectural invariants:
//! - Completely separate process from `dboard-daemon` (never linked into daemon).
//! - Receives one-way non-blocking datagrams on `$XDG_RUNTIME_DIR/dboard-candidates.sock`
//!   bound with strict `0600` permissions.
//! - Uses Wayland `zwlr_layer_shell_v1` overlay (`KeyboardInteractivity::None`)
//!   so it NEVER steals keyboard focus.
//! - Immediately clears & hides all candidates upon receiving a `Hide` packet
//!   (password fields, bypass apps, focus loss, or word commit).

mod font;
mod wayland_layer;

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PACKET_BYTES: usize = 1024;

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_signal(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Decoded candidate strip state from `dboard-daemon`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CandidatePacket {
    pub visible: bool,
    pub candidates: Vec<String>,
}

/// Returns `$XDG_RUNTIME_DIR/dboard-candidates.sock` or `None` if unavailable.
pub fn socket_path() -> Option<PathBuf> {
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    if rt.is_empty() {
        return None;
    }
    let dir = PathBuf::from(rt);
    if !dir.is_dir() {
        return None;
    }
    Some(dir.join("dboard-candidates.sock"))
}

/// Bind the Unix datagram socket with strict `0o600` permissions and `O_NONBLOCK`.
pub fn bind_secure_socket(path: &Path) -> std::io::Result<UnixDatagram> {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    let sock = UnixDatagram::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    sock.set_nonblocking(true)?;
    Ok(sock)
}

/// Parse a compact `{"v":1,"visible":bool,"candidates":[...]}` JSON packet.
pub fn parse_packet(raw: &str) -> Option<CandidatePacket> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let visible = if trimmed.contains(r#""visible":true"#) {
        true
    } else if trimmed.contains(r#""visible":false"#) {
        false
    } else {
        return None;
    };
    if !visible {
        return Some(CandidatePacket {
            visible: false,
            candidates: Vec::new(),
        });
    }
    let marker = r#""candidates":["#;
    let start = trimmed.find(marker)? + marker.len();
    let rest = &trimmed[start..];
    let mut candidates = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ']' => break,
            '"' => {
                let mut item = String::new();
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        if let Some(esc) = chars.next() {
                            match esc {
                                'n' => item.push('\n'),
                                'r' => item.push('\r'),
                                't' => item.push('\t'),
                                other => item.push(other),
                            }
                        }
                    } else if c == '"' {
                        break;
                    } else {
                        item.push(c);
                    }
                }
                if !item.is_empty() && candidates.len() < 3 {
                    candidates.push(item);
                }
            }
            _ => {}
        }
    }
    Some(CandidatePacket {
        visible: !candidates.is_empty(),
        candidates,
    })
}

/// Format top-3 candidates into the compact strip string `[1] a | [2] b | [3] c`.
pub fn format_strip(packet: &CandidatePacket) -> Option<String> {
    if !packet.visible || packet.candidates.is_empty() {
        return None;
    }
    let parts: Vec<String> = packet
        .candidates
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, c)| format!("[{}] {}", i + 1, c))
        .collect();
    Some(parts.join(" | "))
}

fn main() {
    unsafe {
        let _ = libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        let _ = libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }

    let Some(path) = socket_path() else {
        eprintln!("[dboard-popup] XDG_RUNTIME_DIR not available; exiting");
        std::process::exit(1);
    };
    let sock = match bind_secure_socket(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[dboard-popup] Failed to bind {}: {err}", path.display());
            std::process::exit(1);
        }
    };

    let mut wayland_bar = match wayland_layer::WaylandLayerBar::connect() {
        Ok(bar) => {
            eprintln!(
                "[dboard-popup] Wayland zwlr_layer_shell_v1 active on {} (0600, keyboard_interactivity=None)",
                path.display()
            );
            Some(bar)
        }
        Err(err) => {
            eprintln!(
                "[dboard-popup] Wayland layer-shell unavailable ({err}); running in headless log mode on {}",
                path.display()
            );
            None
        }
    };

    let mut buf = [0u8; MAX_PACKET_BYTES];
    while RUNNING.load(Ordering::Relaxed) {
        if wayland_bar.as_ref().is_some_and(|b| b.is_closed()) {
            break;
        }
        let mut fds = [
            libc::pollfd {
                fd: sock.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wayland_bar.as_ref().map_or(-1, |b| b.connection_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let nfds = if fds[1].fd >= 0 { 2 } else { 1 };
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), nfds, 250) };
        if rc <= 0 {
            continue;
        }
        if (fds[1].revents & libc::POLLIN) != 0 {
            if let Some(bar) = wayland_bar.as_mut() {
                if bar.dispatch_pending().is_err() {
                    wayland_bar = None;
                }
            }
        }
        if (fds[0].revents & libc::POLLIN) != 0 {
            while let Ok(len) = sock.recv(&mut buf) {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Some(pkt) = parse_packet(text) {
                        if let Some(line) = format_strip(&pkt) {
                            if let Some(bar) = wayland_bar.as_mut() {
                                let _ = bar.show(&line);
                            } else {
                                println!("{line}");
                            }
                        } else if let Some(bar) = wayland_bar.as_mut() {
                            let _ = bar.hide();
                        }
                    }
                }
            }
        }
    }
    let _ = fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_and_hide_packets_and_formats_top_three() {
        let update = r#"{"v":1,"visible":true,"candidates":["tiếng","tieengs","tiến"]}"#;
        let pkt = parse_packet(update).unwrap();
        assert!(pkt.visible);
        assert_eq!(pkt.candidates, vec!["tiếng", "tieengs", "tiến"]);
        assert_eq!(
            format_strip(&pkt).as_deref(),
            Some("[1] tiếng | [2] tieengs | [3] tiến")
        );

        let hide = r#"{"v":1,"visible":false,"candidates":[]}"#;
        let hpkt = parse_packet(hide).unwrap();
        assert!(!hpkt.visible);
        assert!(hpkt.candidates.is_empty());
        assert_eq!(format_strip(&hpkt), None);
    }

    #[test]
    fn bind_secure_socket_enforces_0600_permissions() {
        let path =
            std::env::temp_dir().join(format!("dboard-popup-test-{}.sock", std::process::id()));
        let sock = bind_secure_socket(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(sock);
        let _ = fs::remove_file(&path);
    }
}
