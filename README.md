# VietC

Bộ gõ tiếng Việt dành cho Linux, hỗ trợ Wayland và X11, được phát triển bằng Rust.

Dự án hiện tại được phát triển tiếp từ mã nguồn VietC gốc và đã được bổ sung các thay đổi về bảo mật, độ ổn định, kiểm thử runtime, clipboard và hành vi gõ Telex.

## Tính năng chính

- Hỗ trợ kiểu gõ Telex và VNI.
- Chuyển đổi chế độ ENG, VNI và TELEX.
- Hỗ trợ Wayland và X11.
- Hoạt động dưới dạng systemd user service.
- Hỗ trợ macro và gõ tắt.
- Ghi nhớ trạng thái theo ứng dụng.
- Hỗ trợ nhập Unicode qua `uinput`.
- Bảo toàn nội dung clipboard khi nhập Unicode.
- Hỗ trợ Telex lặp phím để hủy dấu và dấu mũ.
- Hỗ trợ build và kiểm thử bằng Rust workspace.

## Các đầu việc đã thực hiện

### Audit và hardening

- Audit daemon, engine, CLI và `uinputd`.
- Thiết lập baseline và test seam.
- Bảo vệ privileged uinput socket.
- Chuyển socket sang `$XDG_RUNTIME_DIR/vietc/uinput.sock`.
- Giới hạn quyền socket thành `0600`.
- Xác thực client bằng `SO_PEERCRED`.
- Chuẩn hóa path truyền vào API C.
- Thay single-instance PID check bằng RAII `flock`.
- Xử lý EOF và broken pipe trong CLI.
- Hoàn thiện release gates.
- Bổ sung deterministic CI tests.
- Bổ sung runtime matrix.
- Bổ sung disposable installation tests.
- Bổ sung E2E test cho `PING`, `TYPE`, `PASTE`, `BACKSPACE` và `REPLACE`.
- Kiểm chứng nhập liệu trên Chromium qua CDP.

### Clipboard và Unicode

- Loại bỏ delayed clipboard restoration.
- Khôi phục clipboard đồng bộ sau khi nhập Unicode.
- Bổ sung regression tests bảo toàn clipboard.
- Kiểm chứng modifier được giải phóng sau thao tác nhập liệu.

### Telex lặp phím

- Loại bỏ auto-restore khi nhấn Space.
- Giữ nguyên composition tại word boundary.
- Hủy dấu bằng cách lặp lại phím trigger.
- Hỗ trợ `projject` thành `project`.
- Hỗ trợ `tesst` thành `test`.
- Ngăn mark Telex backtrack xuyên qua nguyên âm khác.
- Giữ nguyên `readme` khi nhập bằng Telex.
- Bổ sung regression tests cho tone, mark và state machine.

### Kiểm thử và chất lượng

- Chạy toàn bộ workspace tests.
- Chạy Rustfmt.
- Chạy Clippy strict với `-D warnings`.
- Chạy `cargo audit`.
- Build release toàn workspace.
- Kiểm chứng daemon bằng systemd user service.
- Ghi lại commit rollback độc lập cho từng nhóm thay đổi.

Danh sách triển khai và bằng chứng kiểm thử được lưu trong [`tasks/implementation-log.md`](tasks/implementation-log.md).

## Phím tắt và điều khiển

| Thao tác | Phím tắt hoặc lệnh |
|---|---|
| Chuyển chế độ nhập | `Ctrl + Space` hoặc `Ctrl + Shift` |
| Xem trạng thái | `vietcctl status` |
| Chuyển sang Telex | `vietcctl method telex` |
| Chuyển sang VNI | `vietcctl method vni` |
| Khởi động lại dịch vụ | `systemctl --user restart vietc.service` |
| Xem trạng thái dịch vụ | `systemctl --user status vietc.service --no-pager` |
| Xem log dịch vụ | `journalctl --user -u vietc.service -f` |

## Cấu hình

Tệp cấu hình người dùng:

```text
~/.config/vietc/config.toml
```

Các nhóm cấu hình chính:

- Kiểu gõ Telex hoặc VNI.
- Phím chuyển chế độ.
- Trạng thái khi khởi động.
- Chế độ keyboard grab.
- Loại bỏ phím trùng lặp.
- Ghi nhớ trạng thái theo ứng dụng.
- Macro và gõ tắt.

## Build từ source

Yêu cầu:

- Linux.
- Rust 1.85 trở lên.
- Cargo.
- Các dependency hệ thống được khai báo trong script cài đặt.

Build release:

```bash
cd /home/thanhhd/Git-projects/vietc
cargo build --workspace --release
```

Chạy kiểm thử:

```bash
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo audit
```

Khởi động lại user service sau khi build:

```bash
systemctl --user daemon-reload
systemctl --user restart vietc.service
systemctl --user status vietc.service --no-pager
```

## Cấu trúc dự án

- `engine/`: bộ xử lý Telex, VNI, composition và state machine.
- `daemon/`: xử lý thiết bị nhập, focus, replay và output command.
- `uinputd/`: dịch vụ nhập liệu đặc quyền qua uinput.
- `cli/`: công cụ điều khiển VietC.
- `ui/`: giao diện và tray application.
- `vk/`: công cụ virtual keyboard phục vụ kiểm thử.
- `scripts/`: script cài đặt và kiểm thử.
- `tasks/`: kế hoạch, checklist, điều tra kỹ thuật và nhật ký triển khai.

## Tài liệu dự án

- [`tasks/plan.md`](tasks/plan.md)
- [`tasks/todo.md`](tasks/todo.md)
- [`tasks/implementation-log.md`](tasks/implementation-log.md)
- [`tasks/cosmic-focus-cache-fix-plan.md`](tasks/cosmic-focus-cache-fix-plan.md)
- [`tasks/cosmic-focus-cache-fix-todo.md`](tasks/cosmic-focus-cache-fix-todo.md)
- [`tasks/global-telex-chromium-crash-plan.md`](tasks/global-telex-chromium-crash-plan.md)
- [`tasks/global-telex-chromium-crash-todo.md`](tasks/global-telex-chromium-crash-todo.md)
- [`tasks/edge-sigill-vietc-technical-investigation.md`](tasks/edge-sigill-vietc-technical-investigation.md)

## Giấy phép và tác giả

Phát hành dưới **Giấy phép MIT**. Xem chi tiết tại [`LICENSE`](LICENSE).

Được phát triển với ❤️ bởi **Khoa Vo ([@vndangkhoa](https://github.com/vndangkhoa))**.
