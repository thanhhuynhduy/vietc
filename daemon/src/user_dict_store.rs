// SPDX-License-Identifier: MIT
//! Atomic `0600` persistence for Dboard's on-device personal dictionary
//! (`~/.local/share/dboard/user_dict.txt`).

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use dboard_engine::UserDict;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn save_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct SaveState {
    pending: std::collections::HashMap<PathBuf, (String, bool)>,
    in_flight: bool,
    shutdown: bool,
}

struct SaveQueue {
    state: Mutex<SaveState>,
    cv: Condvar,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

static QUEUE: OnceLock<std::sync::Arc<SaveQueue>> = OnceLock::new();

fn save_worker() -> &'static std::sync::Arc<SaveQueue> {
    QUEUE.get_or_init(|| {
        let q = std::sync::Arc::new(SaveQueue {
            state: Mutex::new(SaveState {
                pending: std::collections::HashMap::new(),
                in_flight: false,
                shutdown: false,
            }),
            cv: Condvar::new(),
            handle: Mutex::new(None),
        });
        let worker_q = std::sync::Arc::clone(&q);
        let join_handle = std::thread::Builder::new()
            .name("dboard-dict-save".into())
            .spawn(move || loop {
                let item = {
                    let mut guard = worker_q.state.lock().unwrap_or_else(|p| p.into_inner());
                    while guard.pending.is_empty() && !guard.shutdown {
                        guard = worker_q.cv.wait(guard).unwrap_or_else(|p| p.into_inner());
                    }
                    if let Some(path) = guard.pending.keys().next().cloned() {
                        let (content, fsync) = guard.pending.remove(&path).unwrap();
                        guard.in_flight = true;
                        Some((path, content, fsync))
                    } else {
                        break;
                    }
                };
                if let Some((path, content, fsync)) = item {
                    let _ = write_atomic_0600(&path, &content, fsync);
                    let mut guard = worker_q.state.lock().unwrap_or_else(|p| p.into_inner());
                    guard.in_flight = false;
                    worker_q.cv.notify_all();
                }
            })
            .ok();
        *q.handle.lock().unwrap_or_else(|p| p.into_inner()) = join_handle;
        q
    })
}

/// Coalescing single-worker save: always persists the latest snapshot per path
/// in strict monotonic order without blocking `evdev_loop`.
pub fn enqueue_user_dict_save(path: PathBuf, serialized: String) {
    enqueue_atomic_file_write(path, serialized, true);
}

/// Enqueues a non-blocking atomic `0600` file write (e.g. `status`, `method`,
/// `overrides.toml`) onto the background persistence thread so `evdev_loop`
/// never performs synchronous disk I/O or `fsync`.
pub fn enqueue_atomic_file_write(path: PathBuf, content: String, fsync: bool) {
    let q = save_worker();
    let mut guard = q.state.lock().unwrap_or_else(|p| p.into_inner());
    if guard.shutdown {
        drop(guard);
        let _ = write_atomic_0600(&path, &content, fsync);
        return;
    }
    guard.pending.insert(path, (content, fsync));
    q.cv.notify_one();
}

/// Flushes all pending background writes and joins the persistence thread cleanly
/// on daemon shutdown (`SIGTERM`/`SIGINT`) so no One-Tap Undo or status update is lost.
pub fn flush_and_shutdown() {
    let Some(q) = QUEUE.get() else {
        return;
    };
    {
        let mut guard = q.state.lock().unwrap_or_else(|p| p.into_inner());
        guard.shutdown = true;
    }
    q.cv.notify_all();
    if let Some(handle) = q.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
        let _ = handle.join();
    }
}

/// Waits until all currently queued background writes have finished (used by tests).
#[cfg(test)]
pub fn flush_pending_for_test() {
    let Some(q) = QUEUE.get() else {
        return;
    };
    let mut guard = q.state.lock().unwrap_or_else(|p| p.into_inner());
    while !guard.pending.is_empty() || guard.in_flight {
        guard = q.cv.wait(guard).unwrap_or_else(|p| p.into_inner());
    }
}

pub fn write_atomic_0600(path: &Path, content: &str, fsync: bool) -> std::io::Result<()> {
    let _guard = save_lock().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && parent != Path::new("/tmp") {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(
        ".{}.tmp.{}.{seq}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        std::process::id()
    );
    let tmp_path = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(&tmp_name));
    let _ = fs::remove_file(&tmp_path);
    let write_res = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        if fsync {
            file.sync_all()?;
        }
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res
}

pub fn user_dict_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            return Some(PathBuf::from(xdg_data).join("dboard").join("user_dict.txt"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("dboard")
                    .join("user_dict.txt"),
            );
        }
    }
    None
}

pub fn load_user_dict_from(path: &Path) -> UserDict {
    match fs::read_to_string(path) {
        Ok(content) => UserDict::load_from_str(&content),
        Err(_) => UserDict::new(),
    }
}

#[cfg(test)]
pub fn save_user_dict_to(path: &Path, serialized: &str) -> std::io::Result<()> {
    write_atomic_0600(path, serialized, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn atomic_save_and_load_preserves_entries_and_0600_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dboard").join("user_dict.txt");

        let mut dict = UserDict::new();
        dict.record_keep_vn("lets");
        dict.record_keep_raw("teh");

        save_user_dict_to(&path, &dict.serialize()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let loaded = load_user_dict_from(&path);
        assert!(loaded.is_keep_vn("lets"));
        assert!(loaded.is_keep_raw("teh"));
    }

    #[test]
    fn concurrent_saves_from_multiple_threads_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dboard").join("user_dict.txt");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let p = path.clone();
                std::thread::spawn(move || {
                    let mut dict = UserDict::new();
                    dict.record_keep_raw(&format!("word{}", (b'a' + i) as char));
                    save_user_dict_to(&p, &dict.serialize()).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(path.exists());
    }

    #[test]
    fn async_queue_flushes_latest_snapshot_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dboard").join("user_dict.txt");
        let mut dict = UserDict::new();
        dict.record_keep_raw("quux");
        enqueue_user_dict_save(path.clone(), dict.serialize());
        flush_pending_for_test();
        let loaded = load_user_dict_from(&path);
        assert!(loaded.is_keep_raw("quux"));
    }
}
