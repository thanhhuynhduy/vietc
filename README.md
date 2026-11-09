# VietC

Bộ gõ tiếng Việt dành cho Linux, hỗ trợ Wayland và X11, được phát triển bằng Rust.

Dự án được phát triển tiếp từ mã nguồn VietC gốc và đã được nâng cấp toàn diện về thuật toán xử lý ngôn ngữ tiếng Việt / tiếng Anh song ngữ (kế thừa và học hỏi trực tiếp từ bộ gõ **[Ngó Sen](https://github.com/ngosen/ngosen)** và **[bamboo-core](https://github.com/bamboo-vi/bamboo-core)**), kiểm thử tự động (Tier H headless & controlled runtime), bảo toàn clipboard, cũng như độ ổn định của các backend nhập liệu (`evdev`/`uinput`, IBus, Wayland IM).

## Tính năng chính

- Hỗ trợ đầy đủ hai kiểu gõ **Telex** và **VNI** với quy tắc đặt dấu tự do (free-order tone/mark typing).
- Chuyển đổi nhanh chế độ **ENG**, **VNI** và **TELEX** (`Ctrl + Space`, `Ctrl + Shift`, `vietcctl` hoặc khay hệ thống `vietc-tray`).
- Đa dạng backend nhập liệu trên **Wayland** và **X11**:
  - `evdev` + `uinput` / `vietc-uinputd` (hỗ trợ cả grabbed và non-grabbed mode).
  - Native **IBus engine** (`org.freedesktop.IBus.Engine`).
  - Rootless **Wayland Input Method** (`zwp_input_method_v2` + `zwp_virtual_keyboard_v1`).
- **Xử lý song ngữ Việt – Anh thông minh (Smart Bilingual Telex & Auto-Restore)**:
  - Chặn nuốt phím thanh điệu/dấu mũ giữa hai nguyên âm trong các từ tiếng Anh có nguyên âm `e` câm hoặc đa âm tiết (`more`, `core`, `lose`, `rose`, `nose`, `chose`, `those`, `these`, `here`, `there`, `where`, `were`, `care`, `share`, `base`, `case`, `use`, `pure`, `cure`, `hire`, `fire`, `house`, `mouse`, `pause`, `cause`, `nasa`, `para`, `sara`, `exe`).
  - Ngăn việc lặp chữ cái không liền kề tự huỷ dấu làm mất chữ trong các từ tiếng Anh (`order`, `border`, `server`, `sister`, `awkward`).
  - Kiểm tra tính hợp lệ của phụ âm đầu và tiền tố âm tiết ngay khi đang gõ (`is_valid_vietnamese_prefix` / `has_valid_vietnamese_onset`), bỏ qua biến đổi dấu cho các cụm phụ âm đầu tiếng Anh (`str`, `sw`, `bl`, `cl`, `fl`, `gl`, `pl`, `sl`, `br`, `cr`, `dr`, `fr`, `gr`, `pr`, `sc`, `sk`, `sm`, `sn`, `sp`, `st`, `tw`, `wr`).
  - Kết hợp kiểm tra quy tắc âm vị học tiếng Việt (CVC phonotactics + quy tắc phụ âm cuối tắc `c/ch/p/t` chỉ đi với thanh sắc/nặng) cùng **từ điển âm tiết tiếng Việt chuẩn** (`vietnamese.cm.dict` — 7.884 âm tiết, chuẩn hoá không phụ thuộc vị trí đặt dấu cũ/mới `hoà`/`hòa`, `thuỷ`/`thủy`) để khôi phục chính xác từ tiếng Anh tại ranh giới từ mà vẫn giữ nguyên các âm tiết tiếng Việt hợp lệ (`bên`, `tô`, `thí`, `ít`, `sĩ`, `lơ`, `quả`, `gì`, `giếng`...).
- **Tối ưu thay thế hậu tố tối thiểu (`replacement_diff`)**:
  - Chỉ gửi `Backspace` và chèn lại phần hậu tố thực sự thay đổi của từ đang gõ (ví dụ `nghie` + `e` → chỉ xoá `ee` và chèn `ê` để thành `nghiê`, giữ nguyên tiền tố `nghi`), giảm thiểu số lượng sự kiện phím ảo và tránh hiện tượng nháy/rớt chữ.
- Hỗ trợ lặp phím liền kề để hoàn tác dấu thanh, dấu mũ/móc/trăng và `đ` cho cả **Telex** và **VNI** (`ass` → `as`, `tesst` → `test`, `offfer` → `offer`, `isssue` → `issue`, `aww` → `aw`, `addd` → `add`, `teele` → `tele`, `a11` → `a1`...).
- Tự động nhận diện ô mật khẩu / PIN (AT-SPI2, IBus `ContentType`, tiến trình `sudo`/`passwd`, window class/title) và tự động phục hồi chế độ gõ khi rời ô mật khẩu.
- Tự động bỏ qua (bypass) các ứng dụng đầu cuối từ xa / VM / game (`remmina`, `xfreerdp`, `virt-viewer`, `steam`...) theo danh sách cấu hình `excluded_apps`.
- Hỗ trợ **macro / gõ tắt** thống nhất trên cả Telex và VNI.
- Bảo toàn tuyệt đối nội dung **clipboard** (văn bản, ảnh, dữ liệu nhị phân, clipboard rỗng) và xác thực ghi clipboard thành công trước khi gửi `Backspace` xóa ký tự trên màn hình.
- Theo dõi chuyển đổi tiêu điểm (focus) chính xác đến từng widget/ô nhập liệu trong cùng cửa sổ qua AT-SPI object path.

## Kiến trúc & Logic xử lý mới nhất

### 1. Bộ xử lý ngôn ngữ (`vietc-engine`)
- **Phân giải dấu và thanh điệu chính xác cho gõ song ngữ (`telex/mark.rs`, `telex/tone.rs`, `bamboo.rs`)**:
  - Dấu mũ (`aa`, `ee`, `oo`) chỉ được ghép qua các nguyên âm liền kề trong cùng cụm nguyên âm (`neues` → `nếu`, `tieengs` → `tiếng`), không nhảy qua phím thanh điệu (`s, f, r, x, j`) hoặc phụ âm, giữ nguyên các từ như `these`, `here`, `there`, `where`, `were`, `exe`, `readme`.
  - Phím thanh điệu đứng trước nguyên âm phía sau chỉ được tiêu thụ khi cụm nguyên âm tạo thành vần tiếng Việt hợp lệ theo kiểu gõ tự do (`chafo` → `chào`, `neesu` → `nếu`), loại bỏ hoàn toàn lỗi biến các từ tiếng Anh có `e` câm (`more`, `core`, `lose`, `rose`, `nose`, `chose`, `those`, `care`, `base`, `case`, `use`, `house`, `pause`...) thành âm tiết tiếng Việt giả (`mỏe`, `loé`...).
  - Chỉ hoàn tác thanh điệu và dấu `w` khi hai phím kích hoạt giống nhau được gõ **liền kề** (`prev_idx + 1 == i`), bảo toàn chữ cái trong các từ tiếng Anh có chữ lặp cách quãng (`order`, `border`, `server`, `sister`, `awkward`).
  - Hợp nhất hoàn toàn kết quả biến đổi giữa pipeline `telex/{mark,tone,render}.rs` và `BambooEngine` cho mọi thứ tự gõ tự do (`dduojwc` / `dduocwj` → `được`, `neues` / `neesu` → `nếu`).
- **Kiểm tra chính tả kết hợp Ngữ âm học + Từ điển âm tiết (`spelling.rs`, `vietnamese.cm.dict`)**:
  - Kiểm tra cấu trúc âm tiết `Phụ âm đầu (C1) - Nguyên âm (V) - Phụ âm cuối (C2)` qua ma trận `CV_MATRIX` và `VC_MATRIX`, hỗ trợ kiểm tra cả âm tiết hoàn chỉnh (`is_valid_vietnamese_syllable`) lẫn tiền tố âm tiết đang gõ (`is_valid_vietnamese_prefix`).
  - Ràng buộc âm vị học tiếng Việt: âm tiết kết thúc bằng phụ âm tắc vô thanh (`c`, `ch`, `p`, `t`) bắt buộc phải mang thanh **sắc** hoặc **nặng** (nhờ đó các từ tiếng Anh như `chart`, `text`, `boot`, `keep` được tự động khôi phục về tiếng Anh).
  - Tích hợp từ điển **7.884 âm tiết tiếng Việt** (`vietnamese.cm.dict`) được chuẩn hoá bằng hàm `without_tone_position` (tách thanh điệu ra cuối âm tiết để đối sánh độc lập với kiểu đặt dấu cũ/mới như `hoà` và `hòa`), giúp loại bỏ các tổ hợp ghép vần hợp lệ về mặt ma trận nhưng không tồn tại trong tiếng Việt thực tế (`gin`, `win`, `weed`...).
- **Bảo vệ tiền tố tài liệu & đồng bộ trạng thái hiển thị**:
  - Tách biệt `pasted_prefix` khỏi `raw_buffer` khi nhận sự kiện `PhysicalEvent::Paste` để thao tác `Backspace` hoặc phím dấu tiếp theo không tái phân tích chuỗi vừa dán như phím Telex thô.
  - Theo dõi chiều dài thực tế trên màn hình (`previous_screen_len` / `recovered_screen_len`) khi xảy ra `PhysicalEvent::ReplacementFailed` để `Backspace` hoặc lần thay thế kế tiếp không bao giờ xóa lẹm vào văn bản đứng trước từ đang gõ.

### 2. Daemon, Focus & Input Method Backends (`vietc-daemon`)
- **Thay thế hậu tố tối thiểu (`replacement_diff`)**:
  - `Engine::replay_events_to_commands`, `Daemon::replay_and_inject` và `Daemon::replay_backspace` tính toán phần tiền tố chung dài nhất giữa chuỗi đang hiển thị trên màn hình và chuỗi đích, chỉ phát lệnh `Backspace` và `Type` cho phần hậu tố thay đổi.
- **`evdev_loop` & AT-SPI Focus Tracker (`focus_tracker`)**:
  - Kiểm tra thay đổi file cấu hình và trạng thái (`status`, `config.toml`) ngay trong vòng lặp sự kiện.
  - Kết hợp `sender` và `object_path` (`atspi:<sender>:<path>`) để phát hiện việc chuyển đổi giữa các ô nhập liệu khác nhau trong cùng một cửa sổ, tự động bật lại tiếng Việt ngay khi rời ô mật khẩu.
- **IBus Engine (`ibus_engine`) & Wayland Input Method (`wayland_im`)**:
  - Hỗ trợ `SetContentType` (`IBUS_INPUT_PURPOSE_PASSWORD`, `IBUS_INPUT_PURPOSE_PIN`) để tự động commit preedit và tắt ghép chữ trong ô mật khẩu/PIN.
  - Tự động commit `preedit` khi chuyển chế độ bằng `Ctrl + Space` hoặc khi nhấn các phím điều hướng/biên tập (`Tab`, `Arrows`, `Home`, `End`, `PageUp/Down`, `Delete`, `F1..F12`), đồng thời giữ nguyên mạch ghép chữ hoa khi nhấn các phím bổ trợ (`Shift`, `CapsLock`).

### 3. Giao thức & Clipboard (`vietc-protocol`, `vietc-uinputd`)
- **Bảo vệ dữ liệu khi dán Unicode (`uinput_monitor`)**:
  - Sao lưu, ghi và xác thực nội dung clipboard mới (`ClipboardWritten`) **trước** khi gửi chuỗi `Backspace` xóa ký tự của người dùng; nếu clipboard không khả dụng, hủy thao tác ngay mà không làm mất chữ đã gõ.
  - Giới hạn thời gian chạy (bounded timeout 250ms) cho các tiến trình con đọc clipboard (`wl-paste`, `xclip`) để tránh treo vòng lặp nhập liệu.
- **Bảo mật socket đặc quyền**:
  - Đặt socket tại `$XDG_RUNTIME_DIR/vietc/uinput.sock` với quyền `0600`, xác thực kết nối bằng `SO_PEERCRED`, và cấp quyền thiết bị theo phiên đăng nhập đang hoạt động (ACL).

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

- [`engine/`](engine/): bộ xử lý ngôn ngữ Telex, VNI, kiểm tra chính tả ngữ âm & từ điển tiếng Việt (`vietnamese.cm.dict`), auto-restore và state machine (`vietc-engine`).
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

## Nguồn gốc & Ghi công (Credits)

- Dự án gốc **VietC** được khởi tạo bởi **Khoa Vo ([@vndangkhoa](https://github.com/vndangkhoa))**.
- Phần logic kiểm tra chính tả tiếng Việt, từ điển âm tiết chuẩn (`vietnamese.cm.dict`), chuẩn hoá thanh điệu không phụ thuộc vị trí đặt dấu (`without_tone_position`), kiểm tra tiền tố âm tiết đang gõ (`is_valid_vietnamese_prefix`), quy tắc xử lý song ngữ Việt – Anh (chặn nuốt phím dấu giữa các nguyên âm và từ tiếng Anh có `e` câm, huỷ dấu khi lặp phím liền kề) và tính toán thay thế hậu tố tối thiểu được sao chép, chuyển thể và học hỏi từ bộ gõ **[Ngó Sen](https://github.com/ngosen/ngosen)** của tác giả **Nguyen Hoang Ky** và các cộng tác viên Ngó Sen, cùng thư viện lõi **[bamboo-core](https://github.com/bamboo-vi/bamboo-core)** (bản Rust của **[@nguyen10t2](https://github.com/nguyen10t2)**, port từ bản Go **[bamboo-core](https://github.com/BambooEngine/bamboo-core)** của **[@t1ld3x](https://github.com/t1ld3x)**). Xin gửi lời cảm ơn chân thành đến tác giả và các cộng tác viên của dự án **Ngó Sen**!
