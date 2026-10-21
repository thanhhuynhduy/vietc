use std::error::Error;
use std::thread;
use std::time::Duration;

use dbus::blocking::Connection;
use dbus::message::MatchRule;

use crate::focus::SharedFocusSnapshot;
use crate::log::log_info;

const ATSPI_REGISTRY: &str = "org.a11y.atspi.Registry";
const ATSPI_REGISTRY_PATH: &str = "/org/a11y/atspi/registry";
const ATSPI_EVENT: &str = "object:state-changed:focused";
const RETRY_DELAY: Duration = Duration::from_secs(2);
const PROCESS_TIMEOUT: Duration = Duration::from_millis(500);

fn accessibility_bus_address() -> Result<String, Box<dyn Error>> {
    let connection = Connection::new_session()?;
    let proxy = connection.with_proxy("org.a11y.Bus", "/org/a11y/bus", Duration::from_secs(2));
    let (address,): (String,) = proxy.method_call("org.a11y.Bus", "GetAddress", ())?;
    Ok(address)
}

fn observe_atspi_focus(shared_focus: &SharedFocusSnapshot, application_identity: Option<&str>) {
    let Some(application_identity) = application_identity.filter(|value| !value.is_empty()) else {
        return;
    };

    shared_focus
        .lock()
        .unwrap()
        .observe(Some(format!("atspi:{application_identity}")), None);
}

fn watch_once(shared_focus: SharedFocusSnapshot) -> Result<(), Box<dyn Error>> {
    let address = accessibility_bus_address()?;
    let connection = Connection::new_address(&address)?;

    let registry =
        connection.with_proxy(ATSPI_REGISTRY, ATSPI_REGISTRY_PATH, Duration::from_secs(2));
    let _: () = registry.method_call("org.a11y.atspi.Registry", "RegisterEvent", (ATSPI_EVENT,))?;

    let mut rule = MatchRule::new_signal("org.a11y.atspi.Event.Object", "StateChanged");
    rule.path_is_namespace = true;

    let focus = shared_focus.clone();
    connection.add_match(rule, move |_: (), _, message| {
        let mut fields = message.iter_init();
        let state = fields.read::<&str>().unwrap_or_default();
        let enabled = fields.read::<i32>().unwrap_or_default();

        if state.eq_ignore_ascii_case("focused") && enabled != 0 {
            let sender = message.sender().map(|value| value.to_string());
            observe_atspi_focus(&focus, sender.as_deref());
        }
        true
    })?;

    log_info("[vietc] AT-SPI focus watcher active");
    loop {
        connection.process(PROCESS_TIMEOUT)?;
    }
}

pub fn spawn_atspi_focus_watcher(shared_focus: SharedFocusSnapshot) {
    thread::spawn(move || loop {
        if let Err(error) = watch_once(shared_focus.clone()) {
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

    use super::observe_atspi_focus;

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
    fn atspi_focus_ignores_missing_application_identity() {
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));

        observe_atspi_focus(&focus, None);
        observe_atspi_focus(&focus, Some(""));

        let focus = focus.lock().unwrap();
        assert_eq!(focus.identity, None);
        assert_eq!(focus.generation, 0);
    }
}
