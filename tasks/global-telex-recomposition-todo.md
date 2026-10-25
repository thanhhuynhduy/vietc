# Task Breakdown: Global Telex Raw-Keystroke Recomposition

## Phase 1: Đóng băng hợp đồng hiện tại

### Task 1: Xây behavior matrix

**Description:** Liệt kê các input/output hiện đang hỗ trợ, free-order mong muốn, English fallback và chuỗi không hợp lệ.

**Acceptance criteria:**
- [x] Có bảng mark, tone, nucleus, coda, casing và boundary.
- [x] Mỗi behavior có expected output hoặc expected literal fallback.
- [x] `dduojwc`, `dduocwj`, `neues`, `neesu` nằm trong matrix.

**Verification:**
- [x] Review matrix trước khi viết parser.
- [x] Không có expected behavior mơ hồ.

**Dependencies:** None

**Files likely touched:**
- `docs/testing-dictionary.md`
- `engine/src/tests.rs`

**Estimated scope:** S

### Task 2: Tạo golden regression suite

**Description:** Chuyển behavior matrix thành data-driven tests chạy trên engine hiện tại.

**Acceptance criteria:**
- [x] Golden tests bao phủ các quy tắc Telex hiện có.
- [x] English words và boundary cases có regression tests.
- [x] Test báo rõ input nào lệch output.

**Verification:**
- [x] `cargo test -p vietc-engine`

**Dependencies:** Task 1

**Files likely touched:**
- `engine/src/tests.rs`
- `engine/src/test_cases.rs`

**Estimated scope:** M

## Checkpoint 1

- [x] Golden suite pass trên baseline.
- [x] Các thay đổi behavior mong muốn được allowlist rõ ràng.
- [x] Không bắt đầu migration nếu baseline chưa ổn định.

## Phase 2: Xây domain model và parser thuần

### Task 3: Định nghĩa raw và parsed syllable types

**Description:** Tạo các type biểu diễn raw keys, mark, tone và cấu trúc âm tiết mà không render Unicode sớm.

**Acceptance criteria:**
- [x] Tone và mark là hai trạng thái độc lập.
- [x] Type có invariant rõ ràng.
- [x] Không phụ thuộc IBus, Wayland, X11 hoặc uinput.

**Verification:**
- [x] Unit tests cho construction và boundary.
- [x] `cargo clippy -p vietc-engine --all-targets -- -D warnings`

**Dependencies:** Task 2

**Files likely touched:**
- `engine/src/telex/raw.rs`
- `engine/src/telex/model.rs`
- `engine/src/lib.rs`

**Estimated scope:** S

### Task 4: Parse raw keys thành onset, nucleus và coda

**Description:** Cài parser xác định ranh giới và cấu trúc âm tiết từ raw sequence.

**Acceptance criteria:**
- [x] Parse đúng onset/nucleus/coda của corpus.
- [x] Xử lý `qu` và `gi` như glide theo điều kiện.
- [x] Không parse qua whitespace hoặc punctuation boundary.

**Verification:**
- [x] Unit tests theo từng nhóm âm tiết.
- [x] Fuzz/property smoke test không panic.

**Dependencies:** Task 3

**Files likely touched:**
- `engine/src/telex/parser.rs`
- `engine/src/telex/model.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

## Phase 3: Resolver và renderer

### Task 5: Cài mark resolver

**Description:** Áp dụng tất cả mark rule trên parsed nucleus, không dùng backtrack toàn buffer.

**Acceptance criteria:**
- [x] Hỗ trợ `aa`, `aw`, `ee`, `oo`, `ow`, `uw`, `uo+w`, `ua+w`, `dd`.
- [x] Mark trigger hợp lệ không phụ thuộc thứ tự với tone.
- [x] Trigger không áp dụng được có deterministic fallback.

**Verification:**
- [x] Mark matrix tests pass.
- [x] `readme` và English corpus giữ nguyên.

**Dependencies:** Task 4

**Files likely touched:**
- `engine/src/telex/mark.rs`
- `engine/src/telex/parser.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

### Task 6: Cài tone resolver

**Description:** Chọn tone cuối cùng có hiệu lực và đặt nó trên nucleus sau khi mark đã được giải quyết.

**Acceptance criteria:**
- [x] Đủ năm tone.
- [x] Đúng vị trí cho nucleus đơn, đôi và ba nguyên âm.
- [x] Tone tự di chuyển khi mark làm thay đổi nucleus.

**Verification:**
- [x] Tone placement matrix pass.
- [x] `dduojwc -> được` và các hoán vị policy cho phép pass.

**Dependencies:** Task 5

**Files likely touched:**
- `engine/src/telex/tone.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

### Task 7: Cài renderer và replacement diff

**Description:** Render parsed syllable thành Unicode và tính replacement theo ký tự.

**Acceptance criteria:**
- [x] Casing được giữ.
- [x] Delete count theo Unicode scalar/contract hiện hành, không theo byte.
- [x] Output ổn định với cùng raw input.

**Verification:**
- [x] Renderer tests.
- [x] Replacement integration tests.

**Dependencies:** Tasks 5, 6

**Files likely touched:**
- `engine/src/telex/render.rs`
- `engine/src/state.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

## Checkpoint 2

- [x] Parser mới có thể render toàn bộ golden corpus.
- [x] Các mismatch được phân loại thành bug, behavior mới hoặc fallback.
- [x] Không có panic trong generated-input smoke tests.

## Phase 4: Chứng minh tính tổng quát

### Task 8: Thêm permutation test generator

**Description:** Sinh các thứ tự trigger hợp lệ thay vì viết test riêng từng từ.

**Acceptance criteria:**
- [x] Các hoán vị mark/tone hợp lệ hội tụ cùng output.
- [x] Generator bao phủ toàn bộ mark và tone classes.
- [x] Hoán vị ngoài policy không âm thầm tạo từ khác.

**Verification:**
- [x] Chạy permutation suite.
- [x] Báo seed/input khi thất bại.

**Dependencies:** Task 7

**Files likely touched:**
- `engine/src/telex/property_tests.rs`
- `engine/Cargo.toml` nếu cần dev-dependency

**Estimated scope:** M

### Task 9: Thêm invariants cho replay, backspace và boundary

**Description:** Chứng minh live input, replay và thao tác xóa dùng cùng semantics.

**Acceptance criteria:**
- [x] Replay bằng live output.
- [x] Backspace từng raw key trả về state trước đó.
- [x] Trigger không vượt boundary hoặc focus reset.

**Verification:**
- [x] Property tests pass.
- [x] Existing replay tests pass.

**Dependencies:** Task 8

**Files likely touched:**
- `engine/src/tests.rs`
- `engine/src/telex/property_tests.rs`

**Estimated scope:** M

## Phase 5: Tích hợp an toàn

### Task 10: Thêm shadow recomposition mode

**Description:** Chạy parser mới song song engine cũ trong test/debug để đo mismatch mà chưa đổi production output.

**Acceptance criteria:**
- [x] Mismatch report có raw keys, legacy output và new output.
- [x] Allowlist chỉ chứa thay đổi behavior đã phê duyệt.
- [x] Production release không có chi phí shadow nếu flag tắt.

**Verification:**
- [x] Golden suite chạy shadow mode.
- [x] Mismatch ngoài allowlist bằng 0.

**Dependencies:** Task 9

**Files likely touched:**
- `engine/src/bamboo.rs`
- `engine/src/telex/mod.rs`
- `engine/src/tests.rs`

**Estimated scope:** M

### Task 11: Chuyển live composing sang parser mới

**Description:** Dùng raw recomposition làm production path nhưng giữ public Engine contract.

**Acceptance criteria:**
- [x] `EngineEvent`, macro, auto-restore, undo và flush giữ contract.
- [x] Không đổi protocol của daemon/uinputd.
- [x] Có feature flag hoặc rollback path tạm thời.

**Verification:**
- [x] `cargo test --workspace`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `git diff --check`

**Dependencies:** Task 10

**Files likely touched:**
- `engine/src/bamboo.rs`
- `engine/src/engine.rs`
- `engine/src/telex/mod.rs`
- `engine/src/tests.rs`

**Estimated scope:** M

## Checkpoint 3

- [x] Workspace tests pass.
- [x] Shadow mismatch ngoài allowlist bằng 0.
- [x] Không thay đổi API/protocol desktop.
- [x] Code review trước runtime rollout: không phát hiện blocking finding trong recomposition production path, rollback feature, replacement semantics, replay/backspace/boundary, English fallback, casing, macro/flush và runtime capability gate. `cargo test -p vietc-engine` PASS; `cargo test -p vietc-engine --features bamboo-telex-rollback` PASS; `cargo clippy -p vietc-engine --all-targets -- -D warnings` PASS; `git diff --check` PASS.

## Phase 6: COSMIC runtime validation

### Task 12: Chạy Pop!_OS COSMIC matrix

**Description:** Xác minh input thật qua Wayland/uinput trên các ứng dụng đại diện.

**Acceptance criteria:**
- [ ] Text editor, terminal, Chromium, Firefox và Electron pass.
- [x] Controlled application matrix pass cho Chromium, Chrome, Electron, GTK, Qt và terminal trên Pop!_OS COSMIC.
- [x] Direct-uinput protocol pass cho PING, TYPE, PASTE, BACKSPACE và REPLACE.
- [ ] Controlled focus-switch stress chưa PASS: lần chạy duy nhất exit 101 do harness chưa nhận diện `ydotool`; deterministic contract đã PASS và harness đã được sửa nhưng không phát lại privileged suite trên cùng environment.
- [ ] Controlled fast-input stress chưa PASS sau bounded production run mới: exit 101 do timeout. Trace direct-uinput xác nhận replacement đầu từng fail khi clipboard restore read-back diễn ra quá sớm; sau khi chuyển sang bounded polling, production transaction đã `applied`, nhưng chưa chạy lại privileged suite theo giới hạn một lần.
- [x] Evidence được lưu theo runtime test convention.

**Verification:**
- [x] Deterministic runtime suite sau khi triển khai stress tests: 43 passed, 0 failed, exit 0.
- [x] Hai deterministic contract riêng cho focus-switch và fast-input pass, exit 0.
- [ ] Focus-switch privileged stress: 0 passed, 1 failed, exit 101.
- [ ] Fast-input privileged stress mới: 0 passed, 1 failed, exit 101 (`TimedOut`). Production trace trước fix ghi `Unicode insertion failed: clipboard unavailable`; trace sau fix ghi transaction đầu `applied`, nhưng automated observable scenario chưa được chạy lại.
- [ ] Firefox vẫn FAILED với `TargetCopyFailed`.
- [ ] COSMIC Edit vẫn FAILED vì daemonize ra ngoài owned process group.
- [x] Process do stress fixture tạo đã được teardown; không kill process không chứng minh được ownership.

**Terminal condition:** Task 12 dừng ở **BLOCKED/PARTIAL**. Không tự động retry privileged stress trên cùng environment. Chỉ tiếp tục khi blocker Firefox/COSMIC Edit hoặc external runtime environment đã thay đổi và có thể tạo evidence mới.

**Dependencies:** Task 11

**Files likely touched:**
- `daemon/tests/runtime/mod.rs`
- runtime evidence artifacts theo convention

**Estimated scope:** M

### Task 13: Chạy Fedora COSMIC matrix

**Description:** Lặp lại runtime matrix trên Fedora COSMIC để kiểm tra khác biệt packaging, permission và session.

**Acceptance criteria:**
- [ ] Cùng application matrix pass.
- [ ] `/dev/uinput`, service và socket permission đúng.
- [ ] Không có behavior khác Pop!_OS do integration layer.

**Verification:**
- [ ] Runtime evidence đầy đủ. Đã chạy trên Fedora 44 COSMIC/Wayland, kernel `7.3.0-363.vanilla.fc44.x86_64`; matrix còn PARTIAL do Firefox, COSMIC Edit, Qt, terminal và Electron chưa PASS. Hai privileged stress scenario focus-switch và fast-input đã PASS.
- [ ] So sánh kết quả hai distro. Fedora tái hiện blocker Firefox `TargetCopyFailed` và COSMIC Edit daemonize ngoài owned process group giống Pop!_OS.
- [x] `/dev/uinput` writable: mode `0666`, owner `root`, group `input`; user `thanhhd` thuộc group `input` và có ACL `rw-`.
- [x] Production daemon `/usr/bin/vietc-daemon` đang chạy trực tiếp; không có `vietc-daemon.service`, `vietc-uinputd.service` hoặc `vietc-uinputd.socket`.
- [x] Google Chrome controlled runtime PASS, exit 0.
- [x] Chromium controlled runtime PASS sau khi cài package Fedora với executable `chromium-browser`, exit 0.
- [x] GTK controlled runtime PASS, exit 0.
- [ ] Firefox controlled runtime FAIL, exit 101: `TargetCopyFailed`.
- [ ] COSMIC Edit controlled runtime FAIL, exit 101: daemonized outside owned process group.
- [ ] Qt controlled runtime FAIL, exit 101: target exited during startup.
- [ ] Terminal controlled runtime FAIL, exit 101: Konsole đã cài nhưng scenario `TimedOut`.
- [ ] Electron UNAVAILABLE: chưa có executable `electron` được harness hỗ trợ.
- [x] Focus-switch privileged stress PASS trên Fedora COSMIC: target A `tieengs` → `tiếng`, target B `chaof` → `chào`, quay lại target A vẫn giữ `tiếng`; evidence `target/runtime-artifacts/phase6-focus-switch-observed.txt` và `target/runtime-artifacts/fedora-cosmic-focus-switch-complete-words-final.log`.
- [x] Fast-input privileged stress PASS trên Fedora COSMIC ở key delay 8 ms: xác minh `được`, `nếu`, `tiếng`, boundary, macro isolation, backspace và disabled literal; evidence `target/runtime-artifacts/phase6-fast-input-observed.txt` và `target/runtime-artifacts/fedora-cosmic-fast-input-virtual-keyboard.log`.
- [x] Runtime harness chấp nhận cả `chromium` và executable Fedora `chromium-browser` tại capability gate.
- [x] Evidence được lưu trong `target/runtime-artifacts/fedora-cosmic-*.log`, các `controlled-*-result.txt` và timeout artifacts tương ứng.

**Bước tiếp theo: xử lý Firefox `TargetCopyFailed`**

1. Chạy riêng Firefox controlled runtime trên Fedora COSMIC, không chạy lại toàn matrix.
2. Tách scenario thành các checkpoint có evidence:
   - Firefox khởi động và tiến trình còn sống.
   - Cửa sổ thuộc đúng process tree được tìm thấy.
   - COSMIC/Wayland keyboard focus thực sự nằm trong cửa sổ Firefox.
   - Textarea nhận một focus probe ngắn.
   - Chuỗi controlled input được nhập.
   - `Ctrl+A`, `Ctrl+C` và clipboard read-back trả về đúng nội dung.
3. Ghi riêng giá trị lỗi cho từng bước thay vì gộp mọi lỗi thành `TargetCopyFailed`.
4. Trên Wayland, không coi `xdotool --window` là bằng chứng cửa sổ đã nhận keyboard focus. Nếu X11 window ID không điều khiển được Firefox native Wayland, dùng COSMIC toplevel activation theo title/app-id và xác nhận compositor báo `activated`.
5. Nếu nhập liệu thành công nhưng copy thất bại, thay clipboard-only observation bằng một target Firefox có observable channel độc lập, hoặc sửa bounded polling cho clipboard read-back.
6. Nếu focus probe thất bại, lưu title, app-id, PID, process tree, trạng thái activated và giá trị của cả target trước khi kết luận blocker.
7. Acceptance để đóng block:
   - Target Firefox nhận đúng controlled text.
   - Observation đọc lại đúng text, không chỉ dựa vào exit code.
   - Target và toàn bộ child process được terminate/reap.
   - Log và result artifact ghi `PASS`, exit 0.
8. Sau Firefox, xử lý lần lượt COSMIC Edit, Qt, Konsole và Electron. Chỉ khi application matrix Fedora hoàn tất mới so sánh đầy đủ với Pop!_OS và quay lại Task 12.

**Dependencies:** Task 12

**Files likely touched:**
- `daemon/tests/runtime/mod.rs`
- `docs/wayland-rootless.md`
- `packaging/README.md`

**Estimated scope:** M

## Phase 7: Cleanup và rollout

### Task 14: Xóa legacy case-by-case path

**Description:** Xóa backtrack và special branches chỉ sau khi parser mới đã được xác minh.

**Acceptance criteria:**
- [ ] Không còn hai implementation xử lý Telex production.
- [ ] Không còn dead code hoặc duplicate tone/mark tables.
- [ ] File lớn được tách thành module tập trung.

**Verification:**
- [ ] Full workspace tests và Clippy pass.
- [ ] Dead-code audit.
- [ ] Shadow suite không còn cần allowlist ngoài behavior mới.

**Dependencies:** Tasks 12, 13

**Files likely touched:**
- `engine/src/bamboo.rs`
- `engine/src/telex/*.rs`

**Estimated scope:** M

### Task 15: Tài liệu hóa free-order Telex contract

**Description:** Ghi rõ phạm vi hỗ trợ, fallback và giới hạn để tránh tuyên bố quá mức.

**Acceptance criteria:**
- [ ] README có ví dụ free-order.
- [ ] Chính sách trigger hợp lệ và literal fallback được mô tả.
- [ ] Có hướng dẫn báo bug kèm raw sequence.

**Verification:**
- [ ] Documentation review.
- [ ] Ví dụ tài liệu khớp golden tests.

**Dependencies:** Task 14

**Files likely touched:**
- `README.md`
- `README.en.md`
- `docs/testing-dictionary.md`

**Estimated scope:** S

## Final checkpoint

- [ ] Raw keystrokes là source of truth duy nhất.
- [ ] Mark và tone được resolve độc lập.
- [ ] Hoán vị trong policy được kiểm thử bằng generator.
- [ ] Replay, live input và backspace tương đương.
- [ ] Pop!_OS COSMIC runtime matrix pass.
- [ ] Fedora COSMIC runtime matrix pass.
- [ ] Legacy special-case path đã được xóa.
- [ ] Rollback strategy và runtime evidence đã được review.
### Runtime bugfix verification: casing và clipboard

- [x] Fail-first regression chứng minh Caps Lock state không cập nhật trên production evdev path: exit 101 trước fix.
- [x] Theo dõi Caps Lock press/toggle và dịch chữ theo `Shift XOR Caps Lock` trước Engine recomposition.
- [x] Regression bao phủ `TIEENGS`, `DDUOWNGF`, `DDUOJWC`, `NEESU`, câu nhiều từ, Caps+Shift và toggle giữa hai từ.
- [x] Fail-first regression chứng minh socket command có thể ACK trước verified clipboard restore: exit 101 trước fix.
- [x] Direct uinput và socket uinputd restore đồng bộ, verify read-back, không ghi đè user change và cleanup failure/timeout.
- [x] `git diff --check`: exit 0.
- [x] `cargo test -p vietc-engine`: exit 0.
- [x] `cargo test -p vietc-engine --features bamboo-telex-rollback`: exit 0.
- [x] `cargo test -p vietc-uinputd`: exit 0.
- [x] `cargo test -p vietc-daemon`: exit 0.
- [x] `cargo test --workspace`: exit 0.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.
- [x] Release daemon rebuilt/restarted; MainPID `1931295`, binary và SHA-256 recorded.
- [ ] Privileged user scenarios chưa PASS: bounded socket verification exit 101 vì stale socket không có listener; production daemon thực tế dùng direct uinput backend.
- [x] Đã thêm privileged long-text scenario 894 từ cho COSMIC Edit, Microsoft Edge và Firefox, observable read-back riêng theo target, configurable 40-250 ms key delay, exact expected/observed artifacts và mismatch summary; scenario vẫn ignored và chưa chạy trong môi trường hiện tại.
