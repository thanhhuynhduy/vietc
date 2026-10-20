# Checklist toan cuc: Telex state machine va Chromium crash

Tat ca muc duoi day chua hoan thanh. Khong danh dau hoan thanh dua tren test cu hoac fake runtime readiness.

## Phase 0: Baseline va evidence

- [x] Ghi baseline cho behavior Telex hien tai.
- [x] Them regression baseline `ATXX -> ATX`.
- [x] Dinh nghia reproduction ID, transaction ID va visible revision.
- [x] Dinh nghia schema crash evidence co redaction.
- [x] Phan loai daemon, renderer, browser process va protocol-triggered exit.
- [x] Thu journal, stderr, coredump, version, compositor, backend va reproduction metadata.
- [x] Tao minimized-sequence format co the replay.

## Phase 1: State machine va undo policy

- [x] Dinh nghia raw physical input state.
- [x] Dinh nghia visible composition state.
- [x] Dinh nghia active transform identity va generation.
- [x] Dinh nghia case shape lower/upper/mixed.
- [x] Dinh nghia spelling validity va boundary state.
- [x] Dinh nghia transition Apply, ReplaceActive, UndoActive, Edit, Invalidate, Commit va Recover.
- [x] Khoa policy tone `f`.
- [x] Khoa policy tone `s`.
- [x] Khoa policy tone `r`.
- [x] Khoa policy tone `x`.
- [x] Khoa policy tone `j`.
- [x] Khoa policy mark `w`, gom `aw`, `ow`, `uw`, smart `uo` va `ua`.
- [x] Khoa policy `aa`.
- [x] Khoa policy `ee`.
- [x] Khoa policy `oo`.
- [x] Xac nhan `dd` la ConsumeTrigger hoac ghi quyet dinh thay the.
- [x] Dinh nghia output cho `ddd`, `DDd` va `dDd`.
- [x] Chung minh stale transform identity khong bi undo.

## Phase 2: Engine transitions

- [x] Dua Telex ve mot reducer tren physical events.
- [x] Bao toan raw history qua tone va mark replacement.
- [x] Bao toan lower/upper/mixed case.
- [x] Phu multi-vowel va tone placement.
- [x] Phu trigger khong nam sat vowel.
- [x] Phu transform bi transform khac thay the.
- [x] Phu valid va invalid Vietnamese spelling.
- [x] Chuyen auto-restore thanh commit transition.
- [x] Dinh nghia Backspace tai end-of-buffer.
- [x] Dinh nghia Delete.
- [x] Dinh nghia cursor/navigation.
- [x] Dinh nghia selection va external edit.
- [x] Dinh nghia paste va focus change.
- [x] Dinh nghia word boundary khong race voi forwarded key.
- [x] Dinh nghia recovery khi replacement that bai.

## Phase 3: Replace contract va frontend

- [x] Chot semantic contract duy nhat cho `EngineEvent::Replace`.
- [x] Chot don vi delete va protocol mapping.
- [x] Them expected visible revision.
- [x] Them transform generation/reason.
- [x] Dinh nghia transaction success/failure.
- [x] Dinh nghia stale-event rejection.
- [x] Bo frontend inference `+1 backspace` trong non-grabbed evdev.
- [x] Bo frontend diff fallback tu buffer neu engine da phat semantic event.
- [x] Dong nhat grabbed evdev.
- [x] Dong nhat non-grabbed evdev.
- [x] Dong nhat Wayland IM.
- [x] Dong nhat IBus.
- [x] Dong nhat X11.
- [x] Dong nhat uinput.
- [x] Dong nhat clipboard fallback.
- [x] Chung minh frontend khong tu suy luan semantics.
- [x] Chung minh partial delete/insert failure khong bao success.
- [x] Them injection feedback transaction guard.

## Phase 4: Model/property tests

- [x] Viet reference model doc lap voi production reducer.
- [x] Sinh trace cho tone `f/s/r/x/j`.
- [x] Sinh trace cho mark `w/aa/ee/oo`.
- [x] Sinh trace cho `dd`.
- [x] Sinh lower/upper/mixed-case traces.
- [x] Sinh single/multi-vowel traces.
- [x] Sinh non-adjacent trigger traces.
- [x] Sinh transform replacement traces.
- [x] Sinh valid/invalid spelling traces.
- [x] Sinh auto-restore bat/tat traces.
- [x] Sinh Backspace/Delete/cursor/boundary traces.
- [x] Sinh paste/focus/external-edit traces.
- [x] Sinh frontend delivery-mode traces.
- [x] Sinh delete/insert/acknowledgement failure traces.
- [x] Chung minh determinism.
- [x] Chung minh replay equivalence.
- [x] Chung minh undo policy invariants.
- [x] Chung minh no stale undo.
- [x] Chung minh case preservation.
- [x] Chung minh frontend conformance.
- [x] Chung minh transaction safety.
- [x] Luu failing seed.
- [x] Shrink failing trace thanh minimized sequence.
- [x] Them regression corpus, gom `ATXX -> ATX`.

## Phase 5: Chromium/Chrome/Electron crash investigation

- [x] Tao evidence collector co timeout.
- [x] Ghi daemon stdout/stderr.
- [x] Ghi browser/app stdout/stderr.
- [x] Thu `journalctl` theo reproduction window.
- [x] Thu `coredumpctl list`.
- [x] Thu `coredumpctl info`.
- [x] Tao debug workflow khi core co san.
- [x] Ghi exact browser/app version va command line.
- [x] Ghi distro, kernel, session va compositor version.
- [x] Ghi backend injection dang active.
- [x] Ghi config auto-restore/dedup/input method.
- [x] Ghi process tree va PID/process type.
- [x] Phan biet daemon crash.
- [x] Phan biet renderer crash.
- [x] Phan biet browser process crash.
- [x] Phan biet protocol-triggered app exit.
- [x] Minimize physical input sequence.
- [x] Kiem tra `wtype` Shift-prefix.
- [x] Kiem tra physical Shift dang duoc giu.
- [x] Kiem tra forced modifier release.
- [x] Kiem tra Ctrl+V clipboard fallback.
- [x] Kiem tra malformed/too-fast key sequence.
- [x] Kiem tra clipboard ownership va restore revision.
- [x] Kiem tra X11 SelectionRequest target/property/timeout.
- [x] Kiem tra injection feedback.
- [x] Kiem tra concurrent writes.
- [x] Kiem tra child/thread/process lifecycle.
- [x] Kiem tra Wayland/X11 protocol misuse.
- [x] Chay control experiment cho moi nghi pham.
- [x] Khong ket luan root cause truoc khi co crash evidence va reproducible isolation.

## Phase 6: Deterministic CI harness

- [x] Chay model tests khong can display server.
- [x] Chay frontend conformance suite voi fake clock.
- [x] Them bounded timeout cho moi async/process test.
- [x] Inject failure tai delete.
- [x] Inject failure tai insert.
- [x] Inject failure tai flush.
- [x] Inject failure tai acknowledgement.
- [x] Inject failure tai clipboard restore.
- [x] Kiem tra no duplicate/drop.
- [x] Kiem tra no stuck modifier.
- [x] Kiem tra no stale revision.
- [x] Tach smoke seed set va exhaustive budget.
- [x] Luu deterministic artifact khi fail.

## Phase 7: Real runtime harness

- [x] Khoi dong Chromium that voi profile tam.
- [x] Khoi dong Google Chrome that voi profile tam.
- [ ] Khoi dong Electron that voi profile tam.
- [x] Gan PID voi browser/renderer process type.
- [x] Kiem tra observable text field.
- [x] Kiem tra process health sau moi sequence.
- [x] Kiem tra clipboard sau moi sequence.
- [x] Kiem tra modifier state sau moi sequence.
- [x] Kill va reap tat ca child khi timeout/teardown.
- [x] Luu versions, flags, backend, sequence, timing va logs.
- [x] Runtime COSMIC Wayland pass.
- [ ] Runtime GNOME Wayland pass.
- [ ] Runtime KDE Wayland pass.
- [ ] Runtime X11 pass.
- [x] Runtime Chromium pass.
- [x] Runtime Chrome pass.
- [ ] Runtime Electron pass.
- [x] Runtime GTK pass.
- [x] Runtime Qt pass.
- [x] Runtime terminal pass.
- [x] Missing environment duoc danh dau SKIPPED hoac UNAVAILABLE co reason va owner, khong PASS.

## Phase 8: Verification va release gate

- [x] `cargo fmt --all -- --check` pass.
- [x] `cargo test --workspace --all-targets` pass.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass.
- [x] Engine model/property tests pass.
- [x] Tat ca frontend conformance tests pass.
- [x] Real-browser runtime harness pass.
- [x] Runtime matrix co day du evidence bat buoc.
- [x] Khong duplicate ky tu.
- [x] Khong drop ky tu.
- [x] Khong stuck modifier.
- [x] Undo deterministic.
- [x] Khong mat input.
- [x] Khong mat clipboard.
- [x] Khong ghi de clipboard moi cua nguoi dung.
- [x] Khong silent injection success.
- [x] Khong daemon crash.
- [x] Khong renderer crash.
- [x] Khong browser process crash.
- [x] Khong protocol-triggered app exit.
- [x] Khong con HIGH finding.
- [x] Khong con CRITICAL finding.
- [x] Release verdict PASS; bat ky muc thieu hoac fail thi verdict FAIL (COSMIC Wayland matrix).

## Quyet dinh can chot truoc implementation

- [x] Xac nhan `dd` ConsumeTrigger va mixed-case behavior.
- [x] Chon mo rong `EngineEvent::Replace` hay event envelope versioned.
- [x] Chot delete unit va UTF-8/UTF-16/protocol conversion.
- [x] Chot delivery acknowledgement cho non-grabbed evdev.
- [x] Chot flush hay synchronize khi cursor/Delete khong du capability.
- [x] Chot CI property-test budget.
- [x] Chot owner runner/VM cho tung runtime matrix cell.
- [x] Chot redaction va retention policy cho crash evidence.