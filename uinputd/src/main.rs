// SPDX-License-Identifier: MIT
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const UINPUT_MAX_NAME_SIZE: usize = 80;
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_DEV_SETUP: u64 = 0x405c5503;
const EV_KEY: u16 = 0x01;
const CLIPBOARD_TRACE_LIMIT: usize = 128;
const CLIPBOARD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const CLIPBOARD_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_KEY_COUNT: usize = 4_096;
const CLIENT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
static NEXT_CLIPBOARD_SEQUENCE_ID: AtomicU64 = AtomicU64::new(1);
static CLIPBOARD_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn trace_clipboard(sequence_id: u64, phase: &str, detail: &str) {
    let trace_index = CLIPBOARD_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    if trace_index < CLIPBOARD_TRACE_LIMIT {
        eprintln!(
            "[vietc-uinputd] clipboard trace={} sequence_id={} phase={} {}",
            trace_index, sequence_id, phase, detail
        );
    }
}

fn ioctl(fd: i32, request: u64, arg: u64) -> Result<i32, String> {
    let result = unsafe { libc::ioctl(fd, request, arg) };
    if result < 0 {
        Err(format!("ioctl failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(result)
    }
}

#[repr(C)]
struct input_event {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
struct uinput_setup {
    id: input_id,
    name: [i8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
struct input_id {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[derive(Debug, PartialEq, Eq)]
enum ClipboardInjectionAction {
    Restore(String),
    SkipRestore,
}

#[derive(Default)]
struct ClipboardInjectionState {
    saved_clipboard: Option<String>,
    injected_clipboard: Option<String>,
}

impl ClipboardInjectionState {
    fn begin(&mut self, current_clipboard: Option<String>, injected_text: &str) {
        if self.injected_clipboard.is_none() {
            self.saved_clipboard = current_clipboard;
        }
        self.injected_clipboard = Some(injected_text.to_string());
    }

    fn finish(&mut self, current_clipboard: Option<&str>) -> ClipboardInjectionAction {
        let still_contains_injected = matches!(
            (current_clipboard, self.injected_clipboard.as_deref()),
            (Some(current), Some(injected)) if current == injected
        );

        let action = if still_contains_injected {
            self.saved_clipboard
                .clone()
                .map(ClipboardInjectionAction::Restore)
                .unwrap_or(ClipboardInjectionAction::SkipRestore)
        } else {
            ClipboardInjectionAction::SkipRestore
        };

        self.saved_clipboard = None;
        self.injected_clipboard = None;
        action
    }

    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.injected_clipboard.is_some()
    }
}

/// Shared clipboard bookkeeping for one serialized Unicode injection.
struct ClipState {
    inner: Mutex<ClipboardInjectionState>,
}

fn send_key_sequence_with(
    sequence: &[(u16, i32)],
    mut send_key: impl FnMut(u16, i32) -> bool,
) -> bool {
    let mut pressed = Vec::new();
    for &(code, value) in sequence {
        // A failed SYN may still leave the preceding press queued in the device.
        if value == 1 && !pressed.contains(&code) {
            pressed.push(code);
        }
        if !send_key(code, value) {
            for code in pressed.into_iter().rev() {
                let _ = send_key(code, 0);
            }
            return false;
        }
        if value == 0 {
            pressed.retain(|&pressed_code| pressed_code != code);
        }
    }
    true
}

struct UinputDevice {
    fd: i32,
    clip: Arc<ClipState>,
    transaction_lock: Mutex<()>,
}

impl UinputDevice {
    fn new(name: &str) -> Result<Self, String> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| format!("Cannot open /dev/uinput: {} (are you root?)", e))?;

        let fd = file.as_raw_fd();

        ioctl(fd, UI_SET_EVBIT, EV_KEY as u64)?;

        for code in 0..=0x1ffu32 {
            ioctl(fd, UI_SET_KEYBIT, code as u64)?;
        }

        let mut usetup: uinput_setup = unsafe { std::mem::zeroed() };
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(UINPUT_MAX_NAME_SIZE - 1);
        for (i, &byte) in name_bytes.iter().enumerate().take(copy_len) {
            usetup.name[i] = byte as i8;
        }
        usetup.id.bustype = 0x03;
        usetup.id.vendor = 0x1234;
        usetup.id.product = 0x5678;
        usetup.id.version = 1;

        ioctl(fd, UI_DEV_SETUP, &usetup as *const uinput_setup as u64)?;
        ioctl(fd, UI_DEV_CREATE, 0)?;

        std::mem::forget(file);
        std::thread::sleep(std::time::Duration::from_millis(10));

        let clip = Arc::new(ClipState {
            inner: Mutex::new(ClipboardInjectionState::default()),
        });

        eprintln!("[vietc-uinputd] Device '{}' created", name);
        Ok(Self {
            fd,
            clip,
            transaction_lock: Mutex::new(()),
        })
    }

    fn send_event(&self, type_: u16, code: u16, value: i32) -> bool {
        let event = input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        unsafe {
            libc::write(
                self.fd,
                &event as *const input_event as *const libc::c_void,
                std::mem::size_of::<input_event>(),
            ) == std::mem::size_of::<input_event>() as isize
        }
    }

    fn send_key(&self, code: u16, value: i32) -> bool {
        let event_sent = self.send_event(EV_KEY, code, value);
        let sync_sent = self.send_event(0, 0, 0);
        let sent = event_sent && sync_sent;
        std::thread::sleep(std::time::Duration::from_millis(2));
        sent
    }

    fn send_key_sequence(&self, sequence: &[(u16, i32)]) -> bool {
        send_key_sequence_with(sequence, |code, value| self.send_key(code, value))
    }

    fn backspace_n(&self, count: usize) -> bool {
        (0..count).all(|_| self.send_key_sequence(&[(14, 1), (14, 0)]))
    }

    fn paste_shortcut(&self) -> bool {
        self.send_key_sequence(&[(29, 1), (47, 1), (47, 0), (29, 0)])
    }

    fn char_to_keycode(ch: u8) -> Option<(u16, bool)> {
        let lower = ch.to_ascii_lowercase();
        let keycode = match lower {
            b'a' => 30,
            b'b' => 48,
            b'c' => 46,
            b'd' => 32,
            b'e' => 18,
            b'f' => 33,
            b'g' => 34,
            b'h' => 35,
            b'i' => 23,
            b'j' => 36,
            b'k' => 37,
            b'l' => 38,
            b'm' => 50,
            b'n' => 49,
            b'o' => 24,
            b'p' => 25,
            b'q' => 16,
            b'r' => 19,
            b's' => 31,
            b't' => 20,
            b'u' => 22,
            b'v' => 47,
            b'w' => 17,
            b'x' => 45,
            b'y' => 21,
            b'z' => 44,
            b'0' => 11,
            b'1' => 2,
            b'2' => 3,
            b'3' => 4,
            b'4' => 5,
            b'5' => 6,
            b'6' => 7,
            b'7' => 8,
            b'8' => 9,
            b'9' => 10,
            b' ' => 57,
            b'.' => 52,
            b',' => 51,
            b'-' => 12,
            b'=' => 13,
            b';' => 39,
            b'\'' => 40,
            b'/' => 53,
            b'\\' => 43,
            b'[' => 26,
            b']' => 27,
            _ => return None,
        };
        let shift = ch.is_ascii_uppercase()
            || matches!(
                ch,
                b'!' | b'@'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'^'
                    | b'&'
                    | b'*'
                    | b'('
                    | b')'
                    | b'_'
                    | b'+'
                    | b'{'
                    | b'}'
                    | b'|'
                    | b':'
                    | b'"'
                    | b'<'
                    | b'>'
                    | b'?'
                    | b'~'
            );
        Some((keycode, shift))
    }

    fn type_ascii(&self, text: &str) -> bool {
        for byte in text.bytes() {
            let Some((keycode, shift)) = Self::char_to_keycode(byte) else {
                return false;
            };
            let sent = if shift {
                self.send_key_sequence(&[(42, 1), (keycode, 1), (keycode, 0), (42, 0)])
            } else {
                self.send_key_sequence(&[(keycode, 1), (keycode, 0)])
            };
            if !sent {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        true
    }

    fn paste_unicode_unlocked(&self, text: &str, sequence_id: u64) -> bool {
        let snapshot = read_clipboard();
        trace_clipboard(
            sequence_id,
            "snapshot",
            if snapshot.is_some() {
                "state=present"
            } else {
                "state=empty-or-unavailable"
            },
        );

        {
            let mut state = self.clip.inner.lock().unwrap();
            state.begin(snapshot, text);
        }

        trace_clipboard(sequence_id, "temporary-text", "state=writing");
        if !copy_to_clipboard(text) {
            self.clip.inner.lock().unwrap().finish(None);
            trace_clipboard(sequence_id, "temporary-text", "state=write-failed");
            return false;
        }
        let verification_started = std::time::Instant::now();
        if !poll_clipboard_until(
            text,
            CLIPBOARD_VERIFY_TIMEOUT,
            CLIPBOARD_POLL_INTERVAL,
            read_clipboard,
            std::thread::sleep,
            move || verification_started.elapsed(),
        ) {
            self.clip.inner.lock().unwrap().finish(None);
            trace_clipboard(sequence_id, "temporary-text", "state=read-back-failed");
            return false;
        }
        trace_clipboard(sequence_id, "temporary-text", "state=verified");

        let pasted = self.paste_shortcut();
        trace_clipboard(
            sequence_id,
            "paste",
            if pasted {
                "state=sent"
            } else {
                "state=write-failed"
            },
        );

        let action = {
            let mut state = self.clip.inner.lock().unwrap();
            let current = read_clipboard();
            state.finish(current.as_deref())
        };
        let restored = match action {
            ClipboardInjectionAction::Restore(saved) => {
                trace_clipboard(sequence_id, "restore", "state=writing");
                let restored = restore_clipboard_verified(&saved);
                trace_clipboard(
                    sequence_id,
                    "verified-read-back",
                    if restored {
                        "state=restored"
                    } else {
                        "state=failed"
                    },
                );
                restored
            }
            ClipboardInjectionAction::SkipRestore => {
                trace_clipboard(
                    sequence_id,
                    "restore",
                    "state=skipped-user-clipboard-changed",
                );
                true
            }
        };
        pasted && restored
    }

    fn paste_unicode(&self, text: &str) -> bool {
        let sequence_id = NEXT_CLIPBOARD_SEQUENCE_ID.fetch_add(1, Ordering::Relaxed);
        let _transaction = self.transaction_lock.lock().unwrap();
        self.paste_unicode_unlocked(text, sequence_id)
    }

    fn replace(&self, count: usize, text: &str) -> bool {
        let sequence_id = NEXT_CLIPBOARD_SEQUENCE_ID.fetch_add(1, Ordering::Relaxed);
        let _transaction = self.transaction_lock.lock().unwrap();
        if !self.backspace_n(count) {
            return false;
        }

        let is_ascii = text
            .bytes()
            .all(|byte| UinputDevice::char_to_keycode(byte).is_some());
        if is_ascii {
            self.type_ascii(text)
        } else {
            self.paste_unicode_unlocked(text, sequence_id)
        }
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        let _ = unsafe { libc::ioctl(self.fd, UI_DEV_DESTROY, 0) };
        let _ = unsafe { libc::close(self.fd) };
        eprintln!("[vietc-uinputd] Device destroyed");
    }
}

fn read_clipboard() -> Option<String> {
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    let output = if is_wayland {
        Command::new("wl-paste").arg("-n").output()
    } else {
        Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
    };
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn copy_to_clipboard(text: &str) -> bool {
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    let child = if is_wayland {
        Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
    } else {
        Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
    };
    let Ok(mut child) = child else {
        return false;
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.wait();
        return false;
    };
    if stdin.write_all(text.as_bytes()).is_err() {
        drop(stdin);
        let _ = child.wait();
        return false;
    }
    drop(stdin);
    child.wait().map(|status| status.success()).unwrap_or(false)
}

fn poll_clipboard_until(
    expected: &str,
    timeout: std::time::Duration,
    interval: std::time::Duration,
    mut read: impl FnMut() -> Option<String>,
    mut sleep: impl FnMut(std::time::Duration),
    mut elapsed: impl FnMut() -> std::time::Duration,
) -> bool {
    loop {
        if read().as_deref() == Some(expected) {
            return true;
        }
        let elapsed_now = elapsed();
        if elapsed_now >= timeout {
            return false;
        }
        sleep(interval.min(timeout - elapsed_now));
    }
}

fn restore_clipboard_verified_with(
    expected: &str,
    mut write: impl FnMut(&str) -> bool,
    read: impl FnMut() -> Option<String>,
    sleep: impl FnMut(std::time::Duration),
    elapsed: impl FnMut() -> std::time::Duration,
) -> bool {
    write(expected)
        && poll_clipboard_until(
            expected,
            CLIPBOARD_VERIFY_TIMEOUT,
            CLIPBOARD_POLL_INTERVAL,
            read,
            sleep,
            elapsed,
        )
}

fn restore_clipboard_verified(expected: &str) -> bool {
    let started = std::time::Instant::now();
    restore_clipboard_verified_with(
        expected,
        copy_to_clipboard,
        read_clipboard,
        std::thread::sleep,
        move || started.elapsed(),
    )
}

fn runtime_socket_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join("vietc").join("uinput.sock")
}

fn find_socket_path() -> Result<PathBuf, String> {
    let runtime_root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set; refusing an insecure fallback".to_string())?;
    let dir = runtime_root.join("vietc");

    fs::create_dir_all(&dir).map_err(|error| {
        format!(
            "Cannot create socket directory {}: {}",
            dir.display(),
            error
        )
    })?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        format!(
            "Cannot set socket directory permissions on {}: {}",
            dir.display(),
            error
        )
    })?;

    Ok(runtime_socket_path(&runtime_root))
}

fn bind_socket(path: &Path, mode: u32) -> Result<UnixListener, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(|error| {
                format!("Cannot remove stale socket {}: {}", path.display(), error)
            })?;
        }
        Ok(_) => {
            return Err(format!(
                "Refusing to replace non-socket path {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Cannot inspect socket path {}: {}",
                path.display(),
                error
            ));
        }
    }

    let listener = UnixListener::bind(path)
        .map_err(|error| format!("Cannot bind socket {}: {}", path.display(), error))?;

    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
        let cleanup_result = fs::remove_file(path);
        return Err(match cleanup_result {
            Ok(()) => format!(
                "Cannot set socket permissions on {}: {}",
                path.display(),
                error
            ),
            Err(cleanup_error) => format!(
                "Cannot set socket permissions on {}: {}; cleanup failed: {}",
                path.display(),
                error,
                cleanup_error
            ),
        });
    }

    Ok(listener)
}

#[cfg(test)]
fn authorize_and_dispatch<F>(
    peer_uid: libc::uid_t,
    allowed_uid: libc::uid_t,
    dispatch: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    if peer_uid != allowed_uid {
        return Err(format!(
            "peer UID {} is not authorized; expected {}",
            peer_uid, allowed_uid
        ));
    }

    dispatch();
    Ok(())
}

fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t, String> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut credentials_len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut libc::ucred as *mut libc::c_void,
            &mut credentials_len,
        )
    };

    if result != 0 {
        return Err(format!(
            "Cannot read peer credentials: {}",
            std::io::Error::last_os_error()
        ));
    }
    if credentials_len as usize != std::mem::size_of::<libc::ucred>() {
        return Err("Peer credentials have an unexpected size".to_string());
    }

    Ok(credentials.uid)
}

#[cfg(test)]
fn authorize_stream_and_dispatch<F>(
    stream: &UnixStream,
    allowed_uid: libc::uid_t,
    dispatch: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    authorize_and_dispatch(peer_uid(stream)?, allowed_uid, dispatch)
}

fn parse_key_event(payload: &str) -> Option<(u16, i32)> {
    let (code, value) = payload.split_once(':')?;
    let code = code.parse::<u16>().ok()?;
    let value = value.parse::<i32>().ok()?;
    (code <= 0x1ff && (0..=2).contains(&value)).then_some((code, value))
}

fn read_bounded_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();

    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("cannot read request: {error}"))?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let chunk_len = newline.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(chunk_len) > MAX_REQUEST_BYTES {
            return Err("request too large".to_string());
        }

        bytes.extend_from_slice(&available[..chunk_len]);
        reader.consume(chunk_len);
        if newline.is_some() {
            break;
        }
    }

    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }

    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "request is not valid UTF-8".to_string())
}

fn validate_text(text: &str) -> bool {
    text.len() <= MAX_TEXT_BYTES
}

fn handle_client(
    stream: UnixStream,
    allowed_uid: libc::uid_t,
    uinput: &UinputDevice,
) -> Result<(), String> {
    let peer_uid = peer_uid(&stream)?;
    if peer_uid != allowed_uid {
        return Err(format!(
            "peer UID {} is not authorized; expected {}",
            peer_uid, allowed_uid
        ));
    }

    stream
        .set_read_timeout(Some(CLIENT_READ_TIMEOUT))
        .map_err(|error| format!("cannot set client timeout: {error}"))?;

    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;

    loop {
        let line = match read_bounded_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                let _ = writeln!(writer, "ERR {error}");
                break;
            }
        };

        if line.is_empty() {
            continue;
        }

        if line == "PING" {
            let _ = writeln!(writer, "PONG");
        } else if line == "FLUSH" {
            let _ = writeln!(writer, "OK");
        } else if line == "QUIT" {
            let _ = writeln!(writer, "BYE");
            break;
        } else if let Some(payload) = line.strip_prefix("KEY:") {
            let success =
                parse_key_event(payload).is_some_and(|(code, value)| uinput.send_key(code, value));
            let _ = writeln!(
                writer,
                "{}",
                if success {
                    "OK"
                } else {
                    "ERR key injection failed"
                }
            );
        } else if let Some(payload) = line.strip_prefix("REPLACE:") {
            let Some((count, text)) = payload.split_once(':') else {
                let _ = writeln!(writer, "ERR malformed replacement");
                continue;
            };
            let Ok(count) = count.parse::<usize>() else {
                let _ = writeln!(writer, "ERR bad count");
                continue;
            };
            if count > MAX_KEY_COUNT {
                let _ = writeln!(writer, "ERR count too large");
                continue;
            }
            if !validate_text(text) {
                let _ = writeln!(writer, "ERR text too large");
                continue;
            }
            let success = uinput.replace(count, text);
            let response = if success {
                "OK"
            } else {
                "ERR injection failed"
            };
            let _ = writeln!(writer, "{response}");
        } else if let Some(n_str) = line.strip_prefix("BACKSPACE:") {
            if let Ok(n) = n_str.parse::<usize>() {
                if n > MAX_KEY_COUNT {
                    let _ = writeln!(writer, "ERR count too large");
                    continue;
                }
                let success = uinput.backspace_n(n);
                let _ = writeln!(
                    writer,
                    "{}",
                    if success {
                        "OK"
                    } else {
                        "ERR key injection failed"
                    }
                );
            } else {
                let _ = writeln!(writer, "ERR bad count");
            }
        } else if let Some(text) = line.strip_prefix("TYPE:") {
            if !validate_text(text) {
                let _ = writeln!(writer, "ERR text too large");
                continue;
            }
            let is_ascii = text
                .bytes()
                .all(|b| UinputDevice::char_to_keycode(b).is_some());
            let success = if is_ascii {
                uinput.type_ascii(text)
            } else {
                uinput.paste_unicode(text)
            };
            let _ = writeln!(
                writer,
                "{}",
                if success {
                    "OK"
                } else {
                    "ERR injection failed"
                }
            );
        } else if let Some(text) = line.strip_prefix("PASTE:") {
            if !validate_text(text) {
                let _ = writeln!(writer, "ERR text too large");
                continue;
            }
            let success = uinput.paste_unicode(text);
            let _ = writeln!(
                writer,
                "{}",
                if success {
                    "OK"
                } else {
                    "ERR injection failed"
                }
            );
        } else {
            let _ = writeln!(writer, "ERR unknown command");
        }
    }

    Ok(())
}

fn main() {
    let socket_path = match find_socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[vietc-uinputd] {}", error);
            std::process::exit(1);
        }
    };

    let runtime_root = match socket_path.parent().and_then(Path::parent) {
        Some(path) => path,
        None => {
            eprintln!(
                "[vietc-uinputd] Cannot determine runtime directory owner from {}",
                socket_path.display()
            );
            std::process::exit(1);
        }
    };
    let allowed_uid = match fs::metadata(runtime_root) {
        Ok(metadata) => {
            use std::os::unix::fs::MetadataExt;
            metadata.uid()
        }
        Err(error) => {
            eprintln!(
                "[vietc-uinputd] Cannot inspect runtime directory {}: {}",
                runtime_root.display(),
                error
            );
            std::process::exit(1);
        }
    };

    let listener = match bind_socket(&socket_path, 0o600) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("[vietc-uinputd] {}", error);
            std::process::exit(1);
        }
    };

    let uinput = match UinputDevice::new("vietc") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[vietc-uinputd] {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("[vietc-uinputd] Listening on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_client(stream, allowed_uid, &uinput) {
                    eprintln!("[vietc-uinputd] Rejected connection: {}", error);
                }
            }
            Err(e) => {
                eprintln!("[vietc-uinputd] Connection error: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests;
