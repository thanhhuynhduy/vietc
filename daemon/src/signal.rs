use std::sync::atomic::{AtomicBool, Ordering};

pub static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);

extern "C" fn sigexit_handler(_signo: i32) {
    SIGNAL_EXIT.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    use std::mem;
    unsafe {
        let mut act: libc::sigaction = mem::zeroed();
        libc::sigemptyset(&mut act.sa_mask);
        act.sa_flags = 0;
        act.sa_sigaction = sigexit_handler as *const () as usize;
        libc::sigaction(libc::SIGINT, &act, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
    }
}

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstanceLockError {
    AlreadyRunning,
    Open(std::io::Error),
    Lock(std::io::Error),
    Write(std::io::Error),
}

impl std::fmt::Display for InstanceLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(formatter, "another instance is already running"),
            Self::Open(error) => write!(formatter, "cannot open lock file: {}", error),
            Self::Lock(error) => write!(formatter, "cannot acquire instance lock: {}", error),
            Self::Write(error) => write!(formatter, "cannot write lock owner PID: {}", error),
        }
    }
}

impl std::error::Error for InstanceLockError {}

pub struct InstanceLock {
    file: File,
}

impl AsRawFd for InstanceLock {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn acquire_instance_lock(path: &Path) -> Result<InstanceLock, InstanceLockError> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(InstanceLockError::Open)?;

    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::WouldBlock {
            Err(InstanceLockError::AlreadyRunning)
        } else {
            Err(InstanceLockError::Lock(error))
        };
    }

    file.set_len(0).map_err(InstanceLockError::Write)?;
    file.seek(SeekFrom::Start(0))
        .map_err(InstanceLockError::Write)?;
    write!(file, "{}", std::process::id()).map_err(InstanceLockError::Write)?;
    file.flush().map_err(InstanceLockError::Write)?;

    Ok(InstanceLock { file })
}

pub fn ensure_single_instance(name: &str) -> Result<InstanceLock, InstanceLockError> {
    let uid = unsafe { libc::getuid() };
    let path = PathBuf::from(format!("/tmp/{}-{}.lock", name, uid));
    acquire_instance_lock(&path)
}

#[cfg(test)]
mod phase3_tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    #[test]
    fn contender_cannot_acquire_lock_when_pid_text_is_stale() {
        let directory = tempfile::tempdir().unwrap();
        let lock_path = directory.path().join("vietc.lock");

        let holder = acquire_instance_lock(&lock_path).unwrap();
        std::fs::write(&lock_path, "999999").unwrap();

        let contender = acquire_instance_lock(&lock_path);

        assert!(matches!(contender, Err(InstanceLockError::AlreadyRunning)));
        drop(holder);
    }

    #[test]
    fn dropping_guard_releases_kernel_lock() {
        let directory = tempfile::tempdir().unwrap();
        let lock_path = directory.path().join("vietc.lock");

        let holder = acquire_instance_lock(&lock_path).unwrap();
        drop(holder);

        let replacement = acquire_instance_lock(&lock_path);
        assert!(replacement.is_ok());
    }

    #[test]
    fn pid_is_written_through_the_locked_file_descriptor() {
        let directory = tempfile::tempdir().unwrap();
        let lock_path = directory.path().join("vietc.lock");

        let guard = acquire_instance_lock(&lock_path).unwrap();
        let file = OpenOptions::new().read(true).open(&lock_path).unwrap();

        assert!(guard.as_raw_fd() >= 0);
        assert_eq!(
            std::fs::read_to_string(&lock_path).unwrap(),
            std::process::id().to_string()
        );

        drop(file);
        drop(guard);
    }
}
