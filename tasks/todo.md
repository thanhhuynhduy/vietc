# Checklist sửa lỗi audit an toàn

## Phase 0: Baseline và test seam

- [x] Task 1: Tách socket server/dispatcher để test không cần `/dev/uinput`
- [x] Test client UID hợp lệ đi đến dispatcher
- [x] Test client UID không hợp lệ không tạo input event
- [x] Xác nhận protocol command hiện tại không đổi

### Checkpoint 0

- [x] `cargo test -p vietc-uinputd` pass
- [x] `cargo check -p vietc-uinputd --all-targets` pass
- [x] Runtime behavior chưa đổi

## Phase 1: Loại bỏ C-string undefined behavior

- [x] Task 2: Loại bỏ `String::as_ptr()` khỏi `chown`/`chmod`
- [x] Dùng `std::fs::set_permissions`
- [x] Nếu cần `chown`, dùng `CString` và kiểm tra errno
- [x] Chỉ đổi owner/mode sau khi socket đã bind
- [x] Startup failure dọn đúng socket vừa tạo
- [x] Không bỏ qua lỗi tạo directory, ownership hoặc permission

### Checkpoint 1

- [x] `cargo test -p vietc-uinputd` pass
- [x] `cargo clippy -p vietc-uinputd --all-targets -- -D warnings` pass
- [x] `cargo fmt --all -- --check` pass

## Phase 2: Socket authorization và permission

- [x] Task 3: Thêm `SO_PEERCRED` cho từng connection
- [x] Fail-closed khi không đọc được peer credentials
- [x] Chỉ cho phép UID mục tiêu
- [x] Không log nội dung input
- [x] Test đủ `PING`, `TYPE`, `PASTE`, `BACKSPACE`, `REPLACE`
- [x] Test peer không hợp lệ không phát input

- [x] Task 4: Chuyển socket về runtime directory được quản lý
- [x] Runtime directory mode `0700`
- [x] Socket mode `0600`
- [x] Không fallback im lặng sang `/tmp`
- [x] Không unlink symlink hoặc regular file
- [x] Chỉ dọn stale Unix socket thuộc runtime path

- [x] Task 5: Cập nhật installer và upgrade path
- [x] Fresh install truyền đúng UID/runtime path
- [x] Upgrade xử lý socket `0666` cũ
- [x] Uninstall chỉ xóa artifact được quản lý
- [x] Rootless mode không bị ảnh hưởng
- [x] Restart service hai lần liên tiếp pass

### Checkpoint 2

- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass
- [x] `cargo fmt --all -- --check` pass
- [x] ASCII injection pass
- [x] Unicode injection pass
- [x] Replacement pass
- [x] Peer UID khác bị từ chối
- [x] Socket không world-writable
- [x] Fresh install pass
- [x] Upgrade pass
- [x] Rootless mode pass

## Phase 3: Single-instance lock

- [x] Task 6: Thêm process-level regression test cho split-inode race
- [x] Test holder giữ lock dù PID text stale/non-matching
- [x] Test có timeout và cleanup rõ ràng

- [x] Task 7: Tạo RAII lock guard
- [x] Không unlink/reopen khi `flock` trả contention
- [x] PID được ghi qua fd đã lock
- [x] Guard sống suốt vòng đời daemon
- [x] Contention thoát như “đã chạy”
- [x] Lock error thật trả failure
- [x] Hai process đồng thời chỉ có một process tiếp tục

### Checkpoint 3

- [x] `cargo test -p vietc-daemon single_instance` pass
- [x] Regression test contention lặp 50 lần pass
- [x] Shutdown bình thường thả lock
- [x] Process chết bất thường không để kernel lock kẹt

## Phase 4: CLI I/O

- [x] Task 8: Chuyển CLI loop sang `io::Result`
- [x] EOF thoát sạch
- [x] Broken pipe không panic
- [x] Interactive flow giữ nguyên
- [x] `quit` giữ nguyên behavior
- [x] I/O error bất thường có exit code và message phù hợp

### Checkpoint 4

- [x] `cargo test -p vietc-cli` pass
- [x] `printf '' | cargo run -q -p vietc-cli` pass
- [x] `printf 'quit\n' | cargo run -q -p vietc-cli` pass

## Phase 5: Release verification

- [x] `cargo fmt --all -- --check` pass
- [x] `cargo test --workspace --all-targets` pass
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass
- [x] `cargo audit` sạch
- [x] Rootless X11/XWayland pass
- [x] Wayland fallback uinputd pass
- [x] Fresh install pass
- [x] Upgrade từ socket `0666` pass
- [x] Service restart pass
- [x] Unauthorized client test pass
- [x] Dual-daemon test pass
- [x] CLI EOF/broken-pipe pass
- [x] Shutdown/restart loop không leak resource

## Release gate

- [x] Không còn socket `0666`
- [x] Mọi connection uinput được xác thực fail-closed
- [x] Không còn C path pointer không NUL-terminated
- [x] Không còn split-inode lock race
- [x] CLI không panic do stdin/stdout đóng
- [x] Mỗi task có regression test
- [x] Mỗi task là commit độc lập và có thể rollback
- [x] Không gộp refactor module lớn vào bản vá
