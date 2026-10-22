# Phase 6 Runtime Validation Report

## Verdict

Phase 6 và Task 12 vẫn **BLOCKED/PARTIAL**. Controlled matrix hiện có PASS cho Chromium, Chrome, Electron, GTK, Qt, terminal và direct-uinput protocol. Hai stress test mới đã được triển khai cùng deterministic contracts, nhưng privileged focus-switch và fast-input đều không PASS. Firefox và COSMIC Edit vẫn FAILED nên không thể đánh dấu Task 12 completed.

## Environment

- OS: Pop!_OS 24.04 LTS
- Session: Wayland / COSMIC
- `DISPLAY=:1`
- `WAYLAND_DISPLAY=wayland-1`
- `/dev/uinput`: readable và writable
- Tools: `wtype`, `ydotool`, `xdotool`, `wl-copy`, `wl-paste`, Chromium, Firefox, COSMIC Edit
- `vietc.service`: active sau khi rebuild release binary, MainPID `1701715`
- Runtime socket: `$XDG_RUNTIME_DIR/vietc/uinput.sock` READY

## Deterministic validation

Command:

```bash
cargo test -p vietc-daemon --test daemon_suite deterministic_ -- --nocapture
```

Result: exit 0, 43 passed, 0 failed, 0 ignored, 38 filtered out.

Added deterministic contracts verify:
- focus-switch returns to the initial target;
- every target has exact expected/observed output;
- cross-target text and stale composition are rejected;
- fast-input rate is bounded to 1-20 ms per key;
- exact output is required for representative Telex, boundary, backspace, enable/disable, and macro-isolation cases.

Evidence:
- `target/runtime-artifacts/phase6-deterministic-after-stress-implementation.log`
- `target/runtime-artifacts/phase6-stress-deterministic-targeted-after-tool-fix.log`

## Controlled focus-switch stress

Command:

```bash
timeout 120s env VIETC_RUNTIME_TESTS=1 VIETC_CONTROLLED_DISPLAY=1 cargo test -p vietc-daemon --test daemon_suite privileged_focus_switch_stress_is_observable_end_to_end -- --ignored --nocapture --test-threads=1
```

Result: **FAILED**, exit 101, 0 passed, 1 failed, 80 filtered out.

The single allowed run stopped before target interaction because `RuntimeEnvironment` did not yet include `ydotool`. The harness tool list was corrected afterward and deterministic tests pass, but the privileged suite was not replayed on the same environment. No PASS is claimed.

Evidence:
- `target/runtime-artifacts/phase6-focus-switch-stress.log`

## Controlled fast-input stress

Command:

```bash
timeout 120s env VIETC_RUNTIME_TESTS=1 VIETC_CONTROLLED_DISPLAY=1 cargo test -p vietc-daemon --test daemon_suite privileged_fast_input_stress_is_observable_end_to_end -- --ignored --nocapture --test-threads=1
```

Result: **FAILED**, exit 101, 0 passed, 1 failed, 80 filtered out.

Bounded rate: 8 ms/key. The first Telex case produced exact observed output:

```text
expected="được"
observed="dduojwc"
```

Injection success was therefore not treated as PASS. The suite timed out waiting for the required transformed value. Later boundary, backspace, enable/disable, and macro-isolation cases were not reached, so they remain unverified by privileged runtime evidence.

Evidence:
- `target/runtime-artifacts/phase6-fast-input-stress.log`
- `target/runtime-fixtures/fast-input/artifacts/free-order-tone-mark-timeout.txt`

## Existing blockers

- Firefox controlled target: FAILED with `TargetCopyFailed`.
- COSMIC Edit controlled target: FAILED because it daemonized outside the owned process group.
- Focus-switch privileged evidence: FAILED.
- Fast-input privileged evidence: FAILED with raw text observed instead of Telex output.
- Fedora COSMIC requires a real Fedora host.

## Process cleanup

Controlled Chromium children are owned by `OwnedRuntimeChild` and teardown ran after failure. Final inspection found no surviving fixture process owned by the stress tests. No unrelated process was killed.

Evidence:
- `target/runtime-artifacts/phase6-stress-final-state.txt`
- `target/runtime-artifacts/phase6-stress-service-ready-after-build.txt`

## Terminal condition

Stop this checkpoint while blockers depend on Firefox, COSMIC Edit, or the external desktop/input environment. Do not automatically retry privileged stress suites on the same environment. Resume only after an environment or blocker change can produce new evidence.

## Task 12 status

**BLOCKED/PARTIAL**, not completed.

## Casing and clipboard root-cause verification

### Casing

Root cause nằm ở production grabbed-evdev path:

- `daemon/src/evdev_loop.rs`: trạng thái Caps Lock ban đầu được đọc từ LED nhưng không được cập nhật theo sự kiện `KEY_CAPSLOCK`, nên toggle trong lúc daemon chạy vẫn dùng trạng thái cũ.
- Raw key từng đi qua `key_to_char`, làm mất semantics `Shift XOR Caps Lock` trước khi vào Engine.
- Fix dùng `update_keyboard_state` để theo dõi Caps Lock trên key press và `key_to_char_with_case` để dịch raw key trước recomposition. Không special-case từ.

Regression production-path:
- `TIEENGS -> TIẾNG`
- `DDUOWNGF -> ĐƯỜNG`
- `DDUOJWC -> ĐƯỢC`
- `NEESU -> NẾU`
- câu nhiều từ viết hoa
- Caps Lock + Shift cho chữ thường
- toggle Caps Lock giữa hai từ không tạo mixed-case

Fail-first evidence:
- targeted daemon test trước fix: exit 101; hai test Caps Lock/toggle fail.
- sau fix: 3 passed, exit 0.

### Clipboard

Có hai production injection backend:

1. Direct `protocol/src/uinput_monitor.rs`, backend daemon đang dùng thực tế (`Using uinput injection`).
2. Socket `uinputd/src/main.rs`, hiện service `vietc-uinputd.service` không chạy.

Root cause chính ở direct backend: `paste_via_clipboard` trả success ngay sau Ctrl+V rồi schedule restore nền sau debounce. Vì vậy command kế tiếp có thể đọc clipboard injection trước làm snapshot, và daemon có thể coi transaction hoàn tất trước restore/read-back. Socket backend cũng trả `OK` mà không xác minh clipboard đã restore đúng.

Fix:
- Mỗi Unicode command restore clipboard đồng bộ trước khi trả success.
- Restore chỉ xảy ra nếu clipboard vẫn chứa payload injection của transaction.
- Sau restore phải đọc lại và khớp snapshot người dùng; nếu ghi hoặc verify fail, command trả failure.
- User đổi clipboard trong transaction thì skip restore.
- State được reset trên failure, timeout và shutdown.
- TYPE/REPLACE/PASTE socket chỉ trả `OK` khi Unicode injection và verified restore thành công.
- Không dùng sleep dài để che race.

Regression:
- nhiều Unicode replacement liên tiếp giữ nguyên clipboard người dùng;
- mỗi command kết thúc transaction trước khi return;
- user clipboard change không bị ghi đè;
- overlapping replacement không thay snapshot gốc;
- failure/timeout cleanup state;
- socket acknowledgement yêu cầu verified restore.

Fail-first evidence:
- `unicode_command_ack_requires_verified_clipboard_restore`: exit 101 trước fix.
- sau fix: uinputd phase0 suite 13 passed, protocol suite 59 passed.

### Required validation

- `git diff --check`: exit 0
- `cargo test -p vietc-engine`: exit 0
- `cargo test -p vietc-engine --features bamboo-telex-rollback`: exit 0
- `cargo test -p vietc-uinputd`: exit 0
- `cargo test -p vietc-daemon`: exit 0
- `cargo test --workspace`: exit 0
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0

### Production runtime evidence

Release binary cuối đã build và `vietc.service` đã stop/start:

- MainPID: `1983575`
- Binary: `/home/thanhhd/Git-projects/vietc/target/release/vietc`
- SHA-256: `b8ab59030e8cbf4735c7ffe14c86d68b923643e46e3d7ba35a6d4a5ea62a2817`
- Hash `/proc/1983575/exe` khớp `target/release/vietc`.
- Service: active/running.
- Journal xác nhận `Using uinput injection`, keyboard đã grab và event loop đã chạy.
- FD 4 của daemon trỏ tới `/dev/uinput`; virtual device `vietc` tồn tại tại `event19`.
- Socket `$XDG_RUNTIME_DIR/vietc/uinput.sock` trước đó là stale/no-listener; production daemon không dùng socket backend.

Bounded production E2E được chạy đúng một lần qua `ydotool -> evdev -> daemon -> direct-uinput -> Chromium`. Kết quả exit 101, `TimedOut`. Trace đã định vị lỗi clipboard còn lại: `wl-copy` công bố selection bất đồng bộ, trong khi code restore chỉ read-back đúng một lần ngay sau spawn, dẫn tới `Unicode insertion failed: clipboard unavailable`, transaction đầu fail và các replacement sau stale.

Fix bổ sung dùng bounded polling tối đa 250 ms cho cả clipboard injection readiness và restore read-back. Không dùng sleep dài, không special-case từ. Sau rebuild/restart, journal ghi transaction Unicode đầu `applied`, `visible_revision=1`.

Automated observable production path chưa PASS vì không phát lại privileged suite sau bounded run duy nhất. Do đó vẫn cần user test lại hai kịch bản.

Evidence:
- `target/runtime-artifacts/casing-clipboard-production-fast-input.log`
- `target/runtime-artifacts/wayland-clipboard-owner-lifetime.log`
- `target/runtime-artifacts/casing-clipboard-final-validation.log`

### Hai kịch bản user test còn cần

1. Bật Caps Lock, gõ `tieengs dduowngf dduojwc neesu`; giữ Shift và gõ lại một từ; tắt Caps Lock rồi gõ từ kế tiếp. Kỳ vọng: `TIẾNG ĐƯỜNG ĐƯỢC NẾU`, Caps+Shift ra chữ thường, không mixed-case.
2. Copy chuỗi bất kỳ, gõ `tieengs dduowngf dduojwc neesu`, rồi Ctrl+V. Kỳ vọng paste nguyên chuỗi đã copy, không phải `đ` hay replacement trung gian.

--- End ---
