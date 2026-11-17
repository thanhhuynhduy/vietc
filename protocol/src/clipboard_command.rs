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

fn pause(fd: i32, deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    // Round up to milliseconds; short ticks also bound launcher-exit latency.
    let millis = remaining.as_millis().saturating_add(1).min(10) as i32;
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLOUT,
        revents: 0,
    };
    // poll ignores negative descriptors, allowing the same bounded wait after EOF.
    let result = unsafe { libc::poll(&mut pollfd, 1, millis) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    } else if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
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
