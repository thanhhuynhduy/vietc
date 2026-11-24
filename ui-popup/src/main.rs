// SPDX-License-Identifier: MIT
//! Standalone optional floating candidate strip receiver (`dboard-popup`).
//!
//! Architectural invariants:
//! - Completely separate process from `dboard-daemon` (never linked into daemon).
//! - Receives non-blocking datagrams on `$XDG_RUNTIME_DIR/dboard-candidates.sock`
//!   bound with strict `0600` permissions.
//! - Uses Wayland `zwlr_layer_shell_v1` overlay (`KeyboardInteractivity::None`)
//!   so it NEVER steals keyboard focus.
//! - Anchors directly beneath the AT-SPI2 text caret when coordinates are
//!   provided (clamped within output bounds, flipping above the caret near the
//!   bottom edge), falling back cleanly to bottom-center otherwise.
//! - Supports mouse hover & click selection (`wl_pointer`), sending 1-byte
//!   non-blocking slot indices (`0`/`1`/`2`) to `$XDG_RUNTIME_DIR/dboard-select.sock`.
//! - Immediately clears & hides all candidates upon receiving a `Hide` packet
//!   (password fields, bypass apps, focus loss, or word commit).

mod font;
mod pill_render;
mod vector_font;
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

/// Optional screen-space caret rectangle reported by AT-SPI2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaretRect {
    pub x: i32,
    pub y: i32,
    pub h: i32,
}

/// Decoded candidate strip state from `dboard-daemon`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CandidatePacket {
    pub visible: bool,
    pub candidates: Vec<String>,
    pub caret: Option<CaretRect>,
}

/// Returns `$XDG_RUNTIME_DIR/<name>` or `None` if unavailable.
fn runtime_socket_path(name: &str) -> Option<PathBuf> {
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    if rt.is_empty() {
        return None;
    }
    let dir = PathBuf::from(rt);
    if !dir.is_dir() {
        return None;
    }
    Some(dir.join(name))
}

pub fn socket_path() -> Option<PathBuf> {
    runtime_socket_path("dboard-candidates.sock")
}

pub fn select_socket_path() -> Option<PathBuf> {
    runtime_socket_path("dboard-select.sock")
}

fn lock_file_path() -> Option<PathBuf> {
    runtime_socket_path("dboard-popup.lock")
}

/// Acquire a non-blocking exclusive single-instance `flock` before unlinking
/// or binding `dboard-candidates.sock`, preventing duplicate/orphaned popups.
fn acquire_single_instance_lock(path: &Path) -> Option<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(file)
    } else {
        None
    }
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

fn parse_i32_field(json: &str, key: &str) -> Option<i32> {
    let idx = json.find(key)? + key.len();
    let rest = &json[idx..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse::<i32>().ok()
}

fn parse_caret(json: &str) -> Option<CaretRect> {
    let start = json.find(r#""caret":{"#)?;
    let sub = &json[start..];
    let x = parse_i32_field(sub, r#""x":"#)?;
    let y = parse_i32_field(sub, r#""y":"#)?;
    let h = parse_i32_field(sub, r#""h":"#)?;
    if (-32768..=32768).contains(&x) && (-32768..=32768).contains(&y) && h > 0 && h <= 240 {
        Some(CaretRect { x, y, h })
    } else {
        None
    }
}

/// Parse a compact `{"v":1,"visible":bool,"candidates":[...],"caret":{...}}` JSON packet.
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
            caret: None,
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
    let caret = parse_caret(trimmed);
    Some(CandidatePacket {
        visible: !candidates.is_empty(),
        candidates,
        caret,
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

fn send_click_selection(sock: &UnixDatagram, select_path: Option<&Path>, idx: usize) {
    let Some(path) = select_path else {
        return;
    };
    if idx < 3 {
        let payload = [b'0' + (idx as u8)];
        let _ = sock.send_to(&payload, path);
    }
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
    let _instance_lock = match lock_file_path().and_then(|lp| acquire_single_instance_lock(&lp)) {
        Some(lock) => lock,
        None => {
            eprintln!("[dboard-popup] Another instance is already running; exiting");
            return;
        }
    };
    let sock = match bind_secure_socket(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[dboard-popup] Failed to bind {}: {err}", path.display());
            std::process::exit(1);
        }
    };
    let sel_path = select_socket_path();

    let mut wayland_bar = match wayland_layer::WaylandLayerBar::connect() {
        Ok(bar) => {
            eprintln!(
                "[dboard-popup] Wayland zwlr_layer_shell_v1 active on {} (0600, caret-tracking, pointer-click, keyboard_interactivity=None)",
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
    let mut last_visible_at: Option<std::time::Instant> = None;
    let mut last_reconnect_attempt = std::time::Instant::now();
    const STALE_OVERLAY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
    const WAYLAND_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    while RUNNING.load(Ordering::Relaxed) {
        if wayland_bar.as_ref().is_some_and(|b| b.is_closed()) {
            eprintln!("[dboard-popup] Layer surface closed; scheduling Wayland reconnect");
            wayland_bar = None;
            last_visible_at = None;
            last_reconnect_attempt = std::time::Instant::now();
        }
        if wayland_bar.is_none() && last_reconnect_attempt.elapsed() >= WAYLAND_RECONNECT_INTERVAL {
            last_reconnect_attempt = std::time::Instant::now();
            if let Ok(bar) = wayland_layer::WaylandLayerBar::connect() {
                eprintln!("[dboard-popup] Reconnected to Wayland zwlr_layer_shell_v1");
                wayland_bar = Some(bar);
            }
        }
        // Flush outgoing Wayland requests and drain any events already buffered
        // in `wayland-client` BEFORE sleeping in `libc::poll` (per Wayland
        // `prepare_read` contract) so `Release`/`Configure`/pointer events never
        // wait up to 250ms in the internal queue.
        if let Some(bar) = wayland_bar.as_mut() {
            if bar.dispatch_pending().is_err() {
                wayland_bar = None;
                last_visible_at = None;
                last_reconnect_attempt = std::time::Instant::now();
            } else if let Some(clicked_idx) = bar.take_clicked_slot() {
                send_click_selection(&sock, sel_path.as_deref(), clicked_idx);
            }
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
        if rc < 0 {
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
            continue;
        }
        if last_visible_at.is_some_and(|at| at.elapsed() >= STALE_OVERLAY_TIMEOUT) {
            last_visible_at = None;
            if let Some(bar) = wayland_bar.as_mut() {
                if bar.hide().is_err() {
                    wayland_bar = None;
                    last_reconnect_attempt = std::time::Instant::now();
                }
            }
        }
        if rc == 0 {
            continue;
        }
        if (fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            break;
        }
        if nfds == 2 && (fds[1].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            eprintln!("[dboard-popup] Wayland socket hangup; scheduling reconnect");
            wayland_bar = None;
            last_visible_at = None;
            last_reconnect_attempt = std::time::Instant::now();
        }
        // Dispatch pending Wayland pointer events BEFORE applying a new daemon
        // update so a simultaneous click is hit-tested against the candidate
        // slots currently visible on screen.
        if nfds == 2 && (fds[1].revents & libc::POLLIN) != 0 {
            if let Some(bar) = wayland_bar.as_mut() {
                if bar.dispatch_pending().is_err() {
                    wayland_bar = None;
                    last_visible_at = None;
                    last_reconnect_attempt = std::time::Instant::now();
                } else if let Some(clicked_idx) = bar.take_clicked_slot() {
                    send_click_selection(&sock, sel_path.as_deref(), clicked_idx);
                }
            }
        }
        // Drain and coalesce daemon updates before rendering so rapid bursts
        // (100+ keystrokes/sec) never trigger redundant SHM/SDF frames.
        if (fds[0].revents & libc::POLLIN) != 0 {
            let mut latest_pkt: Option<CandidatePacket> = None;
            while let Ok(len) = sock.recv(&mut buf) {
                if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                    if let Some(pkt) = parse_packet(text) {
                        latest_pkt = Some(pkt);
                    }
                }
            }
            if let Some(pkt) = latest_pkt {
                if pkt.visible && !pkt.candidates.is_empty() {
                    last_visible_at = Some(std::time::Instant::now());
                    if let Some(bar) = wayland_bar.as_mut() {
                        if bar.show(&pkt.candidates, pkt.caret).is_err() {
                            wayland_bar = None;
                            last_visible_at = None;
                            last_reconnect_attempt = std::time::Instant::now();
                        }
                    } else if let Some(line) = format_strip(&pkt) {
                        println!("{line}");
                    }
                } else {
                    last_visible_at = None;
                    if let Some(bar) = wayland_bar.as_mut() {
                        if bar.hide().is_err() {
                            wayland_bar = None;
                            last_reconnect_attempt = std::time::Instant::now();
                        }
                    }
                }
            }
        }
        // Also drain any click queued during `show()`'s output-rebind roundtrip.
        if let Some(bar) = wayland_bar.as_mut() {
            if let Some(clicked_idx) = bar.take_clicked_slot() {
                send_click_selection(&sock, sel_path.as_deref(), clicked_idx);
            }
        }
    }
    let _ = fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_caret_and_hide_packets_and_formats_top_three() {
        let update = r#"{"v":1,"visible":true,"candidates":["tiếng","tieengs","tiến"],"caret":{"x":420,"y":310,"h":20}}"#;
        let pkt = parse_packet(update).unwrap();
        assert!(pkt.visible);
        assert_eq!(pkt.candidates, vec!["tiếng", "tieengs", "tiến"]);
        assert_eq!(
            pkt.caret,
            Some(CaretRect {
                x: 420,
                y: 310,
                h: 20
            })
        );
        assert_eq!(
            format_strip(&pkt).as_deref(),
            Some("[1] tiếng | [2] tieengs | [3] tiến")
        );

        let neg_caret =
            r#"{"v":1,"visible":true,"candidates":["chào"],"caret":{"x":-1200,"y":-300,"h":20}}"#;
        let pkt_neg = parse_packet(neg_caret).unwrap();
        assert_eq!(
            pkt_neg.caret,
            Some(CaretRect {
                x: -1200,
                y: -300,
                h: 20
            })
        );

        let no_caret = r#"{"v":1,"visible":true,"candidates":["chào"]}"#;
        let pkt2 = parse_packet(no_caret).unwrap();
        assert_eq!(pkt2.caret, None);

        let hide = r#"{"v":1,"visible":false,"candidates":[]}"#;
        let hpkt = parse_packet(hide).unwrap();
        assert!(!hpkt.visible);
        assert!(hpkt.candidates.is_empty());
        assert_eq!(hpkt.caret, None);
        assert_eq!(format_strip(&hpkt), None);
    }

    #[test]
    fn bind_secure_socket_enforces_0600_permissions() {
        let path = std::env::temp_dir().join(format!(
            "dboard-popup-perm-test-{}.sock",
            std::process::id()
        ));
        let _sock = bind_secure_socket(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn single_instance_lock_prevents_concurrent_duplicate_popups() {
        let path = std::env::temp_dir().join(format!(
            "dboard-popup-lock-test-{}.lock",
            std::process::id()
        ));
        let first = acquire_single_instance_lock(&path);
        assert!(first.is_some());
        assert!(acquire_single_instance_lock(&path).is_none());
        drop(first);
        let second = acquire_single_instance_lock(&path);
        assert!(second.is_some());
        drop(second);
        let _ = fs::remove_file(&path);
    }
}
