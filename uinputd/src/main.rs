// SPDX-License-Identifier: MIT
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long to wait after the last Unicode paste before restoring the user's
/// real clipboard. Each paste pushes this deadline back, so a burst of typing
/// only triggers a single restore once the user pauses. This is what keeps the
/// user's clipboard from being pasted into the text mid-typing: we never
/// overwrite our just-pasted word with the user's clipboard while the target
/// app might still be reading it.
const RESTORE_DEBOUNCE: Duration = Duration::from_millis(600);

const UINPUT_MAX_NAME_SIZE: usize = 80;
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_DEV_SETUP: u64 = 0x405c5503;
const EV_KEY: u16 = 0x01;

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

/// Shared clipboard bookkeeping between the command handler and the background
/// restorer thread.
struct ClipInner {
    /// The user's real clipboard contents, saved before we overwrite the
    /// clipboard to paste Unicode text, so we can restore it afterwards.
    saved_clipboard: Option<String>,
    /// The last text we wrote to the clipboard ourselves (an injected word or
    /// the restored user content). Used to distinguish our own writes from
    /// text the user copied with Ctrl+C.
    last_injected: Option<String>,
    /// When set, the restorer thread should rewrite the user's clipboard at
    /// this instant. `None` means no restore is pending.
    restore_due: Option<Instant>,
    /// Set on shutdown so the restorer thread can exit.
    shutdown: bool,
}

struct ClipState {
    inner: Mutex<ClipInner>,
    cv: Condvar,
}

struct UinputDevice {
    fd: i32,
    clip: Arc<ClipState>,
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
            inner: Mutex::new(ClipInner {
                saved_clipboard: None,
                last_injected: None,
                restore_due: None,
                shutdown: false,
            }),
            cv: Condvar::new(),
        });
        {
            let clip = Arc::clone(&clip);
            std::thread::spawn(move || run_restorer(clip));
        }

        eprintln!("[vietc-uinputd] Device '{}' created", name);
        Ok(Self { fd, clip })
    }

    fn send_event(&self, type_: u16, code: u16, value: i32) {
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
            );
        }
    }

    fn send_key(&self, code: u16, value: i32) {
        self.send_event(EV_KEY, code, value);
        self.send_event(0, 0, 0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    fn backspace_n(&self, count: usize) {
        for _ in 0..count {
            self.send_key(14, 1);
            self.send_key(14, 0);
        }
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

    fn type_ascii(&self, text: &str) {
        for byte in text.bytes() {
            if let Some((keycode, shift)) = Self::char_to_keycode(byte) {
                if shift {
                    self.send_key(42, 1);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                self.send_key(keycode, 1);
                self.send_key(keycode, 0);
                if shift {
                    self.send_key(42, 0);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    fn paste_unicode(&self, text: &str) {
        // Critical section: snapshot the clipboard, decide what to preserve,
        // cancel any pending restore so the restorer cannot fire while we are
        // pasting, and put our word on the clipboard. The read and write happen
        // under the lock so they can never interleave with the restorer.
        {
            let mut st = self.clip.inner.lock().unwrap();
            let current = read_clipboard();
            let is_our_write =
                matches!((&current, &st.last_injected), (Some(c), Some(l)) if c == l);
            if !is_our_write {
                // The user changed the clipboard themselves (a real Ctrl+C).
                st.saved_clipboard = current;
            }
            // Cancel any pending restore; the restorer parks until we schedule
            // a new one after the paste.
            st.restore_due = None;
            copy_to_clipboard(text);
            st.last_injected = Some(text.to_string());
        }

        // Give the selection owner a moment to take ownership before pasting.
        std::thread::sleep(std::time::Duration::from_millis(5));

        self.send_key(29, 1);
        std::thread::sleep(std::time::Duration::from_millis(2));
        self.send_key(47, 1);
        self.send_key(47, 0);
        self.send_key(29, 0);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Schedule a debounced restore. While the user keeps typing this gets
        // pushed back, so the user's clipboard is only restored once typing
        // settles — never overwriting our freshly pasted word mid-stream.
        {
            let mut st = self.clip.inner.lock().unwrap();
            st.restore_due = Some(Instant::now() + RESTORE_DEBOUNCE);
        }
        self.clip.cv.notify_all();
    }
}

/// Background thread: once no Unicode paste has happened for `RESTORE_DEBOUNCE`,
/// rewrite the user's real clipboard so Ctrl+V keeps working.
fn run_restorer(state: Arc<ClipState>) {
    loop {
        let mut st = state.inner.lock().unwrap();
        loop {
            if st.shutdown {
                return;
            }
            match st.restore_due {
                None => {
                    st = state.cv.wait(st).unwrap();
                }
                Some(due) => {
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    let (guard, _) = state.cv.wait_timeout(st, due - now).unwrap();
                    st = guard;
                }
            }
        }
        // Deadline reached. Restore under the lock so the write cannot
        // interleave with a concurrent paste's clipboard write.
        let restored = st.saved_clipboard.clone().unwrap_or_default();
        copy_to_clipboard(&restored);
        st.last_injected = Some(restored);
        st.restore_due = None;
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        {
            let mut st = self.clip.inner.lock().unwrap();
            st.shutdown = true;
        }
        self.clip.cv.notify_all();
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

fn copy_to_clipboard(text: &str) {
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    if is_wayland {
        if let Ok(mut child) = Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    } else if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
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

    let reader = BufReader::new(&stream);
    let mut writer = &stream;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let line = line.trim().to_string();
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
        } else if let Some(payload) = line.strip_prefix("REPLACE:") {
            let Some((count, text)) = payload.split_once(':') else {
                let _ = writeln!(writer, "ERR malformed replacement");
                continue;
            };
            let Ok(count) = count.parse::<usize>() else {
                let _ = writeln!(writer, "ERR bad count");
                continue;
            };
            uinput.backspace_n(count);
            let is_ascii = text
                .bytes()
                .all(|byte| UinputDevice::char_to_keycode(byte).is_some());
            if is_ascii {
                uinput.type_ascii(text);
            } else {
                uinput.paste_unicode(text);
            }
            let _ = writeln!(writer, "OK");
        } else if let Some(n_str) = line.strip_prefix("BACKSPACE:") {
            if let Ok(n) = n_str.parse::<usize>() {
                uinput.backspace_n(n);
                let _ = writeln!(writer, "OK");
            } else {
                let _ = writeln!(writer, "ERR bad count");
            }
        } else if let Some(text) = line.strip_prefix("TYPE:") {
            let is_ascii = text
                .bytes()
                .all(|b| UinputDevice::char_to_keycode(b).is_some());
            if is_ascii {
                uinput.type_ascii(text);
            } else {
                uinput.paste_unicode(text);
            }
            let _ = writeln!(writer, "OK");
        } else if let Some(text) = line.strip_prefix("PASTE:") {
            uinput.paste_unicode(text);
            let _ = writeln!(writer, "OK");
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
mod phase0_tests {
    use super::*;

    #[test]
    fn authorized_peer_reaches_dispatcher() {
        let mut dispatched = false;

        let result = authorize_and_dispatch(1000, 1000, || {
            dispatched = true;
        });

        assert!(result.is_ok());
        assert!(dispatched);
    }

    #[test]
    fn unauthorized_peer_is_rejected_before_dispatch() {
        let mut dispatched = false;

        let result = authorize_and_dispatch(1001, 1000, || {
            dispatched = true;
        });

        assert!(result.is_err());
        assert!(!dispatched);
    }

    #[test]
    fn socket_permissions_are_applied_after_bind() {
        use std::os::unix::fs::PermissionsExt;

        let test_dir =
            std::env::temp_dir().join(format!("vietc-uinputd-phase1-{}", std::process::id()));
        let socket_path = test_dir.join("uinput.sock");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir(&test_dir).unwrap();

        let listener = bind_socket(&socket_path, 0o600).unwrap();
        let mode = fs::metadata(&socket_path).unwrap().permissions().mode() & 0o777;

        assert_eq!(mode, 0o600);

        drop(listener);
        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn bind_failure_does_not_remove_preexisting_regular_file() {
        let test_dir = std::env::temp_dir().join(format!(
            "vietc-uinputd-phase1-existing-{}",
            std::process::id()
        ));
        let socket_path = test_dir.join("uinput.sock");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir(&test_dir).unwrap();
        fs::write(&socket_path, b"keep me").unwrap();

        let result = bind_socket(&socket_path, 0o600);

        assert!(result.is_err());
        assert_eq!(fs::read(&socket_path).unwrap(), b"keep me");

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn runtime_socket_path_is_private_and_canonical() {
        let runtime_root = Path::new("/run/user/1000");

        let socket_path = runtime_socket_path(runtime_root);

        assert_eq!(socket_path, runtime_root.join("vietc").join("uinput.sock"));
    }

    #[test]
    fn authorized_unix_peer_reaches_dispatcher() {
        let test_dir =
            std::env::temp_dir().join(format!("vietc-uinputd-phase2-peer-{}", std::process::id()));
        let socket_path = test_dir.join("uinput.sock");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir(&test_dir).unwrap();

        let listener = bind_socket(&socket_path, 0o600).unwrap();
        let client = UnixStream::connect(&socket_path).unwrap();
        let (server, _) = listener.accept().unwrap();
        let expected_uid = unsafe { libc::getuid() };
        let mut dispatched = false;

        authorize_stream_and_dispatch(&server, expected_uid, || {
            dispatched = true;
        })
        .unwrap();

        assert!(dispatched);

        drop(client);
        drop(server);
        drop(listener);
        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn unix_peer_is_rejected_before_dispatch_when_uid_is_not_allowed() {
        let test_dir = std::env::temp_dir().join(format!(
            "vietc-uinputd-phase2-reject-{}",
            std::process::id()
        ));
        let socket_path = test_dir.join("uinput.sock");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir(&test_dir).unwrap();

        let listener = bind_socket(&socket_path, 0o600).unwrap();
        let client = UnixStream::connect(&socket_path).unwrap();
        let (server, _) = listener.accept().unwrap();
        let current_uid = unsafe { libc::getuid() };
        let denied_uid = current_uid.saturating_add(1);
        let mut dispatched = false;

        let result = authorize_stream_and_dispatch(&server, denied_uid, || {
            dispatched = true;
        });

        assert!(result.is_err());
        assert!(!dispatched);

        drop(client);
        drop(server);
        drop(listener);
        fs::remove_dir_all(&test_dir).unwrap();
    }
}
