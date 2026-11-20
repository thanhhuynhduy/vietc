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
const ROLE_AUTOCOMPLETE: i32 = 4;
const ROLE_ENTRY: i32 = 19;
const ROLE_TEXT: i32 = 60;
const RETRY_DELAY: Duration = Duration::from_secs(2);
const PROCESS_TIMEOUT: Duration = Duration::from_millis(500);
const QUERY_TIMEOUT: Duration = Duration::from_millis(35);

enum PendingAtspiEvent {
    Focused(String, String),
    SelectionChanged(String, String),
}

fn accessibility_bus_address() -> Result<String, Box<dyn Error>> {
    let connection = Connection::new_session()?;
    let proxy = connection.with_proxy("org.a11y.Bus", "/org/a11y/bus", Duration::from_secs(2));
    let (address,): (String,) = proxy.method_call("org.a11y.Bus", "GetAddress", ())?;
    Ok(address)
}

#[cfg(test)]
fn observe_atspi_focus(shared_focus: &SharedFocusSnapshot, application_identity: Option<&str>) {
    observe_atspi_focus_with_path(shared_focus, application_identity, None);
}

fn observe_atspi_focus_with_path(
    shared_focus: &SharedFocusSnapshot,
    application_identity: Option<&str>,
    object_path: Option<&str>,
) {
    let Some(application_identity) = application_identity.filter(|value| !value.is_empty()) else {
        return;
    };

    let identity = match object_path.filter(|path| !path.is_empty()) {
        Some(path) => format!("atspi:{application_identity}:{path}"),
        None => format!("atspi:{application_identity}"),
    };

    let mut focus = shared_focus.lock().unwrap();
    let class = focus.class.clone();
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
    is_password: bool,
    is_omnibox: bool,
    app_name: Option<String>,
}

fn inspect_focused_node(conn: &Connection, sender: &str, path: &str) -> FocusedNodeInspection {
    let proxy = conn.with_proxy(sender, path, QUERY_TIMEOUT);
    let role = proxy
        .method_call::<(i32,), _, _, _>("org.a11y.atspi.Accessible", "GetRole", ())
        .map(|(r,)| r)
        .unwrap_or(0);
    let is_password = role == crate::password_detector::ROLE_PASSWORD_TEXT;
    let is_omnibox = if role == ROLE_AUTOCOMPLETE {
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
        is_password,
        is_omnibox,
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

    let pending: Arc<Mutex<Vec<PendingAtspiEvent>>> = Arc::new(Mutex::new(Vec::new()));

    let mut focus_rule = MatchRule::new_signal("org.a11y.atspi.Event.Object", "StateChanged");
    focus_rule.path_is_namespace = true;
    let pending_focus = Arc::clone(&pending);
    connection.add_match(focus_rule, move |_: (), _, message| {
        let mut fields = message.iter_init();
        let state = fields.read::<&str>().unwrap_or_default();
        let enabled = fields.read::<i32>().unwrap_or_default();

        if state.eq_ignore_ascii_case("focused") && enabled != 0 {
            if let (Some(s), Some(p)) = (message.sender(), message.path()) {
                let s_str = s.to_string();
                let p_str = p.to_string();
                if !s_str.is_empty() && !p_str.is_empty() {
                    pending_focus
                        .lock()
                        .unwrap()
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
                    .unwrap()
                    .push(PendingAtspiEvent::SelectionChanged(s_str, p_str));
            }
        }
        true
    })?;

    let mut active_omnibox: Option<(String, String)> = None;
    log_info("[vietc] AT-SPI focus, password & omnibox selection watcher active");
    loop {
        connection.process(PROCESS_TIMEOUT)?;
        let drained: Vec<PendingAtspiEvent> = pending.lock().unwrap().drain(..).collect();
        for ev in drained {
            match ev {
                PendingAtspiEvent::Focused(sender, path) => {
                    // Inspect role & password state BEFORE bumping focus generation so
                    // evdev_loop sees the updated password flag on the very next keystroke!
                    let info = inspect_focused_node(&connection, &sender, &path);
                    crate::password_detector::record_atspi_focus_state(
                        info.is_password,
                        info.app_name,
                    );
                    observe_atspi_focus_with_path(&shared_focus, Some(&sender), Some(&path));
                    if info.is_omnibox {
                        let inline = query_has_inline_selection(&connection, &sender, &path);
                        shared_focus
                            .lock()
                            .unwrap()
                            .set_omnibox_inline_selection(inline);
                        active_omnibox = Some((sender, path));
                    } else {
                        active_omnibox = None;
                        crate::focus::clear_omnibox_inline_selection(&shared_focus);
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
                            .unwrap()
                            .set_omnibox_inline_selection(inline);
                    }
                }
            }
        }
    }
}

pub fn spawn_atspi_focus_watcher(shared_focus: SharedFocusSnapshot) {
    thread::spawn(move || loop {
        if let Err(error) = watch_once(shared_focus.clone()) {
            crate::password_detector::reset_atspi_focus_state();
            crate::focus::clear_omnibox_inline_selection(&shared_focus);
            log_info(&format!(
                "[vietc] AT-SPI focus watcher unavailable: {}; retrying",
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
        );
        observe_atspi_focus_with_path(
            &focus,
            Some(":1.100"),
            Some("/org/a11y/atspi/accessible/20"),
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
