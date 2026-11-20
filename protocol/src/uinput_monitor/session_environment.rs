// SPDX-License-Identifier: MIT
use std::path::{Path, PathBuf};

use super::UinputInjector;

#[derive(Debug, Clone)]
struct DesktopSession {
    uid: u32,
    gid: u32,
    username: String,
    home: PathBuf,
    xdg_runtime_dir: Option<PathBuf>,
    display: Option<String>,
    wayland_display: Option<String>,
    xauthority: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct SessionPathMetadata {
    owner_uid: u32,
    is_directory: bool,
    is_regular_file: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegeDropStep {
    ClearSupplementaryGroups,
    SetGid,
    SetUid,
}

#[cfg(test)]
fn privilege_drop_steps() -> [PrivilegeDropStep; 3] {
    [
        PrivilegeDropStep::ClearSupplementaryGroups,
        PrivilegeDropStep::SetGid,
        PrivilegeDropStep::SetUid,
    ]
}

#[derive(Debug)]
struct PreparedDesktopCommand {
    program: String,
    uid: u32,
    gid: u32,
    environment: std::collections::BTreeMap<String, String>,
}

fn validate_display(display: &str) -> std::io::Result<()> {
    let rest = display.strip_prefix(':').unwrap_or("");
    let valid = !rest.is_empty()
        && rest.chars().any(|c| c.is_ascii_digit())
        && rest
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.');
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "DISPLAY is not a trusted local display identifier",
        ))
    }
}

fn validate_wayland_display(display: &str) -> std::io::Result<()> {
    let valid = !display.is_empty()
        && !display.contains('/')
        && display != "."
        && display != ".."
        && display.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "WAYLAND_DISPLAY is not a trusted socket name",
        ))
    }
}

fn validate_runtime_dir(
    path: &Path,
    uid: u32,
    metadata: impl FnOnce(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<()> {
    let expected = PathBuf::from(format!("/run/user/{uid}"));
    let details = metadata(path)?;
    if path != expected || details.owner_uid != uid || !details.is_directory {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "XDG_RUNTIME_DIR is not the target user's trusted runtime directory",
        ));
    }
    Ok(())
}

fn validate_xauthority(
    path: &Path,
    session: &DesktopSession,
    metadata: impl FnOnce(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<()> {
    let has_parent_dir = path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir));
    let details = metadata(path)?;
    if !path.is_absolute()
        || has_parent_dir
        || !path.starts_with(&session.home)
        || details.owner_uid != session.uid
        || !details.is_regular_file
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "XAUTHORITY is not a trusted user-owned file",
        ));
    }
    Ok(())
}

fn prepare_desktop_command(
    program: &str,
    session: DesktopSession,
) -> std::io::Result<PreparedDesktopCommand> {
    if session.uid == 0 || session.gid == 0 || session.username.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "a non-root desktop session identity is required",
        ));
    }
    if program.is_empty() || program.contains('/') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "desktop command must be an executable name",
        ));
    }

    let mut environment = std::collections::BTreeMap::new();
    if let Some(display) = session.display.as_deref() {
        validate_display(display)?;
        environment.insert("DISPLAY".to_string(), display.to_string());
    }
    if let Some(display) = session.wayland_display.as_deref() {
        validate_wayland_display(display)?;
        environment.insert("WAYLAND_DISPLAY".to_string(), display.to_string());
    }
    if let Some(runtime_dir) = session.xdg_runtime_dir.as_deref() {
        environment.insert(
            "XDG_RUNTIME_DIR".to_string(),
            runtime_dir.to_string_lossy().into_owned(),
        );
    }
    if let Some(xauthority) = session.xauthority.as_deref() {
        environment.insert(
            "XAUTHORITY".to_string(),
            xauthority.to_string_lossy().into_owned(),
        );
    }
    environment.insert(
        "HOME".to_string(),
        session.home.to_string_lossy().into_owned(),
    );
    environment.insert("USER".to_string(), session.username);

    Ok(PreparedDesktopCommand {
        program: program.to_string(),
        uid: session.uid,
        gid: session.gid,
        environment,
    })
}

fn build_desktop_command_with(
    program: &str,
    is_root: bool,
    resolve_session: impl FnOnce() -> Option<DesktopSession>,
    metadata: impl Fn(&Path) -> std::io::Result<SessionPathMetadata>,
) -> std::io::Result<std::process::Command> {
    if !is_root {
        return Ok(std::process::Command::new(program));
    }

    let session = resolve_session().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "a non-root desktop session identity is required",
        )
    })?;

    if let Some(runtime_dir) = session.xdg_runtime_dir.as_deref() {
        validate_runtime_dir(runtime_dir, session.uid, &metadata)?;
    }
    if let Some(xauthority) = session.xauthority.as_deref() {
        validate_xauthority(xauthority, &session, &metadata)?;
    }

    let prepared = prepare_desktop_command(program, session)?;
    let mut command = std::process::Command::new(&prepared.program);
    command.env_clear();
    command.envs(&prepared.environment);

    use std::os::unix::process::CommandExt;
    let uid = prepared.uid;
    let gid = prepared.gid;
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    Ok(command)
}

/// Thread-safe passwd lookup via `libc::getpwuid_r` returning `(gid, username, home_dir)`.
fn lookup_passwd_entry(uid: u32) -> Option<(u32, String, String)> {
    let mut buf_len = 4096usize;
    loop {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let mut buf = vec![0u8; buf_len];
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf_len < 65536 {
            buf_len *= 2;
            continue;
        }
        if rc != 0 || result.is_null() {
            return None;
        }
        let pwd = unsafe { pwd.assume_init() };
        if pwd.pw_name.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned();
        let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }
            .to_string_lossy()
            .into_owned();
        if name.is_empty() || dir.is_empty() {
            return None;
        }
        return Some((pwd.pw_gid, name, dir));
    }
}

impl UinputInjector {
    /// Get original non-root UID when running as root.
    fn get_original_uid() -> Option<u32> {
        let is_root = unsafe { libc::geteuid() == 0 };
        if !is_root {
            return None;
        }

        if let Ok(uid_str) = std::env::var("SUDO_UID") {
            if let Ok(uid) = uid_str.parse::<u32>() {
                return Some(uid);
            }
        }

        if let Ok(uid_str) = std::env::var("PKEXEC_UID") {
            if let Ok(uid) = uid_str.parse::<u32>() {
                return Some(uid);
            }
        }

        if let Ok(content) = std::fs::read_to_string("/proc/self/loginuid") {
            if let Ok(uid) = content.trim().parse::<u32>() {
                if uid != 4294967295 {
                    return Some(uid);
                }
            }
        }

        None
    }

    fn session_path_metadata(path: &Path) -> std::io::Result<SessionPathMetadata> {
        use std::os::unix::fs::MetadataExt;

        let metadata = std::fs::symlink_metadata(path)?;
        Ok(SessionPathMetadata {
            owner_uid: metadata.uid(),
            is_directory: metadata.is_dir(),
            is_regular_file: metadata.is_file(),
        })
    }

    fn desktop_session() -> Option<DesktopSession> {
        let uid = Self::get_original_uid()?;
        if uid == 0 {
            return None;
        }
        let (gid, username, home) = lookup_passwd_entry(uid)?;
        if gid == 0 {
            return None;
        }

        Some(DesktopSession {
            uid,
            gid,
            username,
            home: PathBuf::from(home),
            xdg_runtime_dir: std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            display: std::env::var("DISPLAY")
                .ok()
                .filter(|value| !value.is_empty()),
            wayland_display: std::env::var("WAYLAND_DISPLAY")
                .ok()
                .filter(|value| !value.is_empty()),
            xauthority: std::env::var_os("XAUTHORITY").map(PathBuf::from),
        })
    }

    /// Build a desktop command under the owning user session. Root callers
    /// must have a validated non-root identity; no root fallback is allowed.
    pub(super) fn user_cmd(program: &str) -> std::io::Result<std::process::Command> {
        let is_root = unsafe { libc::geteuid() == 0 };
        build_desktop_command_with(
            program,
            is_root,
            Self::desktop_session,
            Self::session_path_metadata,
        )
    }
}

#[cfg(test)]
mod privilege_tests {
    use super::*;
    use std::path::PathBuf;

    fn valid_session() -> DesktopSession {
        DesktopSession {
            uid: 1000,
            gid: 1000,
            username: "vietc-user".to_string(),
            home: PathBuf::from("/home/vietc-user"),
            xdg_runtime_dir: Some(PathBuf::from("/run/user/1000")),
            display: Some(":0".to_string()),
            wayland_display: Some("wayland-0".to_string()),
            xauthority: Some(PathBuf::from("/home/vietc-user/.Xauthority")),
        }
    }

    #[test]
    fn privilege_drop_clears_groups_before_gid_and_uid() {
        assert_eq!(
            privilege_drop_steps(),
            [
                PrivilegeDropStep::ClearSupplementaryGroups,
                PrivilegeDropStep::SetGid,
                PrivilegeDropStep::SetUid,
            ]
        );
    }

    #[test]
    fn desktop_command_uses_only_allowlisted_environment() {
        let prepared = prepare_desktop_command("wl-copy", valid_session()).unwrap();

        assert_eq!(prepared.program, "wl-copy");
        assert_eq!(prepared.uid, 1000);
        assert_eq!(prepared.gid, 1000);
        assert_eq!(
            prepared
                .environment
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "DISPLAY",
                "HOME",
                "USER",
                "WAYLAND_DISPLAY",
                "XAUTHORITY",
                "XDG_RUNTIME_DIR",
            ]
        );
    }

    #[test]
    fn root_desktop_command_requires_non_root_session_identity() {
        let mut session = valid_session();
        session.uid = 0;
        session.gid = 0;

        let error = prepare_desktop_command("wl-copy", session).unwrap_err();

        assert!(error.to_string().contains("non-root desktop session"));
    }

    #[test]
    fn rejects_untrusted_wayland_display_path() {
        let mut session = valid_session();
        session.wayland_display = Some("../attacker/socket".to_string());

        let error = prepare_desktop_command("wl-copy", session).unwrap_err();

        assert!(error.to_string().contains("WAYLAND_DISPLAY"));
    }

    #[test]
    fn rejects_runtime_directory_owned_by_another_uid() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: 2000,
            is_directory: true,
            is_regular_file: false,
        };

        let error = validate_runtime_dir(
            session.xdg_runtime_dir.as_deref().unwrap(),
            session.uid,
            |_| Ok(metadata),
        )
        .unwrap_err();

        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }

    #[test]
    fn rejects_xauthority_outside_the_user_home() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: session.uid,
            is_directory: false,
            is_regular_file: true,
        };

        let error = validate_xauthority(Path::new("/root/.Xauthority"), &session, |_| Ok(metadata))
            .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn rejects_xauthority_owned_by_another_uid() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: 2000,
            is_directory: false,
            is_regular_file: true,
        };

        let error = validate_xauthority(session.xauthority.as_deref().unwrap(), &session, |_| {
            Ok(metadata)
        })
        .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn rejects_non_file_xauthority() {
        let session = valid_session();
        let metadata = SessionPathMetadata {
            owner_uid: session.uid,
            is_directory: true,
            is_regular_file: false,
        };

        let error = validate_xauthority(session.xauthority.as_deref().unwrap(), &session, |_| {
            Ok(metadata)
        })
        .unwrap_err();

        assert!(error.to_string().contains("XAUTHORITY"));
    }

    #[test]
    fn root_command_without_session_identity_is_rejected() {
        let result = build_desktop_command_with(
            "wl-copy",
            true,
            || None,
            |_| panic!("metadata must not be read without a session identity"),
        );

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("non-root desktop session identity"));
    }

    #[test]
    fn root_command_validates_session_paths_before_building() {
        let mut session = valid_session();
        session.xdg_runtime_dir = Some(PathBuf::from("/tmp/untrusted-runtime"));

        let result = build_desktop_command_with(
            "wl-copy",
            true,
            || Some(session),
            |_| {
                Ok(SessionPathMetadata {
                    owner_uid: 1000,
                    is_directory: true,
                    is_regular_file: false,
                })
            },
        );

        let error = result.unwrap_err();
        assert!(error.to_string().contains("XDG_RUNTIME_DIR"));
    }
}
