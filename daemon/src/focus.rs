use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::surrounding_context::{
    SurroundingContext, SurroundingContextData, SURROUNDING_CONTEXT_TTL,
};

#[allow(dead_code)]
pub const FOCUS_UNKNOWN_COMPOSITION_TTL: Duration = Duration::from_millis(750);
pub const OMNIBOX_SELECTION_TTL: Duration = Duration::from_millis(500);

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
    /// True once the compositor itself reported the app class. Heuristic
    /// pollers (xdotool/XWayland, /proc) are then ignored: on Wayland they
    /// describe a stale X11 window, not the focused one.
    pub class_from_compositor: bool,
    /// True when AT-SPI2 observed an active inline autocomplete selection after
    /// the caret in a focused browser omnibox / autocomplete entry.
    pub omnibox_inline_selection: bool,
    pub omnibox_selection_at: Option<Instant>,
    /// Surrounding context before the caret observed by AT-SPI2 `TextCaretMoved`
    /// on a non-password editable widget, consumed once when the engine is idle.
    pub surrounding_context: Option<SurroundingContext>,
    /// Timestamp of the most recent pointer click so pre-click surrounding
    /// context is discarded while post-click AT-SPI `TextCaretMoved` context survives.
    pub last_pointer_click_at: Option<Instant>,
    /// Monotonic counter incremented on every physical KeyDown (`evdev_loop`) so
    /// in-flight AT-SPI2 `GetText` responses that began before a keystroke/Backspace
    /// are discarded instead of overwriting fresh editor state.
    pub key_activity_seq: Arc<AtomicU64>,
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
            class_from_compositor: false,
            omnibox_inline_selection: false,
            omnibox_selection_at: None,
            surrounding_context: None,
            last_pointer_click_at: None,
            key_activity_seq: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurroundingFreshnessToken {
    pub key_seq: u64,
    pub generation: u64,
    pub last_pointer_click_at: Option<Instant>,
}

impl FocusSnapshot {
    pub fn observe(&mut self, identity: Option<String>, class: Option<String>) {
        let seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".to_string());
        let target = identity.as_ref().and_then(|window| {
            TargetIdentity::new(seat, None, Some(window.clone()), Some(window.clone()))
        });
        self.observe_target(target, class);
    }

    /// Update only the application class (e.g. its app_id arrived after the
    /// window was activated) without touching identity or generation.
    pub fn mark_class_from_compositor(&mut self) {
        self.class_from_compositor = true;
    }

    pub fn set_class(&mut self, class: Option<String>) {
        self.class = class.filter(|value| !value.is_empty());
    }

    pub fn set_omnibox_inline_selection(&mut self, active: bool) {
        self.omnibox_inline_selection = active;
        self.omnibox_selection_at = active.then(Instant::now);
    }

    pub fn set_surrounding_context(&mut self, data: Option<SurroundingContextData>) {
        self.surrounding_context = data.map(|d| SurroundingContext {
            data: d,
            generation: self.generation,
            captured_at: Instant::now(),
        });
    }

    pub fn surrounding_freshness_token(&self) -> SurroundingFreshnessToken {
        SurroundingFreshnessToken {
            key_seq: self.key_activity_seq.load(Ordering::Acquire),
            generation: self.generation,
            last_pointer_click_at: self.last_pointer_click_at,
        }
    }

    /// Store AT-SPI2 surrounding context only if no physical keystroke, focus
    /// change (`generation`), or pointer click occurred while the asynchronous
    /// D-Bus `GetText` query was in flight. Returns `true` iff the token was
    /// still fresh.
    pub fn set_surrounding_context_if_fresh(
        &mut self,
        data: Option<SurroundingContextData>,
        token: SurroundingFreshnessToken,
    ) -> bool {
        if self.key_activity_seq.load(Ordering::Acquire) != token.key_seq
            || self.generation != token.generation
            || self.last_pointer_click_at != token.last_pointer_click_at
        {
            return false;
        }
        self.set_surrounding_context(data);
        true
    }

    pub fn key_activity_signal(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.key_activity_seq)
    }

    pub fn observe_target(&mut self, target: Option<TargetIdentity>, class: Option<String>) {
        let class = class.filter(|value| !value.is_empty());

        match target {
            Some(target) => {
                if self.target.as_ref() != Some(&target) {
                    self.generation = self.generation.wrapping_add(1);
                    self.generation_signal
                        .store(self.generation, Ordering::Release);
                    self.omnibox_inline_selection = false;
                    self.omnibox_selection_at = None;
                    self.surrounding_context = None;
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
                self.omnibox_inline_selection = false;
                self.omnibox_selection_at = None;
                self.surrounding_context = None;
                self.reliability = FocusReliability::Unavailable;
            }
        }
    }

    /// Record loss of active toplevel focus (e.g., user opened a layer-shell
    /// surface such as `cosmic-launcher`, `wofi`, `rofi`, `fuzzel`, or a Polkit
    /// dialog). Bumps generation to reset composition and clears `class` so
    /// previous app overrides (`bypass_apps`, `english_apps`, `terminal_apps`)
    /// never leak into the overlay, while keeping a `toplevel:` identity so
    /// the 250ms background poller does not overwrite it.
    pub fn observe_focus_lost(&mut self) {
        let seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".to_string());
        let lost_id = "toplevel:focus-lost".to_string();
        self.generation = self.generation.wrapping_add(1);
        self.generation_signal
            .store(self.generation, Ordering::Release);
        self.target = TargetIdentity::new(seat, None, Some(lost_id.clone()), Some(lost_id.clone()));
        self.identity = Some(lost_id);
        self.class = None;
        self.class_from_compositor = true;
        self.omnibox_inline_selection = false;
        self.omnibox_selection_at = None;
        self.surrounding_context = None;
        self.reliability = FocusReliability::Reliable;
    }
}

pub type SharedFocusSnapshot = Arc<Mutex<FocusSnapshot>>;

pub fn clear_surrounding_word(shared: &SharedFocusSnapshot) {
    let mut focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    focus.surrounding_context = None;
}

pub fn record_pointer_click(shared: &SharedFocusSnapshot) {
    let mut focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    focus.last_pointer_click_at = Some(Instant::now());
    focus.surrounding_context = None;
}

pub fn take_surrounding_context(
    shared: &SharedFocusSnapshot,
    expected_generation: u64,
) -> Option<SurroundingContextData> {
    let mut focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = focus.surrounding_context.take()?;
    let after_last_click = focus
        .last_pointer_click_at
        .is_none_or(|click_at| entry.captured_at >= click_at);
    if entry.generation == expected_generation
        && after_last_click
        && entry.captured_at.elapsed() <= SURROUNDING_CONTEXT_TTL
    {
        Some(entry.data)
    } else {
        None
    }
}

pub fn peek_surrounding_context(
    shared: &SharedFocusSnapshot,
    expected_generation: u64,
) -> Option<SurroundingContextData> {
    let focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = focus.surrounding_context.as_ref()?;
    let after_last_click = focus
        .last_pointer_click_at
        .is_none_or(|click_at| entry.captured_at >= click_at);
    if entry.generation == expected_generation
        && after_last_click
        && entry.captured_at.elapsed() <= SURROUNDING_CONTEXT_TTL
    {
        Some(entry.data.clone())
    } else {
        None
    }
}

pub fn clear_omnibox_inline_selection(shared: &SharedFocusSnapshot) {
    let mut focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    focus.omnibox_inline_selection = false;
    focus.omnibox_selection_at = None;
}

pub fn take_omnibox_inline_selection(shared: &SharedFocusSnapshot) -> bool {
    let mut focus = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let active = std::mem::replace(&mut focus.omnibox_inline_selection, false);
    let ts = focus.omnibox_selection_at.take();
    active && ts.is_some_and(|at| at.elapsed() <= OMNIBOX_SELECTION_TTL)
}

#[allow(dead_code)]
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

    #[test]
    fn pointer_click_clears_pre_click_context_but_preserves_post_click_context() {
        use super::{record_pointer_click, take_surrounding_context, SharedFocusSnapshot};
        use crate::surrounding_context::SurroundingContextData;
        use std::sync::{Arc, Mutex};

        let shared: SharedFocusSnapshot = Arc::new(Mutex::new(FocusSnapshot::default()));
        {
            let mut f = shared.lock().unwrap();
            f.observe(Some("editor".into()), Some("gedit".into()));
            f.set_surrounding_context(Some(SurroundingContextData {
                word_at_caret: Some("stale".into()),
                prev_word: None,
                after_sentence_boundary: false,
            }));
        }
        let gen = shared.lock().unwrap().generation;

        // Click clears pre-click context
        record_pointer_click(&shared);
        assert_eq!(take_surrounding_context(&shared, gen), None);

        // Fresh post-click AT-SPI TextCaretMoved context survives until next keypress
        shared
            .lock()
            .unwrap()
            .set_surrounding_context(Some(SurroundingContextData {
                word_at_caret: Some("chao".into()),
                prev_word: Some("xin".into()),
                after_sentence_boundary: false,
            }));
        let taken =
            take_surrounding_context(&shared, gen).expect("post-click context must survive");
        assert_eq!(taken.word_at_caret.as_deref(), Some("chao"));
        assert_eq!(taken.prev_word.as_deref(), Some("xin"));
    }

    #[test]
    fn focus_lost_does_not_spuriously_bump_generation_on_background_poll() {
        let mut snapshot = FocusSnapshot::default();
        snapshot.observe(Some("toplevel:firefox".into()), Some("firefox".into()));
        let gen_before = snapshot.generation;

        snapshot.observe_focus_lost();
        let gen_after_lost = snapshot.generation;
        assert_eq!(gen_after_lost, gen_before.wrapping_add(1));
        assert_eq!(snapshot.identity.as_deref(), Some("toplevel:focus-lost"));
        assert_eq!(snapshot.class, None);
        assert!(snapshot.class_from_compositor);

        // Simulate the 250ms background poller re-observing the authoritative identity
        snapshot.observe(snapshot.identity.clone(), snapshot.class.clone());
        assert_eq!(
            snapshot.generation, gen_after_lost,
            "background poll must not spuriously bump generation and wipe launcher composition"
        );
    }

    #[test]
    fn stale_surrounding_context_is_discarded_when_keystroke_focus_or_click_occurred_in_flight() {
        use crate::surrounding_context::SurroundingContextData;
        use std::sync::atomic::Ordering;

        let mut snapshot = FocusSnapshot::default();
        snapshot.observe(Some("toplevel:firefox".into()), Some("firefox".into()));
        let sig = snapshot.key_activity_signal();
        let token_before_key = snapshot.surrounding_freshness_token();

        // 1. Keystroke occurred in flight -> must discard.
        sig.fetch_add(1, Ordering::AcqRel);
        let stale = SurroundingContextData {
            word_at_caret: Some("chao".into()),
            prev_word: Some("xin".into()),
            after_sentence_boundary: false,
        };
        assert!(!snapshot.set_surrounding_context_if_fresh(Some(stale.clone()), token_before_key));
        assert!(snapshot.surrounding_context.is_none());

        // 2. Focus switched to Window B in flight (without any keystroke) -> must discard.
        let token_window_a = snapshot.surrounding_freshness_token();
        snapshot.observe(Some("toplevel:code".into()), Some("code".into()));
        assert!(!snapshot.set_surrounding_context_if_fresh(Some(stale.clone()), token_window_a));
        assert!(snapshot.surrounding_context.is_none());

        // 3. Pointer click occurred inside same window in flight -> must discard.
        let token_before_click = snapshot.surrounding_freshness_token();
        snapshot.last_pointer_click_at = Some(std::time::Instant::now());
        assert!(!snapshot.set_surrounding_context_if_fresh(Some(stale.clone()), token_before_click));
        assert!(snapshot.surrounding_context.is_none());

        // 4. Fresh query with unchanged key_seq, generation, and pointer click succeeds.
        let token_fresh = snapshot.surrounding_freshness_token();
        assert!(snapshot.set_surrounding_context_if_fresh(Some(stale), token_fresh));
        assert!(snapshot.surrounding_context.is_some());
    }
}
