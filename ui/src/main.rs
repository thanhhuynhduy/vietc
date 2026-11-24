// SPDX-License-Identifier: MIT
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

mod config;
mod tray;

static DAEMON_PID: AtomicI32 = AtomicI32::new(0);
static DAEMON_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);
static LAUNCH_LOCK: Mutex<()> = Mutex::new(());

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin"))
}

fn find_daemon_binary() -> String {
    let dir = exe_dir();
    for candidate in [dir.join("dboard-daemon"), dir.join("dboard")] {
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    for candidate in [
        "/usr/bin/dboard-daemon",
        "/usr/bin/dboard",
        "/usr/local/bin/dboard-daemon",
        "/usr/local/bin/dboard",
        "/app/bin/dboard-daemon",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    "dboard-daemon".to_string()
}

fn is_daemon_running() -> bool {
    let uid = unsafe { libc::getuid() };
    let lock_path = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty() && std::path::Path::new(dir).is_dir())
        .map(|dir| format!("{}/dboard-daemon.lock", dir))
        .unwrap_or_else(|| format!("/tmp/dboard-daemon-{}.lock", uid));
    // Never acquire `LOCK_EX` on `dboard-daemon.lock` here: racing with a
    // starting `dboard-daemon` would cause the daemon to falsely exit.
    if let Ok(pid_text) = std::fs::read_to_string(&lock_path) {
        if let Ok(pid) = pid_text.trim().parse::<libc::pid_t>() {
            if pid > 0 && unsafe { libc::kill(pid, 0) } == 0 {
                if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
                    let name = comm.trim();
                    if name == "dboard" || name == "dboard-daemon" {
                        return true;
                    }
                }
            }
        }
    }
    let uid_str = uid.to_string();
    let check = |name: &str| -> bool {
        std::process::Command::new("pgrep")
            .args(["-u", &uid_str, "-x", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    check("dboard") || check("dboard-daemon")
}

#[cfg(test)]
fn reap_in_background(
    mut child: std::process::Child,
    on_exit: impl FnOnce(std::io::Result<std::process::ExitStatus>) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || on_exit(child.wait()))
}

fn supervise_daemon(child: std::process::Child) {
    let pid = child.id() as i32;
    DAEMON_PID.store(pid, Ordering::SeqCst);
    *DAEMON_CHILD.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if STOPPING.load(Ordering::SeqCst) {
            return;
        }
        let mut guard = DAEMON_CHILD.lock().unwrap_or_else(|p| p.into_inner());
        let Some(child) = guard.as_mut() else {
            return;
        };
        if child.id() as i32 != pid {
            return;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                *guard = None;
                let _ = DAEMON_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
                if !status.success() {
                    eprintln!("[dboard-tray] Supervised daemon exited unsuccessfully");
                }
                return;
            }
            Ok(None) => {}
            Err(_) => {
                *guard = None;
                let _ = DAEMON_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
                return;
            }
        }
    });
}

fn launch_unprivileged(binary: &str) {
    let _launch = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if !STOPPING.load(Ordering::SeqCst) {
        match std::process::Command::new(binary).spawn() {
            Ok(child) => supervise_daemon(child),
            Err(error) => eprintln!("[dboard-tray] Could not start daemon: {error}"),
        }
    }
}

fn try_start_systemd_user_service() -> bool {
    let ok = std::process::Command::new("systemctl")
        .args(["--user", "start", "dboard.service"])
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        std::thread::sleep(std::time::Duration::from_millis(150));
        return is_daemon_running();
    }
    false
}

fn start_daemon() {
    if is_daemon_running() {
        return;
    }
    // Prefer systemd user service (`dboard.service`) so closing the optional tray
    // icon never interrupts typing, and Wayland/AT-SPI/uaccess stay in user session.
    if try_start_systemd_user_service() {
        eprintln!("[dboard-tray] Attached to systemd user service dboard.service");
        return;
    }
    let daemon_bin = find_daemon_binary();
    eprintln!("[dboard-tray] Starting unprivileged daemon: {}", daemon_bin);
    launch_unprivileged(&daemon_bin);
}

/// Returns `true` only when `dboard-tray` itself spawned and owns the daemon
/// child process (as opposed to attaching to `dboard.service`).
pub fn owns_daemon_child() -> bool {
    DAEMON_PID.load(Ordering::SeqCst) != 0
}

fn stop_owned_daemon_child_locked() {
    // Take ownership of the un-reaped `Child` handle so the kernel cannot
    // recycle its PID until after we finish signaling and reaping it.
    let mut child_guard = DAEMON_CHILD.lock().unwrap_or_else(|p| p.into_inner());
    let Some(mut child) = child_guard.take() else {
        DAEMON_PID.store(0, Ordering::SeqCst);
        return;
    };
    drop(child_guard);
    let pid = child.id() as i32;
    DAEMON_PID.store(0, Ordering::SeqCst);
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    eprintln!("[dboard-tray] Stopping tray-spawned daemon (PID {})", pid);
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    for _ in 0..20 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            eprintln!("[dboard-tray] Daemon exited");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Still alive and un-reaped: PID belongs exclusively to our child.
    let _ = child.kill();
    let _ = child.wait();
}

pub fn stop_daemon() {
    STOPPING.store(true, Ordering::SeqCst);
    let _launch = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    stop_owned_daemon_child_locked();
}

/// Restart the daemon after a `keyboard_layout` change so `evdev_loop`,
/// `WaylandVirtualKeyboard` (single-upload XKB keymap), and `Engine` stay 100%
/// synchronized. Runs off the D-Bus tray callback thread.
pub fn restart_daemon_for_layout(layout_label: &str) {
    let label = layout_label.to_string();
    std::thread::spawn(move || {
        let restarted = if owns_daemon_child() {
            let _launch = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            if STOPPING.load(Ordering::SeqCst) {
                return;
            }
            stop_owned_daemon_child_locked();
            let daemon_bin = find_daemon_binary();
            match std::process::Command::new(&daemon_bin).spawn() {
                Ok(child) => {
                    supervise_daemon(child);
                    true
                }
                Err(err) => {
                    eprintln!("[dboard-tray] Failed to restart daemon: {err}");
                    false
                }
            }
        } else {
            let is_systemd = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", "dboard.service"])
                .status()
                .is_ok_and(|s| s.success());
            if is_systemd {
                std::process::Command::new("systemctl")
                    .args(["--user", "restart", "dboard.service"])
                    .status()
                    .is_ok_and(|s| s.success())
            } else {
                false
            }
        };

        let body = if restarted {
            format!("Keyboard layout switched to {label}")
        } else {
            format!("Layout set to {label}. Restart dboard-daemon to apply XKB keymap.")
        };
        if let Ok(mut child) = std::process::Command::new("notify-send")
            .args([
                "-t",
                "2000",
                "-h",
                "string:x-canonical-private-synchronous:dboard-osd",
                "-a",
                "Dboard",
                "Dboard",
                &body,
            ])
            .spawn()
        {
            let _ = child.wait();
        }
    });
}

fn ensure_single_instance(name: &str) {
    let uid = unsafe { libc::getuid() };
    let path = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty() && std::path::Path::new(dir).is_dir())
        .map(|dir| format!("{}/{}.lock", dir, name))
        .unwrap_or_else(|| format!("/tmp/{}-{}.lock", name, uid));
    let Ok(path_c) = std::ffi::CString::new(path) else {
        std::process::exit(1);
    };
    let fd = unsafe {
        libc::open(
            path_c.as_ptr(),
            libc::O_CREAT | libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        eprintln!("[{}] Failed to open lock file", name);
        std::process::exit(1);
    }
    let res = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if res < 0 {
        eprintln!("[{}] Another instance is already running. Exiting.", name);
        std::process::exit(0);
    }
}

pub fn should_exit() -> bool {
    STOPPING.load(Ordering::SeqCst)
}

pub fn request_exit() {
    STOPPING.store(true, Ordering::SeqCst);
}

extern "C" fn handle_termination_signal(_sig: libc::c_int) {
    STOPPING.store(true, Ordering::SeqCst);
}

fn main() {
    unsafe {
        let _ = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        let _ = libc::signal(
            libc::SIGINT,
            handle_termination_signal as *const () as libc::sighandler_t,
        );
        let _ = libc::signal(
            libc::SIGTERM,
            handle_termination_signal as *const () as libc::sighandler_t,
        );
    }
    ensure_single_instance("dboard-tray");

    eprintln!("[dboard-tray] Starting");
    start_daemon();
    tray::run();
    stop_daemon();
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn stop_daemon_is_noop_when_managed_by_systemd_service() {
        let _guard = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        DAEMON_PID.store(0, Ordering::SeqCst);
        stop_owned_daemon_child_locked();
        assert_eq!(DAEMON_PID.load(Ordering::SeqCst), 0);
        STOPPING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn stop_owned_daemon_child_reaps_without_setting_stopping() {
        let _guard = LAUNCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        STOPPING.store(false, Ordering::SeqCst);
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        DAEMON_PID.store(pid, Ordering::SeqCst);
        *DAEMON_CHILD.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);

        stop_owned_daemon_child_locked();

        assert_eq!(DAEMON_PID.load(Ordering::SeqCst), 0);
        assert!(!STOPPING.load(Ordering::SeqCst));
        assert!(DAEMON_CHILD.lock().unwrap().is_none());
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
    }

    #[test]
    fn background_supervision_returns_before_daemon_exits_and_reaps_it() {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let (sender, receiver) = std::sync::mpsc::channel();
        let reaper = reap_in_background(child, move |result| sender.send(result.unwrap()).unwrap());
        let running = receiver.try_recv().is_err();
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        reaper.join().unwrap();
        assert!(running);
        assert!(!receiver.recv().unwrap().success());
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
    }
}
