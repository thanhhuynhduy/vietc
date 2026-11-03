// SPDX-License-Identifier: MIT
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

mod config;
mod tray;

static DAEMON_PID: AtomicI32 = AtomicI32::new(0);
static SUDO_MONITOR_PID: AtomicI32 = AtomicI32::new(0);
static STOPPING: AtomicBool = AtomicBool::new(false);
static LAUNCH_LOCK: Mutex<()> = Mutex::new(());

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin"))
}

fn find_sibling_binary(name: &str) -> String {
    let dir = exe_dir();
    // Try exact name (e.g. "vietc" outside Flatpak)
    let sibling = dir.join(name);
    if sibling.exists() {
        return sibling.to_string_lossy().into_owned();
    }
    // Try name-daemon (e.g. "vietc-daemon" inside Flatpak)
    let daemon = dir.join(format!("{}-daemon", name));
    if daemon.exists() {
        return daemon.to_string_lossy().into_owned();
    }
    name.to_string()
}

fn is_daemon_running() -> bool {
    // Check both "vietc" (outside Flatpak) and "vietc-daemon" (inside Flatpak)
    let check = |name: &str| -> bool {
        std::process::Command::new("pgrep")
            .arg("-x")
            .arg(name)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    check("vietc") || check("vietc-daemon")
}

fn is_flatpak() -> bool {
    std::env::var("FLATPAK_ID").is_ok() || std::path::Path::new("/app/bin").exists()
}

fn needs_root() -> bool {
    if is_flatpak() {
        // Inside Flatpak the sandbox already has device access; sudo won't work.
        return false;
    }
    // Check if we can access /dev/uinput directly (user in input group or has ACL)
    let uinput = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput");
    if uinput.is_ok() {
        return false; // Can grab + inject without root
    }
    let cfg = config::Config::load();
    cfg.grab
}

/// Show a password prompt using available desktop tools.
/// Returns the password, or empty string if cancelled.
fn prompt_password() -> String {
    let title = "Viet+";
    let msg = "Viet+ needs root privileges to capture keyboard input.\nPlease enter your password:";

    // Try zenity (GNOME)
    if let Ok(output) = std::process::Command::new("zenity")
        .args(["--password", "--title", title, "--text", msg])
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            let pw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !pw.is_empty() {
                return pw;
            }
        }
    }

    // Try kdialog (KDE)
    if let Ok(output) = std::process::Command::new("kdialog")
        .args(["--password", msg])
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            let pw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !pw.is_empty() {
                return pw;
            }
        }
    }

    // Try ssh-askpass (X11 fallback)
    if let Ok(output) = std::process::Command::new("ssh-askpass")
        .arg(msg)
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            let pw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !pw.is_empty() {
                return pw;
            }
        }
    }

    // Last resort: terminal prompt
    eprintln!("{}", msg);
    if let Ok(child) = std::process::Command::new("sh")
        .arg("-c")
        .arg("read -s -p 'Password: ' pw && echo \"$pw\"")
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        if let Ok(output) = child.wait_with_output() {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
    }

    String::new()
}

fn reap_in_background(
    mut child: std::process::Child,
    on_exit: impl FnOnce(std::io::Result<std::process::ExitStatus>) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || on_exit(child.wait()))
}

fn supervise_daemon(child: std::process::Child, fallback: Option<String>) {
    // Track sudo's monitor, which relays signals to its privileged command.
    let pid = child.id() as i32;
    DAEMON_PID.store(pid, Ordering::SeqCst);
    if fallback.is_some() {
        SUDO_MONITOR_PID.store(pid, Ordering::SeqCst);
    }
    reap_in_background(child, move |result| {
        let _ = DAEMON_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
        let _ = SUDO_MONITOR_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
        if !result.is_ok_and(|status| status.success()) {
            eprintln!("[vietc-tray] Daemon exited unsuccessfully");
            if let Some(binary) = fallback {
                launch_unprivileged(&binary);
            }
        }
    });
}

fn launch_unprivileged(binary: &str) {
    let _launch = LAUNCH_LOCK.lock().unwrap();
    if !STOPPING.load(Ordering::SeqCst) {
        match std::process::Command::new(binary).spawn() {
            Ok(child) => supervise_daemon(child, None),
            Err(error) => eprintln!("[vietc-tray] Could not start daemon: {error}"),
        }
    }
}

fn start_daemon() {
    let daemon_bin = find_sibling_binary("vietc");

    if needs_root() && !is_daemon_running() {
        let password = prompt_password();
        if password.is_empty() {
            eprintln!("[vietc-tray] No password provided, starting daemon without root");
            launch_unprivileged(&daemon_bin);
        } else {
            // Start daemon with sudo, explicitly passing display/auth environment variables
            let display = std::env::var("DISPLAY").unwrap_or_default();
            let xauthority = std::env::var("XAUTHORITY").unwrap_or_default();
            let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();

            let mut args = vec!["-S".to_string()];
            if !display.is_empty() {
                args.push(format!("DISPLAY={}", display));
            }
            if !xauthority.is_empty() {
                args.push(format!("XAUTHORITY={}", xauthority));
            }
            if !wayland_display.is_empty() {
                args.push(format!("WAYLAND_DISPLAY={}", wayland_display));
            }
            args.push(daemon_bin.clone());

            let mut child = match std::process::Command::new("sudo")
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[vietc-tray] Failed to start daemon with sudo: {}", e);
                    eprintln!(
                        "[vietc-tray] Try: sudo setcap cap_sys_admin+ep $(which vietc-daemon)"
                    );
                    eprintln!("[vietc-tray] Or start manually: sudo vietc-daemon &");
                    launch_unprivileged(&daemon_bin);
                    return;
                }
            };

            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(format!("{}\n", password).as_bytes());
            }
            // No persistent "first launch" flag: starting sudo grants no
            // device permissions to future unprivileged daemon instances.
            supervise_daemon(child, Some(daemon_bin));
        }
        return;
    }

    if !is_daemon_running() {
        eprintln!("[vietc-tray] Starting daemon: {}", daemon_bin);
        launch_unprivileged(&daemon_bin);
    }
}

fn process_tree(root: i32) -> Vec<i32> {
    let mut processes = vec![root];
    let mut index = 0;
    while index < processes.len() && processes.len() < 256 {
        let pid = processes[index];
        if let Ok(children) = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
            for child in children
                .split_whitespace()
                .filter_map(|pid| pid.parse::<i32>().ok())
            {
                if child > 0 && !processes.contains(&child) {
                    processes.push(child);
                }
            }
        }
        index += 1;
    }
    processes.reverse(); // Terminate commands before their signal relays.
    processes
}

fn privileged_kill_command(targets: &[i32]) -> std::process::Command {
    let mut command = std::process::Command::new("sudo");
    command.args(["-n", "/bin/kill", "-KILL", "--"]);
    command.args(targets.iter().filter(|pid| **pid > 0).map(i32::to_string));
    command
}

pub fn stop_daemon() {
    STOPPING.store(true, Ordering::SeqCst);
    let _launch = LAUNCH_LOCK.lock().unwrap();
    let pid = DAEMON_PID.load(Ordering::SeqCst);
    if pid > 0 {
        eprintln!("[vietc-tray] Stopping daemon (PID {})", pid);
        let privileged = SUDO_MONITOR_PID.load(Ordering::SeqCst) == pid;
        let targets = if privileged {
            process_tree(pid)
        } else {
            Vec::new()
        };
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        // Wait up to 2s for graceful exit
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if unsafe { libc::kill(pid, 0) } != 0 {
                eprintln!("[vietc-tray] Daemon exited");
                return;
            }
        }
        // SIGKILL cannot be forwarded by sudo. Escalate against the entire
        // captured command tree with cached authorization, never just the relay.
        if privileged {
            if !privileged_kill_command(&targets)
                .status()
                .is_ok_and(|status| status.success())
            {
                eprintln!("[vietc-tray] Could not terminate privileged daemon; administrator cleanup is required");
            }
        } else {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        return;
    }

    // Fallback: pkill both possible names
    eprintln!("[vietc-tray] Stopping daemon via pkill");
    for name in &["vietc-daemon", "vietc"] {
        let _ = std::process::Command::new("pkill")
            .arg("-x")
            .arg(name)
            .status();
    }
}

fn ensure_single_instance(name: &str) {
    let uid = unsafe { libc::getuid() };
    let path = format!("/tmp/{}-{}.lock", name, uid);
    let path_c = std::ffi::CString::new(path).unwrap();
    let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
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
    // Ensure single instance to avoid duplicate tray icons
    ensure_single_instance("vietc-tray");

    eprintln!("[vietc-tray] Starting");

    // Start daemon (with password prompt if first launch)
    start_daemon();

    // Run the tray
    tray::run();

    // Tray exited normally — stop the daemon
    stop_daemon();
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn privileged_escalation_includes_command_and_monitor_without_a_shell() {
        let command = privileged_kill_command(&[4322, 4321]);
        let arguments: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            arguments,
            ["-n", "/bin/kill", "-KILL", "--", "4322", "4321"]
        );
        assert_eq!(command.get_program(), "sudo");
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
