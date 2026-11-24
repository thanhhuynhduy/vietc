// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const ROLE_PASSWORD_TEXT: i32 = 62;
/// Grace window preventing a compositor `Activated` event from wiping a freshly
/// recorded AT-SPI2 password focus event on the same window transition.
const ATSPI_PASSWORD_SWITCH_GRACE: Duration = Duration::from_millis(150);

static ATSPI_OBSERVED_FOCUS: AtomicBool = AtomicBool::new(false);
static ATSPI_FOCUSED_IS_PASSWORD: AtomicBool = AtomicBool::new(false);
static BG_OBSERVED_WINDOW: AtomicBool = AtomicBool::new(false);
static BG_FOCUSED_IS_SUDO: AtomicBool = AtomicBool::new(false);
static BG_PTY_NO_ECHO: AtomicBool = AtomicBool::new(false);

fn atspi_password_set_at() -> &'static Mutex<Option<Instant>> {
    static CACHE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn atspi_app_set_at() -> &'static Mutex<Option<Instant>> {
    static CACHE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn compositor_title_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Record the active toplevel's window title directly from Wayland toplevel
/// protocols (COSMIC `ext_foreign_toplevel_handle_v1` / wlroots `zwlr_foreign_toplevel_handle_v1`)
/// with zero polling delay.
pub fn record_compositor_window_title(title: Option<String>) {
    if let Ok(mut guard) = compositor_title_cache().lock() {
        *guard = title.filter(|t| !t.is_empty()).map(|t| t.to_lowercase());
    }
}

fn atspi_app_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn bg_title_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Update cached window title and terminal sudo-descendant status from the
/// 250ms background polling thread so the real-time `evdev_loop` thread never
/// blocks on external CLI processes (`gdbus`, `xdotool`, `xprop`, `hyprctl`, `swaymsg`).
pub fn record_background_window_security(title: Option<String>, is_sudo: bool, pty_no_echo: bool) {
    if let Ok(mut guard) = bg_title_cache().lock() {
        *guard = title.filter(|t| !t.is_empty()).map(|t| t.to_lowercase());
    }
    BG_FOCUSED_IS_SUDO.store(is_sudo, Ordering::Release);
    BG_PTY_NO_ECHO.store(pty_no_echo, Ordering::Release);
    BG_OBSERVED_WINDOW.store(true, Ordering::Release);
}

pub fn cached_window_title() -> Option<(bool, Option<String>)> {
    if let Some(title) = compositor_title_cache().lock().ok().and_then(|g| g.clone()) {
        return Some((true, Some(title)));
    }
    if !BG_OBSERVED_WINDOW.load(Ordering::Acquire) {
        return None;
    }
    let title = bg_title_cache().lock().ok().and_then(|g| g.clone());
    Some((true, title))
}

pub fn cached_window_is_sudo() -> Option<bool> {
    if !BG_OBSERVED_WINDOW.load(Ordering::Acquire) {
        return None;
    }
    Some(BG_FOCUSED_IS_SUDO.load(Ordering::Acquire))
}

pub fn cached_pty_no_echo() -> bool {
    BG_OBSERVED_WINDOW.load(Ordering::Acquire) && BG_PTY_NO_ECHO.load(Ordering::Acquire)
}

/// Immediately refresh the kernel PTY `ICANON && !ECHO` status (pure in-kernel
/// `open(O_NONBLOCK)` + `fstat` + `tcgetattr` on `/dev/pts/*`, ~10µs) to close
/// the 250ms background-poll blind window right after pressing `Enter` in a
/// terminal (`sudo`/`ssh`/`passwd`).
pub fn refresh_pty_no_echo_now() -> bool {
    let no_echo = any_user_pty_echo_disabled();
    BG_PTY_NO_ECHO.store(no_echo, Ordering::Release);
    BG_OBSERVED_WINDOW.store(true, Ordering::Release);
    no_echo
}

/// Returns true if a terminal's termios flags indicate a canonical password prompt
/// (`ICANON` enabled, `ECHO` disabled) — used by `sudo`, `ssh`, `passwd`, `su`, `doas`
/// while excluding raw/cbreak interactive shells (`bash`/`zsh`/`fish`) and TUI apps (`vim`/`tmux`).
pub(crate) fn is_canonical_no_echo_lflag(c_lflag: libc::tcflag_t) -> bool {
    (c_lflag & libc::ICANON) != 0 && (c_lflag & libc::ECHO) == 0
}

/// Compositor-agnostic kernel-level check for active terminal password prompts
/// (`sudo`/`ssh`/`passwd`) across COSMIC, GNOME, KDE, wlroots, and X11.
pub fn any_user_pty_echo_disabled() -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(entries) = std::fs::read_dir("/dev/pts") else {
        return false;
    };
    let my_uid = unsafe { libc::geteuid() };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Ok(cpath) = CString::new(entry.path().as_os_str().as_bytes()) else {
            continue;
        };
        let fd = unsafe {
            libc::open(
                cpath.as_ptr(),
                libc::O_RDONLY | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            continue;
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        let matched = unsafe {
            libc::fstat(fd, &mut st) == 0
                && st.st_uid == my_uid
                && (st.st_mode & libc::S_IFMT) == libc::S_IFCHR
                && libc::tcgetattr(fd, &mut termios) == 0
                && is_canonical_no_echo_lflag(termios.c_lflag)
        };
        unsafe {
            libc::close(fd);
        }
        if matched {
            return true;
        }
    }
    false
}

/// Walk up the process tree from `pid` to PID 1 and return all ancestor PIDs.
fn get_ancestor_pids(pid: u32) -> Vec<u32> {
    let mut ancestors = Vec::new();
    let mut current = pid;
    loop {
        ancestors.push(current);
        let path = format!("/proc/{}/status", current);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => break,
        };
        let ppid = match content
            .lines()
            .find(|l| l.starts_with("PPid:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u32>().ok())
        {
            Some(0) | None => break,
            Some(p) => p,
        };
        if ppid == current {
            break;
        }
        current = ppid;
    }
    ancestors
}

/// Check if the given PID or any of its descendants is running a password
/// prompt (sudo, passwd, pkexec, su, doas, ssh, gpg, cryptsetup).
pub fn is_sudo_process(pid: u32) -> bool {
    let daemon_pid = unsafe { libc::getpid() as u32 };
    let daemon_ancestors = get_ancestor_pids(daemon_pid);
    let mut queue = vec![pid];
    let mut index = 0;
    while index < queue.len() && queue.len() < 512 {
        let cur = queue[index];
        index += 1;
        if cur != pid && !daemon_ancestors.contains(&cur) {
            if let Ok(comm) = std::fs::read_to_string(format!("/proc/{cur}/comm")) {
                if matches!(
                    comm.trim(),
                    "sudo" | "passwd" | "pkexec" | "su" | "doas" | "ssh" | "gpg" | "cryptsetup"
                ) {
                    return true;
                }
            }
        }
        let task_dir = format!("/proc/{cur}/task");
        if let Ok(tasks) = std::fs::read_dir(&task_dir) {
            for task_entry in tasks.flatten() {
                let children_path = task_entry.path().join("children");
                if let Ok(children) = std::fs::read_to_string(children_path) {
                    for child in children
                        .split_whitespace()
                        .filter_map(|s| s.parse::<u32>().ok())
                    {
                        if child > 0 && !queue.contains(&child) {
                            queue.push(child);
                        }
                    }
                }
            }
        }
    }
    false
}

/// Record the focused accessible node's role and owning application name from
/// the background AT-SPI2 event watcher (`O(1)` direct query on `(sender, path)`
/// with zero tree traversal and zero typing-thread latency, working at any DOM depth).
pub fn record_atspi_focus_state(is_password: bool, app_name: Option<String>) {
    if let Ok(mut guard) = atspi_password_set_at().lock() {
        *guard = is_password.then(Instant::now);
    }
    ATSPI_FOCUSED_IS_PASSWORD.store(is_password, Ordering::Release);
    ATSPI_OBSERVED_FOCUS.store(true, Ordering::Release);
    if let Some(name) =
        app_name.filter(|n| !n.is_empty() && n != "gnome-shell" && n != "gnome-shell.desktop")
    {
        if let Ok(mut guard) = atspi_app_cache().lock() {
            *guard = Some(name);
        }
        if let Ok(mut guard) = atspi_app_set_at().lock() {
            *guard = Some(Instant::now());
        }
    }
}

/// Clear the cached AT-SPI2 password flag when focus moves to another top-level window
/// (prevents permanent stuck password mode when switching into non-AT-SPI apps like terminals).
pub fn is_atspi_password_focused() -> bool {
    ATSPI_FOCUSED_IS_PASSWORD.load(Ordering::Acquire)
}

pub fn clear_atspi_password_flag() {
    ATSPI_FOCUSED_IS_PASSWORD.store(false, Ordering::Release);
    if let Ok(mut guard) = atspi_password_set_at().lock() {
        *guard = None;
    }
}

/// Clear the cached AT-SPI2 password flag (and stale AT-SPI2 app name) on a
/// compositor toplevel activation or `FocusLost` unless AT-SPI2 *just* reported
/// focus within the grace window (prevents Alt+Tab or layer-shell Polkit/pinentry
/// prompts from having their password state wiped by a slightly later compositor event).
pub fn clear_atspi_password_flag_on_window_switch() {
    let freshly_set = atspi_password_set_at()
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < ATSPI_PASSWORD_SWITCH_GRACE);
    if !freshly_set {
        clear_atspi_password_flag();
    }
    let app_freshly_set = atspi_app_set_at()
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|at| at.elapsed() < ATSPI_PASSWORD_SWITCH_GRACE);
    if !app_freshly_set {
        if let Ok(mut guard) = atspi_app_cache().lock() {
            *guard = None;
        }
        if let Ok(mut guard) = atspi_app_set_at().lock() {
            *guard = None;
        }
    }
}

/// Only clear the AT-SPI2 password flag on window-class transitions whose focus
/// source is NOT AT-SPI2 itself (otherwise a freshly-recorded `atspi:` password
/// focus event would be wiped before `check_password_field()` reads it).
pub fn clear_atspi_password_flag_if_non_atspi(identity: Option<&str>) {
    if !identity.is_some_and(|id| id.starts_with("atspi:")) {
        clear_atspi_password_flag_on_window_switch();
    }
}

/// Clear cached AT-SPI2 focus state if the accessibility bus disconnects.
pub fn reset_atspi_focus_state() {
    ATSPI_OBSERVED_FOCUS.store(false, Ordering::Release);
    ATSPI_FOCUSED_IS_PASSWORD.store(false, Ordering::Release);
    if let Ok(mut guard) = atspi_password_set_at().lock() {
        *guard = None;
    }
    if let Ok(mut guard) = atspi_app_cache().lock() {
        *guard = None;
    }
    if let Ok(mut guard) = atspi_app_set_at().lock() {
        *guard = None;
    }
}

pub struct PasswordDetector {
    cached: Option<bool>,
}

impl PasswordDetector {
    pub fn new() -> Self {
        Self { cached: None }
    }

    /// Non-blocking `O(1)` check of the event-driven AT-SPI2 password state.
    pub fn check(&mut self) -> Option<bool> {
        if !ATSPI_OBSERVED_FOCUS.load(Ordering::Acquire) {
            return None;
        }
        let is_pw = ATSPI_FOCUSED_IS_PASSWORD.load(Ordering::Acquire);
        self.cached = Some(is_pw);
        Some(is_pw)
    }
}

pub fn get_focused_window_class_atspi() -> Option<String> {
    atspi_app_cache()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

#[cfg(test)]
pub(crate) fn atspi_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
pub(crate) fn set_atspi_password_focused_for_test(active: bool, age_ms: u64) {
    ATSPI_OBSERVED_FOCUS.store(true, Ordering::Release);
    ATSPI_FOCUSED_IS_PASSWORD.store(active, Ordering::Release);
    if let Ok(mut guard) = atspi_password_set_at().lock() {
        *guard = if active {
            Instant::now().checked_sub(Duration::from_millis(age_ms))
        } else {
            None
        };
    }
}

#[cfg(test)]
pub(crate) fn is_atspi_password_focused_for_test() -> bool {
    ATSPI_FOCUSED_IS_PASSWORD.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_driven_password_detector_tracks_focus_transitions_without_blocking() {
        let _guard = atspi_test_lock();
        reset_atspi_focus_state();
        let mut detector = PasswordDetector::new();
        assert_eq!(detector.check(), None);

        record_atspi_focus_state(true, Some("microsoft-edge".to_string()));
        assert_eq!(detector.check(), Some(true));
        assert_eq!(
            get_focused_window_class_atspi().as_deref(),
            Some("microsoft-edge")
        );

        record_atspi_focus_state(false, Some("gnome-shell".to_string()));
        assert_eq!(detector.check(), Some(false));
        // gnome-shell must not overwrite the last real focused application name
        assert_eq!(
            get_focused_window_class_atspi().as_deref(),
            Some("microsoft-edge")
        );

        reset_atspi_focus_state();
        assert_eq!(detector.check(), None);
    }

    #[test]
    fn canonical_no_echo_lflag_distinguishes_password_prompts_from_shells_and_tuis() {
        // sudo / ssh / passwd: ICANON set, ECHO cleared -> password prompt
        assert!(is_canonical_no_echo_lflag(libc::ICANON));
        // Normal cooked terminal input: ICANON + ECHO -> NOT password
        assert!(!is_canonical_no_echo_lflag(libc::ICANON | libc::ECHO));
        // Interactive raw/cbreak shell or TUI (bash/zsh/vim/tmux): !ICANON && !ECHO -> NOT password
        assert!(!is_canonical_no_echo_lflag(0));
    }

    #[test]
    fn window_switch_grace_preserves_fresh_atspi_password_flag_and_normalizes_compositor_title() {
        let _guard = atspi_test_lock();
        reset_atspi_focus_state();

        record_atspi_focus_state(true, Some("microsoft-edge".to_string()));
        // Immediate compositor Activated event AND non-AT-SPI class transition
        // must NOT wipe a freshly-set AT-SPI2 password flag within the grace window.
        clear_atspi_password_flag_on_window_switch();
        clear_atspi_password_flag_if_non_atspi(Some("toplevel:42"));
        let mut detector = PasswordDetector::new();
        assert_eq!(detector.check(), Some(true));

        // Wayland compositor titles must be normalized to lowercase for Layer 3 matching
        record_compositor_window_title(Some("Sign in — Password / Mật Khẩu".to_string()));
        assert_eq!(
            cached_window_title(),
            Some((true, Some("sign in — password / mật khẩu".to_string())))
        );
        record_compositor_window_title(None);
        reset_atspi_focus_state();
    }
}
