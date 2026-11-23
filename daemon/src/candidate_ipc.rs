// SPDX-License-Identifier: MIT
//! Optional non-blocking one-way IPC broadcaster for the floating candidate strip
//! (`dboard-popup`).
//!
//! Architectural & security guarantees:
//! - Disabled by default (`candidate_bar = false`): creates NO socket and performs
//!   ZERO syscalls on every keystroke.
//! - Uses an unbound `O_NONBLOCK` `UnixDatagram` socket targeting
//!   `$XDG_RUNTIME_DIR/dboard-candidates.sock`. `WouldBlock` / `NotFound` /
//!   `ConnectionRefused` are silently discarded so a dead or stalled popup can
//!   NEVER block `evdev_loop`.
//! - Security: whenever password protection or app bypass is active,
//!   `publish_state` immediately sends a `Hide` packet and NEVER reads or
//!   transmits buffer contents.

use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};

const MAX_CANDIDATES: usize = 3;
const MAX_CANDIDATE_BYTES: usize = 64;

/// Default socket path in `$XDG_RUNTIME_DIR/dboard-candidates.sock`.
/// Returns `None` if `XDG_RUNTIME_DIR` is unset or not a directory (refuses
/// world-writable `/tmp` fallback to prevent symlink attacks).
pub fn default_socket_path() -> Option<PathBuf> {
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

fn escape_json_str(s: &str, out: &mut String) {
    out.push('"');
    let mut bytes = 0usize;
    for ch in s.chars() {
        let len = ch.len_utf8();
        if bytes + len > MAX_CANDIDATE_BYTES {
            break;
        }
        bytes += len;
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Encode a `Hide` datagram payload.
pub fn encode_hide_packet() -> &'static [u8] {
    br#"{"v":1,"visible":false,"candidates":[]}"#
}

/// Encode up to 3 candidate strings into a compact UTF-8 JSON datagram.
pub fn encode_candidates_packet(candidates: &[&str]) -> String {
    let mut out = String::with_capacity(192);
    out.push_str(r#"{"v":1,"visible":true,"candidates":["#);
    for (written, cand) in candidates
        .iter()
        .copied()
        .filter(|c| !c.is_empty())
        .take(MAX_CANDIDATES)
        .enumerate()
    {
        if written > 0 {
            out.push(',');
        }
        escape_json_str(cand, &mut out);
    }
    out.push_str("]}");
    out
}

/// Non-blocking one-way candidate broadcaster owned by `evdev_loop`.
pub struct CandidateBroadcaster {
    socket: Option<UnixDatagram>,
    target_path: Option<PathBuf>,
    last_visible: bool,
}

impl CandidateBroadcaster {
    /// Construct a broadcaster. When `enabled == false`, performs zero syscalls
    /// and stores `None`.
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self::disabled();
        }
        match default_socket_path() {
            Some(path) => Self::with_path(true, path),
            None => Self::disabled(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            socket: None,
            target_path: None,
            last_visible: false,
        }
    }

    /// Construct a broadcaster targeting an explicit socket path (used in tests).
    pub fn with_path(enabled: bool, path: PathBuf) -> Self {
        if !enabled {
            return Self::disabled();
        }
        let socket = UnixDatagram::unbound().ok().and_then(|sock| {
            sock.set_nonblocking(true).ok()?;
            Some(sock)
        });
        Self {
            socket,
            target_path: Some(path),
            last_visible: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.socket.is_some() && self.target_path.is_some()
    }

    fn send_bytes(&self, bytes: &[u8]) {
        let (Some(sock), Some(path)) = (&self.socket, &self.target_path) else {
            return;
        };
        let _ = sock.send_to(bytes, Path::new(path));
    }

    /// Immediately hide the candidate strip (e.g. on focus loss, password field,
    /// bypass app, or word commit).
    pub fn hide(&mut self) {
        if !self.is_enabled() {
            return;
        }
        if self.last_visible {
            self.send_bytes(encode_hide_packet());
            self.last_visible = false;
        }
    }

    /// Publish up to 3 candidates, or hide immediately if `sensitive_or_bypass`
    /// is true or no non-empty candidates are present.
    pub fn publish_state(&mut self, sensitive_or_bypass: bool, candidates: &[&str]) {
        if !self.is_enabled() {
            return;
        }
        if sensitive_or_bypass {
            self.hide();
            return;
        }
        let has_any = candidates.iter().any(|c| !c.is_empty());
        if !has_any {
            self.hide();
            return;
        }
        let payload = encode_candidates_packet(candidates);
        self.send_bytes(payload.as_bytes());
        self.last_visible = true;
    }

    /// Convenience helper for `evdev_loop`: publish up to 3 candidates
    /// `(visible_vn, raw_ascii, optional_suggestion)` or hide when sensitive/bypass/empty.
    pub fn sync_from_engine(
        &mut self,
        sensitive_or_bypass: bool,
        visible_vn: &str,
        raw_ascii: &str,
        suggestion: Option<&str>,
    ) {
        if !self.is_enabled() {
            return;
        }
        if sensitive_or_bypass || visible_vn.is_empty() {
            self.hide();
            return;
        }
        let mut slots: [&str; 3] = ["", "", ""];
        let mut len = 0usize;
        slots[len] = visible_vn;
        len += 1;
        if !raw_ascii.is_empty() && raw_ascii != visible_vn {
            slots[len] = raw_ascii;
            len += 1;
        }
        if let Some(sug) = suggestion {
            if !sug.is_empty() && sug != visible_vn && sug != raw_ascii && len < 3 {
                slots[len] = sug;
                len += 1;
            }
        }
        self.publish_state(false, &slots[..len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn disabled_broadcaster_creates_no_socket_and_performs_no_io() {
        let dir = std::env::temp_dir().join(format!("dboard-cand-disabled-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock_path = dir.join("candidates.sock");
        let _ = std::fs::remove_file(&sock_path);
        let receiver = UnixDatagram::bind(&sock_path).unwrap();
        receiver.set_nonblocking(true).unwrap();

        let mut bc = CandidateBroadcaster::with_path(false, sock_path.clone());
        assert!(!bc.is_enabled());
        bc.publish_state(false, &["tiếng", "tieengs"]);
        bc.hide();

        let mut buf = [0u8; 256];
        assert!(receiver.recv(&mut buf).is_err());

        drop(receiver);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nonblocking_ipc_never_blocks_when_receiver_missing_or_full() {
        let dir = std::env::temp_dir().join(format!("dboard-cand-nonblock-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let missing_path = dir.join("nonexistent.sock");

        // 1. Missing popup receiver must never block
        let mut bc = CandidateBroadcaster::with_path(true, missing_path);
        let start = Instant::now();
        for _ in 0..500 {
            bc.publish_state(false, &["việt", "vieetj", "viết"]);
        }
        assert!(
            start.elapsed().as_millis() < 200,
            "missing receiver took too long: {:?}",
            start.elapsed()
        );

        // 2. Stalled/full popup receiver must never block evdev_loop
        let stalled_path = dir.join("stalled.sock");
        let _ = std::fs::remove_file(&stalled_path);
        let _stalled_receiver = UnixDatagram::bind(&stalled_path).unwrap();
        let mut bc2 = CandidateBroadcaster::with_path(true, stalled_path);
        let start2 = Instant::now();
        for _ in 0..1000 {
            bc2.publish_state(false, &["chương", "chuowng", "chưởng"]);
        }
        assert!(
            start2.elapsed().as_millis() < 200,
            "stalled receiver blocked sender: {:?}",
            start2.elapsed()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_and_bypass_mode_hides_popup_and_never_leaks_secret_text() {
        let dir = std::env::temp_dir().join(format!("dboard-cand-sec-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock_path = dir.join("sec.sock");
        let _ = std::fs::remove_file(&sock_path);
        let receiver = UnixDatagram::bind(&sock_path).unwrap();
        receiver.set_nonblocking(true).unwrap();

        let mut bc = CandidateBroadcaster::with_path(true, sock_path.clone());
        // Normal composition sends up to 3 candidates
        bc.sync_from_engine(false, "tiếng", "tieengs", Some("tiếngs"));
        let mut buf = [0u8; 256];
        let n = receiver.recv(&mut buf).unwrap();
        let msg = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(msg.contains("\"visible\":true"));
        assert!(msg.contains("tiếng"));
        assert!(msg.contains("tieengs"));
        assert!(msg.contains("tiếngs"));

        // Entering password mode MUST send Hide and NEVER transmit the secret
        bc.sync_from_engine(true, "SuperSecret123!", "SuperSecret123!", None);
        let n2 = receiver.recv(&mut buf).unwrap();
        let msg2 = std::str::from_utf8(&buf[..n2]).unwrap();
        assert_eq!(msg2, r#"{"v":1,"visible":false,"candidates":[]}"#);
        assert!(!msg2.contains("SuperSecret"));

        // Subsequent keystrokes in password mode send nothing at all
        bc.sync_from_engine(true, "AnotherSecret99", "AnotherSecret99", None);
        assert!(receiver.recv(&mut buf).is_err());

        drop(receiver);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
