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

- [ ] Chot semantic contract duy nhat cho `EngineEvent::Replace`.
- [ ] Chot don vi delete va protocol mapping.
- [ ] Them expected visible revision.
- [ ] Them transform generation/reason.
- [ ] Dinh nghia transaction success/failure.
- [ ] Dinh nghia stale-event rejection.
- [ ] Bo frontend inference `+1 backspace` trong non-grabbed evdev.
- [ ] Bo frontend diff fallback tu buffer neu engine da phat semantic event.
- [ ] Dong nhat grabbed evdev.
- [ ] Dong nhat non-grabbed evdev.
- [ ] Dong nhat Wayland IM.
- [ ] Dong nhat IBus.
- [ ] Dong nhat X11.
- [ ] Dong nhat uinput.
- [ ] Dong nhat clipboard fallback.
- [ ] Chung minh frontend khong tu suy luan semantics.
- [ ] Chung minh partial delete/insert failure khong bao success.
- [ ] Them injection feedback transaction guard.

## Phase 4: Model/property tests

- [ ] Viet reference model doc lap voi production reducer.
- [ ] Sinh trace cho tone `f/s/r/x/j`.
- [ ] Sinh trace cho mark `w/aa/ee/oo`.
- [ ] Sinh trace cho `dd`.
- [ ] Sinh lower/upper/mixed-case traces.
- [ ] Sinh single/multi-vowel traces.
- [ ] Sinh non-adjacent trigger traces.
- [ ] Sinh transform replacement traces.
- [ ] Sinh valid/invalid spelling traces.
- [ ] Sinh auto-restore bat/tat traces.
- [ ] Sinh Backspace/Delete/cursor/boundary traces.
- [ ] Sinh paste/focus/external-edit traces.
- [ ] Sinh frontend delivery-mode traces.
- [ ] Sinh delete/insert/acknowledgement failure traces.
- [ ] Chung minh determinism.
- [ ] Chung minh replay equivalence.
- [ ] Chung minh undo policy invariants.
- [ ] Chung minh no stale undo.
- [ ] Chung minh case preservation.
- [ ] Chung minh frontend conformance.
- [ ] Chung minh transaction safety.
- [ ] Luu failing seed.
- [ ] Shrink failing trace thanh minimized sequence.
- [ ] Them regression corpus, gom `ATXX -> ATX`.

## Phase 5: Chromium/Chrome/Electron crash investigation

- [ ] Tao evidence collector co timeout.
- [ ] Ghi daemon stdout/stderr.
- [ ] Ghi browser/app stdout/stderr.
- [ ] Thu `journalctl` theo reproduction window.
- [ ] Thu `coredumpctl list`.
- [ ] Thu `coredumpctl info`.
- [ ] Tao debug workflow khi core co san.
- [ ] Ghi exact browser/app version va command line.
- [ ] Ghi distro, kernel, session va compositor version.
- [ ] Ghi backend injection dang active.
- [ ] Ghi config auto-restore/dedup/input method.
- [ ] Ghi process tree va PID/process type.
- [ ] Phan biet daemon crash.
- [ ] Phan biet renderer crash.
- [ ] Phan biet browser process crash.
- [ ] Phan biet protocol-triggered app exit.
- [ ] Minimize physical input sequence.
- [ ] Kiem tra `wtype` Shift-prefix.
- [ ] Kiem tra physical Shift dang duoc giu.
- [ ] Kiem tra forced modifier release.
- [ ] Kiem tra Ctrl+V clipboard fallback.
- [ ] Kiem tra malformed/too-fast key sequence.
- [ ] Kiem tra clipboard ownership va restore revision.
- [ ] Kiem tra X11 SelectionRequest target/property/timeout.
- [ ] Kiem tra injection feedback.
- [ ] Kiem tra concurrent writes.
- [ ] Kiem tra child/thread/process lifecycle.
- [ ] Kiem tra Wayland/X11 protocol misuse.
- [ ] Chay control experiment cho moi nghi pham.
- [ ] Khong ket luan root cause truoc khi co crash evidence va reproducible isolation.

## Phase 6: Deterministic CI harness

- [ ] Chay model tests khong can display server.
- [ ] Chay frontend conformance suite voi fake clock.
- [ ] Them bounded timeout cho moi async/process test.
- [ ] Inject failure tai delete.
- [ ] Inject failure tai insert.
- [ ] Inject failure tai flush.
- [ ] Inject failure tai acknowledgement.
- [ ] Inject failure tai clipboard restore.
- [ ] Kiem tra no duplicate/drop.
- [ ] Kiem tra no stuck modifier.
- [ ] Kiem tra no stale revision.
- [ ] Tach smoke seed set va exhaustive budget.
- [ ] Luu deterministic artifact khi fail.

## Phase 7: Real runtime harness

- [ ] Khoi dong Chromium that voi profile tam.
- [ ] Khoi dong Google Chrome that voi profile tam.
- [ ] Khoi dong Electron that voi profile tam.
- [ ] Gan PID voi browser/renderer process type.
- [ ] Kiem tra observable text field.
- [ ] Kiem tra process health sau moi sequence.
- [ ] Kiem tra clipboard sau moi sequence.
- [ ] Kiem tra modifier state sau moi sequence.
- [ ] Kill va reap tat ca child khi timeout/teardown.
- [ ] Luu versions, flags, backend, sequence, timing va logs.
- [ ] Runtime COSMIC Wayland pass.
- [ ] Runtime GNOME Wayland pass.
- [ ] Runtime KDE Wayland pass.
- [ ] Runtime X11 pass.
- [ ] Runtime Chromium pass.
- [ ] Runtime Chrome pass.
- [ ] Runtime Electron pass.
- [ ] Runtime GTK pass.
- [ ] Runtime Qt pass.
- [ ] Runtime terminal pass.
- [ ] Missing environment duoc danh dau SKIP co reason va owner, khong PASS.

## Phase 8: Verification va release gate

- [ ] `cargo fmt --all -- --check` pass.
- [ ] `cargo test --workspace --all-targets` pass.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass.
- [ ] Engine model/property tests pass.
- [ ] Tat ca frontend conformance tests pass.
- [ ] Real-browser runtime harness pass.
- [ ] Runtime matrix co day du evidence bat buoc.
- [ ] Khong duplicate ky tu.
- [ ] Khong drop ky tu.
- [ ] Khong stuck modifier.
- [ ] Undo deterministic.
- [ ] Khong mat input.
- [ ] Khong mat clipboard.
- [ ] Khong ghi de clipboard moi cua nguoi dung.
- [ ] Khong silent injection success.
- [ ] Khong daemon crash.
- [ ] Khong renderer crash.
- [ ] Khong browser process crash.
- [ ] Khong protocol-triggered app exit.
- [ ] Khong con HIGH finding.
- [ ] Khong con CRITICAL finding.
- [ ] Release verdict PASS; bat ky muc thieu hoac fail thi verdict FAIL.

## Quyet dinh can chot truoc implementation

- [ ] Xac nhan `dd` ConsumeTrigger va mixed-case behavior.
- [ ] Chon mo rong `EngineEvent::Replace` hay event envelope versioned.
- [ ] Chot delete unit va UTF-8/UTF-16/protocol conversion.
- [ ] Chot delivery acknowledgement cho non-grabbed evdev.
- [ ] Chot flush hay synchronize khi cursor/Delete khong du capability.
- [ ] Chot CI property-test budget.
- [ ] Chot owner runner/VM cho tung runtime matrix cell.
- [ ] Chot redaction va retention policy cho crash evidence.