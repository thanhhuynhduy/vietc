# Wayland clipboard replacement: transfer evidence, not editor acknowledgment

## Incident and scope

Reported on Pop!_OS 24.04/COSMIC Wayland while typing a Vietnamese prompt in Antigravity after copying a video filename ending `.mp4`. The filename appeared inside the prompt and its tail became `.mtự`.

Read-only diagnostics on the development desktop showed a running user service with evdev keyboard grab and uinput injection, despite the initial assumption that IBus was active. This does not prove every earlier incident used the same build/backend.

The old uinput clipboard path restored the saved clipboard after its own reads matched the injected text for 20 ms. That condition is true even if the editor has never requested the data. A deterministic probe reproduced the schedule: restore the filename, let a late reader paste it, then delete `p4` and insert `tự`. This explains a possible mechanism, not the exact observed editor event history or duplicated prompt fragments.

## Implementation

With the default `wayland-protocol` feature, Unicode clipboard injection on Wayland now uses an owned native data-control source:

1. Snapshot clipboard data before modifying it. Snapshot failure sends no replacement keys.
2. Discover exactly one seat; prefer `ext_data_control_manager_v1`, falling back to `zwlr_data_control_manager_v1`.
3. Publish immutable UTF-8 bytes and wait for a compositor sync barrier. No `wl-paste` of the injected payload is used as readiness or completion evidence.
4. Arm transfer accounting with another ordering barrier. Requests observed before arming cannot acknowledge this paste.
5. Check focus generation/ownership, dispatch Backspace and Ctrl+V once, then wait for a complete eligible text-payload write and FD close.
6. Restore the saved clipboard only after that transfer evidence. Restoration errors are cleanup errors, not permission to repeat the document edit. Keep the snapshot/source if cleanup is unconfirmed.

The legacy X11 stability heuristic is unchanged and is not claimed fixed by this patch.

## Failure and resource policy

- After uncertain dispatch, do not restore the old filename, repeat Ctrl+V, or issue compensating deletions. Retain the immutable source when possible so a late reader can still receive its payload.
- Keep edit uncertainty separate from source ownership. Worker disconnect/error does not prove the editor did nothing.
- While uncertain, the evdev capture path clears composition and forwards native keys instead of continuing automatic replacements. Dedup must not consume those keys, and consumed-trigger releases must remain balanced. Changing to a new tracked focus generation explicitly resets this composition latch; it does not replay the uncertain edit.
- Unknown clipboard ownership is not an external-copy acknowledgment. Keep the saved snapshot rather than blindly restoring over a possible new owner. Transport failure can require a daemon restart/manual recovery; automatic clipboard recovery is not guaranteed.
- Retire old sources without joining their drains on the keyboard-capture thread. Accepted transfers retain their immutable bytes until completion or their five-second transfer deadline. The injector retains at most eight retired handles, reaping finished workers; backlog suspends composition rather than spawning unbounded workers.
- Payload limit: 1 MiB. At most 16 outstanding transfer FDs per source. Socket/control/transfer work is bounded; malformed or stalled readers do not produce success evidence.
- Native restoration waits for the copy launcher's successful exit and verifies the restored data, including raw bytes for the backed-up non-text MIME. A spawned process alone is not restoration success.
- The existing snapshot format preserves plain text or one non-text representation, not every rich-text/file-list MIME simultaneously. Full multi-format clipboard preservation is not added by this change.

## Important limits — do not overstate this fix

**PayloadServed is not TargetInserted.** Generic data-control requests expose MIME and an FD, not the requesting application's identity. A clipboard manager requesting after arming is indistinguishable from Antigravity. Consequently:

- This removes the false claim that a stable clipboard means the editor consumed it, and protects the delayed-request/no-request schedules covered by the tests.
- A manager request after arming can still satisfy transfer accounting before the editor requests the current selection. Automatic restoration remains best-effort in that case.
- Source cancellation can identify replacement; the protocol does not provide an atomic compare-and-set clipboard restore. External-copy races remain possible.
- An editor may obtain bytes before applying its document mutation; its asynchronous edits may still overlap later keystrokes.
- A strict guarantee without borrowing the clipboard requires an input-method/application integration with an appropriate commit contract. This patch must not be described as proving all Antigravity races fixed.

The native source deliberately refuses UID 0 and inherited `WAYLAND_SOCKET`. This candidate targets the **desktop user's service**, including the inspected UID 1000 COSMIC installation. Root-launched Wayland Unicode injection now fails before replacement key dispatch instead of connecting a privileged clipboard owner or silently using the old heuristic. A privilege-separated native helper is not implemented here. No device permissions, installed binaries, user config or service state are changed by editing/building this repository.

## Validation and rollout

Headless commands (from repository root):

```sh
env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test -p vietc-protocol --lib --locked --offline

env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test -p vietc-daemon --bin vietc --locked --offline \
  evdev_loop::grabbed_event_stream_tests::uncertain_ -- --nocapture
```

Tests use real local pipes, subprocess fixtures and a fake Wayland compositor over a socket pair; they do not connect to the user's compositor, read their clipboard, create input devices or inject keys. The old stability assertion was first run red to demonstrate the defect; its retained characterization test explicitly documents why the heuristic is not transport evidence. New-path regression tests cover delayed requests, no request, pre-arm requests, source cancellation, actual complete bytes/EOF, stalled/short/broken writes, transport loss, retirement, cleanup failure and balanced native forwarding.

Deployment helper (run as the desktop user; it asks for sudo only to replace `/usr/bin/vietc-daemon` and re-apply capabilities, and asks for confirmation before restarting the service):

```sh
scripts/candidate-deploy.sh status     # installed vs candidate checksum, latest backup
scripts/candidate-deploy.sh install    # backs up binary/caps/config if needed, installs, restarts, prints startup log
scripts/candidate-deploy.sh rollback   # verifies backup checksum, restores binary + caps, restarts
```

Backups live in `~/.local/share/vietc-rollback/<timestamp>/`. The config file is copied but never modified by the script.

Before claiming a desktop fix:

1. Complete headless tests, build and code review. Preserve the previous installed binary and config for rollback.
2. Arrange a user-approved maintenance window or disposable COSMIC session. Do not automatically stop a daemon grabbing the user's working keyboard.
3. Install/start the candidate deliberately as the desktop user. Verify actual evdev/uinput selection, native data-control availability and that no second Vietnamese IME composes concurrently.
4. In a disposable prompt containing synthetic prefix/suffix, copy a synthetic `.mp4` filename, switch to Antigravity, and type the reported Vietnamese sequence. Exercise slow/busy editor reads, rapid typing, clipboard manager on/off, Ctrl+C during typing and ownership changes.
5. Observe exact prompt text/caret plus clipboard restoration and raw-key balance. Never infer editor correctness from the `payload served` log alone. If failure persists, record a controlled replay and classify whether manager requests or asynchronous document edits remain responsible.
6. Report build identity, backend, clipboard-manager state, attempts/failures and missing cases. A passing headless suite is not desktop evidence.
