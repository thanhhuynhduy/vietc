// SPDX-License-Identifier: MIT
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Success confirms only launcher exit; the caller must verify publication.
pub(crate) fn run_copy(mut command: Command, bytes: &[u8], timeout: Duration) -> bool {
    let Some(deadline) = Instant::now().checked_add(timeout) else {
        return false;
    };
    if Instant::now() >= deadline {
        return false;
    }
    let Ok(mut child) = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let result = (|| -> io::Result<bool> {
        let mut stdin = child.stdin.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let fd = stdin.as_raw_fd();
        // The owned pipe stays open throughout both fcntl calls and all writes.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut remaining = bytes;
        while !remaining.is_empty() {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            match stdin.write(&remaining[..remaining.len().min(4096)]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => remaining = &remaining[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => pause(fd, deadline)?,
                Err(e) => return Err(e),
            }
        }
        drop(stdin); // EOF must precede waiting for the launcher.
        loop {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status.success()),
                Ok(None) => pause(-1, deadline)?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    })();
    if matches!(result, Ok(true)) {
        return true;
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// Captures child stdout with a bounded timeout and byte limit without deadlocking
/// when stdout exceeds the OS pipe buffer (~64 KiB).
pub(crate) fn run_capture(
    mut command: Command,
    timeout: Duration,
    max_bytes: usize,
) -> Option<std::process::Output> {
    use std::io::Read;
    let deadline = Instant::now().checked_add(timeout)?;
    if Instant::now() >= deadline {
        return None;
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }

    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut eof = false;
    while !eof {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match stdout.read(&mut chunk) {
            Ok(0) => eof = true,
            Ok(n) => {
                if out.len().saturating_add(n) > max_bytes {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                out.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if pause_events(fd, libc::POLLIN, deadline).is_err() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    drop(stdout);

    loop {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Some(std::process::Output {
                    status,
                    stdout: out,
                    stderr: Vec::new(),
                });
            }
            Ok(None) => {
                if pause_events(-1, 0, deadline).is_err() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn pause(fd: i32, deadline: Instant) -> io::Result<()> {
    pause_events(fd, libc::POLLOUT, deadline)
}

fn pause_events(fd: i32, events: i16, deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    // Round up to milliseconds; short ticks also bound launcher-exit latency.
    let millis = remaining.as_millis().saturating_add(1).min(10) as i32;
    let mut pollfd = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // poll ignores negative descriptors, allowing the same bounded wait after EOF.
    let result = unsafe { libc::poll(&mut pollfd, 1, millis) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    } else if fd >= 0
        && (((events & libc::POLLOUT) != 0
            && pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0)
            || ((events & libc::POLLIN) != 0
                && pollfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0))
    {
        return Err(io::ErrorKind::BrokenPipe.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn closes_stdin_and_waits_for_success_including_empty_payload() {
        for bytes in [&b"hello"[..], &b""[..]] {
            assert!(run_copy(
                shell("cat >/dev/null; exit 0"),
                bytes,
                Duration::from_secs(2)
            ));
        }
    }

    #[test]
    fn transfers_large_payload_completely() {
        let bytes = vec![b'x'; 2 * 1024 * 1024];
        assert!(run_copy(
            shell("test \"$(wc -c)\" -eq 2097152"),
            &bytes,
            Duration::from_secs(3)
        ));
    }

    #[test]
    fn captures_payload_larger_than_pipe_buffer_without_deadlock() {
        let output = run_capture(
            shell("dd if=/dev/zero bs=1024 count=256 2>/dev/null"),
            Duration::from_secs(2),
            1024 * 1024,
        )
        .expect("256 KiB stdout must drain without pipe buffer deadlock");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 256 * 1024);
    }

    #[test]
    fn rejects_nonzero_exit_after_reading() {
        assert!(!run_copy(
            shell("cat >/dev/null; exit 7"),
            b"hello",
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn rejects_closed_stdin_even_with_successful_exit() {
        assert!(!run_copy(
            shell("exec 0<&-; exit 0"),
            &vec![0; 1024 * 1024],
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn bounds_stalled_write_and_wait() {
        for bytes in [&b""[..], &vec![0; 1024 * 1024][..]] {
            let mut command = Command::new("/bin/sleep");
            command.arg("10");
            let start = Instant::now();
            assert!(!run_copy(command, bytes, Duration::from_millis(100)));
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }

    #[test]
    fn rejects_spawn_failure_and_expired_deadline() {
        assert!(!run_copy(
            Command::new("/nonexistent/vietc-copy-helper"),
            b"",
            Duration::from_secs(1)
        ));
        assert!(!run_copy(shell("exit 0"), b"", Duration::ZERO));
    }
}
