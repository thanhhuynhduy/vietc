// SPDX-License-Identifier: MIT
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

mod config;
mod tray;

static DAEMON_PID: AtomicI32 = AtomicI32::new(0);
static STOPPING: AtomicBool = AtomicBool::new(false);
static LAUNCH_LOCK: Mutex<()> = Mutex::new(());

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin"))
}

fn find_daemon_binary() -> String {
    for candidate in [
        "/usr/bin/vietc-daemon",
        "/usr/bin/vietc",
        "/usr/local/bin/vietc-daemon",
        "/usr/local/bin/vietc",
        "/app/bin/vietc-daemon",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    let dir = exe_dir();
    for candidate in [dir.join("vietc-daemon"), dir.join("vietc")] {
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    "vietc-daemon".to_string()
}

fn is_daemon_running() -> bool {
    let uid = unsafe { libc::getuid() };
    let lock_path = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty() && std::path::Path::new(dir).is_dir())
        .map(|dir| format!("{}/vietc-daemon.lock", dir))
        .unwrap_or_else(|| format!("/tmp/vietc-daemon-{}.lock", uid));
    if let Ok(path_c) = std::ffi::CString::new(lock_path) {
        let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd >= 0 {
            let locked = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } != 0;
            if !locked {
                let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
            }
            unsafe {
                libc::close(fd);
            }
            if locked {
                return true;
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
    check("vietc") || check("vietc-daemon")
}

fn reap_in_background(
    mut child: std::process::Child,
    on_exit: impl FnOnce(std::io::Result<std::process::ExitStatus>) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || on_exit(child.wait()))
}

fn supervise_daemon(child: std::process::Child) {
    let pid = child.id() as i32;
    DAEMON_PID.store(pid, Ordering::SeqCst);
    reap_in_background(child, move |result| {
        let _ = DAEMON_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
        if !result.is_ok_and(|status| status.success()) {
            eprintln!("[vietc-tray] Supervised daemon exited unsuccessfully");
        }
    });
}

fn launch_unprivileged(binary: &str) {
    let _launch = LAUNCH_LOCK.lock().unwrap();
    if !STOPPING.load(Ordering::SeqCst) {
        match std::process::Command::new(binary).spawn() {
            Ok(child) => supervise_daemon(child),
            Err(error) => eprintln!("[vietc-tray] Could not start daemon: {error}"),
        }
    }
}

fn try_start_systemd_user_service() -> bool {
    let ok = std::process::Command::new("systemctl")
        .args(["--user", "start", "vietc.service"])
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
    // Prefer systemd user service (`vietc.service`) so closing the optional tray
    // icon never interrupts typing, and Wayland/AT-SPI/uaccess stay in user session.
    if try_start_systemd_user_service() {
        eprintln!("[vietc-tray] Attached to systemd user service vietc.service");
        return;
    }
    let daemon_bin = find_daemon_binary();
    eprintln!("[vietc-tray] Starting unprivileged daemon: {}", daemon_bin);
    launch_unprivileged(&daemon_bin);
}

pub fn stop_daemon() {
    STOPPING.store(true, Ordering::SeqCst);
    let _launch = LAUNCH_LOCK.lock().unwrap();
    let pid = DAEMON_PID.load(Ordering::SeqCst);
    // Only stop a daemon directly spawned by this tray process (`DAEMON_PID > 0`).
    // Never `pkill` a `vietc.service` instance managed by systemd.
    if pid > 0 {
        eprintln!("[vietc-tray] Stopping tray-spawned daemon (PID {})", pid);
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if unsafe { libc::kill(pid, 0) } != 0 {
                eprintln!("[vietc-tray] Daemon exited");
                return;
            }
        }
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

fn ensure_single_instance(name: &str) {
    let uid = unsafe { libc::getuid() };
    let path = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty() && std::path::Path::new(dir).is_dir())
        .map(|dir| format!("{}/{}.lock", dir, name))
        .unwrap_or_else(|| format!("/tmp/{}-{}.lock", name, uid));
    let path_c = std::ffi::CString::new(path).unwrap();
    let fd = unsafe {
        libc::open(
            path_c.as_ptr(),
            libc::O_CREAT | libc::O_RDWR | libc::O_NOFOLLOW,
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

fn main() {
    unsafe {
        let _ = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    ensure_single_instance("vietc-tray");

    eprintln!("[vietc-tray] Starting");
    start_daemon();
    tray::run();
    stop_daemon();
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn stop_daemon_is_noop_when_managed_by_systemd_service() {
        DAEMON_PID.store(0, Ordering::SeqCst);
        stop_daemon();
        assert_eq!(DAEMON_PID.load(Ordering::SeqCst), 0);
        STOPPING.store(false, Ordering::SeqCst);
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
