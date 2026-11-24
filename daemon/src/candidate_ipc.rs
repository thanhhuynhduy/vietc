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

use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_CANDIDATES: usize = 3;
const MAX_CANDIDATE_BYTES: usize = 64;

/// Lock-free packed caret screen rectangle:
/// bit 63 = valid flag, bits 42..63 = biased x (signed 21-bit),
/// bits 21..42 = biased y (signed 21-bit), bits 0..21 = h (21-bit).
static CARET_RECT_PACKED: AtomicU64 = AtomicU64::new(0);
const CARET_COORD_BIAS: i32 = 0x10_0000;

/// Record the active caret's screen-space `(x, y, height)` from the background
/// AT-SPI2 thread with zero locking on `evdev_loop`.
pub fn record_caret_rect(rect: Option<(i32, i32, i32)>) {
    let packed = match rect {
        Some((x, y, h))
            if (-CARET_COORD_BIAS..CARET_COORD_BIAS).contains(&x)
                && (-CARET_COORD_BIAS..CARET_COORD_BIAS).contains(&y)
                && (1..0x20_0000).contains(&h) =>
        {
            (1u64 << 63)
                | (((x + CARET_COORD_BIAS) as u64) << 42)
                | (((y + CARET_COORD_BIAS) as u64) << 21)
                | (h as u64)
        }
        _ => 0,
    };
    CARET_RECT_PACKED.store(packed, Ordering::Release);
}

/// Read the most recent valid caret screen-space `(x, y, height)`.
pub fn current_caret_rect() -> Option<(i32, i32, i32)> {
    let packed = CARET_RECT_PACKED.load(Ordering::Acquire);
    if (packed & (1u64 << 63)) == 0 {
        return None;
    }
    let mask = 0x1F_FFFFu64;
    let x = (((packed >> 42) & mask) as i32) - CARET_COORD_BIAS;
    let y = (((packed >> 21) & mask) as i32) - CARET_COORD_BIAS;
    let h = (packed & mask) as i32;
    Some((x, y, h))
}

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

/// Non-blocking click-selection feedback socket `$XDG_RUNTIME_DIR/dboard-select.sock`.
pub fn default_select_socket_path() -> Option<PathBuf> {
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    if rt.is_empty() {
        return None;
    }
    let dir = PathBuf::from(rt);
    if !dir.is_dir() {
        return None;
    }
    Some(dir.join("dboard-select.sock"))
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
#[allow(dead_code)]
pub fn encode_candidates_packet(candidates: &[&str]) -> String {
    encode_candidates_packet_with_caret(candidates, None)
}

/// Encode up to 3 candidate strings plus optional AT-SPI2 caret coordinates.
pub fn encode_candidates_packet_with_caret(
    candidates: &[&str],
    caret: Option<(i32, i32, i32)>,
) -> String {
    let mut out = String::with_capacity(224);
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
    out.push(']');
    if let Some((x, y, h)) = caret {
        out.push_str(&format!(r#","caret":{{"x":{x},"y":{y},"h":{h}}}"#));
    }
    out.push('}');
    out
}

fn bind_select_receiver(path: &Path) -> Option<UnixDatagram> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let sock = UnixDatagram::bind(path).ok()?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    sock.set_nonblocking(true).ok()?;
    Some(sock)
}

/// Non-blocking candidate broadcaster + click-selection receiver owned by `evdev_loop`.
pub struct CandidateBroadcaster {
    socket: Option<UnixDatagram>,
    target_path: Option<PathBuf>,
    select_rx: Option<UnixDatagram>,
    select_path: Option<PathBuf>,
    last_visible: bool,
}

impl Drop for CandidateBroadcaster {
    fn drop(&mut self) {
        self.hide();
        if let Some(path) = self.select_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl CandidateBroadcaster {
    /// Construct a broadcaster. When `enabled == false`, performs zero syscalls
    /// and stores `None`.
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self::disabled();
        }
        match default_socket_path() {
            Some(path) => {
                let mut bc = Self::with_path(true, path);
                if let Some(sel_path) = default_select_socket_path() {
                    if let Some(rx) = bind_select_receiver(&sel_path) {
                        bc.select_rx = Some(rx);
                        bc.select_path = Some(sel_path);
                    }
                }
                bc
            }
            None => Self::disabled(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            socket: None,
            target_path: None,
            select_rx: None,
            select_path: None,
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
            select_rx: None,
            select_path: None,
            last_visible: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.socket.is_some() && self.target_path.is_some()
    }

    pub fn select_rx_fd(&self) -> Option<RawFd> {
        self.select_rx.as_ref().map(AsRawFd::as_raw_fd)
    }

    /// Drain pending click-selection datagrams from `dboard-popup` (non-blocking,
    /// bounded per poll tick so a flood can never starve `evdev_loop`) and return
    /// the latest clicked candidate slot index (`0..=2`).
    pub fn poll_clicked_index(&self) -> Option<usize> {
        let rx = self.select_rx.as_ref()?;
        let mut latest = None;
        let mut buf = [0u8; 8];
        for _ in 0..64 {
            let Ok(len) = rx.recv(&mut buf) else {
                break;
            };
            if len >= 1 {
                match buf[0] {
                    b'0' => latest = Some(0),
                    b'1' => latest = Some(1),
                    b'2' => latest = Some(2),
                    _ => {}
                }
            }
        }
        latest
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
        let payload = encode_candidates_packet_with_caret(candidates, current_caret_rect());
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

    #[test]
    fn caret_rect_supports_negative_multi_monitor_coordinates() {
        record_caret_rect(Some((-1200, -450, 22)));
        assert_eq!(current_caret_rect(), Some((-1200, -450, 22)));
        record_caret_rect(Some((0, 0, 18)));
        assert_eq!(current_caret_rect(), Some((0, 0, 18)));
        record_caret_rect(None);
        assert_eq!(current_caret_rect(), None);
    }

    #[test]
    fn drop_broadcaster_hides_visible_popup() {
        let dir = std::env::temp_dir().join(format!("dboard_cand_drop_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock_path = dir.join("candidates.sock");
        let _ = std::fs::remove_file(&sock_path);
        let receiver = UnixDatagram::bind(&sock_path).unwrap();
        receiver.set_nonblocking(true).unwrap();

        let mut buf = [0u8; 256];
        {
            let mut bc = CandidateBroadcaster::with_path(true, sock_path.clone());
            bc.sync_from_engine(false, "tiếng", "tieengs", None);
            let n = receiver.recv(&mut buf).unwrap();
            assert!(std::str::from_utf8(&buf[..n])
                .unwrap()
                .contains("\"visible\":true"));
        }
        // Dropping the broadcaster on daemon shutdown MUST emit a Hide packet.
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf[..n]).unwrap(),
            r#"{"v":1,"visible":false,"candidates":[]}"#
        );

        drop(receiver);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
