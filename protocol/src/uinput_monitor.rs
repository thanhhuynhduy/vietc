// SPDX-License-Identifier: MIT
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::inject::{InjectResult, KeyInjector};

/// How long to wait after the last Unicode paste before restoring the user's
/// real clipboard. Each paste pushes this deadline back, so a burst of typing
/// only triggers a single restore once the user pauses — the user's clipboard
/// is never pasted into the text while the target app might still be reading
/// our freshly pasted word.
const CLIPBOARD_READY_TIMEOUT: Duration = Duration::from_millis(750);
const CLIPBOARD_READY_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PASTE_OBSERVATION_DELAY: Duration = Duration::from_millis(20);
const CLIPBOARD_TRACE_LIMIT: usize = 128;
static NEXT_CLIPBOARD_SEQUENCE_ID: AtomicU64 = AtomicU64::new(1);
static CLIPBOARD_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn trace_clipboard(sequence_id: u64, phase: &str, detail: &str) {
    let trace_index = CLIPBOARD_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    if trace_index < CLIPBOARD_TRACE_LIMIT {
        eprintln!(
            "[vietc] clipboard trace={} sequence_id={} phase={} {}",
            trace_index, sequence_id, phase, detail
        );
    }
}

const UINPUT_MAX_NAME_SIZE: usize = 80;
const UI_SET_EVBIT: u64 = 0x40045564;
const UI_SET_KEYBIT: u64 = 0x40045565;
#[allow(dead_code)]
const UI_SET_ABSBIT: u64 = 0x40045566;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const UI_DEV_SETUP: u64 = 0x405c5503;
const EV_KEY: u16 = 0x01;
#[allow(dead_code)]
const EV_ABS: u16 = 0x03;
/// Maximum keyboard keycode (0-255). Stops before mouse button codes
/// (BTN_MISC = 256, BTN_LEFT = 272) to avoid X11 treating the uinput
/// device as a pointer.
const KEYBOARD_KEY_MAX: u16 = 255;

#[derive(Debug, Clone)]
struct DesktopSession {
    uid: u32,
    gid: u32,
    username: String,
    home: PathBuf,
    xdg_runtime_dir: Option<PathBuf>,
    display: Option<String>,
    wayland_display: Option<String>,
    xauthority: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct SessionPathMetadata {
    owner_uid: u32,
    is_directory: bool,
    is_regular_file: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegeDropStep {
    ClearSupplementaryGroups,
    SetGid,
    SetUid,
}

#[cfg(test)]
fn privilege_drop_steps() -> [PrivilegeDropStep; 3] {
    [
        PrivilegeDropStep::ClearSupplementaryGroups,
        PrivilegeDropStep::SetGid,
        PrivilegeDropStep::SetUid,
    ]
}

#[derive(Debug)]
struct PreparedDesktopCommand {
    program: String,
    uid: u32,
    gid: u32,
    environment: std::collections::BTreeMap<String, String>,
}

fn validate_display(display: &str) -> std::io::Result<()> {
    let valid = display.starts_with(':')
        && display
            .chars()
            .skip(1)
            .all(|character| character.is_ascii_digit() || character == '.');
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "DISPLAY is not a trusted local display identifier",
        ))
    }
}

fn validate_wayland_display(display: &str) -> std::io::Result<()> {
    let valid = !display.is_empty()
        && !display.contains('/')
        && display != "."
        && display != ".."
        && display.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "WAYLAND_DISPLAY is not a trusted socket name",
        ))
    }
}

fn validate_runtime_dir(
    path: &Path,
    uid: u32,
    metadata: impl FnOnce(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<()> {
    let expected = PathBuf::from(format!("/run/user/{uid}"));
    let details = metadata(path)?;
    if path != expected || details.owner_uid != uid || !details.is_directory {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "XDG_RUNTIME_DIR is not the target user's trusted runtime directory",
        ));
    }
    Ok(())
}

fn validate_xauthority(
    path: &Path,
    session: &DesktopSession,
    metadata: impl FnOnce(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<()> {
    let details = metadata(path)?;
    if !path.is_absolute()
        || !path.starts_with(&session.home)
        || details.owner_uid != session.uid
        || !details.is_regular_file
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "XAUTHORITY is not a trusted user-owned file",
        ));
    }
    Ok(())
}

fn prepare_desktop_command(
    program: &str,
    session: DesktopSession,
) -> std::io::Result<PreparedDesktopCommand> {
    if session.uid == 0 || session.gid == 0 || session.username.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "a non-root desktop session identity is required",
        ));
    }
    if program.is_empty() || program.contains('/') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "desktop command must be an executable name",
        ));
    }

    let mut environment = std::collections::BTreeMap::new();
    if let Some(display) = session.display.as_deref() {
        validate_display(display)?;
        environment.insert("DISPLAY".to_string(), display.to_string());
    }
    if let Some(display) = session.wayland_display.as_deref() {
        validate_wayland_display(display)?;
        environment.insert("WAYLAND_DISPLAY".to_string(), display.to_string());
    }
    if let Some(runtime_dir) = session.xdg_runtime_dir.as_deref() {
        environment.insert(
            "XDG_RUNTIME_DIR".to_string(),
            runtime_dir.to_string_lossy().into_owned(),
        );
    }
    if let Some(xauthority) = session.xauthority.as_deref() {
        environment.insert(
            "XAUTHORITY".to_string(),
            xauthority.to_string_lossy().into_owned(),
        );
    }
    environment.insert(
        "HOME".to_string(),
        session.home.to_string_lossy().into_owned(),
    );
    environment.insert("USER".to_string(), session.username);

    Ok(PreparedDesktopCommand {
        program: program.to_string(),
        uid: session.uid,
        gid: session.gid,
        environment,
    })
}

fn build_desktop_command_with(
    program: &str,
    is_root: bool,
    resolve_session: impl FnOnce() -> Option<DesktopSession>,
    metadata: impl Fn(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<std::process::Command> {
    if !is_root {
        return Ok(std::process::Command::new(program));
    }

    let session = resolve_session().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "a non-root desktop session identity is required",
        )
    })?;

    if let Some(runtime_dir) = session.xdg_runtime_dir.as_deref() {
        validate_runtime_dir(runtime_dir, session.uid, &metadata)?;
    }
    if let Some(xauthority) = session.xauthority.as_deref() {
        validate_xauthority(xauthority, &session, &metadata)?;
    }

    let prepared = prepare_desktop_command(program, session)?;
    let mut command = std::process::Command::new(&prepared.program);
    command.env_clear();
    command.envs(&prepared.environment);

    use std::os::unix::process::CommandExt;
    let uid = prepared.uid;
    let gid = prepared.gid;
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    Ok(command)
}

/// Largest non-text clipboard payload we are willing to back up and restore.
const MAX_BINARY_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;

/// Clipboard contents that are not plain text and therefore cannot go through
/// the text snapshot path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NonTextClipboard {
    Empty,
    Data { mime: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardKind {
    Empty,
    Text,
    Other(String),
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/plain") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// Decide how a clipboard offering `types` must be backed up: plain text is
/// preserved as text; anything else (image, file list...) is preserved as raw
/// bytes of its first offered type; nothing offered means an empty clipboard.
fn classify_clipboard_types(types: &[String]) -> ClipboardKind {
    if types.is_empty() {
        ClipboardKind::Empty
    } else if types.iter().any(|mime| is_text_mime(mime)) {
        ClipboardKind::Text
    } else {
        ClipboardKind::Other(types[0].clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplacementPhase {
    Prepared,
    ClipboardWritten,
    PasteDispatched,
    TargetObserved,
    Applied,
    ClipboardRestored,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
enum ClipboardAction {
    WriteInjected(String),
    Restore(String),
    SkipRestore,
}

struct ClipboardTransaction {
    id: u64,
    focus_generation: u64,
    saved_clipboard: Option<String>,
    injected_clipboard: String,
    phase: ReplacementPhase,
    started: Instant,
}

#[derive(Default)]
struct ClipboardBurstState {
    transaction: Option<ClipboardTransaction>,
    last_unrestored_injected: Option<String>,
}

impl ClipboardBurstState {
    fn begin_injection(
        &mut self,
        current_clipboard: Option<String>,
        injected_text: &str,
        focus_generation: u64,
    ) -> ClipboardAction {
        let filtered_clipboard = current_clipboard.filter(|clip| {
            self.last_unrestored_injected.as_deref() != Some(clip.as_str())
        });
        let saved_clipboard = self
            .transaction
            .as_ref()
            .and_then(|transaction| transaction.saved_clipboard.clone())
            .or(filtered_clipboard);
        self.last_unrestored_injected = None;
        self.transaction = Some(ClipboardTransaction {
            id: NEXT_CLIPBOARD_SEQUENCE_ID.fetch_add(1, Ordering::Relaxed),
            focus_generation,
            saved_clipboard,
            injected_clipboard: injected_text.to_string(),
            phase: ReplacementPhase::Prepared,
            started: Instant::now(),
        });
        ClipboardAction::WriteInjected(injected_text.to_string())
    }

    fn transition(&mut self, transaction_id: u64, phase: ReplacementPhase) -> bool {
        let Some(transaction) = self.transaction.as_mut() else {
            return false;
        };
        if transaction.id != transaction_id {
            return false;
        }

        let valid = matches!(
            (transaction.phase, phase),
            (
                ReplacementPhase::Prepared,
                ReplacementPhase::ClipboardWritten
            ) | (
                ReplacementPhase::ClipboardWritten,
                ReplacementPhase::PasteDispatched
            ) | (
                ReplacementPhase::PasteDispatched,
                ReplacementPhase::TargetObserved
            ) | (ReplacementPhase::TargetObserved, ReplacementPhase::Applied)
                | (
                    ReplacementPhase::Applied,
                    ReplacementPhase::ClipboardRestored
                )
                | (_, ReplacementPhase::Failed)
        );
        if valid {
            transaction.phase = phase;
        }
        valid
    }

    fn mark_failed(&mut self, transaction_id: u64) {
        let _ = self.transition(transaction_id, ReplacementPhase::Failed);
    }

    fn finish_restore(&mut self, current_clipboard: Option<&str>) -> ClipboardAction {
        let Some(transaction) = self.transaction.as_ref() else {
            return ClipboardAction::SkipRestore;
        };
        if transaction.phase != ReplacementPhase::Applied {
            return ClipboardAction::SkipRestore;
        }

        let still_contains_injected =
            current_clipboard == Some(transaction.injected_clipboard.as_str());
        if !still_contains_injected {
            self.reset();
            return ClipboardAction::SkipRestore;
        }

        match transaction.saved_clipboard.clone() {
            Some(saved) => ClipboardAction::Restore(saved),
            None => {
                let injected = transaction.injected_clipboard.clone();
                self.reset();
                self.last_unrestored_injected = Some(injected);
                ClipboardAction::SkipRestore
            }
        }
    }

    fn forget_unrestored(&mut self) {
        self.last_unrestored_injected = None;
    }

    fn complete_restore(&mut self, transaction_id: u64) -> bool {
        if !self.transition(transaction_id, ReplacementPhase::ClipboardRestored) {
            return false;
        }
        self.reset();
        self.last_unrestored_injected = None;
        true
    }

    fn is_active(&self) -> bool {
        self.transaction.is_some()
    }

    fn transaction_id(&self) -> Option<u64> {
        self.transaction.as_ref().map(|transaction| transaction.id)
    }

    fn matches(&self, transaction_id: u64, focus_generation: u64) -> bool {
        self.transaction.as_ref().is_some_and(|transaction| {
            transaction.id == transaction_id && transaction.focus_generation == focus_generation
        })
    }

    fn cancel_for_focus_change(&mut self, focus_generation: u64) -> bool {
        let should_cancel = self.transaction.as_ref().is_some_and(|transaction| {
            transaction.focus_generation != focus_generation
                && transaction.phase != ReplacementPhase::Applied
                && transaction.phase != ReplacementPhase::ClipboardRestored
        });
        if should_cancel {
            self.transaction = None;
        }
        should_cancel
    }

    #[cfg(test)]
    fn phase(&self) -> Option<ReplacementPhase> {
        self.transaction
            .as_ref()
            .map(|transaction| transaction.phase)
    }

    fn elapsed(&self) -> Option<Duration> {
        self.transaction
            .as_ref()
            .map(|transaction| transaction.started.elapsed())
    }

    fn reset(&mut self) {
        self.transaction = None;
    }
}

fn write_all_with(
    mut remaining: &[u8],
    mut write: impl FnMut(&[u8]) -> std::io::Result<usize>,
) -> std::io::Result<()> {
    while !remaining.is_empty() {
        match write(remaining) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write uinput event data",
                ));
            }
            Ok(written) if written <= remaining.len() => {
                remaining = &remaining[written..];
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "uinput writer reported more bytes than requested",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

fn serialize_transaction<T>(
    transaction_lock: &Mutex<()>,
    operation: impl FnOnce() -> T,
) -> std::io::Result<T> {
    let _transaction = transaction_lock
        .lock()
        .map_err(|_| std::io::Error::other("uinput transaction lock poisoned"))?;
    Ok(operation())
}

/// Shared clipboard bookkeeping between the injection path and the background
/// restorer thread.
struct ClipInner {
    burst: ClipboardBurstState,
    /// When set, the restorer thread should rewrite the user's clipboard at
    /// this instant. `None` means no restore is pending.
    restore_due: Option<Instant>,
    /// Set on shutdown so the restorer thread can exit.
    shutdown: bool,
    /// The currently active window class (lowercase).
    active_window: String,
}

struct ClipState {
    inner: Mutex<ClipInner>,
    io_lock: Mutex<()>,
    cv: Condvar,
    focus_generation: Mutex<Option<Arc<AtomicU64>>>,
}

struct ClipboardRestorer {
    state: Arc<ClipState>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ClipboardRestorer {
    fn spawn(state: Arc<ClipState>, worker: impl FnOnce(Arc<ClipState>) + Send + 'static) -> Self {
        let worker_state = Arc::clone(&state);
        let handle = std::thread::spawn(move || worker(worker_state));
        Self {
            state,
            handle: Some(handle),
        }
    }

    fn shutdown_and_join(&mut self) -> std::thread::Result<()> {
        {
            let mut inner = self.state.inner.lock().unwrap();
            inner.shutdown = true;
            inner.restore_due = None;
            inner.burst.reset();
        }
        self.state.cv.notify_all();

        if let Some(handle) = self.handle.take() {
            handle.join()
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    fn is_joined(&self) -> bool {
        self.handle.is_none()
    }
}

pub struct UinputInjector {
    file: File,
    transaction_lock: Mutex<()>,
    clip: Arc<ClipState>,
    restorer: ClipboardRestorer,
    caps_lock: std::sync::atomic::AtomicBool,
    terminal_app: std::sync::atomic::AtomicBool,
}

impl UinputInjector {
    fn send_enter(&self) -> std::io::Result<()> {
        serialize_transaction(&self.transaction_lock, || {
            self.send_uinput_event_unlocked(EV_KEY, 28, 1)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(2));

            self.send_uinput_event_unlocked(EV_KEY, 28, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(2));
            Ok(())
        })?
    }

    pub fn new(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Ensure uinput kernel module is loaded before trying to open the device
        let _ = std::process::Command::new("modprobe")
            .args(["uinput"])
            .output();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")?;

        let fd = file.as_raw_fd();

        // Enable EV_KEY
        ioctl(fd, UI_SET_EVBIT, EV_KEY as u64)
            .map_err(|e| format!("UI_SET_EVBIT failed: {}", e))?;

        // Enable all keyboard keycodes we'll need (0-255).
        // KEY_MAX (0x1ff) includes mouse button codes (BTN_LEFT=272 etc.)
        // which would make X11 treat this device as a pointer and conflict
        // with the real mouse.
        for code in 0..=KEYBOARD_KEY_MAX {
            ioctl(fd, UI_SET_KEYBIT, code as u64)
                .map_err(|e| format!("UI_SET_KEYBIT {} failed: {}", code, e))?;
        }

        // Create uinput device
        let mut usetup: uinput_setup = unsafe { std::mem::zeroed() };
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(UINPUT_MAX_NAME_SIZE - 1);
        for (i, &byte) in name_bytes.iter().enumerate().take(copy_len) {
            usetup.name[i] = byte as i8;
        }
        usetup.name[copy_len] = 0;
        usetup.id.bustype = 0x03; // BUS_USB
        usetup.id.vendor = 0x1234;
        usetup.id.product = 0x5678;
        usetup.id.version = 1;

        ioctl(fd, UI_DEV_SETUP, &usetup as *const uinput_setup as u64)
            .map_err(|e| format!("UI_DEV_SETUP failed: {}", e))?;

        ioctl(fd, UI_DEV_CREATE, 0).map_err(|e| format!("UI_DEV_CREATE failed: {}", e))?;

        // Small delay for device to be ready
        std::thread::sleep(std::time::Duration::from_millis(10));

        let clip = Arc::new(ClipState {
            inner: Mutex::new(ClipInner {
                burst: ClipboardBurstState::default(),
                restore_due: None,
                shutdown: false,
                active_window: String::new(),
            }),
            io_lock: Mutex::new(()),
            cv: Condvar::new(),
            focus_generation: Mutex::new(None),
        });
        let restorer = ClipboardRestorer::spawn(Arc::clone(&clip), run_restorer);

        Ok(Self {
            file,
            transaction_lock: Mutex::new(()),
            clip,
            restorer,
            caps_lock: std::sync::atomic::AtomicBool::new(false),
            terminal_app: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn write_events_unlocked(&self, events: &[input_event]) -> std::io::Result<()> {
        let bytes = unsafe {
            std::slice::from_raw_parts(events.as_ptr() as *const u8, std::mem::size_of_val(events))
        };

        write_all_with(bytes, |remaining| {
            let written = unsafe {
                libc::write(
                    self.file.as_raw_fd(),
                    remaining.as_ptr() as *const libc::c_void,
                    remaining.len(),
                )
            };
            if written < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        })
    }

    fn send_uinput_event_unlocked(&self, type_: u16, code: u16, value: i32) -> std::io::Result<()> {
        let event = input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };

        self.write_events_unlocked(&[event])
    }

    fn write_events(&self, events: &[input_event]) -> std::io::Result<()> {
        serialize_transaction(&self.transaction_lock, || {
            self.write_events_unlocked(events)
        })?
    }

    fn send_key_stroke(&self, keycode: u16, shift: bool) -> std::io::Result<()> {
        let mut events = Vec::with_capacity(8);
        if shift {
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: EV_KEY,
                code: 42,
                value: 1,
            });
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: 0,
                code: 0,
                value: 0,
            });
        }

        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_KEY,
            code: keycode,
            value: 1,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: 0,
            code: 0,
            value: 0,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: EV_KEY,
            code: keycode,
            value: 0,
        });
        events.push(input_event {
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: 0,
            code: 0,
            value: 0,
        });

        if shift {
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: EV_KEY,
                code: 42,
                value: 0,
            });
            events.push(input_event {
                time: timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_: 0,
                code: 0,
                value: 0,
            });
        }

        self.write_events(&events)
    }

    fn send_backspaces(&self, count: usize) -> std::io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let mut remaining = count;
        while remaining > 0 {
            let chunk = remaining.min(4);
            let mut events = Vec::with_capacity(chunk * 4);
            for _ in 0..chunk {
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: EV_KEY,
                    code: 14,
                    value: 1,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: 0,
                    code: 0,
                    value: 0,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: EV_KEY,
                    code: 14,
                    value: 0,
                });
                events.push(input_event {
                    time: timeval {
                        tv_sec: 0,
                        tv_usec: 0,
                    },
                    type_: 0,
                    code: 0,
                    value: 0,
                });
            }
            self.write_events(&events)?;
            remaining -= chunk;
            if remaining > 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
}

impl KeyInjector for UinputInjector {
    fn send_backspace(&self) -> InjectResult {
        if self.send_backspaces(1).is_ok() {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let result = serialize_transaction(&self.transaction_lock, || {
            self.send_uinput_event_unlocked(EV_KEY, keycode, value)?;
            self.send_uinput_event_unlocked(0, 0, 0)
        });
        if value == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
        if matches!(result, Ok(Ok(()))) {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let caps_active = self.caps_lock.load(Ordering::Relaxed);
        if let Some(keycode) = char_to_linux_keycode(ch) {
            let needs_shift = if ch.is_ascii_alphabetic() {
                ch.is_uppercase() ^ caps_active
            } else {
                "!@#$%^&*()_+{}|:\"<>?".contains(ch)
            };
            return if self.send_key_stroke(keycode, needs_shift).is_ok() {
                InjectResult::Success
            } else {
                InjectResult::Failed
            };
        }
        // Vietnamese Unicode char: map to base ASCII and send via uinput
        let ascii = strip_vn_diacritic(ch);
        if let Some(keycode) = char_to_linux_keycode(ascii) {
            let needs_shift = if ascii.is_ascii_alphabetic() {
                ascii.is_uppercase() ^ caps_active
            } else {
                ascii.is_uppercase()
            };
            return if self.send_key_stroke(keycode, needs_shift).is_ok() {
                InjectResult::Success
            } else {
                InjectResult::Failed
            };
        }
        InjectResult::NotSupported
    }

    fn send_string(&self, s: &str) -> InjectResult {
        // ASCII characters: inject directly via uinput keycodes
        let is_ascii = s.chars().all(|c| char_to_linux_keycode(c).is_some());

        if is_ascii {
            for ch in s.chars() {
                let result = self.send_char(ch);
                if result != InjectResult::Success {
                    return result;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            return InjectResult::Success;
        }

        // Fallback: clipboard copy + paste
        if self.paste_via_clipboard(s, self.current_focus_generation()) {
            InjectResult::Success
        } else {
            eprintln!("[vietc] Unicode insertion failed: clipboard unavailable");
            InjectResult::Failed
        }
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        self.inject_replacement_atomic(backspaces, text, self.current_focus_generation())
    }

    fn bind_focus_generation(&self, generation: Arc<AtomicU64>) -> InjectResult {
        *self.clip.focus_generation.lock().unwrap() = Some(generation);
        InjectResult::Success
    }

    fn cancel_pending_replacement(&self, focus_generation: u64, reason: &str) -> InjectResult {
        let mut state = self.clip.inner.lock().unwrap();
        if state.burst.cancel_for_focus_change(focus_generation) {
            state.restore_due = None;
            trace_clipboard(
                0,
                "cancelled",
                &format!("focus_generation={focus_generation} reason={reason}"),
            );
        }
        self.clip.cv.notify_all();
        InjectResult::Success
    }

    fn inject_replacement_for_generation(
        &self,
        backspaces: usize,
        text: &str,
        focus_generation: u64,
    ) -> InjectResult {
        if self.current_focus_generation() != focus_generation {
            return InjectResult::Failed;
        }
        self.inject_replacement_atomic(backspaces, text, focus_generation)
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }

    /// Record that Unicode text was pasted via clipboard (for future delete/backspace support)
    fn update_pasted_text(&self, text: &str) -> InjectResult {
        // Text tracking happens through OutputCommand pipeline in daemon
        // This is called after clipboard paste to inform engine of pasted content
        eprintln!(
            "[vietc] update_pasted_text: recorded '{}' (len={})",
            text.escape_default(),
            text.len()
        );
        InjectResult::Success
    }

    fn set_active_window(&self, window_class: &str) -> InjectResult {
        let mut st = self.clip.inner.lock().unwrap();
        st.active_window = window_class.to_lowercase();
        InjectResult::Success
    }

    fn set_caps_lock(&self, enabled: bool) -> InjectResult {
        self.caps_lock.store(enabled, Ordering::Relaxed);
        InjectResult::Success
    }

    fn set_terminal_app(&self, is_terminal: bool) -> InjectResult {
        self.terminal_app.store(is_terminal, Ordering::Relaxed);
        InjectResult::Success
    }
}

impl UinputInjector {
    fn current_focus_generation(&self) -> u64 {
        self.clip
            .focus_generation
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |generation| generation.load(Ordering::Acquire))
    }

    /// Get the original non-root username when running as root.
    /// Checks SUDO_USER (sudo), PKEXEC_UID (pkexec), /proc/self/loginuid,
    /// and falls back to `logname`.
    fn get_original_username() -> Option<String> {
        let is_root = unsafe { libc::getuid() == 0 };
        if !is_root {
            return None;
        }

        if let Ok(user) = std::env::var("SUDO_USER") {
            if !user.is_empty() {
                return Some(user);
            }
        }

        if let Ok(uid_str) = std::env::var("PKEXEC_UID") {
            if let Ok(uid) = uid_str.parse::<u32>() {
                unsafe {
                    let pw = libc::getpwuid(uid);
                    if !pw.is_null() {
                        let name = std::ffi::CStr::from_ptr((*pw).pw_name)
                            .to_string_lossy()
                            .into_owned();
                        if !name.is_empty() {
                            return Some(name);
                        }
                    }
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string("/proc/self/loginuid") {
            if let Ok(uid) = content.trim().parse::<u32>() {
                if uid != 4294967295 {
                    unsafe {
                        let pw = libc::getpwuid(uid);
                        if !pw.is_null() {
                            let name = std::ffi::CStr::from_ptr((*pw).pw_name)
                                .to_string_lossy()
                                .into_owned();
                            if !name.is_empty() {
                                return Some(name);
                            }
                        }
                    }
                }
            }
        }

        if let Ok(output) = std::process::Command::new("logname").output() {
            if output.status.success() {
                let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }

        None
    }

    /// Get original non-root UID and GID when running as root.
    fn get_original_uid_gid() -> Option<(u32, u32)> {
        let is_root = unsafe { libc::getuid() == 0 };
        if !is_root {
            return None;
        }

        let mut target_uid = None;

        if let Ok(uid_str) = std::env::var("SUDO_UID") {
            if let Ok(uid) = uid_str.parse::<u32>() {
                target_uid = Some(uid);
            }
        }

        if target_uid.is_none() {
            if let Ok(uid_str) = std::env::var("PKEXEC_UID") {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    target_uid = Some(uid);
                }
            }
        }

        if target_uid.is_none() {
            if let Ok(content) = std::fs::read_to_string("/proc/self/loginuid") {
                if let Ok(uid) = content.trim().parse::<u32>() {
                    if uid != 4294967295 {
                        target_uid = Some(uid);
                    }
                }
            }
        }

        if let Some(uid) = target_uid {
            unsafe {
                let pw = libc::getpwuid(uid);
                if !pw.is_null() {
                    let gid = (*pw).pw_gid;
                    return Some((uid, gid));
                }
            }
        }

        None
    }

    /// Send backspaces and replacement text through one uinput device.
    /// ASCII is injected directly. Unicode is copied to the clipboard and
    /// pasted with Ctrl+V through the same device, avoiding mixed-channel
    /// editing transactions.
    fn inject_replacement_atomic(
        &self,
        backspaces: usize,
        text: &str,
        focus_generation: u64,
    ) -> InjectResult {
        if text.is_empty() {
            if backspaces == 0 || self.send_backspaces(backspaces).is_ok() {
                return InjectResult::Success;
            }
            return InjectResult::Failed;
        }

        // Fast path: pure ASCII (including newline) via uinput batch — single channel
        if text
            .chars()
            .all(|c| char_to_linux_keycode(c).is_some() || c == '\n')
        {
            if backspaces > 0 {
                if self.send_backspaces(backspaces).is_err() {
                    return InjectResult::Failed;
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            for ch in text.chars() {
                if ch == '\n' {
                    if self.send_enter().is_err() {
                        return InjectResult::Failed;
                    }
                } else if self.send_char(ch) != InjectResult::Success {
                    return InjectResult::Failed;
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            return InjectResult::Success;
        }

        // Keep delete and insert on the same uinput channel. Mixing uinput
        // Backspace with Wayland virtual-keyboard text can crash Microsoft Edge
        // with SIGILL even for a minimal replacement such as `Da` -> `Dạ`.
        if backspaces > 0 {
            if self.send_backspaces(backspaces).is_err() {
                return InjectResult::Failed;
            }
            // Let the focused application consume the complete delete phase
            // before Ctrl+V starts the insert phase. Without this boundary,
            // Chromium-family clients can process the paste after only part of
            // the Backspace batch, leaving the first raw character duplicated.
            std::thread::sleep(Duration::from_millis(10));
        }

        if self.paste_via_clipboard(text, focus_generation) {
            InjectResult::Success
        } else {
            eprintln!("[vietc] Unicode insertion failed: clipboard unavailable");
            InjectResult::Failed
        }
    }

    /// Read the user's current clipboard contents (wl-paste on Wayland, xclip
    /// on X11). Returns None if no clipboard tool is available or it is empty.
    fn read_clipboard() -> Option<String> {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let (prog, args): (&str, &[&str]) = if is_wayland {
            ("wl-paste", &["-n"])
        } else {
            ("xclip", &["-selection", "clipboard", "-o"])
        };
        let mut cmd = Self::user_cmd(prog).ok()?;
        cmd.args(args);
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// MIME types currently offered on the clipboard. `Some(vec![])` means the
    /// clipboard is empty; `None` means it could not be queried.
    fn clipboard_types() -> Option<Vec<String>> {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        if !is_wayland {
            return None;
        }
        let mut cmd = Self::user_cmd("wl-paste").ok()?;
        cmd.args(["-l"]);
        let output = cmd.output().ok()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return stderr
                .contains("Nothing is copied")
                .then(Vec::new);
        }
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }

    /// Snapshot a clipboard that does not hold plain text (empty, image,
    /// file list...). Plain-text clipboards use the text path instead.
    fn backup_non_text_clipboard() -> Option<NonTextClipboard> {
        let types = Self::clipboard_types()?;
        match classify_clipboard_types(&types) {
            ClipboardKind::Text => None,
            ClipboardKind::Empty => Some(NonTextClipboard::Empty),
            ClipboardKind::Other(mime) => {
                let mut cmd = Self::user_cmd("wl-paste").ok()?;
                cmd.args(["--type", &mime]);
                let output = cmd.output().ok()?;
                if !output.status.success() || output.stdout.len() > MAX_BINARY_CLIPBOARD_BYTES {
                    return None;
                }
                Some(NonTextClipboard::Data {
                    mime,
                    bytes: output.stdout,
                })
            }
        }
    }

    fn restore_non_text_clipboard(backup: &NonTextClipboard) -> bool {
        let Ok(mut cmd) = Self::user_cmd("wl-copy") else {
            return false;
        };
        match backup {
            NonTextClipboard::Empty => {
                cmd.arg("--clear");
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                cmd.status().map(|status| status.success()).unwrap_or(false)
            }
            NonTextClipboard::Data { mime, bytes } => {
                use std::io::Write;
                cmd.args(["--type", mime]);
                cmd.stdin(std::process::Stdio::piped());
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                let Ok(mut child) = cmd.spawn() else {
                    return false;
                };
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(bytes);
                }
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                true
            }
        }
    }

    /// Read the clipboard, retrying briefly while it does not yet show
    /// `expected`. A transient read failure right after a paste must not be
    /// mistaken for "the user changed the clipboard", which would skip the
    /// restore and leave our injected text in the user's clipboard.
    fn read_clipboard_expecting(expected: &str) -> Option<String> {
        let mut last = None;
        for attempt in 0..4 {
            last = Self::read_clipboard();
            if last.as_deref() == Some(expected) {
                return last;
            }
            if attempt < 3 {
                std::thread::sleep(CLIPBOARD_READY_POLL_INTERVAL * 2);
            }
        }
        last
    }

    /// Inject Unicode `text` by placing it on the clipboard and sending Ctrl+V,
    /// while preserving the user's own clipboard contents. Without this, every
    /// Vietnamese word the user types would overwrite whatever they had copied
    /// with Ctrl+C, so a subsequent Ctrl+V would paste the wrong thing.
    ///
    /// Returns whether the text was successfully copied to the clipboard.
    fn paste_via_clipboard(&self, text: &str, focus_generation: u64) -> bool {
        let t_total = std::time::Instant::now();
        let _clipboard_io = self.clip.io_lock.lock().unwrap();

        let needs_snapshot = {
            let mut st = self.clip.inner.lock().unwrap();
            st.restore_due = None;
            !st.burst.is_active()
        };
        let non_text_backup = if needs_snapshot {
            Self::backup_non_text_clipboard()
        } else {
            None
        };
        let current_clipboard = (needs_snapshot && non_text_backup.is_none())
            .then(Self::read_clipboard)
            .flatten();

        let sequence_id = {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst
                .begin_injection(current_clipboard.clone(), text, focus_generation);
            st.burst.transaction_id().unwrap()
        };
        trace_clipboard(
            sequence_id,
            "prepared",
            if current_clipboard.is_some() {
                "clipboard=present"
            } else {
                "clipboard=empty-or-unavailable"
            },
        );

        trace_clipboard(sequence_id, "prepared", "state=writing");
        if !Self::copy_to_clipboard(text) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=clipboard-write");
            st.burst.reset();
            return false;
        }

        if !Self::wait_for_clipboard_value(text) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            let elapsed_ms = st.burst.elapsed().map_or(0, |elapsed| elapsed.as_millis());
            trace_clipboard(
                sequence_id,
                "failed",
                &format!("phase=clipboard-read-back elapsed_ms={elapsed_ms}"),
            );
            st.burst.reset();
            return false;
        }
        {
            let mut st = self.clip.inner.lock().unwrap();
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::ClipboardWritten);
            debug_assert!(transitioned);
        }
        trace_clipboard(sequence_id, "clipboard_written", "state=verified");

        if self.send_ctrl_v().is_err() {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            trace_clipboard(sequence_id, "failed", "phase=paste-dispatch");
            st.burst.reset();
            return false;
        }
        {
            let mut st = self.clip.inner.lock().unwrap();
            let transitioned = st
                .burst
                .transition(sequence_id, ReplacementPhase::PasteDispatched);
            debug_assert!(transitioned);
            st.restore_due = Some(Instant::now() + CLIPBOARD_READY_TIMEOUT);
        }
        trace_clipboard(sequence_id, "paste_dispatched", "state=sent");

        if !Self::wait_for_target_consumption(&self.clip, sequence_id, text, focus_generation) {
            let mut st = self.clip.inner.lock().unwrap();
            st.burst.mark_failed(sequence_id);
            st.restore_due = None;
            trace_clipboard(sequence_id, "failed", "phase=target-observation");
            st.burst.reset();
            return false;
        }
        trace_clipboard(sequence_id, "target_observed", "state=clipboard-consumed");
        trace_clipboard(sequence_id, "applied", "state=ready-to-restore");

        let current_clipboard = Self::read_clipboard_expecting(text);
        let action = {
            let mut st = self.clip.inner.lock().unwrap();
            let action = st.burst.finish_restore(current_clipboard.as_deref());
            if non_text_backup.is_some() && current_clipboard.as_deref() == Some(text) {
                // The user's clipboard was empty/non-text: there is no text to
                // restore through the burst machinery, so forget the "unrestored
                // injected text" marker and restore the original data below.
                st.burst.forget_unrestored();
            }
            action
        };
        if let (Some(backup), true) = (
            non_text_backup.as_ref(),
            current_clipboard.as_deref() == Some(text),
        ) {
            trace_clipboard(sequence_id, "restore", "state=writing-non-text");
            let restored = Self::restore_non_text_clipboard(backup);
            trace_clipboard(
                sequence_id,
                "restore",
                if restored {
                    "state=non-text-restored"
                } else {
                    "state=non-text-restore-failed"
                },
            );
            self.clip.inner.lock().unwrap().burst.reset();
            let elapsed = t_total.elapsed().as_millis();
            if elapsed > 20 {
                eprintln!(
                    "[vietc] clipboard sequence_id={} elapsed_ms={}",
                    sequence_id, elapsed
                );
            }
            return restored;
        }

        let success = match action {
            ClipboardAction::Restore(saved) => {
                trace_clipboard(sequence_id, "restore", "state=writing");
                if !Self::copy_to_clipboard(&saved) {
                    self.clip.inner.lock().unwrap().burst.reset();
                    trace_clipboard(sequence_id, "restore", "state=write-failed");
                    false
                } else if !Self::wait_for_clipboard_value(&saved) {
                    self.clip.inner.lock().unwrap().burst.reset();
                    trace_clipboard(sequence_id, "verified-read-back", "state=failed");
                    false
                } else {
                    self.clip
                        .inner
                        .lock()
                        .unwrap()
                        .burst
                        .complete_restore(sequence_id);
                    trace_clipboard(sequence_id, "verified-read-back", "state=restored");
                    true
                }
            }
            ClipboardAction::SkipRestore => {
                trace_clipboard(
                    sequence_id,
                    "restore",
                    "state=skipped-user-clipboard-changed",
                );
                true
            }
            ClipboardAction::WriteInjected(_) => false,
        };

        let elapsed = t_total.elapsed().as_millis();
        if elapsed > 20 {
            eprintln!(
                "[vietc] clipboard sequence_id={} elapsed_ms={}",
                sequence_id, elapsed
            );
        }
        success
    }

    fn session_path_metadata(path: &Path) -> std::io::Result<SessionPathMetadata> {
        use std::os::unix::fs::MetadataExt;

        let metadata = std::fs::metadata(path)?;
        Ok(SessionPathMetadata {
            owner_uid: metadata.uid(),
            is_directory: metadata.is_dir(),
            is_regular_file: metadata.is_file(),
        })
    }

    fn desktop_session() -> Option<DesktopSession> {
        let (uid, gid) = Self::get_original_uid_gid()?;
        if uid == 0 || gid == 0 {
            return None;
        }

        let username = Self::get_original_username()?;
        let password_entry = unsafe { libc::getpwuid(uid) };
        if password_entry.is_null() {
            return None;
        }
        let home = unsafe {
            std::ffi::CStr::from_ptr((*password_entry).pw_dir)
                .to_string_lossy()
                .into_owned()
        };
        if home.is_empty() {
            return None;
        }

        Some(DesktopSession {
            uid,
            gid,
            username,
            home: PathBuf::from(home),
            xdg_runtime_dir: std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            display: std::env::var("DISPLAY")
                .ok()
                .filter(|value| !value.is_empty()),
            wayland_display: std::env::var("WAYLAND_DISPLAY")
                .ok()
                .filter(|value| !value.is_empty()),
            xauthority: std::env::var_os("XAUTHORITY").map(PathBuf::from),
        })
    }

    /// Build a desktop command under the owning user session. Root callers
    /// must have a validated non-root identity; no root fallback is allowed.
    fn user_cmd(program: &str) -> std::io::Result<std::process::Command> {
        let is_root = unsafe { libc::getuid() == 0 };
        build_desktop_command_with(
            program,
            is_root,
            Self::desktop_session,
            Self::session_path_metadata,
        )
    }

    fn wait_for_clipboard_value(expected: &str) -> bool {
        let deadline = Instant::now() + CLIPBOARD_READY_TIMEOUT;
        loop {
            if Self::read_clipboard().as_deref() == Some(expected) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(CLIPBOARD_READY_POLL_INTERVAL);
        }
    }

    fn wait_for_target_consumption(
        state: &ClipState,
        transaction_id: u64,
        injected_text: &str,
        focus_generation: u64,
    ) -> bool {
        let observed = Self::poll_paste_observation_with(
            transaction_id,
            injected_text,
            || {
                let state = state.inner.lock().unwrap();
                state
                    .burst
                    .matches(transaction_id, focus_generation)
                    .then_some(transaction_id)
            },
            Self::read_clipboard,
            Instant::now,
            std::thread::sleep,
        );
        if !observed {
            return false;
        }

        let mut inner = state.inner.lock().unwrap();
        if !inner.burst.matches(transaction_id, focus_generation) {
            return false;
        }
        if inner
            .burst
            .transition(transaction_id, ReplacementPhase::TargetObserved)
            && inner
                .burst
                .transition(transaction_id, ReplacementPhase::Applied)
        {
            inner.restore_due = None;
            true
        } else {
            false
        }
    }

    fn poll_paste_observation_with(
        transaction_id: u64,
        injected_text: &str,
        mut active_transaction: impl FnMut() -> Option<u64>,
        mut read_clipboard: impl FnMut() -> Option<String>,
        mut now: impl FnMut() -> Instant,
        mut sleep: impl FnMut(Duration),
    ) -> bool {
        let started = now();
        let deadline = started + CLIPBOARD_READY_TIMEOUT;
        loop {
            if active_transaction() != Some(transaction_id) {
                return false;
            }
            if read_clipboard().as_deref() != Some(injected_text) {
                return false;
            }

            let current = now();
            if current.duration_since(started) >= PASTE_OBSERVATION_DELAY {
                return true;
            }
            if current >= deadline {
                return false;
            }
            sleep(CLIPBOARD_READY_POLL_INTERVAL.min(deadline - current));
        }
    }

    /// Copy text to clipboard using xclip (X11) or wl-copy (Wayland).
    /// NOTE: direct X11 API is avoided here because it can interact badly with
    /// the evdev keyboard grab and/or focus — xclip is simpler and works reliably
    /// on the host.
    fn copy_to_clipboard(s: &str) -> bool {
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let (prog, args): (&str, &[&str]) = if is_wayland {
            ("wl-copy", &[])
        } else {
            ("xclip", &["-selection", "clipboard", "-i"])
        };
        let Ok(mut cmd) = Self::user_cmd(prog) else {
            return false;
        };
        cmd.args(args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(mut child) => {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(s.as_bytes());
                }
                if is_wayland {
                    // wl-copy may retain clipboard ownership after its launcher
                    // exits. Reap asynchronously while readiness and restoration
                    // are verified independently by bounded read-back polling.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return true;
                }
                // X11: wait for xclip to finish writing
                child.wait().map(|s| s.success()).unwrap_or(false)
            }
            Err(e) => {
                eprintln!("[vietc] copy_to_clipboard: {} spawn failed: {}", prog, e);
                false
            }
        }
    }

    /// Force-release all modifier keys so the next injected character
    /// is never accidentally combined with a stuck modifier (Chrome, etc.).
    fn clear_modifiers_unlocked(&self) -> std::io::Result<()> {
        for code in &[
            29, 97, // L/R Ctrl
            42, 54, // L/R Shift
            56, 100, // L/R Alt
            125, 126,
        ]
        // L/R Meta (Win)
        {
            self.send_uinput_event_unlocked(EV_KEY, *code, 0)?;
        }
        self.send_uinput_event_unlocked(0, 0, 0)?; // SYN
        std::thread::sleep(std::time::Duration::from_millis(5));
        Ok(())
    }

    /// Send Ctrl+V through our uinput device, with a forced modifier clear
    /// afterwards so apps (Chrome, etc.) never see a stuck Ctrl key.
    fn send_ctrl_v(&self) -> std::io::Result<()> {
        let is_terminal = self.terminal_app.load(Ordering::Relaxed) || {
            let st = self.clip.inner.lock().unwrap();
            let app = &st.active_window;
            app.contains("terminal")
                || app.contains("konsole")
                || app.contains("kitty")
                || app.contains("alacritty")
                || app.contains("xterm")
        };

        serialize_transaction(&self.transaction_lock, || {
            if !is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 0)?;
                self.send_uinput_event_unlocked(EV_KEY, 54, 0)?;
                self.send_uinput_event_unlocked(0, 0, 0)?;
            }
            self.send_uinput_event_unlocked(EV_KEY, 29, 1)?;
            if is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 1)?;
            }
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.send_uinput_event_unlocked(EV_KEY, 47, 1)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.send_uinput_event_unlocked(EV_KEY, 47, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            if is_terminal {
                self.send_uinput_event_unlocked(EV_KEY, 42, 0)?;
            }
            self.send_uinput_event_unlocked(EV_KEY, 29, 0)?;
            self.send_uinput_event_unlocked(0, 0, 0)?;
            std::thread::sleep(std::time::Duration::from_millis(3));

            self.clear_modifiers_unlocked()
        })?
    }
}

impl Drop for UinputInjector {
    fn drop(&mut self) {
        let _ = self.restorer.shutdown_and_join();
        let _ = ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY, 0);
    }
}

/// Background thread: once no Unicode paste has happened for `RESTORE_DEBOUNCE`,
/// rewrite the user's real clipboard so Ctrl+V keeps working.
fn run_restorer(state: Arc<ClipState>) {
    run_restorer_with(
        state,
        UinputInjector::read_clipboard,
        UinputInjector::copy_to_clipboard,
    );
}

fn run_restorer_with(
    state: Arc<ClipState>,
    mut read_clipboard: impl FnMut() -> Option<String>,
    mut copy_to_clipboard: impl FnMut(&str) -> bool,
) {
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
        st.restore_due = None;
        drop(st);

        let _clipboard_io = state.io_lock.lock().unwrap();
        let current_clipboard = read_clipboard();
        let action = {
            let mut st = state.inner.lock().unwrap();
            if st.shutdown {
                st.burst.reset();
                return;
            }
            st.burst.finish_restore(current_clipboard.as_deref())
        };

        if let ClipboardAction::Restore(restored) = action {
            let current_clipboard = read_clipboard();
            let should_restore = {
                let mut st = state.inner.lock().unwrap();
                if st.shutdown {
                    st.burst.reset();
                    return;
                }
                matches!(
                    st.burst.finish_restore(current_clipboard.as_deref()),
                    ClipboardAction::Restore(_)
                )
            };
            if !should_restore {
                continue;
            }

            let transaction_id = {
                let st = state.inner.lock().unwrap();
                st.burst.transaction_id()
            };
            let restored_ok = copy_to_clipboard(&restored);
            let mut st = state.inner.lock().unwrap();
            if st.shutdown {
                st.burst.reset();
                return;
            }
            if let Some(transaction_id) = transaction_id {
                if restored_ok {
                    let _ = st.burst.complete_restore(transaction_id);
                } else if st.burst.transaction_id() == Some(transaction_id) {
                    st.burst.reset();
                }
            }
        }
    }
}

fn strip_vn_diacritic(ch: char) -> char {
    match ch {
        'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ' | 'ấ' | 'ẩ'
        | 'ẫ' | 'ậ' => 'a',
        'À' | 'Á' | 'Ả' | 'Ã' | 'Ạ' | 'Ă' | 'Ằ' | 'Ắ' | 'Ẳ' | 'Ẵ' | 'Ặ' | 'Â' | 'Ầ' | 'Ấ' | 'Ẩ'
        | 'Ẫ' | 'Ậ' => 'A',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'e',
        'È' | 'É' | 'Ẻ' | 'Ẽ' | 'Ẹ' | 'Ê' | 'Ề' | 'Ế' | 'Ể' | 'Ễ' | 'Ệ' => 'E',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'Ì' | 'Í' | 'Ỉ' | 'Ĩ' | 'Ị' => 'I',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ' | 'ớ' | 'ở'
        | 'ỡ' | 'ợ' => 'o',
        'Ò' | 'Ó' | 'Ỏ' | 'Õ' | 'Ọ' | 'Ô' | 'Ồ' | 'Ố' | 'Ổ' | 'Ỗ' | 'Ộ' | 'Ơ' | 'Ờ' | 'Ớ' | 'Ở'
        | 'Ỡ' | 'Ợ' => 'O',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'u',
        'Ù' | 'Ú' | 'Ủ' | 'Ũ' | 'Ụ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ử' | 'Ữ' | 'Ự' => 'U',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        'Ỳ' | 'Ý' | 'Ỷ' | 'Ỹ' | 'Ỵ' => 'Y',
        'đ' => 'd',
        'Đ' => 'D',
        other => other,
    }
}

fn char_to_linux_keycode(ch: char) -> Option<u16> {
    match ch.to_ascii_lowercase() {
        'a' => Some(30),
        'b' => Some(48),
        'c' => Some(46),
        'd' => Some(32),
        'e' => Some(18),
        'f' => Some(33),
        'g' => Some(34),
        'h' => Some(35),
        'i' => Some(23),
        'j' => Some(36),
        'k' => Some(37),
        'l' => Some(38),
        'm' => Some(50),
        'n' => Some(49),
        'o' => Some(24),
        'p' => Some(25),
        'q' => Some(16),
        'r' => Some(19),
        's' => Some(31),
        't' => Some(20),
        'u' => Some(22),
        'v' => Some(47),
        'w' => Some(17),
        'x' => Some(45),
        'y' => Some(21),
        'z' => Some(44),
        '0' => Some(11),
        '1' => Some(2),
        '2' => Some(3),
        '3' => Some(4),
        '4' => Some(5),
        '5' => Some(6),
        '6' => Some(7),
        '7' => Some(8),
        '8' => Some(9),
        '9' => Some(10),
        ' ' => Some(57),
        '.' => Some(52),
        ',' => Some(51),
        '-' => Some(12),
        '=' => Some(13),
        ';' => Some(39),
        '\'' => Some(40),
        '/' => Some(53),
        '\\' => Some(43),
        _ => None,
    }
}

// ioctl helper
fn ioctl(
    fd: std::os::unix::io::RawFd,
    request: u64,
    arg: u64,
) -> Result<i32, Box<dyn std::error::Error>> {
    unsafe {
        let result = libc::ioctl(fd, request, arg);
        if result < 0 {
            Err(format!("ioctl failed: {}", std::io::Error::last_os_error()).into())
        } else {
            Ok(result)
        }
    }
}

#[repr(C)]
struct input_event {
    time: timeval,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct timeval {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
}

#[repr(C)]
struct uinput_setup {
    id: input_id,
    name: [i8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct input_id {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[cfg(test)]
mod clipboard_burst_tests {
    use super::*;

    #[test]
    fn restored_burst_resets_snapshot_for_next_user_clipboard() {
        let mut state = ClipboardBurstState::default();

        assert_eq!(
            state.begin_injection(Some("clipboard A".to_string()), "vietc A", 1),
            ClipboardAction::WriteInjected("vietc A".to_string())
        );
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("vietc A")),
            ClipboardAction::Restore("clipboard A".to_string())
        );
        assert!(state.complete_restore(id));

        assert_eq!(
            state.begin_injection(Some("clipboard B".to_string()), "vietc B", 1),
            ClipboardAction::WriteInjected("vietc B".to_string())
        );
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("vietc B")),
            ClipboardAction::Restore("clipboard B".to_string())
        );
    }

    #[test]
    fn user_clipboard_change_during_wait_is_not_overwritten() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(Some("clipboard A".to_string()), "vietc A", 1);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));

        assert_eq!(
            state.finish_restore(Some("clipboard B")),
            ClipboardAction::SkipRestore
        );
        assert!(!state.is_active());
    }

    #[test]
    fn clipboard_is_not_restored_before_transaction_is_applied() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 1);
        let id = state.transaction_id().unwrap();
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::SkipRestore
        );
        assert!(state.is_active());

        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::Restore("user clipboard".to_string())
        );
    }

    #[test]
    fn transaction_ids_are_monotonic_and_stale_transitions_are_rejected() {
        let mut state = ClipboardBurstState::default();

        state.begin_injection(None, "đ", 1);
        let id = state.transaction_id().unwrap();
        let first = state.transaction_id().unwrap();
        assert!(!state.transition(id, ReplacementPhase::Applied));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        state.mark_failed(id);
        assert_eq!(state.phase(), Some(ReplacementPhase::Failed));
        state.reset();

        state.begin_injection(None, "đường", 1);
        let second = state.transaction_id().unwrap();
        assert!(second > first);
        assert!(!state.transition(first, ReplacementPhase::ClipboardWritten));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
        assert!(!state.complete_restore(first));
        assert_eq!(state.transaction_id(), Some(second));
    }

    #[test]
    fn focus_change_cancels_waiting_transaction_and_rejects_stale_completion() {
        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 7);
        let stale_id = state.transaction_id().unwrap();
        assert!(state.transition(stale_id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(stale_id, ReplacementPhase::PasteDispatched));

        assert!(state.cancel_for_focus_change(8));
        assert!(!state.transition(stale_id, ReplacementPhase::TargetObserved));
        assert!(!state.complete_restore(stale_id));
        assert!(!state.is_active());

        state.begin_injection(Some("new clipboard".to_string()), "Việt", 8);
        let current_id = state.transaction_id().unwrap();
        assert!(current_id > stale_id);
        assert!(!state.transition(stale_id, ReplacementPhase::Applied));
        assert!(state.matches(current_id, 8));
        assert!(!state.matches(current_id, 7));
        assert_eq!(state.phase(), Some(ReplacementPhase::Prepared));
    }

    #[test]
    fn applied_transaction_is_not_cancelled_by_late_focus_change() {
        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user clipboard".to_string()), "tiếng", 11);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));

        assert!(!state.cancel_for_focus_change(12));
        assert!(state.matches(id, 11));
        assert_eq!(
            state.finish_restore(Some("tiếng")),
            ClipboardAction::Restore("user clipboard".to_string())
        );
    }

    #[test]
    fn each_unicode_command_finishes_its_clipboard_transaction_before_returning() {
        let mut state = ClipboardBurstState::default();

        for injected in ["đ", "đư", "đườ", "đường"] {
            state.begin_injection(Some("user clipboard".to_string()), injected, 1);
            let id = state.transaction_id().unwrap();
            assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
            assert!(state.transition(id, ReplacementPhase::PasteDispatched));
            assert!(state.transition(id, ReplacementPhase::TargetObserved));
            assert!(state.transition(id, ReplacementPhase::Applied));
            assert_eq!(
                state.finish_restore(Some(injected)),
                ClipboardAction::Restore("user clipboard".to_string())
            );
            assert!(state.complete_restore(id));
            assert!(!state.is_active());
        }
    }
}
#[cfg(test)]
mod clipboard_observation_tests {
    use super::*;
    use std::cell::Cell;

    fn observe_with_delay(delay: Duration) -> bool {
        let transaction_id = 42;
        let elapsed = Cell::new(Duration::ZERO);
        UinputInjector::poll_paste_observation_with(
            transaction_id,
            "injected",
            || Some(transaction_id),
            || Some("injected".to_string()),
            || Instant::now() + elapsed.get(),
            |duration| {
                let simulated_delay = duration.min(delay.saturating_sub(elapsed.get()));
                elapsed.set(elapsed.get() + simulated_delay.max(duration));
            },
        )
    }

    #[test]
    fn bounded_polling_accepts_supported_clipboard_delays() {
        for delay_ms in [20, 50, 100, 250] {
            assert!(observe_with_delay(Duration::from_millis(delay_ms)));
        }
    }

    #[test]
    fn clipboard_unavailable_fails_without_retrying_dispatch() {
        let reads = Cell::new(0);
        let result = UinputInjector::poll_paste_observation_with(
            7,
            "injected",
            || Some(7),
            || {
                reads.set(reads.get() + 1);
                None
            },
            Instant::now,
            |_| panic!("unavailable clipboard must fail immediately"),
        );

        assert!(!result);
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn stale_transaction_aborts_before_observation_completion() {
        let active = Cell::new(Some(10));
        let elapsed = Cell::new(Duration::ZERO);
        let result = UinputInjector::poll_paste_observation_with(
            10,
            "injected",
            || active.get(),
            || Some("injected".to_string()),
            || Instant::now() + elapsed.get(),
            |duration| {
                elapsed.set(elapsed.get() + duration);
                active.set(Some(11));
            },
        );

        assert!(!result);
    }

    #[test]
    fn slow_paste_is_observed_once_without_duplicate_completion() {
        let elapsed = Cell::new(Duration::ZERO);
        let reads = Cell::new(0);
        let result = UinputInjector::poll_paste_observation_with(
            12,
            "injected",
            || Some(12),
            || {
                reads.set(reads.get() + 1);
                Some("injected".to_string())
            },
            || Instant::now() + elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
        );

        assert!(result);
        assert!(reads.get() > 1);

        let mut state = ClipboardBurstState::default();
        state.begin_injection(Some("user".to_string()), "injected", 1);
        let id = state.transaction_id().unwrap();
        assert!(state.transition(id, ReplacementPhase::ClipboardWritten));
        assert!(state.transition(id, ReplacementPhase::PasteDispatched));
        assert!(state.transition(id, ReplacementPhase::TargetObserved));
        assert!(state.transition(id, ReplacementPhase::Applied));
        assert!(state.complete_restore(id));
        assert!(!state.complete_restore(id));
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    #[test]
    fn short_writes_are_completed() {
        let mut writes = VecDeque::from([Ok(2), Ok(3)]);
        let mut calls = 0;

        let result = write_all_with(&[1, 2, 3, 4, 5], |_| {
            calls += 1;
            writes.pop_front().unwrap()
        });

        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn interrupted_write_is_retried() {
        let mut writes = VecDeque::from([Err(io::Error::from(io::ErrorKind::Interrupted)), Ok(4)]);

        let result = write_all_with(&[1, 2, 3, 4], |_| writes.pop_front().unwrap());

        assert!(result.is_ok());
    }

    #[test]
    fn zero_byte_write_is_an_error_not_a_loop() {
        let result = write_all_with(&[1], |_| Ok(0));

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn hard_error_propagates() {
        let result = write_all_with(&[1], |_| {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "uinput closed"))
        });

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn concurrent_transactions_do_not_interleave() {
        let transaction_lock = Arc::new(Mutex::new(()));
        let barrier = Arc::new(Barrier::new(3));
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut workers = Vec::new();

        for transaction in [[1, 2, 3], [4, 5, 6]] {
            let transaction_lock = Arc::clone(&transaction_lock);
            let barrier = Arc::clone(&barrier);
            let output = Arc::clone(&output);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                serialize_transaction(&transaction_lock, || {
                    for value in transaction {
                        output.lock().unwrap().push(value);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                })
                .unwrap();
            }));
        }

        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        let output = output.lock().unwrap().clone();
        assert!(output == [1, 2, 3, 4, 5, 6] || output == [4, 5, 6, 1, 2, 3]);
    }
}

#[cfg(test)]
mod privilege_tests {
    use super::*;
    use std::path::PathBuf;

    fn valid_session() -> DesktopSession {
        DesktopSession {
            uid: 1000,
            gid: 1000,
            username: "vietc-user".to_string(),
            home: PathBuf::from("/home/vietc-user"),
            xdg_runtime_dir: Some(PathBuf::from("/run/user/1000")),
            display: Some(":0".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            xauthority: Some(PathBuf::from("/home/vietc-user/.Xauthority")),
        }
    }

    #[test]
    fn privilege_drop_clears_groups_before_gid_and_uid() {
        assert_eq!(
            privilege_drop_steps(),
            [
                PrivilegeDropStep::ClearSupplementaryGroups,
                PrivilegeDropStep::SetGid,
                PrivilegeDropStep::SetUid,
            ]
        );
    }

    #[test]
    fn desktop_command_uses_only_allowlisted_environment() {
        let prepared = prepare_desktop_command("wl-copy", valid_session()).unwrap();

        assert_eq!(prepared.program, "wl-copy");
        assert_eq!(prepared.uid, 1000);
        assert_eq!(prepared.gid, 1000);
        assert_eq!(
            prepared
                .environment
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "DISPLAY",
                "HOME",
                "USER",
                "WAYLAND_DISPLAY",
                "XAUTHORITY",
                "XDG_RUNTIME_DIR",
            ]
        );
    }

    #[test]
    fn root_desktop_command_requires_non_root_session_identity() {
        let mut session = valid_session();
        session.uid = 0;
        session.gid = 0;

        let error = prepare_desktop_command("wl-copy", session).unwrap_err();

        assert!(error.to_string().contains("non-root desktop session"));
    }

    #[test]
    fn rejects_untrusted_wayland_display_path() {
        let mut session = valid_session();
        session.wayland_display = Some("../attacker/socket".to_string());

        let error = prepare_desktop_command("wl-copy", session).unwrap_err();

        assert!(error.to_string().contains("WAYLAND_DISPLAY"));
    }

    #[test]
    fn rejects_runtime_directory_owned_by_another_uid() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: 2000,
            is_directory: true,
            is_regular_file: false,
        };

        let error = validate_runtime_dir(
            session.xdg_runtime_dir.as_deref().unwrap(),
            session.uid,
            |_| Ok(metadata),
        )
        .unwrap_err();

        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn rejects_xauthority_outside_the_user_home() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: session.uid,
            is_directory: false,
            is_regular_file: true,
        };

        let error = validate_xauthority(Path::new("/root/.Xauthority"), &session, |_| Ok(metadata))
            .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn rejects_xauthority_owned_by_another_uid() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: 2000,
            is_directory: false,
            is_regular_file: true,
        };

        let error = validate_xauthority(session.xauthority.as_deref().unwrap(), &session, |_| {
            Ok(metadata)
        })
        .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn rejects_non_file_xauthority() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: session.uid,
            is_directory: true,
            is_regular_file: false,
        };

        let error = validate_xauthority(session.xauthority.as_deref().unwrap(), &session, |_| {
            Ok(metadata)
        })
        .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn root_command_without_session_identity_is_rejected() {
        let result = build_desktop_command_with(
            "wl-copy",
            true,
            || None,
            |_| panic!("metadata must not be read without a session identity"),
        );

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("non-root desktop session identity"));
    }

    #[test]
    fn root_command_validates_session_paths_before_building() {
        let mut session = valid_session();
        session.xdg_runtime_dir = Some(PathBuf::from("/tmp/untrusted-runtime"));

        let result = build_desktop_command_with(
            "wl-copy",
            true,
            || Some(session),
            |_| {
                Ok(SessionPathMetadata {
                    owner_uid: 1000,
                    is_directory: true,
                    is_regular_file: false,
                })
            },
        );

        let error = result.unwrap_err();
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }
}

#[cfg(test)]
mod restorer_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    #[test]
    fn shutdown_wakes_and_joins_restorer_deterministically() {
        let exited = Arc::new(AtomicBool::new(false));
        let worker_exited = Arc::clone(&exited);
        let state = Arc::new(ClipState {
            inner: Mutex::new(ClipInner {
                burst: ClipboardBurstState::default(),
                restore_due: None,
                shutdown: false,
                active_window: String::new(),
            }),
            io_lock: Mutex::new(()),
            cv: Condvar::new(),
            focus_generation: Mutex::new(None),
        });
        let (started_tx, started_rx) = mpsc::channel();

        let mut restorer = ClipboardRestorer::spawn(Arc::clone(&state), move |state| {
            started_tx.send(()).unwrap();
            run_restorer_with(state, || None, |_| true);
            worker_exited.store(true, Ordering::SeqCst);
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        restorer.shutdown_and_join().unwrap();

        assert!(exited.load(Ordering::SeqCst));
        assert!(restorer.is_joined());
    }

    #[test]
    fn shutdown_prevents_restore_after_join() {
        let restored = Arc::new(Mutex::new(Vec::new()));
        let worker_restored = Arc::clone(&restored);
        let state = Arc::new(ClipState {
            inner: Mutex::new(ClipInner {
                burst: {
                    let mut burst = ClipboardBurstState::default();
                    burst.begin_injection(Some("user clipboard".to_string()), "injected text", 1);
                    burst
                },
                restore_due: Some(Instant::now()),
                shutdown: false,
                active_window: String::new(),
            }),
            io_lock: Mutex::new(()),
            cv: Condvar::new(),
            focus_generation: Mutex::new(None),
        });

        let mut restorer = ClipboardRestorer::spawn(Arc::clone(&state), move |state| {
            run_restorer_with(
                state,
                || Some("injected text".to_string()),
                |text| {
                    worker_restored.lock().unwrap().push(text.to_string());
                    true
                },
            );
        });

        restorer.shutdown_and_join().unwrap();
        let restore_count_after_join = restored.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(20));

        assert_eq!(restored.lock().unwrap().len(), restore_count_after_join);
    }

    #[test]
    fn user_clipboard_change_during_restore_io_is_not_overwritten() {
        let reads = Arc::new(Mutex::new(VecDeque::from([
            Some("injected text".to_string()),
            Some("new user clipboard".to_string()),
        ])));
        let worker_reads = Arc::clone(&reads);
        let restored = Arc::new(Mutex::new(Vec::new()));
        let worker_restored = Arc::clone(&restored);
        let state = Arc::new(ClipState {
            inner: Mutex::new(ClipInner {
                burst: {
                    let mut burst = ClipboardBurstState::default();
                    burst.begin_injection(
                        Some("old user clipboard".to_string()),
                        "injected text",
                        1,
                    );
                    burst
                },
                restore_due: Some(Instant::now()),
                shutdown: false,
                active_window: String::new(),
            }),
            io_lock: Mutex::new(()),
            cv: Condvar::new(),
            focus_generation: Mutex::new(None),
        });
        let worker_state = Arc::clone(&state);

        let handle = std::thread::spawn(move || {
            run_restorer_with(
                worker_state,
                || worker_reads.lock().unwrap().pop_front().flatten(),
                |text| {
                    worker_restored.lock().unwrap().push(text.to_string());
                    true
                },
            );
        });

        for _ in 0..100 {
            if reads.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        {
            let mut inner = state.inner.lock().unwrap();
            inner.shutdown = true;
        }
        state.cv.notify_all();
        handle.join().unwrap();

        assert!(restored.lock().unwrap().is_empty());
    }
}

#[cfg(test)]
mod clipboard_backup_tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn empty_clipboard_is_backed_up_as_empty() {
        assert_eq!(classify_clipboard_types(&[]), ClipboardKind::Empty);
    }

    #[test]
    fn any_plain_text_offer_uses_the_text_path() {
        assert_eq!(
            classify_clipboard_types(&types(&["text/html", "text/plain;charset=utf-8"])),
            ClipboardKind::Text
        );
        assert_eq!(
            classify_clipboard_types(&types(&["UTF8_STRING"])),
            ClipboardKind::Text
        );
    }

    #[test]
    fn image_only_clipboard_keeps_its_own_type_instead_of_becoming_text() {
        assert_eq!(
            classify_clipboard_types(&types(&["image/png"])),
            ClipboardKind::Other("image/png".to_string())
        );
        assert_eq!(
            classify_clipboard_types(&types(&["text/uri-list", "x-special/gnome-copied-files"])),
            ClipboardKind::Other("text/uri-list".to_string())
        );
    }
}
