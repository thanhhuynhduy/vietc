// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryMode {
    Grabbed,
    Forwarded,
    WaylandPreedit,
    IbusPreedit,
    X11,
    Clipboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailurePoint {
    Delete,
    Insert,
    Flush,
    Acknowledgement,
    ClipboardRestore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FakeClock {
    now: Duration,
    deadline: Duration,
}

impl FakeClock {
    fn bounded(timeout: Duration) -> Self {
        assert!(timeout > Duration::ZERO);
        Self {
            now: Duration::ZERO,
            deadline: timeout,
        }
    }

    fn advance(&mut self, duration: Duration) -> Result<(), &'static str> {
        self.now = self.now.saturating_add(duration);
        if self.now > self.deadline {
            Err("bounded deterministic timeout exceeded")
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transaction {
    id: u64,
    expected_revision: u64,
    delete_before_cursor: usize,
    insert: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FrontendState {
    visible: String,
    visible_revision: u64,
    shift_pressed: bool,
    clipboard: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    applied: bool,
    actual: String,
    visible_revision: u64,
}

fn apply_transaction(
    mode: DeliveryMode,
    state: &mut FrontendState,
    transaction: &Transaction,
    failure: Option<FailurePoint>,
    clock: &mut FakeClock,
) -> Outcome {
    clock.advance(Duration::from_millis(1)).unwrap();

    if transaction.expected_revision != state.visible_revision {
        return Outcome {
            applied: false,
            actual: state.visible.clone(),
            visible_revision: state.visible_revision,
        };
    }

    let original = state.clone();
    let mut candidate = state.visible.clone();

    if failure == Some(FailurePoint::Delete) {
        return Outcome {
            applied: false,
            actual: original.visible,
            visible_revision: original.visible_revision,
        };
    }

    for _ in 0..transaction.delete_before_cursor {
        candidate.pop();
    }

    if failure == Some(FailurePoint::Insert) {
        return Outcome {
            applied: false,
            actual: original.visible,
            visible_revision: original.visible_revision,
        };
    }
    candidate.push_str(&transaction.insert);

    if failure == Some(FailurePoint::Flush) || failure == Some(FailurePoint::Acknowledgement) {
        return Outcome {
            applied: false,
            actual: original.visible,
            visible_revision: original.visible_revision,
        };
    }

    if mode == DeliveryMode::Clipboard {
        let saved = state.clipboard.clone();
        state.clipboard = transaction.insert.clone();
        clock.advance(Duration::from_millis(1)).unwrap();
        if failure == Some(FailurePoint::ClipboardRestore) {
            state.clipboard = original.clipboard;
            return Outcome {
                applied: false,
                actual: original.visible,
                visible_revision: original.visible_revision,
            };
        }
        state.clipboard = saved;
    }

    state.visible = candidate;
    state.visible_revision += 1;
    Outcome {
        applied: true,
        actual: state.visible.clone(),
        visible_revision: state.visible_revision,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DeterministicFailureArtifact {
    schema_version: u32,
    seed: u64,
    minimized_trace: Vec<String>,
    delivery_mode: DeliveryMode,
    failure_point: FailurePoint,
    expected_result: String,
    actual_result: String,
    transaction_id: u64,
    expected_visible_revision: u64,
    actual_visible_revision: u64,
}

impl DeterministicFailureArtifact {
    fn stable_path(&self, root: &Path) -> PathBuf {
        root.join(format!(
            "seed-{seed:016x}-{mode:?}-{failure:?}.json",
            seed = self.seed,
            mode = self.delivery_mode,
            failure = self.failure_point
        ))
    }

    fn write(&self, root: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(root)?;
        let path = self.stable_path(root);
        let payload = serde_json::to_vec_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(&path, payload)?;
        Ok(path)
    }
}

fn modes() -> [DeliveryMode; 6] {
    [
        DeliveryMode::Grabbed,
        DeliveryMode::Forwarded,
        DeliveryMode::WaylandPreedit,
        DeliveryMode::IbusPreedit,
        DeliveryMode::X11,
        DeliveryMode::Clipboard,
    ]
}

fn initial_state() -> FrontendState {
    FrontendState {
        visible: "tieeng".to_string(),
        visible_revision: 7,
        shift_pressed: false,
        clipboard: "controlled-fixture".to_string(),
    }
}

fn transaction() -> Transaction {
    Transaction {
        id: 42,
        expected_revision: 7,
        delete_before_cursor: 6,
        insert: "tiếng".to_string(),
    }
}

#[test]
fn frontend_conformance_uses_fake_clock_and_has_a_bounded_deadline() {
    for mode in modes() {
        let mut state = initial_state();
        let mut clock = FakeClock::bounded(TEST_TIMEOUT);
        let outcome = apply_transaction(mode, &mut state, &transaction(), None, &mut clock);

        assert!(outcome.applied, "mode={mode:?}");
        assert_eq!(outcome.actual, "tiếng", "mode={mode:?}");
        assert_eq!(outcome.visible_revision, 8, "mode={mode:?}");
        assert!(!state.shift_pressed, "mode={mode:?} left Shift stuck");
        assert!(clock.now <= clock.deadline, "mode={mode:?}");
    }
}

#[test]
fn deterministic_failure_matrix_is_atomic_and_preserves_revision() {
    for mode in modes() {
        for failure in [
            FailurePoint::Delete,
            FailurePoint::Insert,
            FailurePoint::Flush,
            FailurePoint::Acknowledgement,
            FailurePoint::ClipboardRestore,
        ] {
            if failure == FailurePoint::ClipboardRestore && mode != DeliveryMode::Clipboard {
                continue;
            }

            let mut state = initial_state();
            let before = state.clone();
            let mut clock = FakeClock::bounded(TEST_TIMEOUT);
            let outcome =
                apply_transaction(mode, &mut state, &transaction(), Some(failure), &mut clock);

            assert!(!outcome.applied, "mode={mode:?} failure={failure:?}");
            assert_eq!(
                state.visible, before.visible,
                "partial delete/insert caused duplicate or dropped characters"
            );
            assert_eq!(
                state.visible_revision, before.visible_revision,
                "failed transaction exposed a stale visible revision"
            );
            assert_eq!(
                state.shift_pressed, before.shift_pressed,
                "failed transaction left a modifier stuck"
            );
            assert_eq!(
                state.clipboard, before.clipboard,
                "failed transaction did not restore the controlled clipboard fixture"
            );
        }
    }
}

#[test]
fn stale_revision_is_rejected_without_visible_mutation() {
    for mode in modes() {
        let mut state = initial_state();
        let before = state.clone();
        let mut stale = transaction();
        stale.expected_revision -= 1;
        let mut clock = FakeClock::bounded(TEST_TIMEOUT);

        let outcome = apply_transaction(mode, &mut state, &stale, None, &mut clock);

        assert!(!outcome.applied, "mode={mode:?}");
        assert_eq!(state, before, "mode={mode:?}");
    }
}

#[test]
fn success_has_no_duplicate_drop_or_stuck_modifier() {
    for mode in modes() {
        let mut state = initial_state();
        let mut clock = FakeClock::bounded(TEST_TIMEOUT);

        let outcome = apply_transaction(mode, &mut state, &transaction(), None, &mut clock);

        assert_eq!(
            outcome.actual.chars().collect::<Vec<_>>(),
            ['t', 'i', 'ế', 'n', 'g']
        );
        assert_eq!(state.visible, "tiếng");
        assert_eq!(state.visible_revision, 8);
        assert!(!state.shift_pressed);
    }
}

#[test]
fn deterministic_failure_artifact_contains_replay_metadata_and_stable_path() {
    let directory = tempfile::tempdir().expect("artifact directory");
    let artifact = DeterministicFailureArtifact {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        seed: 0x4154_5858,
        minimized_trace: vec!["KEY_A:press".to_string(), "KEY_A:release".to_string()],
        delivery_mode: DeliveryMode::Clipboard,
        failure_point: FailurePoint::ClipboardRestore,
        expected_result: "tiếng".to_string(),
        actual_result: "tieeng".to_string(),
        transaction_id: 42,
        expected_visible_revision: 7,
        actual_visible_revision: 7,
    };

    let path = artifact
        .write(directory.path())
        .expect("deterministic artifact must persist");
    let payload = std::fs::read_to_string(&path).expect("artifact must be readable");
    let decoded: DeterministicFailureArtifact =
        serde_json::from_str(&payload).expect("artifact must be valid JSON");

    assert_eq!(decoded, artifact);
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "seed-0000000041545858-Clipboard-ClipboardRestore.json"
    );
    assert!(!payload.contains("/home/"));
    assert!(!payload.contains("password"));
}

#[test]
fn fake_clock_rejects_work_past_the_configured_timeout() {
    let mut clock = FakeClock::bounded(Duration::from_millis(2));
    assert_eq!(
        clock.advance(Duration::from_millis(3)),
        Err("bounded deterministic timeout exceeded")
    );
}
