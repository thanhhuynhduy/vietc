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

struct SaveQueue {
    pending: Mutex<Option<(PathBuf, String)>>,
    cv: Condvar,
}

fn save_worker() -> &'static std::sync::Arc<SaveQueue> {
    static QUEUE: OnceLock<std::sync::Arc<SaveQueue>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let q = std::sync::Arc::new(SaveQueue {
            pending: Mutex::new(None),
            cv: Condvar::new(),
        });
        let worker_q = std::sync::Arc::clone(&q);
        let _ = std::thread::Builder::new()
            .name("dboard-dict-save".into())
            .spawn(move || loop {
                let item = {
                    let mut guard = worker_q.pending.lock().unwrap_or_else(|p| p.into_inner());
                    while guard.is_none() {
                        guard = worker_q.cv.wait(guard).unwrap_or_else(|p| p.into_inner());
                    }
                    guard.take()
                };
                if let Some((path, serialized)) = item {
                    let _ = save_user_dict_to(&path, &serialized);
                }
            });
        q
    })
}

/// Coalescing single-worker save: always persists the latest snapshot in strict
/// monotonic order without spawning detached threads that could race and overwrite
/// newer data with stale snapshots.
pub fn enqueue_user_dict_save(path: PathBuf, serialized: String) {
    let q = save_worker();
    {
        let mut guard = q.pending.lock().unwrap_or_else(|p| p.into_inner());
        *guard = Some((path, serialized));
    }
    q.cv.notify_one();
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

pub fn save_user_dict_to(path: &Path, serialized: &str) -> std::io::Result<()> {
    let _guard = save_lock().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = path.with_extension(format!("tmp.{}.{seq}", std::process::id()));
    let _ = fs::remove_file(&tmp_path);
    let write_res = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_path)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res
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
}
