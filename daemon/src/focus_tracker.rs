use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dbus::arg::{RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;

use crate::focus::SharedFocusSnapshot;
use crate::log::log_info;

const ATSPI_REGISTRY: &str = "org.a11y.atspi.Registry";
const ATSPI_REGISTRY_PATH: &str = "/org/a11y/atspi/registry";
const ATSPI_EVENT_FOCUS: &str = "object:state-changed:focused";
const ATSPI_EVENT_SELECTION: &str = "object:text-selection-changed";
const ATSPI_EVENT_CARET: &str = "object:text-caret-moved";
const ROLE_AUTOCOMPLETE: i32 = 4;
const ROLE_ENTRY: i32 = 19;
const ROLE_TEXT: i32 = 60;
const ROLE_PARAGRAPH: i32 = 71;
const ROLE_DOCUMENT_TEXT: i32 = 100;
const RETRY_DELAY: Duration = Duration::from_secs(2);
const PROCESS_TIMEOUT: Duration = Duration::from_millis(500);
const QUERY_TIMEOUT: Duration = Duration::from_millis(35);

#[derive(Debug, PartialEq, Eq)]
enum PendingAtspiEvent {
    Focused(String, String),
    SelectionChanged(String, String),
    CaretMoved(String, String, i32),
}

/// Coalesce a burst of queued AT-SPI2 signals so rapid Alt+Tab / Tab switching
/// never stalls Layer 1 password detection behind obsolete widgets:
/// - Keep only the latest `Focused` event (discarding all earlier events).
/// - Keep at most the latest `SelectionChanged` event after it.
/// - Preserve `CaretMoved` ordering (further coalesced to `latest_caret`).
fn coalesce_atspi_events(drained: Vec<PendingAtspiEvent>) -> Vec<PendingAtspiEvent> {
    let start_idx = drained
        .iter()
        .rposition(|ev| matches!(ev, PendingAtspiEvent::Focused(_, _)))
        .unwrap_or(0);
    let mut tail: Vec<PendingAtspiEvent> = drained.into_iter().skip(start_idx).collect();
    let last_sel_idx = tail
        .iter()
        .rposition(|ev| matches!(ev, PendingAtspiEvent::SelectionChanged(_, _)));
    if let Some(keep_idx) = last_sel_idx {
        let mut idx = 0usize;
        tail.retain(|ev| {
            let keep = !matches!(ev, PendingAtspiEvent::SelectionChanged(_, _)) || idx == keep_idx;
            idx += 1;
            keep
        });
    }
    tail
}

fn accessibility_bus_address() -> Result<String, Box<dyn Error>> {
    let connection = Connection::new_session()?;
    let proxy = connection.with_proxy("org.a11y.Bus", "/org/a11y/bus", Duration::from_secs(2));
    let (address,): (String,) = proxy.method_call("org.a11y.Bus", "GetAddress", ())?;
    Ok(address)
}

#[cfg(test)]
fn observe_atspi_focus(shared_focus: &SharedFocusSnapshot, application_identity: Option<&str>) {
    observe_atspi_focus_with_path(shared_focus, application_identity, None, None);
}

fn observe_atspi_focus_with_path(
    shared_focus: &SharedFocusSnapshot,
    application_identity: Option<&str>,
    object_path: Option<&str>,
    app_name: Option<&str>,
) {
    let Some(application_identity) = application_identity.filter(|value| !value.is_empty()) else {
        return;
    };

    let identity = match object_path.filter(|path| !path.is_empty()) {
        Some(path) => format!("atspi:{application_identity}:{path}"),
        None => format!("atspi:{application_identity}"),
    };

    let mut focus = shared_focus.lock().unwrap_or_else(|p| p.into_inner());
    let fallback_class = app_name
        .filter(|n| !n.is_empty() && *n != "gnome-shell" && *n != "gnome-shell.desktop")
        .map(str::to_string);
    // Preserve authoritative compositor class when `toplevel_tracker` is active
    // (`class_from_compositor == true`), falling back to AT-SPI2 only when
    // `focus.class` is `None` (e.g. `FocusLost` on Polkit / `cosmic-osd`).
    // When `class_from_compositor == false`, prefer fresh AT-SPI2 `app_name`
    // immediately so window switches never inherit a stale class for 250ms.
    let class = if focus.class_from_compositor {
        focus.class.clone().or(fallback_class)
    } else {
        fallback_class.or_else(|| focus.class.clone())
    };
    focus.observe(Some(identity), class);
}

fn is_omnibox_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    lower.contains("omnibox")
        || lower.contains("address and search")
        || lower.contains("address bar")
        || lower.contains("url bar")
        || lower.contains("location bar")
        || lower.contains("search or enter")
        || lower.contains("search with")
        || lower.contains("enter address")
        || lower.contains("thanh địa chỉ")
        || lower.contains("tìm kiếm hoặc nhập")
}

fn is_inline_autocomplete_selection(caret: i32, start: i32, end: i32, char_count: i32) -> bool {
    let sel_min = start.min(end);
    let sel_max = start.max(end);
    // Inline URL autocomplete highlights the suggested suffix `[ttps://...]` after at
    // least 1 typed character (`sel_min >= 1`), extending strictly to the end of the
    // entry (`sel_max == char_count`), with the caret at a selection boundary.
    char_count > 0
        && sel_max == char_count
        && sel_max > sel_min
        && sel_min >= 1
        && (caret == sel_min || caret == sel_max)
}

struct FocusedNodeInspection {
    role_known: bool,
    is_password: bool,
    is_omnibox: bool,
    is_editable_text: bool,
    app_name: Option<String>,
}

fn inspect_focused_node(
    conn: &Connection,
    sender: &str,
    path: &str,
    shared_focus: &SharedFocusSnapshot,
) -> FocusedNodeInspection {
    let proxy = conn.with_proxy(sender, path, QUERY_TIMEOUT);
    let role_res = proxy
        .method_call::<(i32,), _, _, _>("org.a11y.atspi.Accessible", "GetRole", ())
        .or_else(|_| {
            conn.with_proxy(sender, path, Duration::from_millis(80))
                .method_call::<(i32,), _, _, _>("org.a11y.atspi.Accessible", "GetRole", ())
        });
    let Ok((role,)) = role_res else {
        // Fail-safe on timeout/frozen peer: do NOT falsely clear active password
        // protection and skip further RPCs on the unresponsive peer.
        return FocusedNodeInspection {
            role_known: false,
            is_password: false,
            is_omnibox: false,
            is_editable_text: false,
            app_name: None,
        };
    };
    let is_password = role == crate::password_detector::ROLE_PASSWORD_TEXT;
    if is_password {
        // Fail-closed immediately after `GetRole` (1st RPC) before slower
        // `GetApplication`/`Name` RPCs so fast `Tab` -> password typing cannot
        // slip through Layer 1 or leak into `dboard-popup` / `UserDict`.
        crate::password_detector::record_atspi_focus_state(true, None);
        crate::candidate_ipc::record_caret_rect(None);
        crate::focus::clear_surrounding_word(shared_focus);
    }
    let is_editable_text = !is_password
        && matches!(
            role,
            ROLE_AUTOCOMPLETE | ROLE_ENTRY | ROLE_TEXT | ROLE_PARAGRAPH | ROLE_DOCUMENT_TEXT
        );
    let is_omnibox = if is_password {
        false
    } else if role == ROLE_AUTOCOMPLETE {
        true
    } else if role == ROLE_ENTRY || role == ROLE_TEXT {
        proxy
            .method_call::<(Variant<Box<dyn RefArg>>,), _, _, _>(
                "org.freedesktop.DBus.Properties",
                "Get",
                ("org.a11y.atspi.Accessible", "Name"),
            )
            .ok()
            .and_then(|(variant,)| variant.0.as_str().map(is_omnibox_label))
            .unwrap_or(false)
    } else {
        false
    };
    let app_name = proxy
        .method_call::<((String, dbus::Path<'static>),), _, _, _>(
            "org.a11y.atspi.Accessible",
            "GetApplication",
            (),
        )
        .ok()
        .and_then(|((app_bus, app_path),)| {
            if app_bus.is_empty() {
                return None;
            }
            let app_proxy = conn.with_proxy(app_bus, app_path, QUERY_TIMEOUT);
            app_proxy
                .method_call::<(Variant<Box<dyn RefArg>>,), _, _, _>(
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    ("org.a11y.atspi.Accessible", "Name"),
                )
                .ok()
                .and_then(|(v,)| v.0.as_str().map(|s| s.to_lowercase()))
        });
    FocusedNodeInspection {
        role_known: true,
        is_password,
        is_omnibox,
        is_editable_text,
        app_name,
    }
}

fn query_has_inline_selection(conn: &Connection, sender: &str, path: &str) -> bool {
    let proxy = conn.with_proxy(sender, path, QUERY_TIMEOUT);
    let Ok((count,)): Result<(i32,), _> =
        proxy.method_call("org.a11y.atspi.Text", "GetNSelections", ())
    else {
        return false;
    };
    if count <= 0 {
        return false;
    }
    let Ok((char_count,)): Result<(i32,), _> =
        proxy.method_call("org.a11y.atspi.Text", "GetCharacterCount", ())
    else {
        return false;
    };
    let Ok((start, end)): Result<(i32, i32), _> =
        proxy.method_call("org.a11y.atspi.Text", "GetSelection", (0i32,))
    else {
        return false;
    };
    let caret = proxy
        .method_call::<(Variant<Box<dyn RefArg>>,), _, _, _>(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.a11y.atspi.Text", "CaretOffset"),
        )
        .ok()
        .and_then(|(v,)| v.0.as_i64())
        .map(|v| v as i32)
        .unwrap_or(start.min(end));
    is_inline_autocomplete_selection(caret, start, end, char_count)
}

fn watch_once(shared_focus: SharedFocusSnapshot) -> Result<(), Box<dyn Error>> {
    let address = accessibility_bus_address()?;
    let connection = Connection::new_address(&address)?;

    let registry =
        connection.with_proxy(ATSPI_REGISTRY, ATSPI_REGISTRY_PATH, Duration::from_secs(2));
    let _: () = registry.method_call(
        "org.a11y.atspi.Registry",
        "RegisterEvent",
        (ATSPI_EVENT_FOCUS,),
    )?;
    let _ = registry.method_call::<(), _, _, _>(
        "org.a11y.atspi.Registry",
        "RegisterEvent",
        (ATSPI_EVENT_SELECTION,),
    );
    let _ = registry.method_call::<(), _, _, _>(
        "org.a11y.atspi.Registry",
        "RegisterEvent",
        (ATSPI_EVENT_CARET,),
    );

    let pending: Arc<Mutex<Vec<PendingAtspiEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let has_pending_focus = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut focus_rule = MatchRule::new_signal("org.a11y.atspi.Event.Object", "StateChanged");
    focus_rule.path_is_namespace = true;
    let pending_focus = Arc::clone(&pending);
    let pending_focus_flag = Arc::clone(&has_pending_focus);
    connection.add_match(focus_rule, move |_: (), _, message| {
        let mut fields = message.iter_init();
        let state = fields.read::<&str>().unwrap_or_default();
        let enabled = fields.read::<i32>().unwrap_or_default();

        if state.eq_ignore_ascii_case("focused") && enabled != 0 {
            if let (Some(s), Some(p)) = (message.sender(), message.path()) {
                let s_str = s.to_string();
                let p_str = p.to_string();
                if !s_str.is_empty() && !p_str.is_empty() {
                    pending_focus_flag.store(true, std::sync::atomic::Ordering::Release);
                    pending_focus
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(PendingAtspiEvent::Focused(s_str, p_str));
                }
            }
        }
        true
    })?;

    let mut sel_rule = MatchRule::new_signal("org.a11y.atspi.Event.Object", "TextSelectionChanged");
    sel_rule.path_is_namespace = true;
    let pending_sel = Arc::clone(&pending);
    connection.add_match(sel_rule, move |_: (), _, message| {
        if let (Some(s), Some(p)) = (message.sender(), message.path()) {
            let s_str = s.to_string();
            let p_str = p.to_string();
            if !s_str.is_empty() && !p_str.is_empty() {
                pending_sel
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(PendingAtspiEvent::SelectionChanged(s_str, p_str));
            }
        }
        true
    })?;

    let mut caret_rule = MatchRule::new_signal("org.a11y.atspi.Event.Object", "TextCaretMoved");
    caret_rule.path_is_namespace = true;
    let pending_caret = Arc::clone(&pending);
    connection.add_match(caret_rule, move |_: (), _, message| {
        let mut fields = message.iter_init();
        let _detail = fields.read::<&str>().unwrap_or_default();
        let caret = fields.read::<i32>().unwrap_or(-1);
        if caret >= 0 {
            if let (Some(s), Some(p)) = (message.sender(), message.path()) {
                let s_str = s.to_string();
                let p_str = p.to_string();
                if !s_str.is_empty() && !p_str.is_empty() {
                    pending_caret
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(PendingAtspiEvent::CaretMoved(s_str, p_str, caret));
                }
            }
        }
        true
    })?;

    let mut active_omnibox: Option<(String, String)> = None;
    let mut active_text_node: Option<(String, String)> = None;
    log_info("[dboard] AT-SPI focus, password, omnibox & surrounding-text watcher active");
    loop {
        if crate::signal::SIGNAL_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        connection.process(PROCESS_TIMEOUT)?;
        has_pending_focus.store(false, std::sync::atomic::Ordering::Release);
        let drained: Vec<PendingAtspiEvent> = coalesce_atspi_events(
            pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain(..)
                .collect(),
        );
        let mut latest_caret: Option<(String, String, i32)> = None;
        for ev in drained {
            match ev {
                PendingAtspiEvent::Focused(sender, path) => {
                    latest_caret = None;
                    // Inspect role & password state BEFORE bumping focus generation so
                    // evdev_loop sees the updated password flag on the very next keystroke!
                    let info = inspect_focused_node(&connection, &sender, &path, &shared_focus);
                    let atspi_app = info.app_name.clone();
                    if info.role_known {
                        crate::password_detector::record_atspi_focus_state(
                            info.is_password,
                            info.app_name,
                        );
                    }
                    observe_atspi_focus_with_path(
                        &shared_focus,
                        Some(&sender),
                        Some(&path),
                        atspi_app.as_deref(),
                    );
                    if info.is_omnibox {
                        let inline = query_has_inline_selection(&connection, &sender, &path);
                        shared_focus
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_omnibox_inline_selection(inline);
                        active_omnibox = Some((sender.clone(), path.clone()));
                    } else {
                        active_omnibox = None;
                        crate::focus::clear_omnibox_inline_selection(&shared_focus);
                    }
                    if info.is_editable_text {
                        let token = shared_focus
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .surrounding_freshness_token();
                        let caret = crate::surrounding_context::query_caret_offset(
                            &connection,
                            &sender,
                            &path,
                        );
                        let ctx = caret.and_then(|c| {
                            crate::surrounding_context::query_surrounding_context(
                                &connection,
                                &sender,
                                &path,
                                c,
                            )
                        });
                        let rect = if info.is_password {
                            None
                        } else {
                            caret.and_then(|c| {
                                crate::surrounding_context::query_caret_extents(
                                    &connection,
                                    &sender,
                                    &path,
                                    c,
                                )
                            })
                        };
                        let fresh = shared_focus
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_surrounding_context_if_fresh(ctx, token);
                        if fresh {
                            crate::candidate_ipc::record_caret_rect(rect);
                        }
                        active_text_node = Some((sender, path));
                    } else {
                        active_text_node = None;
                        crate::candidate_ipc::record_caret_rect(None);
                        crate::focus::clear_surrounding_word(&shared_focus);
                    }
                }
                PendingAtspiEvent::SelectionChanged(sender, path) => {
                    if active_omnibox
                        .as_ref()
                        .is_some_and(|(s, p)| s == &sender && p == &path)
                    {
                        let inline = query_has_inline_selection(&connection, &sender, &path);
                        shared_focus
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_omnibox_inline_selection(inline);
                    }
                    if active_text_node
                        .as_ref()
                        .is_some_and(|(s, p)| s == &sender && p == &path)
                    {
                        crate::focus::clear_surrounding_word(&shared_focus);
                    }
                }
                PendingAtspiEvent::CaretMoved(sender, path, caret) => {
                    if active_text_node
                        .as_ref()
                        .is_some_and(|(s, p)| s == &sender && p == &path)
                    {
                        latest_caret = Some((sender, path, caret));
                    }
                }
            }
        }
        if let Some((sender, path, caret)) = latest_caret {
            if !has_pending_focus.load(std::sync::atomic::Ordering::Acquire)
                && !crate::password_detector::is_atspi_password_focused()
            {
                let token = shared_focus
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .surrounding_freshness_token();
                let ctx = crate::surrounding_context::query_surrounding_context(
                    &connection,
                    &sender,
                    &path,
                    caret,
                );
                let rect = crate::surrounding_context::query_caret_extents(
                    &connection,
                    &sender,
                    &path,
                    caret,
                );
                let fresh = shared_focus
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set_surrounding_context_if_fresh(ctx, token);
                if fresh {
                    crate::candidate_ipc::record_caret_rect(rect);
                }
            }
        }
    }
}

pub fn spawn_atspi_focus_watcher(shared_focus: SharedFocusSnapshot) {
    thread::spawn(move || loop {
        if crate::signal::SIGNAL_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if let Err(error) = watch_once(shared_focus.clone()) {
            crate::password_detector::reset_atspi_focus_state();
            crate::focus::clear_omnibox_inline_selection(&shared_focus);
            crate::focus::clear_surrounding_word(&shared_focus);
            log_info(&format!(
                "[dboard] AT-SPI focus watcher unavailable: {}; retrying",
                error
            ));
            thread::sleep(RETRY_DELAY);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::focus::{FocusReliability, FocusSnapshot};

    use super::{observe_atspi_focus, observe_atspi_focus_with_path};

    #[test]
    fn atspi_focus_deduplicates_events_from_same_application() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));

        observe_atspi_focus(&focus, Some(":1.100"));
        observe_atspi_focus(&focus, Some(":1.100"));

        let focus = focus.lock().unwrap();
        assert_eq!(focus.identity.as_deref(), Some("atspi::1.100"));
        assert_eq!(focus.generation, 1);
        assert_eq!(focus.reliability, FocusReliability::Reliable);
    }

    #[test]
    fn atspi_focus_switch_increments_generation_without_user_text() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));

        observe_atspi_focus(&focus, Some(":1.100"));
        observe_atspi_focus(&focus, Some(":1.200"));

        let focus = focus.lock().unwrap();
        assert_eq!(focus.identity.as_deref(), Some("atspi::1.200"));
        assert_eq!(focus.generation, 2);
    }

    #[test]
    fn atspi_focus_switch_between_widgets_in_same_application_increments_generation() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));

        observe_atspi_focus_with_path(
            &focus,
            Some(":1.100"),
            Some("/org/a11y/atspi/accessible/10"),
            None,
        );
        observe_atspi_focus_with_path(
            &focus,
            Some(":1.100"),
            Some("/org/a11y/atspi/accessible/20"),
            None,
        );

        let focus = focus.lock().unwrap();
        assert_eq!(
            focus.identity.as_deref(),
            Some("atspi::1.100:/org/a11y/atspi/accessible/20")
        );
        assert_eq!(focus.generation, 2);
    }

    #[test]
    fn atspi_focus_ignores_missing_application_identity() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));

        observe_atspi_focus(&focus, None);
        observe_atspi_focus(&focus, Some(""));

        let focus = focus.lock().unwrap();
        assert_eq!(focus.identity, None);
        assert_eq!(focus.generation, 0);
    }

    #[test]
    fn atspi_focus_updates_stale_class_immediately_without_compositor_authority() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));
        {
            let mut snap = focus.lock().unwrap();
            snap.observe(Some("poller:1".into()), Some("firefox".into()));
            assert!(!snap.class_from_compositor);
        }

        // Without compositor authority, fresh AT-SPI2 app_name must replace stale class immediately.
        super::observe_atspi_focus_with_path(&focus, Some(":1.9"), Some("/w1"), Some("keepassxc"));
        assert_eq!(focus.lock().unwrap().class.as_deref(), Some("keepassxc"));

        // With compositor authority, compositor class is preserved unless cleared by FocusLost.
        {
            let mut snap = focus.lock().unwrap();
            snap.observe(Some("toplevel:1".into()), Some("firefox".into()));
            snap.mark_class_from_compositor();
        }
        super::observe_atspi_focus_with_path(
            &focus,
            Some(":1.10"),
            Some("/w2"),
            Some("Web Content"),
        );
        assert_eq!(focus.lock().unwrap().class.as_deref(), Some("firefox"));

        // After FocusLost clears class, AT-SPI2 fallback still populates Layer 2 class.
        focus.lock().unwrap().observe_focus_lost();
        super::observe_atspi_focus_with_path(
            &focus,
            Some(":1.11"),
            Some("/w3"),
            Some("cosmic-osd"),
        );
        assert_eq!(focus.lock().unwrap().class.as_deref(), Some("cosmic-osd"));
    }

    #[test]
    fn coalesce_atspi_events_keeps_only_latest_focus_and_selection() {
        use super::{coalesce_atspi_events, PendingAtspiEvent};

        let events = vec![
            PendingAtspiEvent::Focused(":1.1".into(), "/w1".into()),
            PendingAtspiEvent::SelectionChanged(":1.1".into(), "/w1".into()),
            PendingAtspiEvent::CaretMoved(":1.1".into(), "/w1".into(), 5),
            PendingAtspiEvent::Focused(":1.2".into(), "/pw".into()),
            PendingAtspiEvent::SelectionChanged(":1.2".into(), "/pw".into()),
            PendingAtspiEvent::SelectionChanged(":1.2".into(), "/pw".into()),
            PendingAtspiEvent::CaretMoved(":1.2".into(), "/pw".into(), 0),
        ];
        let coalesced = coalesce_atspi_events(events);
        assert_eq!(
            coalesced,
            vec![
                PendingAtspiEvent::Focused(":1.2".into(), "/pw".into()),
                PendingAtspiEvent::SelectionChanged(":1.2".into(), "/pw".into()),
                PendingAtspiEvent::CaretMoved(":1.2".into(), "/pw".into(), 0),
            ]
        );
    }
}

#[cfg(test)]
mod omnibox_tests {
    use super::{is_inline_autocomplete_selection, is_omnibox_label};

    #[test]
    fn recognizes_browser_omnibox_labels_and_inline_autocomplete_offsets() {
        assert!(is_omnibox_label("Address and search bar"));
        assert!(is_omnibox_label("Search with Google or enter address"));
        assert!(is_omnibox_label("Thanh địa chỉ và tìm kiếm"));
        assert!(!is_omnibox_label("Compose email body"));
        assert!(!is_omnibox_label("Email address"));
        assert!(!is_omnibox_label("Street address"));

        // `h[ttps://github.com]` -> caret at 1, selection 1..18, char_count 18
        assert!(is_inline_autocomplete_selection(1, 1, 18, 18));
        // Mid-URL selection `https://[github].com` (8..14, char_count 18) is NOT inline suffix
        assert!(!is_inline_autocomplete_selection(8, 8, 14, 18));
        // Select-all `0..18` is NOT an inline suffix after a typed character
        assert!(!is_inline_autocomplete_selection(0, 0, 18, 18));
        // Empty selection `1..1` is NOT an inline autocomplete suffix
        assert!(!is_inline_autocomplete_selection(1, 1, 1, 18));
    }
}
