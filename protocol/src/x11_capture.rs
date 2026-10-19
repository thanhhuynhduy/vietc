// SPDX-License-Identifier: MIT
use crate::dylib::DynamicLibrary;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type Display = c_void;

const KEY_PRESS: c_int = 2;

const CONTROL_MASK: c_int = 4;
const MOD1_MASK: c_int = 8;
const MOD4_MASK: c_int = 64;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 1;
const XRECORD_READINESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct XRecordProcess {
    child: XRecordChild,
    stdout: std::process::ChildStdout,
    pipe_fd: i32,
}

#[derive(Debug)]
struct XRecordChild {
    child: Child,
}

impl XRecordChild {
    fn new(child: Child) -> Self {
        Self { child }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn kill_and_wait(&mut self) {
        kill_and_wait(&mut self.child);
    }
}

impl Drop for XRecordChild {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

fn set_nonblocking(fd: i32) -> std::io::Result<()> {
    set_nonblocking_with(
        fd,
        |fd| {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(flags)
            }
        },
        |fd, flags| {
            if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        },
    )
}

fn set_nonblocking_with(
    fd: i32,
    mut get_flags: impl FnMut(i32) -> std::io::Result<i32>,
    mut set_flags: impl FnMut(i32, i32) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let flags = get_flags(fd)?;
    set_flags(fd, flags | libc::O_NONBLOCK)
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_xrecord(
    command: &mut Command,
    readiness_timeout: Duration,
) -> std::io::Result<XRecordProcess> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;

    let setup_result = (|| {
        let mut stderr = child.stderr.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "vietc-xrecord stderr is unavailable",
            )
        })?;
        set_nonblocking(stderr.as_raw_fd())?;

        let deadline = Instant::now() + readiness_timeout;
        let mut readiness = Vec::new();
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("vietc-xrecord exited before readiness handshake: {status}"),
                ));
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "vietc-xrecord readiness handshake timed out",
                ));
            }

            let remaining = deadline.saturating_duration_since(now);
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let mut pfd = PollFd {
                fd: stderr.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            let poll_result = unsafe { poll(&mut pfd, 1, timeout_ms) };
            if poll_result == -1 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if poll_result == 0 {
                continue;
            }

            let mut buffer = [0u8; 128];
            match stderr.read(&mut buffer) {
                Ok(0) => {
                    let reap_deadline = Instant::now() + Duration::from_millis(50);
                    loop {
                        if let Some(status) = child.try_wait()? {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                format!(
                                    "vietc-xrecord exited before readiness handshake: {status}"
                                ),
                            ));
                        }
                        if Instant::now() >= reap_deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "vietc-xrecord closed stderr before readiness handshake",
                    ));
                }
                Ok(count) => {
                    readiness.extend_from_slice(&buffer[..count]);
                    if readiness.contains(&b'\n') {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "vietc-xrecord stdout is unavailable",
            )
        })?;
        let pipe_fd = stdout.as_raw_fd();
        set_nonblocking(pipe_fd)?;

        Ok((stdout, pipe_fd))
    })();

    match setup_result {
        Ok((stdout, pipe_fd)) => Ok(XRecordProcess {
            child: XRecordChild::new(child),
            stdout,
            pipe_fd,
        }),
        Err(error) => {
            kill_and_wait(&mut child);
            Err(error)
        }
    }
}

#[repr(C)]
struct XKeyEvent {
    _type: c_int,
    _serial: u64,
    _send_event: c_int,
    _display: *mut Display,
    window: u64,
    _root: u64,
    _subwindow: u64,
    _time: u64,
    _x: c_int,
    _y: c_int,
    _x_root: c_int,
    _y_root: c_int,
    state: c_int,
    keycode: u32,
    _same_screen: c_int,
}

type KeySym = u64;

pub struct X11KeyEvent {
    pub keycode: u32,
    pub ch: Option<char>,
    pub pressed: bool,
    pub state: c_int,
}

pub static SKIP_RECORD_EVENTS: AtomicBool = AtomicBool::new(false);

struct LookupLib {
    _library: DynamicLibrary<'static>,
    display: *mut Display,
    x_close_display: unsafe extern "C" fn(*mut Display) -> c_int,
    x_lookup_string:
        unsafe extern "C" fn(*mut XKeyEvent, *mut c_char, c_int, *mut KeySym, *mut c_int) -> c_int,
}

unsafe impl Send for LookupLib {}

impl Drop for LookupLib {
    fn drop(&mut self) {
        unsafe {
            (self.x_close_display)(self.display);
        }
    }
}

impl LookupLib {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let library = DynamicLibrary::open_candidates(&[c"libX11.so.6", c"libX11.so"])?;
            let x_open_display: unsafe extern "C" fn(*const c_char) -> *mut Display =
                library.symbol(c"XOpenDisplay")?;
            let x_close_display = library.symbol(c"XCloseDisplay")?;
            let x_lookup_string = library.symbol(c"XLookupString")?;
            let display = x_open_display(std::ptr::null());
            if display.is_null() {
                return Err("Cannot open X11 display. Is DISPLAY set?".into());
            }

            Ok(Self {
                _library: library,
                display,
                x_close_display,
                x_lookup_string,
            })
        }
    }

    fn lookup_keycode(&self, keycode: u32, state: c_int) -> Option<char> {
        unsafe {
            let mut xke: XKeyEvent = std::mem::zeroed();
            xke._type = KEY_PRESS;
            xke.keycode = keycode;
            xke.state = state;

            let mut buf = [0u8; 32];
            let mut keysym: KeySym = 0;
            let len = (self.x_lookup_string)(
                &mut xke as *mut XKeyEvent,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
                &mut keysym,
                std::ptr::null_mut(),
            );

            if len > 0 {
                let s = std::str::from_utf8(&buf[..len as usize]).ok()?;
                s.chars().next()
            } else {
                None
            }
        }
    }
}

const PIPE_EVENT_SIZE: usize = 8;

#[derive(Clone, Copy, Debug)]
struct PipeEvent {
    keycode: u8,
    pressed: u8,
    state: u16,
}

#[derive(Default)]
struct PipeFrameDecoder {
    pending: Vec<u8>,
}

impl PipeFrameDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<PipeEvent> {
        self.pending.extend_from_slice(bytes);

        let frame_count = self.pending.len() / PIPE_EVENT_SIZE;
        let mut events = Vec::with_capacity(frame_count);
        for frame in self.pending[..frame_count * PIPE_EVENT_SIZE].chunks_exact(PIPE_EVENT_SIZE) {
            events.push(PipeEvent {
                keycode: frame[0],
                pressed: frame[1],
                state: u16::from_ne_bytes([frame[2], frame[3]]),
            });
        }

        self.pending.drain(..frame_count * PIPE_EVENT_SIZE);
        events
    }

    fn finish(&self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "partial XRecord pipe frame at EOF: {} of {} bytes",
                    self.pending.len(),
                    PIPE_EVENT_SIZE
                ),
            ))
        }
    }
}

fn drain_pipe_frames(
    reader: &mut impl Read,
    decoder: &mut PipeFrameDecoder,
) -> std::io::Result<Vec<PipeEvent>> {
    let mut events = Vec::new();
    let mut buffer = [0u8; 256];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                decoder.finish()?;
                return Ok(events);
            }
            Ok(count) => events.extend(decoder.push(&buffer[..count])),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(events);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

pub struct X11Capture {
    child: XRecordChild,
    pipe_fd: i32,
    pipe_stdout: Option<std::process::ChildStdout>,
    pipe_decoder: PipeFrameDecoder,
    lookup: LookupLib,
    event_queue: VecDeque<X11KeyEvent>,
    pub focus_lost: bool,
}

unsafe impl Send for X11Capture {}

impl X11Capture {
    pub fn new() -> Option<Self> {
        let lookup = match LookupLib::new() {
            Ok(lookup) => lookup,
            Err(error) => {
                eprintln!("[vietc] X11Capture: failed to initialize X11 lookup: {error}");
                return None;
            }
        };
        let xrecord_path = find_xrecord_binary();
        eprintln!(
            "[vietc] X11Capture: spawning vietc-xrecord from {}",
            xrecord_path
        );
        let mut command = Command::new(&xrecord_path);
        let process = match spawn_xrecord(&mut command, XRECORD_READINESS_TIMEOUT) {
            Ok(process) => process,
            Err(error) => {
                eprintln!("[vietc] X11Capture: failed to start vietc-xrecord: {error}");
                return None;
            }
        };

        Some(Self {
            child: process.child,
            pipe_fd: process.pipe_fd,
            pipe_stdout: Some(process.stdout),
            pipe_decoder: PipeFrameDecoder::default(),
            lookup,
            event_queue: VecDeque::new(),
            focus_lost: false,
        })
    }

    pub fn grab_keyboard(&mut self) -> bool {
        false
    }
    pub fn ungrab_keyboard(&mut self) {}
    pub fn is_grabbed(&self) -> bool {
        false
    }

    /// Wait for events from the C helper pipe with timeout.
    pub fn wait_for_event(&mut self, timeout_ms: u64) -> bool {
        // If SKIP_RECORD_EVENTS is true, aggressively drain all pending events
        if SKIP_RECORD_EVENTS.load(Ordering::Relaxed) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
            loop {
                self.drain_pipe();
                if std::time::Instant::now() >= deadline {
                    break;
                }
                let mut pfd = PollFd {
                    fd: self.pipe_fd,
                    events: POLLIN,
                    revents: 0,
                };
                unsafe {
                    poll(&mut pfd, 1, 5);
                }
                if pfd.revents & POLLIN == 0 {
                    std::thread::sleep(std::time::Duration::from_micros(500));
                    self.drain_pipe();
                    break;
                }
            }
        }

        self.drain_pipe();

        if !self.event_queue.is_empty() {
            return true;
        }

        let mut pfd = PollFd {
            fd: self.pipe_fd,
            events: POLLIN,
            revents: 0,
        };
        unsafe {
            poll(&mut pfd, 1, timeout_ms as i32);
        }

        if pfd.revents & POLLIN != 0 {
            self.drain_pipe();
        }

        if let Ok(None) = self.child.try_wait() {
        } else {
            self.restart_xrecord();
        }

        !self.event_queue.is_empty()
    }

    fn drain_pipe(&mut self) {
        if let Some(ref mut stdout) = self.pipe_stdout {
            match drain_pipe_frames(stdout, &mut self.pipe_decoder) {
                Ok(events) => {
                    for ev in events {
                        if SKIP_RECORD_EVENTS.load(Ordering::Relaxed) {
                            if ev.keycode == 0 && ev.state == 2 {
                                self.focus_lost = true;
                            }
                        } else if ev.keycode == 0 && ev.pressed == 0 {
                            if ev.state == 2 {
                                self.focus_lost = true;
                            }
                        } else {
                            self.event_queue.push_back(X11KeyEvent {
                                keycode: ev.keycode as u32,
                                ch: None,
                                pressed: ev.pressed == 1,
                                state: ev.state as c_int,
                            });
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[vietc] XRecord pipe read failed: {error}");
                }
            }
        }
    }

    fn restart_xrecord(&mut self) {
        self.child.kill_and_wait();
        self.pipe_stdout = None;
        self.pipe_decoder = PipeFrameDecoder::default();

        let xrecord_path = find_xrecord_binary();
        let mut command = Command::new(&xrecord_path);
        match spawn_xrecord(&mut command, XRECORD_READINESS_TIMEOUT) {
            Ok(process) => {
                self.child = process.child;
                self.pipe_fd = process.pipe_fd;
                self.pipe_stdout = Some(process.stdout);
                eprintln!("[vietc] vietc-xrecord restarted");
            }
            Err(error) => {
                eprintln!("[vietc] failed to restart vietc-xrecord: {error}");
            }
        }
    }

    pub fn next_event(&mut self) -> Option<X11KeyEvent> {
        if let Some(mut event) = self.event_queue.pop_front() {
            event.ch = self.lookup.lookup_keycode(event.keycode, event.state);
            Some(event)
        } else {
            None
        }
    }

    pub fn is_modifier_pressed(&self, state: c_int) -> bool {
        (state & (CONTROL_MASK | MOD1_MASK | MOD4_MASK)) != 0
    }

    /// Drain any pending events from the pipe without adding them to the queue.
    /// Used after injection to clear feedback events while SKIP_RECORD_EVENTS is set.
    pub fn drain_injected(&mut self) {
        self.drain_pipe();
    }

    pub fn with_grab<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        f()
    }

    pub fn without_grab<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        f()
    }
}

fn find_xrecord_binary() -> String {
    if let Ok(output) = std::process::Command::new("which")
        .arg("vietc-xrecord")
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join("vietc-xrecord");
            if path.exists() {
                return path.to_string_lossy().to_string();
            }
        }
    }

    for p in &["/usr/bin/vietc-xrecord", "/usr/local/bin/vietc-xrecord"] {
        if std::path::Path::new(p).exists() {
            return p.to_string();
        }
    }

    "vietc-xrecord".to_string()
}

#[cfg(test)]
mod dynamic_loader_tests {
    #[test]
    fn x11_capture_uses_the_checked_shared_symbol_loader() {
        let source = include_str!("x11_capture.rs");
        let production_source = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .expect("test module must follow production code");

        assert!(!production_source.contains("dlsym("));
        assert!(!production_source.contains("dlopen("));
        assert!(!production_source.contains("dlclose("));
        assert!(!production_source.contains(r#"b"libX11.so.6\0""#));
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::{drain_pipe_frames, PipeFrameDecoder};
    use std::io::Cursor;

    fn frame(keycode: u8, pressed: u8, state: u16) -> [u8; 8] {
        let state = state.to_ne_bytes();
        [keycode, pressed, state[0], state[1], 0, 0, 0, 0]
    }

    #[test]
    fn retains_a_frame_split_across_one_and_seven_bytes() {
        let bytes = frame(42, 1, 7);
        let mut decoder = PipeFrameDecoder::default();

        assert!(decoder.push(&bytes[..1]).is_empty());

        let events = decoder.push(&bytes[1..]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].keycode, 42);
        assert_eq!(events[0].pressed, 1);
        assert_eq!(events[0].state, 7);
    }

    #[test]
    fn retains_frames_split_across_three_and_five_or_seven_and_one_bytes() {
        for split_at in [3, 7] {
            let bytes = frame(43, 0, 0x1234);
            let mut decoder = PipeFrameDecoder::default();

            assert!(decoder.push(&bytes[..split_at]).is_empty());

            let events = decoder.push(&bytes[split_at..]);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].keycode, 43);
            assert_eq!(events[0].pressed, 0);
            assert_eq!(events[0].state, 0x1234);
        }
    }

    #[test]
    fn decodes_multiple_frames_and_retains_the_trailing_remainder() {
        let first = frame(10, 1, 2);
        let second = frame(11, 0, 3);
        let third = frame(12, 1, 4);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&first);
        bytes.extend_from_slice(&second);
        bytes.extend_from_slice(&third[..5]);

        let mut decoder = PipeFrameDecoder::default();
        let events = decoder.push(&bytes);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].keycode, 10);
        assert_eq!(events[1].keycode, 11);

        let events = decoder.push(&third[5..]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].keycode, 12);
        assert_eq!(events[0].state, 4);
    }

    #[test]
    fn eof_with_a_partial_frame_returns_a_clear_error() {
        let bytes = frame(44, 1, 5);
        let mut decoder = PipeFrameDecoder::default();
        assert!(decoder.push(&bytes[..6]).is_empty());

        let error = decoder
            .finish()
            .expect_err("partial frame must fail at EOF");
        assert!(error.to_string().contains("partial XRecord pipe frame"));
        assert!(error.to_string().contains("6"));
    }

    #[test]
    fn pipe_reader_propagates_partial_frame_at_eof() {
        let bytes = frame(45, 1, 6);
        let mut reader = Cursor::new(bytes[..7].to_vec());
        let mut decoder = PipeFrameDecoder::default();

        let error = drain_pipe_frames(&mut reader, &mut decoder)
            .expect_err("drain must not silently discard a partial frame at EOF");

        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(error.to_string().contains("partial XRecord pipe frame"));
        assert!(error.to_string().contains("7"));
    }
}

#[cfg(test)]
mod xrecord_tests {
    use super::{set_nonblocking_with, spawn_xrecord, XRecordChild};
    use std::fs;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn startup_and_restart_share_the_bounded_spawn_helper() {
        let source = include_str!("x11_capture.rs");
        let production_source = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .expect("test modules must follow production code");

        assert_eq!(
            production_source
                .matches("spawn_xrecord(&mut command")
                .count(),
            2,
            "initial startup and restart must both use the bounded helper"
        );
        assert!(
            !production_source.contains("read_line(&mut line)"),
            "production readiness handling must not use blocking read_line"
        );
    }

    #[test]
    fn helper_without_ready_newline_times_out_and_is_reaped() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        let timeout = Duration::from_millis(50);
        let started = Instant::now();

        let error = spawn_xrecord(&mut command, timeout)
            .expect_err("helper without readiness output must time out");

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn timeout_kills_and_reaps_the_helper_process() {
        let pid_file = std::env::temp_dir().join(format!(
            "vietc-xrecord-timeout-{}-{}.pid",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let script = format!("echo $$ > {}; sleep 5", pid_file.display());
        let mut command = Command::new("sh");
        command.args(["-c", &script]);

        spawn_xrecord(&mut command, Duration::from_millis(100))
            .expect_err("helper without readiness output must time out");

        let pid: libc::pid_t = fs::read_to_string(&pid_file)
            .expect("helper must publish its pid")
            .trim()
            .parse()
            .expect("helper pid must be numeric");
        let process_exists = unsafe { libc::kill(pid, 0) } == 0;
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(&pid_file);

        assert!(
            !process_exists && error.raw_os_error() == Some(libc::ESRCH),
            "timed-out helper must be killed and reaped"
        );
    }

    #[test]
    fn dropping_xrecord_child_kills_and_reaps_the_process() {
        let child = Command::new("sh")
            .args(["-c", "sleep 5"])
            .spawn()
            .expect("fixture helper must start");
        let pid = child.id() as libc::pid_t;

        drop(XRecordChild::new(child));

        let process_exists = unsafe { libc::kill(pid, 0) } == 0;
        let error = std::io::Error::last_os_error();
        assert!(
            !process_exists && error.raw_os_error() == Some(libc::ESRCH),
            "dropping the lifecycle owner must kill and reap the helper"
        );
    }

    #[test]
    fn helper_exit_before_readiness_is_reported() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 23"]);

        let error = spawn_xrecord(&mut command, Duration::from_secs(1))
            .expect_err("early helper exit must fail startup");

        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(error
            .to_string()
            .contains("exited before readiness handshake"));
        assert!(error.to_string().contains("23"));
    }

    #[test]
    fn f_getfl_failure_is_propagated_without_attempting_f_setfl() {
        let mut setfl_called = false;

        let error = set_nonblocking_with(
            12,
            |_| Err(std::io::Error::from_raw_os_error(libc::EBADF)),
            |_, _| {
                setfl_called = true;
                Ok(())
            },
        )
        .expect_err("F_GETFL failure must fail setup");

        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        assert!(!setfl_called);
    }

    #[test]
    fn f_setfl_failure_is_propagated_after_successful_f_getfl() {
        let error = set_nonblocking_with(
            12,
            |_| Ok(libc::O_RDONLY),
            |_, flags| {
                assert_ne!(flags & libc::O_NONBLOCK, 0);
                Err(std::io::Error::from_raw_os_error(libc::EPERM))
            },
        )
        .expect_err("F_SETFL failure must fail setup");

        assert_eq!(error.raw_os_error(), Some(libc::EPERM));
    }
}
