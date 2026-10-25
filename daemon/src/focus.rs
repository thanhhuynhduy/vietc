use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FOCUS_UNKNOWN_COMPOSITION_TTL: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusReliability {
    Reliable,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIdentity {
    pub seat: String,
    pub app_id: Option<String>,
    pub toplevel: Option<String>,
    pub surface_or_window: Option<String>,
}

impl TargetIdentity {
    pub fn new(
        seat: impl Into<String>,
        app_id: Option<String>,
        toplevel: Option<String>,
        surface_or_window: Option<String>,
    ) -> Option<Self> {
        let normalize = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
        let identity = Self {
            seat: seat.into(),
            app_id: normalize(app_id),
            toplevel: normalize(toplevel),
            surface_or_window: normalize(surface_or_window),
        };
        if identity.toplevel.is_none() && identity.surface_or_window.is_none() {
            None
        } else {
            Some(identity)
        }
    }
}

#[derive(Debug, Clone)]
pub struct FocusSnapshot {
    pub identity: Option<String>,
    pub target: Option<TargetIdentity>,
    pub class: Option<String>,
    pub generation: u64,
    pub generation_signal: Arc<AtomicU64>,
    pub reliability: FocusReliability,
}

impl Default for FocusSnapshot {
    fn default() -> Self {
        Self {
            identity: None,
            target: None,
            class: None,
            generation: 0,
            generation_signal: Arc::new(AtomicU64::new(0)),
            reliability: FocusReliability::Unavailable,
        }
    }
}

impl FocusSnapshot {
    pub fn observe(&mut self, identity: Option<String>, class: Option<String>) {
        let seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".to_string());
        let target = identity.as_ref().and_then(|window| {
            TargetIdentity::new(seat, None, Some(window.clone()), Some(window.clone()))
        });
        self.observe_target(target, class);
    }

    pub fn observe_target(&mut self, target: Option<TargetIdentity>, class: Option<String>) {
        let class = class.filter(|value| !value.is_empty());

        match target {
            Some(target) => {
                if self.target.as_ref() != Some(&target) {
                    self.generation = self.generation.wrapping_add(1);
                    self.generation_signal
                        .store(self.generation, Ordering::Release);
                }
                self.identity = target
                    .surface_or_window
                    .clone()
                    .or_else(|| target.toplevel.clone());
                self.target = Some(target);
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
        should_expire_unknown_composition, FocusReliability, FocusSnapshot, TargetIdentity,
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
    fn same_app_id_with_different_toplevel_increments_generation() {
        let mut snapshot = FocusSnapshot::default();
        let target_a = TargetIdentity::new(
            "seat0",
            Some("org.example.Editor".to_string()),
            Some("toplevel-a".to_string()),
            Some("surface-a".to_string()),
        )
        .unwrap();
        let target_b = TargetIdentity::new(
            "seat0",
            Some("org.example.Editor".to_string()),
            Some("toplevel-b".to_string()),
            Some("surface-b".to_string()),
        )
        .unwrap();

        snapshot.observe_target(Some(target_a), Some("org.example.Editor".to_string()));
        assert_eq!(snapshot.generation, 1);

        snapshot.observe_target(Some(target_b), Some("org.example.Editor".to_string()));
        assert_eq!(snapshot.generation, 2);
    }

    #[test]
    fn returning_to_previous_target_creates_a_new_generation() {
        let mut snapshot = FocusSnapshot::default();
        let target_a = TargetIdentity::new(
            "seat0",
            Some("app-a".to_string()),
            Some("toplevel-a".to_string()),
            Some("surface-a".to_string()),
        )
        .unwrap();
        let target_b = TargetIdentity::new(
            "seat0",
            Some("app-b".to_string()),
            Some("toplevel-b".to_string()),
            Some("surface-b".to_string()),
        )
        .unwrap();

        snapshot.observe_target(Some(target_a.clone()), Some("app-a".to_string()));
        snapshot.observe_target(Some(target_b), Some("app-b".to_string()));
        snapshot.observe_target(Some(target_a), Some("app-a".to_string()));

        assert_eq!(snapshot.generation, 3);
    }

    #[test]
    fn closed_target_becomes_unavailable_without_fabricating_a_generation() {
        let mut snapshot = FocusSnapshot::default();
        snapshot.observe(Some("window-a".to_string()), Some("editor".to_string()));
        let generation = snapshot.generation;

        snapshot.observe_target(None, None);

        assert_eq!(snapshot.generation, generation);
        assert_eq!(snapshot.reliability, FocusReliability::Unavailable);
        assert_eq!(snapshot.identity.as_deref(), Some("window-a"));
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
