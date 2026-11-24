// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRestoreConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_restore_keys")]
    pub trigger_keys: Vec<String>,

    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, toml::Value>,
}

impl Default for AutoRestoreConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_keys: default_restore_keys(),
            extra: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppStateConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub english_apps: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vietnamese_apps: Option<Vec<String>>,

    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_input_method")]
    pub input_method: String,

    #[serde(default = "default_toggle_key")]
    pub toggle_key: String,

    #[serde(default = "default_toggle_modes")]
    pub toggle_modes: Vec<String>,

    #[serde(default = "default_start_enabled")]
    pub start_enabled: bool,

    #[serde(default = "default_false")]
    pub autocorrect: bool,

    #[serde(default = "default_false")]
    pub auto_capitalize: bool,

    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,

    #[serde(default = "default_false")]
    pub candidate_bar: bool,

    #[serde(default)]
    pub auto_restore: AutoRestoreConfig,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_state: Option<AppStateConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macros: Option<std::collections::HashMap<String, String>>,

    #[serde(default = "default_grab")]
    pub grab: bool,

    #[serde(default = "default_false")]
    pub debug: bool,

    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, toml::Value>,
}

fn default_input_method() -> String {
    "telex".into()
}
fn default_toggle_key() -> String {
    "space".into()
}
fn default_toggle_modes() -> Vec<String> {
    vec!["vn".into(), "en".into()]
}
fn default_start_enabled() -> bool {
    true
}
fn default_keyboard_layout() -> String {
    "us".into()
}
fn default_grab() -> bool {
    true
}
fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_restore_keys() -> Vec<String> {
    vec!["space".into(), "escape".into()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input_method: default_input_method(),
            toggle_key: default_toggle_key(),
            toggle_modes: default_toggle_modes(),
            start_enabled: default_start_enabled(),
            autocorrect: default_false(),
            auto_capitalize: default_false(),
            keyboard_layout: default_keyboard_layout(),
            candidate_bar: default_false(),
            auto_restore: AutoRestoreConfig::default(),
            app_state: None,
            macros: None,
            grab: default_grab(),
            debug: default_false(),
            extra: Default::default(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        for path in config_paths() {
            if let Ok(content) = fs::read_to_string(&path) {
                match toml::from_str::<Config>(&content) {
                    Ok(config) => return config,
                    Err(err) => {
                        eprintln!(
                            "[dboard-tray] Warning: invalid TOML in {}: {}",
                            path.display(),
                            err
                        );
                    }
                }
            }
        }
        Self::default()
    }

    /// Load config strictly before mutating & saving back to `~/.config/dboard/config.toml`.
    /// If the user's config file exists but has a TOML syntax error (or cannot be read),
    /// returns `Err` instead of falling back to defaults and wiping custom user settings.
    pub fn load_for_update() -> Result<Self, String> {
        Self::load_for_update_from(&user_config_path())
    }

    fn load_for_update_from(path: &std::path::Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(content) => toml::from_str::<Config>(&content).map_err(|err| {
                format!(
                    "Refusing to overwrite invalid config {}: {}",
                    path.display(),
                    err
                )
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::load()),
            Err(err) => Err(format!(
                "Refusing to overwrite unreadable config {}: {}",
                path.display(),
                err
            )),
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::DirBuilderExt;
        let path = user_config_path();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)?;
            }
        }
        self.save_to(&path)
    }

    #[allow(dead_code)]
    fn save_to(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);
        let content = toml::to_string_pretty(self)?;
        let temporary = path.with_file_name(format!(
            ".dboard-config-{}-{}.tmp",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let mode = fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        let result = (|| -> std::io::Result<()> {
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(())
    }
}

fn config_base_dir() -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .or_else(dirs::config_dir)
}

fn user_config_path() -> PathBuf {
    if let Some(base) = config_base_dir() {
        return base.join("dboard").join("config.toml");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/dboard-{uid}")).join("config.toml")
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config_dir) = config_base_dir() {
        paths.push(config_dir.join("dboard").join("config.toml"));
    }
    paths.push(PathBuf::from("/etc/dboard/config.toml"));
    paths
}

pub fn is_autostart_installed() -> bool {
    if let Some(config_dir) = config_base_dir() {
        config_dir
            .join("autostart")
            .join("dboard-tray.desktop")
            .exists()
    } else {
        false
    }
}

pub fn uninstall_autostart() {
    if let Some(config_dir) = config_base_dir() {
        let desktop_file = config_dir.join("autostart").join("dboard-tray.desktop");
        if desktop_file.exists() {
            let _ = fs::remove_file(desktop_file);
            eprintln!("[dboard] Removed autostart entry");
        }
    }
}

fn escape_desktop_exec(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '`' => escaped.push_str("\\`"),
            '$' => escaped.push_str("\\$"),
            '%' => escaped.push_str("%%"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn install_autostart() {
    if let Some(config_dir) = config_base_dir() {
        let autostart_dir = config_dir.join("autostart");
        let desktop_file = autostart_dir.join("dboard-tray.desktop");
        let _ = fs::create_dir_all(&autostart_dir);

        let exec_path = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("/usr/bin/dboard-tray"))
            .to_string_lossy()
            .into_owned();
        let escaped_exec = escape_desktop_exec(&exec_path);

        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Dboard\n\
             Comment=Vietnamese Input Method\n\
             Exec=\"{}\"\n\
             Icon=dboard\n\
             Terminal=false\n\
             Categories=Utility;System;\n\
             X-GNOME-Autostart-enabled=true\n\
             StartupNotify=false\n",
            escaped_exec
        );

        let tmp_file =
            autostart_dir.join(format!(".dboard-tray.desktop.tmp.{}", std::process::id()));
        let _ = fs::remove_file(&tmp_file);
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let written = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_file)
            .and_then(|mut f| {
                f.write_all(content.as_bytes())?;
                f.sync_all()
            })
            .and_then(|()| fs::rename(&tmp_file, &desktop_file));
        if written.is_err() {
            let _ = fs::remove_file(&tmp_file);
        } else {
            eprintln!("[dboard] Installed autostart entry");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_save_preserves_config_and_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("dboard-ui-config-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, "deduplicate_keys = true\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let config: Config = toml::from_str("deduplicate_keys = true\n").unwrap();
        config.save_to(&path).unwrap();
        let saved: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["deduplicate_keys"].as_bool(), Some(true));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mode_changes_preserve_all_unrelated_daemon_config() {
        let source = include_str!("../../dboard.toml").to_string()
            + "\n[password_detection]\nenabled = true\ncheck_atspi2 = false\n";
        let before: toml::Value = toml::from_str(&source).unwrap();
        let mut config: Config = toml::from_str(&source).unwrap();
        config.input_method = "vni".into();
        config.start_enabled = false;
        let after: toml::Value = toml::from_str(&toml::to_string_pretty(&config).unwrap()).unwrap();
        for (key, value) in before.as_table().unwrap() {
            if key != "input_method" && key != "start_enabled" {
                assert_eq!(after.get(key), Some(value), "lost config key: {key}");
            }
        }
    }

    #[test]
    fn minimal_config_save_does_not_clobber_daemon_default_macros_or_app_state() {
        let minimal = "input_method = \"telex\"\nautocorrect = true\n";
        let mut config: Config = toml::from_str(minimal).unwrap();
        config.candidate_bar = true;
        let saved_str = toml::to_string_pretty(&config).unwrap();
        let saved: toml::Value = toml::from_str(&saved_str).unwrap();
        assert!(
            saved.get("macros").is_none(),
            "minimal config save must not emit empty [macros] table: {saved_str}"
        );
        assert!(
            saved.get("app_state").is_none(),
            "minimal config save must not emit empty [app_state] table: {saved_str}"
        );
    }

    #[test]
    fn load_for_update_refuses_to_overwrite_invalid_toml() {
        let directory =
            std::env::temp_dir().join(format!("dboard-ui-config-invalid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.toml");
        let broken = "input_method = \"telex\"\n[macros\nko = \"không\"\n";
        fs::write(&path, broken).unwrap();

        let err = Config::load_for_update_from(&path)
            .expect_err("invalid TOML must return Err instead of falling back to defaults");
        assert!(err.contains("Refusing to overwrite invalid config"));
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);

        // Missing file is allowed and returns a default/system config.
        let missing = directory.join("missing.toml");
        assert!(Config::load_for_update_from(&missing).is_ok());

        fs::remove_dir_all(directory).unwrap();
    }
}
