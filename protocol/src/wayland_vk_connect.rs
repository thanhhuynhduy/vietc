// SPDX-License-Identifier: MIT
//! Non-blocking Wayland Unix socket discovery and connection with stale-socket
//! probing and bounded `EINPROGRESS` wait (`<= 50ms` so `evdev_loop` never stalls).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Tight upper bound for `EINPROGRESS` `POLLOUT` wait per candidate socket.
const CONNECT_INPROGRESS_TIMEOUT_MS: i32 = 50;

/// Resolve candidate Wayland socket paths in priority order (`WAYLAND_DISPLAY`
/// first, then `wayland-0..10`) and return the first live non-blocking stream.
/// Stale socket files left behind by a crashed compositor are skipped cleanly.
pub(crate) fn connect_wayland_socket() -> io::Result<UnixStream> {
    let candidates = collect_wayland_socket_candidates()?;
    let mut last_err: Option<io::Error> = None;
    for path in candidates {
        match connect_unix_nonblocking(&path) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no live Wayland socket found in XDG_RUNTIME_DIR",
        )
    }))
}

fn collect_wayland_socket_candidates() -> io::Result<Vec<PathBuf>> {
    if std::env::var_os("WAYLAND_SOCKET").is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "inherited WAYLAND_SOCKET is unsupported",
        ));
    }
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR missing"))?,
    );
    if !runtime.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDG_RUNTIME_DIR must be absolute",
        ));
    }

    let mut candidates = Vec::new();
    if let Some(display) = std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty()) {
        let candidate = PathBuf::from(display);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            runtime.join(candidate)
        };
        if path.exists() {
            candidates.push(path);
        }
    }

    for index in 0..10 {
        let candidate = runtime.join(format!("wayland-{index}"));
        if candidate.exists() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    if candidates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no Wayland socket found in XDG_RUNTIME_DIR",
        ));
    }
    Ok(candidates)
}

pub(crate) fn connect_unix_nonblocking(path: &Path) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Wayland socket path",
        ));
    }
    addr.sun_family = libc::AF_UNIX as _;
    for (dst, &src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = src as _;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let rc = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&addr as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&addr) as _,
        )
    };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINPROGRESS) {
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(CONNECT_INPROGRESS_TIMEOUT_MS as u64);
            loop {
                let remaining_ms = deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                if remaining_ms == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Wayland socket connect timed out",
                    ));
                }
                let mut pfd = libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let ready = unsafe { libc::poll(&mut pfd, 1, remaining_ms) };
                if ready < 0 {
                    let poll_err = io::Error::last_os_error();
                    if poll_err.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    return Err(poll_err);
                }
                if ready == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Wayland socket connect timed out",
                    ));
                }
                break;
            }
            let mut so_err: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let gso = unsafe {
                libc::getsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&mut so_err as *mut libc::c_int).cast(),
                    &mut len,
                )
            };
            if gso < 0 {
                return Err(io::Error::last_os_error());
            }
            if so_err != 0 {
                return Err(io::Error::from_raw_os_error(so_err));
            }
        } else {
            return Err(err);
        }
    }
    Ok(UnixStream::from(fd))
}

pub(crate) fn create_keymap_memfd(keymap_bytes: &[u8]) -> io::Result<(OwnedFd, u32)> {
    let raw = unsafe { libc::memfd_create(c"dboard-vk-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut buf = Vec::with_capacity(keymap_bytes.len() + 1);
    buf.extend_from_slice(keymap_bytes);
    buf.push(0); // XKB v1 Wayland format requires NUL-terminated buffer included in size.

    let mut offset = 0;
    while offset < buf.len() {
        let written = unsafe {
            libc::write(
                fd.as_raw_fd(),
                buf[offset..].as_ptr().cast(),
                buf.len() - offset,
            )
        };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "memfd_create write returned 0 bytes",
            ));
        }
        offset += written as usize;
    }
    let _ = unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_SET) };
    Ok((fd, buf.len() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn skips_stale_dead_socket_and_connects_to_live_socket() {
        let dir =
            std::env::temp_dir().join(format!("dboard-vk-connect-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a stale socket file (bound then listener dropped without unlinking)
        let stale = dir.join("wayland-0");
        {
            let _dead = UnixListener::bind(&stale).unwrap();
        }
        assert!(stale.exists());
        assert!(connect_unix_nonblocking(&stale).is_err());

        // Create a live socket and confirm non-blocking connect succeeds
        let live = dir.join("wayland-1");
        let _listener = UnixListener::bind(&live).unwrap();
        assert!(connect_unix_nonblocking(&live).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
