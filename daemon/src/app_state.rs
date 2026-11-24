// SPDX-License-Identifier: MIT
use std::collections::HashMap;
use std::fs;
use std::process::Command;

use crate::password_detector::PasswordDetector;

struct HyprWindowInfo {
    address: String,
    class: String,
    title: String,
    pid: u32,
}

fn get_hyprland_active_window() -> Option<HyprWindowInfo> {
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err()
        && !std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|d| d.to_lowercase().contains("hyprland"))
    {
        return None;
    }

    let output = run_as_user("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    if json_str.trim().is_empty() || json_str.trim() == "{}" {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(&json_str).ok()?;
    let address = val.get("address")?.as_str()?.to_string();
    if address.is_empty() || address == "0x0" {
        return None;
    }
    let class = val
        .get("class")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let title = val
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let pid = val.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) as u32;

    Some(HyprWindowInfo {
        address,
        class,
        title,
        pid,
    })
}

fn get_sway_focused_window() -> Option<(String, String, String, u32)> {
    if std::env::var("SWAYSOCK").is_err()
        && !std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_lowercase().contains("sway"))
    {
        return None;
    }

    let output = run_as_user("swaymsg")
        .args(["-t", "get_tree"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let val: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    find_sway_focused_node(&val)
}

fn find_sway_focused_node(val: &serde_json::Value) -> Option<(String, String, String, u32)> {
    if val.get("focused").and_then(|b| b.as_bool()) == Some(true) {
        let id = val
            .get("id")
            .and_then(|i| i.as_i64())
            .map(|i| format!("0x{:x}", i))
            .unwrap_or_default();
        let class = val
            .get("app_id")
            .and_then(|a| a.as_str())
            .or_else(|| {
                val.get("window_properties")
                    .and_then(|w| w.get("class"))
                    .and_then(|c| c.as_str())
            })
            .unwrap_or_default()
            .to_lowercase();
        let title = val
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_lowercase();
        let pid = val.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
        return Some((id, class, title, pid));
    }

    if let Some(nodes) = val.get("nodes").and_then(|n| n.as_array()) {
        for node in nodes {
            if let Some(res) = find_sway_focused_node(node) {
                return Some(res);
            }
        }
    }
    if let Some(floating) = val.get("floating_nodes").and_then(|n| n.as_array()) {
        for node in floating {
            if let Some(res) = find_sway_focused_node(node) {
                return Some(res);
            }
        }
    }
    None
}

/// Get the active window's PID (Hyprland, Sway)
pub fn get_active_window_pid() -> Option<u32> {
    if let Some(info) = get_hyprland_active_window() {
        if info.pid > 0 {
            return Some(info.pid);
        }
    }

    if let Some((_, _, _, pid)) = get_sway_focused_window() {
        if pid > 0 {
            return Some(pid);
        }
    }

    None
}

/// Get the active window's title (lowercase, Hyprland / Sway)
pub fn get_active_window_title() -> Option<String> {
    if let Some(info) = get_hyprland_active_window() {
        if !info.title.is_empty() {
            return Some(info.title);
        }
    }

    if let Some((_, _, title, _)) = get_sway_focused_window() {
        if !title.is_empty() {
            return Some(title);
        }
    }

    None
}

fn normalize_active_window_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("0x0") {
        None
    } else {
        Some(value.to_string())
    }
}

/// Get the active window's Wayland ID (unique per window on Hyprland / Sway).
pub fn get_active_window_id() -> Option<String> {
    if let Some(info) = get_hyprland_active_window() {
        if let Some(id) = normalize_active_window_id(&info.address) {
            return Some(id);
        }
    }

    if let Some((id, _, _, _)) = get_sway_focused_window() {
        if let Some(id) = normalize_active_window_id(&id) {
            return Some(id);
        }
    }

    None
}

/// Detect the currently focused window's class name (Hyprland / Sway / wlrctl / /proc / AT-SPI2)
pub fn get_focused_window_class() -> Option<String> {
    if let Some(info) = get_hyprland_active_window() {
        if !info.class.is_empty() {
            return Some(info.class);
        }
    }

    if let Some((_, class, _, _)) = get_sway_focused_window() {
        if !class.is_empty() {
            return Some(class);
        }
    }

    if let Some(class) = get_wayland_window_class() {
        return Some(class);
    }

    if let Some(class) = get_proc_window_class() {
        return Some(class);
    }

    if let Some(class) = crate::password_detector::get_focused_window_class_atspi() {
        return Some(class);
    }

    None
}

fn run_as_user(program: &str) -> std::process::Command {
    let is_root = unsafe { libc::getuid() == 0 };
    if is_root {
        if let Ok(uid_str) = std::env::var("SUDO_UID") {
            if let Ok(uid) = uid_str.parse::<u32>() {
                use std::os::unix::process::CommandExt;
                let mut cmd = std::process::Command::new(program);
                cmd.uid(uid);
                if let Ok(gid) = std::env::var("SUDO_GID")
                    .and_then(|g| g.parse::<u32>().map_err(|_| std::env::VarError::NotPresent))
                {
                    cmd.gid(gid);
                }
                for key in [
                    "HOME",
                    "WAYLAND_DISPLAY",
                    "XDG_RUNTIME_DIR",
                    "DBUS_SESSION_BUS_ADDRESS",
                ] {
                    if let Ok(v) = std::env::var(key) {
                        cmd.env(key, v);
                    }
                }
                return cmd;
            }
        }
    }
    std::process::Command::new(program)
}

fn parse_wlrctl_focused_app_id(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let app_id = line.split(':').next().unwrap_or(line).trim();
    (!app_id.is_empty()).then(|| app_id.to_lowercase())
}

fn get_wayland_window_class() -> Option<String> {
    // Try wlr-foreign-toplevel-management protocol via wlrctl filtered to focused toplevel
    let output = Command::new("wlrctl")
        .args(["toplevel", "list", "state:focused"])
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return parse_wlrctl_focused_app_id(&stdout);
    }

    None
}

fn get_proc_window_class() -> Option<String> {
    // Read /proc/active-windows if available (some compositors expose this)
    let content = fs::read_to_string("/proc/active-windows").ok()?;
    // Format: pid window_class window_title
    content
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_lowercase())
}

/// Whether the focused app `class` matches a configured app `pattern`.
///
/// Patterns of 4+ characters match as a case-insensitive substring so that
/// `telegram` still matches `telegramdesktop`. Very short patterns such as
/// the terminal `st` are ambiguous as substrings (`com.sy`**`st`**`em76.*`,
/// `visual-`**`st`**`udio-code`), so they must match a whole token of the
/// class (split on non-alphanumeric characters).
pub fn class_matches(class: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let class = class.to_lowercase();
    let pattern = pattern.to_lowercase();
    if pattern.chars().count() > 3 {
        return class.contains(&pattern);
    }
    class
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| token == pattern)
}

/// Manages per-app IME state
pub struct AppStateManager {
    /// Current app class (lowercase)
    current_app: String,
    /// Per-app overrides (user toggled manually)
    overrides: HashMap<String, bool>,
    /// Default English apps from config
    english_apps: Vec<String>,
    /// Default Vietnamese apps from config
    vietnamese_apps: Vec<String>,
    /// Bypass apps from config
    bypass_apps: Vec<String>,
    /// Terminal emulator class names (force VNI mode)
    terminal_apps: Vec<String>,
    /// Input method forced in terminals
    terminal_input_method: String,
    /// User's global input method (VNI/Telex)
    global_method: String,
    /// Effective method after terminal override
    effective_method: String,
    /// Global enabled state
    global_enabled: bool,
    /// Password detection config
    password_enabled: bool,
    check_atspi2: bool,
    check_window_title: bool,
    title_keywords: Vec<String>,
    password_apps: Vec<String>,
    /// Password detector (AT-SPI2)
    password_detector: PasswordDetector,
    /// Cached password field state
    is_password_field: bool,
}

impl AppStateManager {
    pub fn new(
        english_apps: Vec<String>,
        vietnamese_apps: Vec<String>,
        bypass_apps: Vec<String>,
        terminal_apps: Vec<String>,
        terminal_input_method: String,
        global_method: String,
        global_enabled: bool,
    ) -> Self {
        let effective_method = Self::compute_effective_method(
            &global_method,
            &terminal_input_method,
            &terminal_apps,
            "",
        );
        Self {
            current_app: String::new(),
            overrides: HashMap::new(),
            english_apps: english_apps.iter().map(|s| s.to_lowercase()).collect(),
            vietnamese_apps: vietnamese_apps.iter().map(|s| s.to_lowercase()).collect(),
            bypass_apps: bypass_apps.iter().map(|s| s.to_lowercase()).collect(),
            terminal_apps: terminal_apps.iter().map(|s| s.to_lowercase()).collect(),
            terminal_input_method,
            global_method,
            effective_method,
            global_enabled,
            password_enabled: false,
            check_atspi2: true,
            check_window_title: true,
            title_keywords: Vec::new(),
            password_apps: Vec::new(),
            password_detector: PasswordDetector::new(),
            is_password_field: false,
        }
    }

    /// Compute effective method: use terminal override if current_app is a terminal,
    /// otherwise use the global method.
    fn compute_effective_method(
        global_method: &str,
        terminal_method: &str,
        terminal_apps: &[String],
        current_app: &str,
    ) -> String {
        for pattern in terminal_apps {
            if class_matches(current_app, pattern) {
                return terminal_method.to_string();
            }
        }
        global_method.to_string()
    }

    /// Update password detection config
    pub fn set_password_config(
        &mut self,
        enabled: bool,
        check_atspi2: bool,
        check_window_title: bool,
        title_keywords: Vec<String>,
        password_apps: Vec<String>,
    ) {
        self.password_enabled = enabled;
        self.check_atspi2 = check_atspi2;
        self.check_window_title = check_window_title;
        self.title_keywords = title_keywords.iter().map(|s| s.to_lowercase()).collect();
        self.password_apps = password_apps.iter().map(|s| s.to_lowercase()).collect();
    }

    /// Set the user's global enabled state
    pub fn set_global_enabled(&mut self, enabled: bool) {
        self.global_enabled = enabled;
    }

    /// Clear app overrides
    #[allow(dead_code)]
    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
        let _ = self.save_overrides();
    }

    /// Record a user's manual toggle for the currently focused application and
    /// persist it asynchronously so returning to this app preserves the choice
    /// instead of reverting to `english_apps`/`vietnamese_apps` defaults.
    pub fn set_current_app_override(&mut self, enabled: bool) {
        if self.current_app.is_empty() {
            return;
        }
        let key = self.current_app.to_lowercase();
        if self.overrides.get(&key).copied() != Some(enabled) {
            self.overrides.insert(key, enabled);
            let _ = self.save_overrides();
        }
    }

    /// Set the user's global input method and recompute effective method
    pub fn set_global_method(&mut self, method: &str) {
        self.global_method = method.to_string();
        self.update_effective_method();
    }

    /// Recompute effective method based on terminal override
    pub fn update_effective_method(&mut self) {
        self.effective_method = Self::compute_effective_method(
            &self.global_method,
            &self.terminal_input_method,
            &self.terminal_apps,
            &self.current_app,
        );
    }

    /// Get the effective input method (terminal override applied)
    pub fn effective_method(&self) -> &str {
        &self.effective_method
    }

    /// Check if the current app is a terminal emulator
    pub fn is_terminal_app(&self) -> bool {
        for pattern in &self.terminal_apps {
            if class_matches(&self.current_app, pattern) {
                return true;
            }
        }
        false
    }

    /// Check if the current focused widget is a password field
    /// Returns true if password detected, forcing English mode
    pub fn check_password_field(&mut self) -> bool {
        if !self.password_enabled {
            self.is_password_field = false;
            return false;
        }

        let was_password = self.is_password_field;

        // Layer 1: AT-SPI2 (event-driven O(1) role check across any web/native DOM depth).
        // When AT-SPI2 reports `false` (e.g. a terminal widget whose role is ROLE_TERMINAL),
        // fall through to Layers 2-4 so `pinentry`, `polkit`, and `sudo`/`passwd` are caught.
        if self.check_atspi2 && self.password_detector.check() == Some(true) {
            self.is_password_field = true;
            if !was_password {
                log_password_detection("AT-SPI2", &self.current_app);
            }
            return true;
        }

        // Layer 2: Window class match (for known password dialogs).
        // When `current_app` is empty (e.g. `FocusLost` on a Wayland layer-shell
        // Polkit / `cosmic-osd` / `pinentry` overlay), fall back to the live
        // AT-SPI2 focused app name so Layer 2 protection is never blinded.
        let atspi_fallback = if self.current_app.is_empty() {
            crate::password_detector::get_focused_window_class_atspi()
        } else {
            None
        };
        let layer2_class = if !self.current_app.is_empty() {
            self.current_app.as_str()
        } else {
            atspi_fallback.as_deref().unwrap_or("")
        };
        for pattern in &self.password_apps {
            if class_matches(layer2_class, pattern) {
                self.is_password_field = true;
                if !was_password {
                    log_password_detection("window-class", layer2_class);
                }
                return true;
            }
        }

        // Layer 3: Window title heuristic (for sudo prompts, browser dialogs)
        if self.check_window_title {
            if let Some((_, Some(title))) = crate::password_detector::cached_window_title() {
                for keyword in &self.title_keywords {
                    if title.contains(keyword.as_str()) {
                        self.is_password_field = true;
                        if !was_password {
                            log_password_detection("window-title", keyword);
                        }
                        return true;
                    }
                }
            }
        }

        // Layer 4: Process-based + kernel PTY `!ECHO && ICANON` detection (sudo/ssh/passwd)
        let is_sudo = crate::password_detector::cached_window_is_sudo().unwrap_or(false)
            || (self.is_terminal_app() && crate::password_detector::cached_pty_no_echo());
        if is_sudo {
            self.is_password_field = true;
            if !was_password {
                log_password_detection("process-sudo", &self.current_app);
            }
            return true;
        }

        self.is_password_field = false;
        false
    }

    /// Returns whether password protection is currently active for the focused field/window.
    pub fn is_password_active(&self) -> bool {
        self.is_password_field
    }

    /// Check if focused app changed with a pre-detected class and return whether engine should be enabled
    pub fn update_with_app(&mut self, new_class: String) -> Option<bool> {
        if new_class == self.current_app {
            return None; // No change
        }

        let old_app = self.current_app.clone();
        self.current_app = new_class;

        eprintln!("[dboard] App: {} → {}", old_app, self.current_app);

        // Recompute effective method on window change
        self.update_effective_method();
        Some(self.get_default_state())
    }

    /// Get the default Vietnamese state for the current app
    pub fn get_default_state(&self) -> bool {
        if !self.global_enabled {
            return false;
        }

        // Check user override first (case-insensitive app_id match)
        let current_lower = self.current_app.to_lowercase();
        if let Some(&override_state) = self.overrides.get(&current_lower) {
            return override_state;
        }

        // Check config defaults
        for pattern in &self.english_apps {
            if class_matches(&self.current_app, pattern) {
                return false;
            }
        }

        for pattern in &self.vietnamese_apps {
            if class_matches(&self.current_app, pattern) {
                return true;
            }
        }

        // Default: enabled
        true
    }

    /// Update app lists from reloaded config
    pub fn update_lists(
        &mut self,
        english_apps: Vec<String>,
        vietnamese_apps: Vec<String>,
        bypass_apps: Vec<String>,
        terminal_apps: Vec<String>,
        terminal_input_method: String,
    ) -> &Self {
        self.english_apps = english_apps.iter().map(|s| s.to_lowercase()).collect();
        self.vietnamese_apps = vietnamese_apps.iter().map(|s| s.to_lowercase()).collect();
        self.bypass_apps = bypass_apps.iter().map(|s| s.to_lowercase()).collect();
        self.terminal_apps = terminal_apps.iter().map(|s| s.to_lowercase()).collect();
        self.terminal_input_method = terminal_input_method;
        self.update_effective_method();
        eprintln!(
            "[dboard] App lists updated: {} English, {} Vietnamese, {} Bypass, {} Terminal",
            self.english_apps.len(),
            self.vietnamese_apps.len(),
            self.bypass_apps.len(),
            self.terminal_apps.len()
        );
        self
    }

    /// Check if the currently active application should bypass the IME completely
    /// Clears the tracked active application on `FocusLost` (e.g. Polkit/launcher
    /// layer-shell overlay) so bypass/terminal/english overrides from the previous
    /// window are never inherited.
    pub fn clear_current_app(&mut self) {
        self.current_app.clear();
        self.effective_method = self.global_method.clone();
    }

    pub fn is_current_app_bypassed(&self) -> bool {
        for pattern in &self.bypass_apps {
            if class_matches(&self.current_app, pattern) {
                return true;
            }
        }
        false
    }

    /// Save overrides asynchronously via the coalesced background persistence worker
    /// so `evdev_loop` never blocks on disk I/O or `fsync`.
    #[allow(dead_code)]
    pub fn save_overrides(&self) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(test)]
        {
            let _ = self;
            Ok(())
        }
        #[cfg(not(test))]
        {
            let path = override_path();
            let content = toml::to_string(&self.overrides)?;
            crate::user_dict_store::enqueue_atomic_file_write(path, content, true);
            Ok(())
        }
    }

    /// Load overrides from config file
    pub fn load_overrides(&mut self) {
        #[cfg(test)]
        {
            let _ = self;
        }
        #[cfg(not(test))]
        {
            let path = override_path();
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(overrides) = toml::from_str::<HashMap<String, bool>>(&content) {
                    self.overrides = overrides
                        .into_iter()
                        .map(|(k, v)| (k.to_lowercase(), v))
                        .collect();
                    eprintln!("[dboard] Loaded {} app overrides", self.overrides.len());
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn current_app(&self) -> &str {
        &self.current_app
    }
}

fn log_password_detection(method: &str, context: &str) {
    eprintln!(
        "[dboard] Password field detected via {}: {}",
        method, context
    );
}

pub fn refresh_background_window_security() {
    let title = get_active_window_title();
    let is_sudo = get_active_window_pid().is_some_and(crate::password_detector::is_sudo_process);
    let pty_no_echo = crate::password_detector::any_user_pty_echo_disabled();
    crate::password_detector::record_background_window_security(title, is_sudo, pty_no_echo);
}

#[cfg(not(test))]
fn override_path() -> std::path::PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("dboard")
        .join("overrides.toml")
}

#[cfg(test)]
mod active_window_id_tests {
    use super::normalize_active_window_id;

    #[test]
    fn rejects_empty_and_no_window_sentinels() {
        assert_eq!(normalize_active_window_id(""), None);
        assert_eq!(normalize_active_window_id("   "), None);
        assert_eq!(normalize_active_window_id("0"), None);
        assert_eq!(normalize_active_window_id("0x0"), None);
        assert_eq!(normalize_active_window_id("0X0"), None);
    }

    #[test]
    fn preserves_valid_window_identity() {
        assert_eq!(
            normalize_active_window_id(" 0x3a00004\n"),
            Some("0x3a00004".to_string())
        );
        assert_eq!(
            normalize_active_window_id("73400324"),
            Some("73400324".to_string())
        );
    }
}

#[cfg(test)]
mod class_match_tests {
    use super::class_matches;

    #[test]
    fn long_patterns_match_as_case_insensitive_substrings() {
        assert!(class_matches("telegramdesktop", "telegram"));
        assert!(class_matches("com.system76.CosmicTerm", "cosmicterm"));
        assert!(class_matches("org.gnome.Terminal", "terminal"));
        assert!(!class_matches("microsoft-edge", "terminal"));
    }

    #[test]
    fn short_patterns_only_match_whole_tokens() {
        assert!(class_matches("st", "st"));
        assert!(class_matches("st-256color", "st"));
        assert!(!class_matches("com.system76.CosmicEdit", "st"));
        assert!(!class_matches("visual-studio-code", "st"));
        assert!(!class_matches("telegramdesktop", "st"));
    }

    #[test]
    fn empty_pattern_never_matches() {
        assert!(!class_matches("anything", ""));
    }
}

#[cfg(test)]
mod remote_bypass_tests {
    use super::AppStateManager;
    use crate::config::AppStateConfig;

    fn manager() -> AppStateManager {
        let defaults = AppStateConfig::default();
        AppStateManager::new(
            defaults.english_apps,
            defaults.vietnamese_apps,
            defaults.bypass_apps,
            defaults.terminal_apps,
            defaults.terminal_input_method,
            "telex".into(),
            true,
        )
    }

    #[test]
    fn remote_desktop_and_vm_viewers_are_bypassed_by_default() {
        for class in [
            "rustdesk",
            "com.rustdesk.RustDesk",
            "org.remmina.Remmina",
            "AnyDesk",
            "virt-manager",
            "VirtualBox Machine",
            "org.gnome.Connections",
        ] {
            let mut state = manager();
            state.update_with_app(class.to_string());
            assert!(state.is_current_app_bypassed(), "{class} must be bypassed");
        }
    }

    #[test]
    fn local_apps_are_not_bypassed() {
        for class in [
            "microsoft-edge",
            "com.system76.CosmicTerm",
            "antigravity",
            "com.microsoft.VSCode",
            "telegramdesktop",
        ] {
            let mut state = manager();
            state.update_with_app(class.to_string());
            assert!(
                !state.is_current_app_bypassed(),
                "{class} must not be bypassed"
            );
        }
    }

    #[test]
    fn parse_wlrctl_focused_app_id_handles_standard_output() {
        assert_eq!(
            super::parse_wlrctl_focused_app_id("org.mozilla.firefox: GitHub - Mozilla Firefox\n"),
            Some("org.mozilla.firefox".to_string())
        );
        assert_eq!(
            super::parse_wlrctl_focused_app_id("Alacritty: bash\n"),
            Some("alacritty".to_string())
        );
        assert_eq!(super::parse_wlrctl_focused_app_id("   \n"), None);
    }

    #[test]
    fn per_app_override_persists_across_focus_switches_for_vscode() {
        let mut state = manager();

        // VS Code starts in English via `english_apps` ("code").
        assert_eq!(
            state.update_with_app("com.microsoft.VSCode".to_string()),
            Some(false)
        );
        // User manually switches VS Code to Telex (enabled = true).
        state.set_current_app_override(true);
        assert!(state.get_default_state());

        // Switching to Edge and back to VS Code must keep Telex enabled.
        assert_eq!(
            state.update_with_app("microsoft-edge".to_string()),
            Some(true)
        );
        assert_eq!(
            state.update_with_app("com.microsoft.VSCode".to_string()),
            Some(true)
        );

        // Case-insensitive app_id match must also preserve the override.
        assert_eq!(state.update_with_app("firefox".to_string()), Some(true));
        assert_eq!(
            state.update_with_app("com.microsoft.vscode".to_string()),
            Some(true)
        );
    }
}
