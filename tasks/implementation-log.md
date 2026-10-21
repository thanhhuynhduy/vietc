# Nhật ký audit, hardening và sửa lỗi VietC

## Mục đích

Tài liệu này ghi lại các hạng mục đã thực hiện trên dự án, nguyên nhân thay đổi, cách kiểm chứng và commit rollback tương ứng.

## 1. Audit và hardening

### Phase 0: Thiết lập baseline

- Xác định phạm vi audit cho daemon, engine, CLI và `uinputd`.
- Bổ sung test seam cho các luồng cần quyền đặc quyền.
- Thiết lập quy trình kiểm chứng bằng unit test, integration test, Clippy, rustfmt và `cargo audit`.

### Phase 1: Bảo vệ privileged uinput socket

- Chuyển socket sang `$XDG_RUNTIME_DIR/vietc/uinput.sock`.
- Giới hạn quyền socket thành `0600`.
- Xác thực client bằng `SO_PEERCRED`.
- Sửa xử lý path trước khi truyền vào API C yêu cầu chuỗi kết thúc bằng NUL.
- Commit rollback: `35dd302`.

### Phase 2: Single-instance locking

- Thay cơ chế kiểm tra PID có race condition bằng khóa `flock`.
- Dùng RAII để khóa được giải phóng khi tiến trình kết thúc.
- Commit rollback: `ca65a31`.

### Phase 3: CLI I/O

- Xử lý EOF sạch, không lặp vô hạn.
- Xử lý broken pipe bằng lỗi có kiểm soát thay vì panic.
- Giữ nguyên hành vi interactive của lệnh `quit`.
- Commit rollback: `df5caef`.

### Phase 4 và Phase 5: Release gates

- Hoàn thiện tài liệu audit và checklist runtime.
- Thêm deterministic CI tests.
- Thêm runtime matrix và disposable installation tests.
- Thêm E2E test gửi trực tiếp các lệnh `PING`, `TYPE`, `PASTE`, `BACKSPACE` và `REPLACE` qua socket.
- Kiểm chứng kết quả nhập liệu trên Chromium thông qua CDP.
- Các commit tài liệu và release gate: `56f5012`, `7e7fb5b`, `6d261e0`.

## 2. Sửa lỗi clipboard

### Hiện tượng

Sau khi người dùng copy nội dung, việc gõ chữ tiếng Việt Unicode có thể làm clipboard bị thay bằng nội dung vừa gõ. Khi paste, ứng dụng trả về chữ vừa nhập thay vì dữ liệu đã copy.

### Nguyên nhân

Luồng nhập Unicode dùng clipboard tạm thời để phát nội dung qua Ctrl+V, nhưng việc khôi phục clipboard chạy trễ trong background. Điều này tạo race condition giữa lần paste của người dùng và tác vụ phục hồi clipboard.

### Thay đổi

- Loại bỏ delayed clipboard restorer.
- Khôi phục clipboard đồng bộ ngay sau khi hoàn thành Ctrl+V.
- Thêm regression tests để bảo vệ nội dung clipboard gốc.

### Kết quả mong đợi

1. Copy một đoạn văn bản.
2. Gõ chữ tiếng Việt có dấu.
3. Paste.
4. Nội dung paste vẫn là dữ liệu đã copy ban đầu.

## 3. Chuyển sang Telex lặp phím kiểu iPhone

### Hiện tượng trước khi sửa

- Gõ `project` hiển thị `prọect`.
- Khi nhấn Space, auto-restore đổi lại thành `project`.
- Gõ `projject` chưa sử dụng lần `j` thứ hai để bỏ dấu nặng và giữ một ký tự `j`.

### Hành vi yêu cầu

- `project ` phải giữ `prọect ` nếu người dùng chỉ gõ một `j`.
- `projject ` phải thành `project `.
- `tesst ` phải thành `test `.
- Space không được phục hồi raw keystrokes.
- Người dùng phải lặp lại trigger Telex để hủy biến đổi.

### Thay đổi production

- Bỏ commit-time auto-restore trong `Daemon::word_to_commit`.
- Bỏ flush-time auto-restore trong `Engine::process_key`.
- Giữ nguyên logic `undo_telex_transform`.
- Dùng `UndoPolicy::AppendTrigger` để:
  - bỏ dấu do trigger trước tạo ra;
  - giữ lại một trigger literal trong từ.

### Regression tests

- `single_tone_trigger_is_not_auto_restored_on_space`
- `repeated_tone_trigger_cancels_tone_and_keeps_one_literal_trigger`
- `process_key_keeps_composed_word_on_flush`
- `word_boundary_commits_without_restoring_invalid_composition`
- Kiểm thử state-machine cho cả hai giá trị `auto_restore`.

### Commit

- `f316561` - `fix: replace space auto-restore with repeated Telex triggers`

## 4. Kiểm chứng đã thực hiện

Các lệnh sau đã chạy thành công:

```bash
cargo test --workspace --all-targets
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo audit
cargo build --workspace --release
```

Kết quả:

- Toàn bộ workspace tests pass.
- Rustfmt pass.
- Clippy strict với `-D warnings` pass.
- `cargo audit` không phát hiện lỗ hổng trong dependency lockfile.
- Release build thành công.

## 5. Triển khai runtime đã thực hiện

Service thực tế của môi trường kiểm thử là user service:

```bash
systemctl --user restart vietc.service
systemctl --user status vietc.service --no-pager
```

Binary được user service sử dụng:

```text
/home/thanhhd/Git-projects/vietc/target/release/vietc
```

Sau khi restart:

- Service ở trạng thái `active (running)`.
- Config được đọc từ `~/.config/vietc/config.toml`.
- Input method được xác nhận là `telex`.
- Daemon tìm thấy thiết bị bàn phím và khởi động AT-SPI focus watcher.

Không có system-level `vietc.service` trên máy kiểm thử. Vì vậy, các lệnh `sudo systemctl restart vietc` trong README không áp dụng cho cấu hình đang chạy này.

## 6. Checklist kiểm thử thủ công

- [ ] `project ` hiển thị `prọect ` và không tự phục hồi khi nhấn Space.
- [ ] `projject ` hiển thị `project `.
- [ ] `tesst ` hiển thị `test `.
- [ ] Copy nội dung, gõ Unicode rồi paste vẫn trả về clipboard ban đầu.
- [ ] Ctrl+Space vẫn chuyển chế độ nhập liệu.
- [ ] Macro vẫn hoạt động.
- [ ] Backspace không xóa nhầm ký tự đứng trước composition.
- [ ] Service restart không để lại tiến trình clipboard helper ngoài ý muốn.

## 7. Sửa lỗi Telex backtrack qua nguyên âm

### Hiện tượng

- Gõ `readme` cho kết quả sai là `rêadme`.
- Gõ thêm một `e` nữa mới tạo ra `readmee`.

### Nguyên nhân

Logic tìm mark Telex quét lùi tối đa năm ký tự nhưng không dừng khi gặp một nguyên âm khác. Vì vậy, chữ `e` cuối trong `readme` có thể ghép nhầm với chữ `e` đầu, dù giữa chúng đã có nguyên âm `a`.

### Thay đổi

- Dừng việc tìm mark khi quá trình quét lùi gặp nguyên âm gần nhất mà không khớp rule.
- Không cho phép các rule `aa`, `ee`, `oo` và rule mark tương tự tác động xuyên qua một nguyên âm khác.
- Giữ nguyên flexible backtrack qua phụ âm cho các trường hợp Telex hợp lệ.

### Regression test

Test `repeated_vowel_does_not_create_mark_across_another_vowel` xác nhận:

```text
readme -> readme
```

Test đã được chạy ở trạng thái RED trước khi sửa và trả về `rêadm`, sau đó pass khi production logic được cập nhật.

### Kiểm chứng

- Focused regression test: pass.
- `cargo test --workspace --all-targets`: pass.
- `cargo fmt --all -- --check`: pass.
- Clippy strict với `-D warnings`: pass.
- Release build: pass.
- User service đã được restart thành công với release binary mới.

## 8. Rollback

Rollback riêng thay đổi Telex mới nhất:

```bash
git revert f316561
cargo build --workspace --release
systemctl --user restart vietc.service
```

Các phase hardening có commit độc lập nên có thể rollback từng hạng mục mà không cần đảo ngược toàn bộ dự án.