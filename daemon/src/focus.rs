use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FOCUS_UNKNOWN_COMPOSITION_TTL: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusReliability {
    Reliable,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusSnapshot {
    pub identity: Option<String>,
    pub class: Option<String>,
    pub generation: u64,
    pub reliability: FocusReliability,
}

impl Default for FocusSnapshot {
    fn default() -> Self {
        Self {
            identity: None,
            class: None,
            generation: 0,
            reliability: FocusReliability::Unavailable,
        }
    }
}

impl FocusSnapshot {
    pub fn observe(&mut self, identity: Option<String>, class: Option<String>) {
        let identity = identity.filter(|value| !value.is_empty());
        let class = class.filter(|value| !value.is_empty());

        match identity {
            Some(identity) => {
                if self.identity.as_deref() != Some(identity.as_str()) {
                    self.generation = self.generation.wrapping_add(1);
                }
                self.identity = Some(identity);
                self.class = class;
                self.reliability = FocusReliability::Reliable;
            }
            None => {
                self.class = class;
                self.reliability = FocusReliability::Unavailable;
            }
        }
    }
}

pub type SharedFocusSnapshot = Arc<Mutex<FocusSnapshot>>;

pub fn should_expire_unknown_composition(
    reliability: FocusReliability,
    idle: Duration,
    has_composition: bool,
) -> bool {
    reliability == FocusReliability::Unavailable
        && has_composition
        && idle >= FOCUS_UNKNOWN_COMPOSITION_TTL
}

#[cfg(test)]
mod tests {
    use super::{
        should_expire_unknown_composition, FocusReliability, FocusSnapshot,
        FOCUS_UNKNOWN_COMPOSITION_TTL,
    };
    use std::time::Duration;

    #[test]
    fn unknown_focus_does_not_create_identity_or_generation() {
        let mut snapshot = FocusSnapshot::default();

        snapshot.observe(None, None);

        assert_eq!(snapshot.identity, None);
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.reliability, FocusReliability::Unavailable);
    }

    #[test]
    fn known_identity_change_increments_generation_once() {
        let mut snapshot = FocusSnapshot::default();

        snapshot.observe(Some("window-a".to_string()), Some("edge".to_string()));
        assert_eq!(snapshot.generation, 1);

        snapshot.observe(Some("window-a".to_string()), Some("edge".to_string()));
        assert_eq!(snapshot.generation, 1);

        snapshot.observe(Some("window-b".to_string()), Some("telegram".to_string()));
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.identity.as_deref(), Some("window-b"));
    }

    #[test]
    fn class_only_update_does_not_create_focus_transition() {
        let mut snapshot = FocusSnapshot::default();
        snapshot.observe(Some("window-a".to_string()), Some("edge".to_string()));

        snapshot.observe(
            Some("window-a".to_string()),
            Some("microsoft-edge".to_string()),
        );

        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.class.as_deref(), Some("microsoft-edge"));
    }

    #[test]
    fn unknown_focus_expires_pending_composition_at_ttl() {
        assert!(should_expire_unknown_composition(
            FocusReliability::Unavailable,
            FOCUS_UNKNOWN_COMPOSITION_TTL,
            true,
        ));
        assert!(should_expire_unknown_composition(
            FocusReliability::Unavailable,
            Duration::from_millis(900),
            true,
        ));
    }

    #[test]
    fn continuous_typing_below_ttl_preserves_composition() {
        assert!(!should_expire_unknown_composition(
            FocusReliability::Unavailable,
            Duration::from_millis(749),
            true,
        ));
        assert!(!should_expire_unknown_composition(
            FocusReliability::Reliable,
            Duration::from_secs(5),
            true,
        ));
        assert!(!should_expire_unknown_composition(
            FocusReliability::Unavailable,
            Duration::from_secs(5),
            false,
        ));
    }

    #[test]
    fn unavailable_observation_preserves_last_known_identity_without_transition() {
        let mut snapshot = FocusSnapshot::default();
        snapshot.observe(Some("window-a".to_string()), Some("edge".to_string()));

        snapshot.observe(None, None);

        assert_eq!(snapshot.identity.as_deref(), Some("window-a"));
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.reliability, FocusReliability::Unavailable);
    }
}
