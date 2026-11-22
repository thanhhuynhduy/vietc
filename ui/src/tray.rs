// SPDX-License-Identifier: MIT
use crate::config;
use ksni::{menu::*, MenuItem, Tray};

fn is_flatpak() -> bool {
    std::path::Path::new("/app/bin/dboard-daemon").exists()
}

fn ipc_file_path(name: &str) -> std::path::PathBuf {
    if let Some(config_dir) = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })
        .or_else(dirs::config_dir)
    {
        return config_dir.join("dboard").join(name);
    }
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        if !rt.is_empty() && std::path::Path::new(&rt).is_dir() {
            return std::path::PathBuf::from(rt).join("dboard").join(name);
        }
    }
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/tmp/dboard-{}-{}", name, uid))
}

fn write_ipc_file(name: &str, value: &str) {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let path = ipc_file_path(name);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && parent != std::path::Path::new("/tmp") {
            let _ = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent);
        }
        let tmp_path = parent.join(format!(".{}.tray.tmp.{}", name, unsafe { libc::getpid() }));
        let _ = std::fs::remove_file(&tmp_path);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_path)
        {
            if f.write_all(value.as_bytes()).is_ok() && std::fs::rename(&tmp_path, &path).is_ok() {
                return;
            }
        }
        let _ = std::fs::remove_file(&tmp_path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        let _ = f.write_all(value.as_bytes());
    }
}

fn write_status(state: &str) {
    write_ipc_file("status", state);
}

fn read_method(fallback: &str) -> String {
    let path = ipc_file_path("method");
    if let Ok(content) = std::fs::read_to_string(&path) {
        let trimmed = content.trim();
        if trimmed == "vni" || trimmed == "telex" {
            return trimmed.to_string();
        }
    }
    if fallback == "vni" || fallback == "telex" {
        return fallback.to_string();
    }
    let cfg = config::Config::load();
    if cfg.input_method.eq_ignore_ascii_case("vni") {
        "vni".into()
    } else {
        "telex".into()
    }
}

fn write_method(method: &str) {
    write_ipc_file("method", method);
}

fn read_status(fallback: &str) -> String {
    let path = ipc_file_path("status");
    if let Ok(content) = std::fs::read_to_string(&path) {
        let trimmed = content.trim();
        if trimmed == "vn" || trimmed == "en" {
            return trimmed.to_string();
        }
    }
    if fallback == "vn" || fallback == "en" {
        return fallback.to_string();
    }
    let cfg = config::Config::load();
    if cfg.start_enabled {
        "vn".into()
    } else {
        "en".into()
    }
}

fn ensure_icons() {
    let svg_vn = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect x="8" y="8" width="112" height="112" rx="24" fill="#e02424"/>
  <text x="64" y="96" text-anchor="middle" fill="#ffffff" font-size="48" font-weight="bold" font-family="system-ui, sans-serif">VN</text>
</svg>"##;

    let svg_tlx = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect x="8" y="8" width="112" height="112" rx="24" fill="#2563eb"/>
  <text x="64" y="96" text-anchor="middle" fill="#ffffff" font-size="48" font-weight="bold" font-family="system-ui, sans-serif">TLX</text>
</svg>"##;

    let svg_en = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect x="8" y="8" width="112" height="112" rx="24" fill="#4b5563"/>
  <text x="64" y="96" text-anchor="middle" fill="#ffffff" font-size="48" font-weight="bold" font-family="system-ui, sans-serif">EN</text>
</svg>"##;

    if let Some(home) = dirs::home_dir() {
        let base = home.join(".local/share/icons");
        let _ = std::fs::create_dir_all(&base);
        let _ = std::fs::write(base.join("dboard-vn.svg"), svg_vn);
        let _ = std::fs::write(base.join("dboard-tlx.svg"), svg_tlx);
        let _ = std::fs::write(base.join("dboard-en.svg"), svg_en);

        for dir in &["scalable", "256x256", "128x128", "64x64", "48x48", "32x32"] {
            let apps = base.join("hicolor").join(dir).join("apps");
            let _ = std::fs::create_dir_all(&apps);
            let _ = std::fs::write(apps.join("dboard-vn.svg"), svg_vn);
            let _ = std::fs::write(apps.join("dboard-tlx.svg"), svg_tlx);
            let _ = std::fs::write(apps.join("dboard-en.svg"), svg_en);
        }
    }
}

struct VietTray {
    mode: String,
    im: String,
    autostart: bool,
}

impl VietTray {
    fn cycle_mode(&mut self) {
        let cfg = config::Config::load();
        let modes: Vec<String> = cfg
            .toggle_modes
            .iter()
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| matches!(m.as_str(), "en" | "vn" | "vni" | "telex"))
            .collect();
        let (next_mode, next_im): (String, String) =
            if modes.len() >= 2 && modes.iter().any(|m| m == "vni" || m == "telex") {
                let cur_idx = modes
                    .iter()
                    .position(|m| {
                        (m == "en" && self.mode == "en")
                            || (m == "vn" && self.mode == "vn")
                            || (self.mode == "vn" && m == &self.im)
                    })
                    .unwrap_or(0);
                let target = &modes[(cur_idx + 1) % modes.len()];
                match target.as_str() {
                    "en" => ("en".into(), self.im.clone()),
                    "vni" => ("vn".into(), "vni".into()),
                    "telex" => ("vn".into(), "telex".into()),
                    _ => ("vn".into(), self.im.clone()),
                }
            } else if self.mode == "en" {
                ("vn".into(), self.im.clone())
            } else {
                ("en".into(), self.im.clone())
            };
        write_status(&next_mode);
        write_method(&next_im);
        self.mode = next_mode;
        self.im = next_im;
    }

    fn toggle_shortcut_label() -> String {
        let key = config::Config::load().toggle_key;
        let trimmed = key.trim();
        if trimmed.contains('+') {
            trimmed
                .split('+')
                .map(|part| {
                    let mut chars = part.trim().chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" + ")
        } else {
            let mut chars = trimmed.chars();
            let cap = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Space".into(),
            };
            format!("Ctrl + {cap}")
        }
    }
}

impl Tray for VietTray {
    fn id(&self) -> String {
        "io.github.dboard.Tray".into()
    }
    fn title(&self) -> String {
        "Dboard".into()
    }

    fn icon_name(&self) -> String {
        let is_tlx = self.mode == "vn" && self.im == "telex";
        if is_flatpak() {
            if is_tlx {
                "io.github.dboard.dboard-tlx".into()
            } else if self.mode == "vn" {
                "io.github.dboard.dboard-vn".into()
            } else {
                "io.github.dboard.dboard-en".into()
            }
        } else if is_tlx {
            "dboard-tlx".into()
        } else if self.mode == "vn" {
            "dboard-vn".into()
        } else {
            "dboard-en".into()
        }
    }

    fn icon_theme_path(&self) -> String {
        if is_flatpak() {
            let flatpak_path = std::path::Path::new("/app/share/icons");
            if flatpak_path.exists() {
                return "/app/share/icons".into();
            }
        }
        if let Some(home) = dirs::home_dir() {
            let user_path = home.join(".local/share/icons");
            if user_path.exists() {
                return user_path.to_string_lossy().into_owned();
            }
        }
        dirs::data_dir()
            .map(|d| d.join("icons").to_string_lossy().into_owned())
            .unwrap_or_else(|| "/usr/share/icons".into())
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.cycle_mode();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let is_vn = self.mode == "vn";
        let is_telex = self.im == "telex";
        let selected_mode = if !is_vn {
            0_usize
        } else if is_telex {
            1_usize
        } else {
            2_usize
        };

        vec![
            CheckmarkItem {
                label: "Start with System".into(),
                checked: self.autostart,
                activate: Box::new(|this: &mut VietTray| {
                    if this.autostart {
                        config::uninstall_autostart();
                        this.autostart = false;
                    } else {
                        config::install_autostart();
                        this.autostart = true;
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: format!("Switch Mode ({})", Self::toggle_shortcut_label()),
                activate: Box::new(|this: &mut VietTray| {
                    this.cycle_mode();
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Input Method".into(),
                submenu: vec![RadioGroup {
                    selected: selected_mode,
                    select: Box::new(|this: &mut VietTray, idx: usize| {
                        let (mode, im) = match idx {
                            0 => ("en", this.im.as_str()),
                            1 => ("vn", "telex"),
                            _ => ("vn", "vni"),
                        };
                        write_status(mode);
                        write_method(im);
                        this.mode = mode.into();
                        this.im = im.into();
                    }),
                    options: vec![
                        RadioItem {
                            label: "English (ENG)".into(),
                            ..Default::default()
                        },
                        RadioItem {
                            label: "Telex (TLX)".into(),
                            ..Default::default()
                        },
                        RadioItem {
                            label: "VNI (VN)".into(),
                            ..Default::default()
                        },
                    ],
                }
                .into()],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "About: Dboard".into(),
                activate: Box::new(|_| {
                    if let Ok(mut child) = std::process::Command::new("xdg-open")
                        .arg("https://github.com/thanhhuynhduy/vietc")
                        .spawn()
                    {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    crate::stop_daemon();
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn run() {
    ensure_icons();

    let initial_mode = read_status("");
    let initial_im = read_method("");
    let initial_autostart = config::is_autostart_installed();

    let tray = VietTray {
        mode: initial_mode.clone(),
        im: initial_im.clone(),
        autostart: initial_autostart,
    };

    let service = ksni::TrayService::new(tray);
    let handle = service.handle();
    let shutdown_handle = service.handle();
    service.spawn();

    std::thread::spawn(move || {
        let dir = ipc_file_path("status")
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let _ = std::fs::create_dir_all(&dir);

        let mut inotify_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        let mut wd = -1;

        if inotify_fd >= 0 {
            if let Ok(c_path) = std::ffi::CString::new(dir.to_string_lossy().as_bytes()) {
                wd = unsafe {
                    libc::inotify_add_watch(
                        inotify_fd,
                        c_path.as_ptr(),
                        libc::IN_CLOSE_WRITE
                            | libc::IN_MOVED_TO
                            | libc::IN_MODIFY
                            | libc::IN_CREATE,
                    )
                };
            }
        }

        let mut last_mode = initial_mode;
        let mut last_im = initial_im;
        let mut last_autostart = initial_autostart;
        let mut buffer = [0u8; 4096];

        while !crate::should_exit() {
            if inotify_fd >= 0 && wd >= 0 {
                let mut pfd = libc::pollfd {
                    fd: inotify_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let res = unsafe { libc::poll(&mut pfd, 1, 500) };
                if res > 0 && (pfd.revents & libc::POLLIN) != 0 {
                    while unsafe {
                        libc::read(
                            inotify_fd,
                            buffer.as_mut_ptr() as *mut libc::c_void,
                            buffer.len(),
                        )
                    } > 0
                    {}
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if inotify_fd < 0 {
                    inotify_fd =
                        unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
                }
                if inotify_fd >= 0 && wd < 0 {
                    if let Ok(c_path) = std::ffi::CString::new(dir.to_string_lossy().as_bytes()) {
                        wd = unsafe {
                            libc::inotify_add_watch(
                                inotify_fd,
                                c_path.as_ptr(),
                                libc::IN_CLOSE_WRITE
                                    | libc::IN_MOVED_TO
                                    | libc::IN_MODIFY
                                    | libc::IN_CREATE,
                            )
                        };
                    }
                }
            }

            let mode = read_status(&last_mode);
            let im = read_method(&last_im);
            let autostart = config::is_autostart_installed();
            if mode != last_mode || im != last_im || autostart != last_autostart {
                last_mode = mode.clone();
                last_im = im.clone();
                last_autostart = autostart;
                handle.update(move |t| {
                    t.mode = mode;
                    t.im = im;
                    t.autostart = autostart;
                });
            }
        }
        if inotify_fd >= 0 {
            unsafe {
                libc::close(inotify_fd);
            }
        }
    });

    while !crate::should_exit() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    shutdown_handle.shutdown();
}
