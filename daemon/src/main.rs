// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

mod app_state;
mod candidate_ipc;
mod commands;
mod config;
mod daemon;
mod device;
mod display;
mod doctor;
mod env;
mod evdev_loop;
mod event;
mod focus;
mod focus_sync;
mod focus_tracker;
mod hotplug;
mod inject;
mod key_dedup;
mod log;
#[cfg(test)]
mod office_adversarial_exhaustive_suite;
#[cfg(test)]
mod office_stress_tests;
#[cfg(test)]
mod office_worker_stress_matrix;
mod password_detector;
mod pointer_watcher;
#[cfg(test)]
mod probe_office_exhaustive;
#[cfg(test)]
mod qc_multi_env_stress;
mod signal;
mod stdin;
mod surrounding_context;
#[cfg(feature = "wayland")]
mod toplevel_tracker;
mod user_dict_store;

use daemon::Daemon;
use device::open_keyboard_devices;
use env::recover_display_env;
use evdev_loop::{run_with_evdev, EvdevSharedState};
use log::log_info;
use signal::{ensure_single_instance, install_signal_handlers};
use stdin::run_stdin_mode;

/// Boost nice priority without hard-pinning CPU cores across heterogeneous topologies.
fn boost_thread_priority() {
    unsafe {
        let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -10);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("doctor" | "--doctor")
    ) {
        recover_display_env();
        std::process::exit(doctor::run_doctor());
    }
    recover_display_env();
    let config_path = config::find_config_path();
    let config = match config::Config::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!(
                "[dboard] WARN: failed to parse config {}: {err} — falling back to defaults",
                config_path.display()
            );
            config::Config::default()
        }
    };
    let custom_log_file = config.log_file.as_deref().map(std::path::Path::new);
    let active_log = log::init(config.debug, config.log_level.as_deref(), custom_log_file)?;
    install_signal_handlers();
    let _instance_lock = match ensure_single_instance("dboard-daemon") {
        Ok(lock) => lock,
        Err(signal::InstanceLockError::AlreadyRunning) => {
            eprintln!("[dboard-daemon] Another instance is already running. Exiting.");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    let engine_enabled = Arc::new(AtomicBool::new(config.start_enabled));
    let mut daemon = Daemon::new(config, config_path.clone(), engine_enabled.clone());

    let _ = thread::Builder::new()
        .name("dboard-warmup".into())
        .spawn(dboard_engine::warm_up_symspell);

    daemon.write_status();
    daemon.write_method_status();

    let display = display::detect_display_server();
    let compositor = display::detect_compositor();

    log_info(&format!("Dboard Daemon v{}", env!("CARGO_PKG_VERSION")));
    log_info(&format!(
        "[dboard] Active log file: {}",
        active_log.display()
    ));
    log_info(&format!(
        "Display: {:?} ({})",
        display,
        compositor.unwrap_or_else(|| "unknown".into())
    ));
    log_info(&format!("Input method: {:?}", daemon.config.input_method));
    log_info(&format!(
        "Toggle key: {}",
        config::format_toggle_key(&daemon.config.toggle_key)
    ));
    log_info(&format!(
        "App memory: {}",
        if daemon.config.app_state.enabled {
            "ON"
        } else {
            "OFF"
        }
    ));

    boost_thread_priority();

    let shared_focus = Arc::new(Mutex::new(focus::FocusSnapshot::default()));
    focus_tracker::spawn_atspi_focus_watcher(shared_focus.clone());
    #[cfg(feature = "wayland")]
    toplevel_tracker::spawn_toplevel_focus_tracker(shared_focus.clone());
    let pointer_activity = pointer_watcher::spawn_pointer_watcher(shared_focus.clone());
    let config_changed = Arc::new(AtomicBool::new(false));
    let status_changed = Arc::new(AtomicBool::new(false));

    {
        let shared_focus = shared_focus.clone();
        let config_changed = config_changed.clone();
        let config_path = config_path.clone();
        let status_changed = status_changed.clone();
        let engine_enabled = engine_enabled.clone();
        let initial_method = daemon.config.input_method.clone();
        let mut watched_config_path = config_path.clone();
        let user_config_candidate = config::user_config_path();
        let mut last_modified = std::fs::metadata(&watched_config_path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::now());

        thread::spawn(move || {
            let mut window_check_counter = 0;
            let status_path = config::user_ipc_file_path("status");
            let method_path = config::user_ipc_file_path("method");
            let mut last_method = initial_method;
            loop {
                if signal::SIGNAL_EXIT.load(Ordering::SeqCst) {
                    break;
                }
                let (compositor_class, authoritative_id) = {
                    let focus = shared_focus
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let auth = matches!(
                        &focus.identity,
                        Some(existing)
                            if existing.starts_with("atspi:") || existing.starts_with("toplevel:")
                    );
                    (focus.class_from_compositor, auth)
                };
                let identity = if authoritative_id {
                    None
                } else {
                    app_state::get_active_window_id()
                };
                let class = if compositor_class {
                    None
                } else {
                    app_state::get_focused_window_class()
                };
                let mut focus = shared_focus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let previous_identity = focus.identity.clone();
                let previous_generation = focus.generation;
                let authoritative = matches!(
                    &focus.identity,
                    Some(existing) if existing.starts_with("atspi:") || existing.starts_with("toplevel:")
                );
                let effective_identity = match (&focus.identity, identity) {
                    (Some(existing), _) if authoritative => Some(existing.clone()),
                    (Some(existing), None) => Some(existing.clone()),
                    (_, new_id) => new_id,
                };
                let class = if focus.class_from_compositor {
                    focus.class.clone()
                } else if authoritative {
                    class.or_else(|| focus.class.clone())
                } else {
                    class
                };
                focus.observe(effective_identity, class);
                if focus.generation != previous_generation {
                    log_info(&format!(
                        "[dboard] bg: focus {:?} -> {:?} generation={}",
                        previous_identity, focus.identity, focus.generation
                    ));
                }
                drop(focus);
                app_state::refresh_background_window_security();

                if !user_dict_store::has_pending_write(&status_path) {
                    if let Ok(content) = std::fs::read_to_string(&status_path) {
                        let s = content.trim();
                        if s == "vn" || s == "en" {
                            let is_vn = s == "vn";
                            let current_enabled = engine_enabled.load(Ordering::SeqCst);
                            if is_vn != current_enabled {
                                status_changed.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }

                if !user_dict_store::has_pending_write(&method_path) {
                    if let Ok(content) = std::fs::read_to_string(&method_path) {
                        let m = content.trim();
                        if !m.is_empty() && m != last_method {
                            status_changed.store(true, Ordering::SeqCst);
                            last_method = m.to_string();
                        }
                    }
                }

                window_check_counter += 1;
                if window_check_counter >= 6 {
                    window_check_counter = 0;
                    if let Some(ref user_cfg) = user_config_candidate {
                        if *user_cfg != watched_config_path && user_cfg.exists() {
                            watched_config_path = user_cfg.clone();
                            last_modified = std::time::UNIX_EPOCH;
                        }
                    }
                    if let Ok(metadata) = std::fs::metadata(&watched_config_path) {
                        if let Ok(modified) = metadata.modified() {
                            if modified > last_modified {
                                last_modified = modified;
                                config_changed.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }

                thread::sleep(std::time::Duration::from_millis(250));
            }
        });
    }

    // Pure evdev hardware keyboard interception + uinput injection.
    match open_keyboard_devices() {
        Ok(mut devices) => {
            match run_with_evdev(
                &mut devices,
                &mut daemon,
                EvdevSharedState {
                    focus: shared_focus.clone(),
                    config_changed: config_changed.clone(),
                    status_changed: status_changed.clone(),
                    pointer_activity: pointer_activity.clone(),
                },
                display,
            ) {
                Ok(()) => {
                    log_info("[dboard] evdev session ended");
                    crate::user_dict_store::flush_and_shutdown();
                    return Ok(());
                }
                Err(e) => {
                    log_info(&format!(
                        "[dboard] evdev exited with error: {} — falling back to stdin mode",
                        e
                    ));
                }
            }
        }
        Err(e) => {
            log_info(&format!("[dboard] evdev not available: {}", e));
        }
    }

    log_info("[dboard] Running in stdin mode");
    let res = run_stdin_mode(
        &mut daemon,
        shared_focus,
        config_changed,
        status_changed,
        pointer_activity,
        engine_enabled,
        display,
    );
    crate::user_dict_store::flush_and_shutdown();
    res
}
