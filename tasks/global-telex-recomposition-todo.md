# Task Breakdown: Global Telex Raw-Keystroke Recomposition

## Phase 1: Đóng băng hợp đồng hiện tại

### Task 1: Xây behavior matrix

**Description:** Liệt kê các input/output hiện đang hỗ trợ, free-order mong muốn, English fallback và chuỗi không hợp lệ.

**Acceptance criteria:**
- [ ] Có bảng mark, tone, nucleus, coda, casing và boundary.
- [ ] Mỗi behavior có expected output hoặc expected literal fallback.
- [ ] `dduojwc`, `dduocwj`, `neues`, `neesu` nằm trong matrix.

**Verification:**
- [ ] Review matrix trước khi viết parser.
- [ ] Không có expected behavior mơ hồ.

**Dependencies:** None

**Files likely touched:**
- `docs/testing-dictionary.md`
- `engine/src/tests.rs`

**Estimated scope:** S

### Task 2: Tạo golden regression suite

**Description:** Chuyển behavior matrix thành data-driven tests chạy trên engine hiện tại.

**Acceptance criteria:**
- [ ] Golden tests bao phủ các quy tắc Telex hiện có.
- [ ] English words và boundary cases có regression tests.
- [ ] Test báo rõ input nào lệch output.

**Verification:**
- [ ] `cargo test -p vietc-engine`

**Dependencies:** Task 1

**Files likely touched:**
- `engine/src/tests.rs`
- `engine/src/test_cases.rs`

**Estimated scope:** M

## Checkpoint 1

- [ ] Golden suite pass trên baseline.
- [ ] Các thay đổi behavior mong muốn được allowlist rõ ràng.
- [ ] Không bắt đầu migration nếu baseline chưa ổn định.

## Phase 2: Xây domain model và parser thuần

### Task 3: Định nghĩa raw và parsed syllable types

**Description:** Tạo các type biểu diễn raw keys, mark, tone và cấu trúc âm tiết mà không render Unicode sớm.

**Acceptance criteria:**
- [ ] Tone và mark là hai trạng thái độc lập.
- [ ] Type có invariant rõ ràng.
- [ ] Không phụ thuộc IBus, Wayland, X11 hoặc uinput.

**Verification:**
- [ ] Unit tests cho construction và boundary.
- [ ] `cargo clippy -p vietc-engine --all-targets -- -D warnings`

**Dependencies:** Task 2

**Files likely touched:**
- `engine/src/telex/raw.rs`
- `engine/src/telex/model.rs`
- `engine/src/lib.rs`

**Estimated scope:** S

### Task 4: Parse raw keys thành onset, nucleus và coda

**Description:** Cài parser xác định ranh giới và cấu trúc âm tiết từ raw sequence.

**Acceptance criteria:**
- [ ] Parse đúng onset/nucleus/coda của corpus.
- [ ] Xử lý `qu` và `gi` như glide theo điều kiện.
- [ ] Không parse qua whitespace hoặc punctuation boundary.

**Verification:**
- [ ] Unit tests theo từng nhóm âm tiết.
- [ ] Fuzz/property smoke test không panic.

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
- [ ] Hỗ trợ `aa`, `aw`, `ee`, `oo`, `ow`, `uw`, `uo+w`, `ua+w`, `dd`.
- [ ] Mark trigger hợp lệ không phụ thuộc thứ tự với tone.
- [ ] Trigger không áp dụng được có deterministic fallback.

**Verification:**
- [ ] Mark matrix tests pass.
- [ ] `readme` và English corpus giữ nguyên.

**Dependencies:** Task 4

**Files likely touched:**
- `engine/src/telex/mark.rs`
- `engine/src/telex/parser.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

### Task 6: Cài tone resolver

**Description:** Chọn tone cuối cùng có hiệu lực và đặt nó trên nucleus sau khi mark đã được giải quyết.

**Acceptance criteria:**
- [ ] Đủ năm tone.
- [ ] Đúng vị trí cho nucleus đơn, đôi và ba nguyên âm.
- [ ] Tone tự di chuyển khi mark làm thay đổi nucleus.

**Verification:**
- [ ] Tone placement matrix pass.
- [ ] `dduojwc -> được` và các hoán vị policy cho phép pass.

**Dependencies:** Task 5

**Files likely touched:**
- `engine/src/telex/tone.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

### Task 7: Cài renderer và replacement diff

**Description:** Render parsed syllable thành Unicode và tính replacement theo ký tự.

**Acceptance criteria:**
- [ ] Casing được giữ.
- [ ] Delete count theo Unicode scalar/contract hiện hành, không theo byte.
- [ ] Output ổn định với cùng raw input.

**Verification:**
- [ ] Renderer tests.
- [ ] Replacement integration tests.

**Dependencies:** Tasks 5, 6

**Files likely touched:**
- `engine/src/telex/render.rs`
- `engine/src/state.rs`
- `engine/src/telex/tests.rs`

**Estimated scope:** M

## Checkpoint 2

- [ ] Parser mới có thể render toàn bộ golden corpus.
- [ ] Các mismatch được phân loại thành bug, behavior mới hoặc fallback.
- [ ] Không có panic trong generated-input smoke tests.

## Phase 4: Chứng minh tính tổng quát

### Task 8: Thêm permutation test generator

**Description:** Sinh các thứ tự trigger hợp lệ thay vì viết test riêng từng từ.

**Acceptance criteria:**
- [ ] Các hoán vị mark/tone hợp lệ hội tụ cùng output.
- [ ] Generator bao phủ toàn bộ mark và tone classes.
- [ ] Hoán vị ngoài policy không âm thầm tạo từ khác.

**Verification:**
- [ ] Chạy permutation suite.
- [ ] Báo seed/input khi thất bại.

**Dependencies:** Task 7

**Files likely touched:**
- `engine/src/telex/property_tests.rs`
- `engine/Cargo.toml` nếu cần dev-dependency

**Estimated scope:** M

### Task 9: Thêm invariants cho replay, backspace và boundary

**Description:** Chứng minh live input, replay và thao tác xóa dùng cùng semantics.

**Acceptance criteria:**
- [ ] Replay bằng live output.
- [ ] Backspace từng raw key trả về state trước đó.
- [ ] Trigger không vượt boundary hoặc focus reset.

**Verification:**
- [ ] Property tests pass.
- [ ] Existing replay tests pass.

**Dependencies:** Task 8

**Files likely touched:**
- `engine/src/tests.rs`
- `engine/src/telex/property_tests.rs`

**Estimated scope:** M

## Phase 5: Tích hợp an toàn

### Task 10: Thêm shadow recomposition mode

**Description:** Chạy parser mới song song engine cũ trong test/debug để đo mismatch mà chưa đổi production output.

**Acceptance criteria:**
- [ ] Mismatch report có raw keys, legacy output và new output.
- [ ] Allowlist chỉ chứa thay đổi behavior đã phê duyệt.
- [ ] Production release không có chi phí shadow nếu flag tắt.

**Verification:**
- [ ] Golden suite chạy shadow mode.
- [ ] Mismatch ngoài allowlist bằng 0.

**Dependencies:** Task 9

**Files likely touched:**
- `engine/src/bamboo.rs`
- `engine/src/telex/mod.rs`
- `engine/src/tests.rs`

**Estimated scope:** M

### Task 11: Chuyển live composing sang parser mới

**Description:** Dùng raw recomposition làm production path nhưng giữ public Engine contract.

**Acceptance criteria:**
- [ ] `EngineEvent`, macro, auto-restore, undo và flush giữ contract.
- [ ] Không đổi protocol của daemon/uinputd.
- [ ] Có feature flag hoặc rollback path tạm thời.

**Verification:**
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `git diff --check`

**Dependencies:** Task 10

**Files likely touched:**
- `engine/src/bamboo.rs`
- `engine/src/engine.rs`
- `engine/src/telex/mod.rs`
- `engine/src/tests.rs`

**Estimated scope:** M

## Checkpoint 3

- [ ] Workspace tests pass.
- [ ] Shadow mismatch ngoài allowlist bằng 0.
- [ ] Không thay đổi API/protocol desktop.
- [ ] Code review trước runtime rollout.

## Phase 6: COSMIC runtime validation

### Task 12: Chạy Pop!_OS COSMIC matrix

**Description:** Xác minh input thật qua Wayland/uinput trên các ứng dụng đại diện.

**Acceptance criteria:**
- [ ] Text editor, terminal, Chromium, Firefox và Electron pass.
- [ ] Focus change, typing nhanh, backspace, paste và restart daemon pass.
- [ ] Evidence được lưu theo runtime test convention.

**Verification:**
- [ ] Chạy privileged runtime suite với environment flags hiện có.
- [ ] Không có mismatch giữa expected và observed text.

**Dependencies:** Task 11

**Files likely touched:**
- `daemon/tests/runtime.rs`
- runtime evidence artifacts theo convention

**Estimated scope:** M

### Task 13: Chạy Fedora COSMIC matrix

**Description:** Lặp lại runtime matrix trên Fedora COSMIC để kiểm tra khác biệt packaging, permission và session.

**Acceptance criteria:**
- [ ] Cùng application matrix pass.
- [ ] `/dev/uinput`, service và socket permission đúng.
- [ ] Không có behavior khác Pop!_OS do integration layer.

**Verification:**
- [ ] Runtime evidence đầy đủ.
- [ ] So sánh kết quả hai distro.

**Dependencies:** Task 12

**Files likely touched:**
- `daemon/tests/runtime.rs`
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