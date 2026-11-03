# VietC

Bộ gõ tiếng Việt dành cho Linux, hỗ trợ Wayland và X11, được phát triển bằng Rust.

Dự án hiện tại được phát triển tiếp từ mã nguồn VietC gốc và đã được bổ sung các nâng cấp toàn diện về bảo mật, độ ổn định, kiểm thử tự động (Tier H headless & controlled runtime), bảo toàn clipboard, cũng như độ chính xác của bộ xử lý Telex/VNI và các backend nhập liệu (`evdev`/`uinput`, IBus, Wayland IM).

## Tính năng chính

- Hỗ trợ đầy đủ hai kiểu gõ **Telex** và **VNI**.
- Chuyển đổi nhanh chế độ **ENG**, **VNI** và **TELEX** (`Ctrl + Space`, `Ctrl + Shift`, `vietcctl` hoặc khay hệ thống `vietc-tray`).
- Đa dạng backend nhập liệu trên **Wayland** và **X11**:
  - `evdev` + `uinput` / `vietc-uinputd` (hỗ trợ cả grabbed và non-grabbed mode).
  - Native **IBus engine** (`org.freedesktop.IBus.Engine`).
  - Rootless **Wayland Input Method** (`zwp_input_method_v2` + `zwp_virtual_keyboard_v1`).
- Tự động nhận diện ô mật khẩu / PIN (AT-SPI2, IBus `ContentType`, tiến trình `sudo`/`passwd`, window class/title) và tự động phục hồi chế độ gõ khi rời ô mật khẩu.
- Tự động bỏ qua (bypass) các ứng dụng đầu cuối từ xa / VM / game (`remmina`, `xfreerdp`, `virt-viewer`, `steam`...) theo danh sách cấu hình `excluded_apps`.
- Hỗ trợ **Smart English Auto-Restore** (tự động khôi phục từ tiếng Anh tại ranh giới từ như `fix`, `cargo`, `status`, `world`, `class`, `password`...) có thể bật/tắt trong cấu hình.
- Hỗ trợ lặp phím để hoàn tác dấu thanh, dấu mũ/móc/trăng và `đ` cho cả **Telex** và **VNI** (`ass` → `as`, `awa` → `wa`, `ww` → `w`, `a11` → `a1`...).
- Hỗ trợ **macro / gõ tắt** thống nhất trên cả Telex và VNI.
- Bảo toàn tuyệt đối nội dung **clipboard** (văn bản, ảnh, dữ liệu nhị phân, clipboard rỗng) và xác thực ghi clipboard thành công trước khi gửi `Backspace` xóa ký tự trên màn hình.
- Theo dõi chuyển đổi tiêu điểm (focus) chính xác đến từng widget/ô nhập liệu trong cùng cửa sổ qua AT-SPI object path.

## Các hạng mục kỹ thuật & bản sửa lỗi gần nhất

### 1. Bộ xử lý ngôn ngữ (`vietc-engine`)
- **Đồng bộ tuyệt đối màn hình (`screen`) và `engine.buffer()`**:
  - Ngăn bộ khớp mũ `aa`/`ee`/`oo` vượt qua ký tự `'w'` hoặc ghi đè nguyên âm đã mang dấu trăng/móc (`"awa"` → `"wa"`, `"owo"` → `"wo"`).
  - Chỉ hoàn tác `ww` khi ký tự `'w'` đầu tiên thực sự đã được tiêu thụ để đặt dấu (`"ww"` → `"w"`).
  - Hỗ trợ hoàn tác phím dấu lặp lại trong chế độ VNI (`"a11"` → `"a1"`, `"a112"` → `"a12"`).
- **Bảo vệ tiền tố tài liệu (`SENTINEL_PREFIX|`)**:
  - Tách biệt `pasted_prefix` khỏi `raw_buffer` khi nhận sự kiện `PhysicalEvent::Paste` để thao tác `Backspace` hoặc phím dấu tiếp theo không tái phân tích chuỗi vừa dán như phím Telex thô.
  - Theo dõi chiều dài thực tế trên màn hình (`previous_screen_len` / `recovered_screen_len`) khi xảy ra `PhysicalEvent::ReplacementFailed` để `Backspace` hoặc lần thay thế kế tiếp không bao giờ xóa lẹm vào văn bản đứng trước từ đang gõ.
- **Chuẩn hóa ngữ âm & quy tắc dấu tiếng Việt**:
  - Nhận diện chính xác âm đệm (glide) `qu` và `gi` (`"qua"`, `"gia"`, `"quyen"`, `"gieng"`) là âm tiết hợp lệ (`ParseStatus::Complete`).
  - Xóa dấu thanh cũ trước khi chuyển vị trí dấu trong VNI (`"ho1a2"` → `"hoà"`) và kiểm tra tính hợp lệ không cho phép quá 1 nguyên âm mang dấu thanh trong cùng một âm tiết.
  - Thống nhất xử lý macro tại ranh giới từ cho cả VNI và Telex, xóa đúng số ký tự viết tắt trước khi chèn nội dung mở rộng.
  - Mở rộng bộ nhận diện từ tiếng Anh có phụ âm kép `'ss'` (`"ass"`, `"class"`, `"password"`, `"pass"`, `"less"`, `"boss"`...) khi bật `auto_restore`.
  - Đảm bảo tính nhất quán của trạng thái máy hữu hạn (`BoundaryState::Composing`, `TransformKind::Mark("aw")`, `TransformKind::Mark("uo+w")`).

### 2. Daemon, Focus & Input Method Backends (`vietc-daemon`)
- **`evdev_loop`**:
  - Kiểm tra thay đổi file cấu hình và trạng thái (`status`, `config.toml`) trước khi bỏ qua chu kỳ `poll` rỗng để cập nhật có hiệu lực ngay lập tức kể cả khi bàn phím đang nghỉ.
  - Kiểm tra đổi focus và trạng thái ô mật khẩu trước khi xử lý `KEY_BACKSPACE` hoặc bỏ qua phím khi engine tắt, giúp tự động bật lại tiếng Việt ngay khi rời ô mật khẩu trong cùng cửa sổ và không xóa nhầm ký tự ở ô mới khi vừa chuyển focus.
- **AT-SPI Focus Tracker (`focus_tracker`)**:
  - Kết hợp cả `sender` và `object_path` (`atspi:<sender>:<path>`) để phát hiện việc chuyển đổi giữa các ô nhập liệu khác nhau trong cùng một ứng dụng.
- **IBus Engine (`ibus_engine`)**:
  - Hỗ trợ `SetContentType` và thuộc tính `ContentType` (`IBUS_INPUT_PURPOSE_PASSWORD`, `IBUS_INPUT_PURPOSE_PIN`) để tự động commit preedit và tắt ghép chữ trong ô mật khẩu/PIN.
  - Tự động commit `preedit` đang gõ dở trước khi chuyển chế độ bằng `Ctrl + Space` hoặc khi đổi phương thức gõ từ bên ngoài.
  - Không flush `preedit` khi nhấn các phím bổ trợ đơn thuần (`Shift`, `Ctrl`, `CapsLock`, `Alt`, `Super`, `ISO_Level3_Shift`), giữ nguyên mạch ghép chữ hoa giữa từ.
- **Wayland Input Method (`wayland_im`)**:
  - Bổ sung `conn.flush()`, `conn.prepare_read()` + `poll` và `guard.read()` trong vòng lặp sự kiện để nhận đầy đủ sự kiện từ Wayland socket.
  - Nhận diện phím tắt với `Super` (`MOD_NAME_LOGO`) và tự động commit `preedit` khi nhấn các phím điều hướng/biên tập (`Tab`, `Arrows`, `Home`, `End`, `PageUp/Down`, `Delete`, `F1..F12`).

### 3. Giao thức & Clipboard (`vietc-protocol`, `vietc-uinputd`)
- **Bảo vệ dữ liệu khi dán Unicode (`uinput_monitor`)**:
  - Sao lưu, ghi và xác thực nội dung clipboard mới (`ClipboardWritten`) **trước** khi gửi chuỗi `Backspace` xóa ký tự của người dùng; nếu clipboard không khả dụng, hủy thao tác ngay mà không làm mất chữ đã gõ.
  - Giới hạn thời gian chạy (bounded timeout 250ms) cho các tiến trình con đọc clipboard (`wl-paste`, `xclip`) để tránh treo vòng lặp nhập liệu khi tiện ích clipboard bị treo.
- **Bảo mật socket đặc quyền**:
  - Đặt socket tại `$XDG_RUNTIME_DIR/vietc/uinput.sock` với quyền `0600`, xác thực kết nối bằng `SO_PEERCRED`, và cấp quyền thiết bị theo phiên đăng nhập đang hoạt động (ACL) thay vì `0666`.

## Phím tắt và điều khiển

| Thao tác | Phím tắt hoặc lệnh |
|---|---|
| Chuyển chế độ nhập (`ENG` → `VNI` → `TELEX`) | `Ctrl + Space` hoặc `vietcctl cycle` |
| Chuyển nhanh kiểu gõ | `Ctrl + Shift` |
| Xem trạng thái hiện tại | `vietcctl status` |
| Chuyển sang Telex | `vietcctl telex` |
| Chuyển sang VNI | `vietcctl vni` |
| Chuyển sang tiếng Anh | `vietcctl en` |
| Khởi động lại dịch vụ | `systemctl --user restart vietc.service` |
| Xem trạng thái dịch vụ | `systemctl --user status vietc.service --no-pager` |
| Xem log dịch vụ | `journalctl --user -u vietc.service -f` |

## Cấu hình

Tệp cấu hình người dùng:

```text
~/.config/vietc/config.toml
```

Các nhóm cấu hình chính:

- `input_method`: `"telex"` hoặc `"vni"`.
- `toggle_key` / `toggle_method_key`: phím chuyển chế độ và kiểu gõ.
- `start_enabled`: trạng thái bật/tắt khi khởi động.
- `grab`: chế độ chiếm quyền bàn phím (`evdev` grab).
- `deduplicate_keys`, `deduplicate_window_ms`, `deduplicate_two_back`: chống lặp phím phần cứng.
- `excluded_apps`: danh sách ứng dụng tự động chuyển sang chế độ gõ thô (EN).
- `[password_detection]`: tự động chuyển sang EN trong ô mật khẩu.
- `[auto_restore]`: tự động khôi phục từ tiếng Anh khi kết thúc từ.
- `[macros]`: bảng từ viết tắt.

## Build và Kiểm thử từ source

### Yêu cầu
- Linux (Wayland hoặc X11).
- Rust 1.85 trở lên và Cargo.
- Các thư viện hệ thống theo script `install.sh` (`libdbus-1-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libx11-dev`, `libxtst-dev`...).

### Build release
```bash
cargo build --workspace --release
# Build thêm ứng dụng khay hệ thống (tray UI):
cd ui && cargo build --release
```

### Chạy kiểm thử an toàn (Headless Tier H — không can thiệp vào phiên màn hình đang chạy)
> **Lưu ý an toàn:** Không chạy `cargo test` trần hoặc `sudo cargo test` trên máy đang mở phiên làm việc thật vì các bài test tích hợp có thể tạo bàn phím ảo `/dev/uinput` hoặc ghi đè clipboard của người dùng. Luôn dùng target `test-ci-smoke` hoặc ẩn biến môi trường hiển thị:

```bash
# Chạy toàn bộ bộ kiểm thử CI smoke (installer, build targets, unit, property, golden, deterministic runtime):
make test-ci-smoke CARGO_FLAGS='--locked --offline'

# Kiểm thử riêng engine (bao gồm cả feature rollback):
env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test -p vietc-engine --tests --locked --offline
env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test -p vietc-engine --tests --locked --offline --features bamboo-telex-rollback

# Kiểm thử deterministic runtime của daemon:
env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test -p vietc-daemon --test daemon_suite runtime:: --locked --offline
```

### Kiểm tra định dạng và chất lượng mã nguồn
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Cấu trúc dự án

- [`engine/`](engine/): bộ xử lý ngôn ngữ Telex, VNI, kiểm tra chính tả tiếng Việt, auto-restore và state machine (`vietc-engine`).
- [`protocol/`](protocol/): giao thức IPC, quản lý uinput, đồng bộ và khôi phục clipboard, tiêm phím X11/Wayland (`vietc-protocol`).
- [`daemon/`](daemon/): dịch vụ bộ gõ chính (`vietc-daemon`), bao gồm `evdev_loop`, `ibus_engine`, `wayland_im`, `focus_tracker`, `password_detector` và `app_state`.
- [`uinputd/`](uinputd/): dịch vụ nhập liệu đặc quyền qua `/dev/uinput` (`vietc-uinputd`).
- [`cli/`](cli/): công cụ dòng lệnh tương tác để thử nghiệm và gỡ lỗi bộ gõ (`vietc-cli`).
- [`vietcctl/`](vietcctl/): tiện ích điều khiển nhanh trạng thái và kiểu gõ (`vietcctl`).
- [`ui/`](ui/): ứng dụng khay hệ thống và giao diện cấu hình (`vietc-tray`).
- [`vk/`](vk/): công cụ bàn phím ảo phục vụ kiểm thử (`vietc-vk`).
- [`docs/`](docs/): tài liệu kiến trúc, từ điển kiểm thử và playbook kiểm thử.
- [`scripts/`](scripts/): script kiểm thử trình cài đặt và mục tiêu biên dịch.
- [`tasks/`](tasks/): kế hoạch, checklist, điều tra kỹ thuật và nhật ký triển khai.

## Tài liệu dự án

- [`docs/vietnamese-typing-agent-playbook.md`](docs/vietnamese-typing-agent-playbook.md) — Playbook kiểm thử và đánh giá chất lượng gõ tiếng Việt.
- [`docs/testing-dictionary.md`](docs/testing-dictionary.md) — Từ điển kịch bản kiểm thử (`TEST-NNN`).
- [`CHANGELOG.md`](CHANGELOG.md) / [`CHANGELOG.vi.md`](CHANGELOG.vi.md) — Nhật ký thay đổi chi tiết (Tiếng Anh / Tiếng Việt).
- [`tasks/implementation-log.md`](tasks/implementation-log.md) — Nhật ký triển khai kỹ thuật.

## Giấy phép và tác giả

Phát hành dưới **Giấy phép MIT**. Xem chi tiết tại [`LICENSE`](LICENSE).

Được phát triển với ❤️ bởi **Khoa Vo ([@vndangkhoa](https://github.com/vndangkhoa))**.

