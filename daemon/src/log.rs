use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn default_log_path() -> PathBuf {
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        if !state_home.is_empty() {
            return PathBuf::from(state_home).join("dboard").join("dboard.log");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let state_dir = PathBuf::from(&home)
                .join(".local")
                .join("state")
                .join("dboard");
            if state_dir.exists() || fs::create_dir_all(&state_dir).is_ok() {
                return state_dir.join("dboard.log");
            }
            let config_dir = PathBuf::from(home).join(".config").join("dboard");
            return config_dir.join("dboard.log");
        }
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/dboard-{}.log", uid))
}

pub fn init(
    debug_mode: bool,
    custom_level: Option<&str>,
    custom_log_file: Option<&Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let log_file_path = match custom_log_file {
        Some(p) => p.to_path_buf(),
        None => default_log_path(),
    };

    if let Some(parent) = log_file_path.parent() {
        use std::os::unix::fs::DirBuilderExt;
        if !parent.as_os_str().is_empty() && parent != Path::new("/tmp") {
            let _ = fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent);
        }
    }

    // Rotate log if > 10MB
    if let Ok(meta) = fs::metadata(&log_file_path) {
        if meta.len() > 10 * 1024 * 1024 {
            let mut old_path = log_file_path.clone();
            old_path.set_extension("log.old");
            let _ = fs::rename(&log_file_path, &old_path);
        }
    }

    let default_level = if debug_mode {
        "debug"
    } else {
        custom_level.unwrap_or("info")
    };

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&log_file_path)?;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(Arc::new(file));

    let (journald_layer, stderr_layer) = match tracing_journald::layer() {
        Ok(journald) => (Some(journald), None),
        Err(error) => {
            eprintln!("[dboard] native journald logging unavailable ({error}); using stderr");
            (
                None,
                Some(tracing_subscriber::fmt::layer().with_target(true)),
            )
        }
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(journald_layer)
        .with(stderr_layer)
        .try_init()?;

    Ok(log_file_path)
}

pub fn log_info(msg: &str) {
    tracing::info!("{msg}");
}

pub fn log_debug(msg: &str) {
    tracing::debug!("{msg}");
}

pub fn log_trace(msg: &str) {
    tracing::trace!("{msg}");
}

pub fn is_debug_active(config_debug: bool) -> bool {
    config_debug || tracing::enabled!(tracing::Level::DEBUG)
}
