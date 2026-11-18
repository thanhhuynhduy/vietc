# VietC

<p align="center">
  <strong>Bộ gõ tiếng Việt &amp; song ngữ Việt–Anh thuần <code>evdev</code>/<code>uinput</code> cho Linux (Wayland &amp; X11)</strong><br/>
  <a href="README.en.md">English Documentation</a>
</p>

**VietC** là bộ gõ tiếng Việt độc lập ở tầng thiết bị Linux (`evdev` + `uinput`), **hoàn toàn không phụ thuộc IBus hay Fcitx5**, hoạt động đồng nhất trên mọi môi trường Wayland (GNOME, KDE Plasma, COSMIC, Hyprland, Sway) và X11 mà không có gạch chân preedit.

Lõi ngôn ngữ [`vietc-engine`](engine/) mô phỏng cơ chế **Dual-Hypothesis Bilingual Decoder** của **Gboard trên Android**, kết hợp quy tắc **đặt dấu thanh kiểu cũ (`hòa`, `khỏe`, `thủy`)**.

---

## Điểm nổi bật

- **Kiến trúc thuần `evdev` + `uinput` (Rootless qua `uaccess`):**
  - Bắt phím trực tiếp từ `/dev/input/event*` và phát phím qua `/dev/uinput` + giao thức clipboard nguyên tử (`ext-data-control-v1` / `zwlr-data-control-v1` trên Wayland, X11 Selection trên X11).
  - Cấp quyền theo phiên người dùng đang ngồi tại máy (`TAG+="uaccess", GROUP="root", MODE="0600"` trong [`70-vietc.rules`](packaging/70-vietc.rules)) — daemon chạy ở user-space thường, không cần `root`, không cần `setcap`, không cần thêm user vào nhóm `input`.
- **Cơ chế song ngữ Việt–Anh học từ Gboard Android:**
  - **Song song hai giả thuyết (Dual-Hypothesis):** Luôn lưu chuỗi phím thô (`raw_buffer`) song song với âm tiết tiếng Việt đang dựng.
  - **Ngữ cảnh Bigram (`prev_context`) + Phân tầng từ vựng Unigram:** Tự động phân giải các từ trùng phím Telex giữa tiếng Việt và tiếng Anh dựa trên ngôn ngữ của từ đứng ngay trước:
    - `"bánh tét"` giữ tiếng Việt, `"cargo test"` khôi phục tiếng Anh.
    - `"cảm ơn"` giữ tiếng Việt, `"my own"` khôi phục tiếng Anh.
    - `"hai bên"` giữ tiếng Việt, `"have been"` khôi phục tiếng Anh.
  - **Nhấn `Backspace` ngay sau `Space` để hoàn tác Auto-Restore:** Nếu gõ `"test"` + `Space` tự động đổi thành `"test "` nhưng bạn thực sự muốn gõ `"tét"`, chỉ cần bấm `Backspace` một lần để quay lại `"tét"` và khóa ưu tiên tiếng Việt cho lần nhấn `Space` kế tiếp.
  - **Tự nhận diện token kỹ thuật & chữ hoa hỗn hợp:** Giữ nguyên `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, các từ tiếng Anh có phụ âm/nguyên âm đôi (`cheese`, `loose`, `keeper`, `stuff`, `business`, `address`, `IEEE`) và từ viết tắt tiếng Việt thông dụng (`đc`, `đk`, `ko`, `vs`).
- **Quy tắc dấu câu kiểu cũ (`Kiểu cũ`) & Gõ dấu tự do (Telex / VNI):**
  - Nguyên âm đôi mở `oa`, `oe`, `uy` đặt dấu vào nguyên âm đầu: `hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`.
  - Khi thêm phụ âm cuối (âm tiết đóng), dấu tự động dịch sang nguyên âm thứ hai: `hòa` + `n` → `hoàn`, `toán`, `huỳnh`, `tuýt`.
  - Hỗ trợ đầy đủ cả **Telex** và **VNI**, xóa dấu bằng `z` (Telex) / `0` (VNI), ghi đè mũ/móc (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), vần `-ươu` (`huouw` → `hươu`, `ruouwj` → `rượu`), và gõ tự do với âm đệm `qu`/`gi` (`gifa` → `già`, `qusa` → `quá`, `quo7` → `quơ`, `dan9` → `đan`).
- **An toàn mật khẩu & ứng dụng từ xa:**
  - Tự động tạm ngưng biến đổi phím trong ô nhập mật khẩu qua cây AT-SPI2 (`ROLE_PASSWORD_TEXT`), tiến trình xác thực terminal (`sudo`, `passwd`, `pkexec`, `ssh`, `gpg`, `cryptsetup`) và ứng dụng Remote Desktop / VM / Game.

---

## Cài đặt

### Cách 1: Cài đặt từ mã nguồn (Khuyên dùng)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
sudo bash ./install.sh --from-source
```

Sau khi cài đặt lần đầu, đăng xuất rồi đăng nhập lại để `systemd-logind` áp dụng ACL `uaccess` cho phiên làm việc. Dịch vụ `vietc.service` sẽ tự khởi chạy cùng phiên đồ họa.

### Cách 2: Đóng gói và cài đặt gói `.deb` (Ubuntu / Debian / Mint)

```bash
make deb
sudo apt install ./packaging/deb/vietc_0.2.0-1_amd64.deb
```

### Gỡ cài đặt

```bash
sudo bash ./uninstall.sh
```

---

## Phím tắt & Sử dụng

| Thao tác | Mặc định | Ghi chú |
|---|---|---|
| Bật / Tắt tiếng Việt | `Ctrl + Space` | Tùy chỉnh qua `toggle_key` trong `config.toml` |
| Đổi kiểu gõ (`Telex` ↔ `VNI`) | `Ctrl + Left Shift` | Tùy chỉnh qua `toggle_method_key` trong `config.toml` |
| Xóa thanh điệu đang có | `z` (Telex) / `0` (VNI) | Ví dụ: `toansz` → `toan`, `toan10` → `toan` |
| Hoàn tác Auto-Restore vừa xảy ra | `Backspace` ngay sau `Space` | Ví dụ: `"test "` + `Backspace` → `"tét"` |
| Khay hệ thống (tùy chọn) | `vietc-tray` | Cho phép bật/tắt, đổi Telex/VNI và cấu hình nhanh |
| Kiểm thử CLI độc lập | `vietc-cli` | Kiểm tra trực tiếp engine từ dòng lệnh |

---

## Cấu hình (`~/.config/vietc/config.toml`)

Bản mẫu đầy đủ nằm tại [`vietc.toml`](vietc.toml):

```toml
input_method = "telex"          # "telex" hoặc "vni"
toggle_key = "space"            # Ctrl + <toggle_key> bật/tắt bộ gõ
toggle_method_key = "shift"     # Ctrl + <toggle_method_key> chuyển Telex <-> VNI
start_enabled = true
grab = true
debug = false

[auto_restore]
enabled = true
trigger_keys = ["space", "escape"]

[app_state]
enabled = true
terminal_input_method = "vni"
english_apps = ["code", "jetbrains", "vim", "nvim", "kitty", "alacritty", "foot", "ghostty"]
vietnamese_apps = ["telegram", "discord", "slack", "firefox", "chromium", "gedit", "gnome-text-editor"]
bypass_apps = ["steam", "dota", "csgo", "minecraft", "remmina", "rustdesk", "virt-manager"]

[macros]
ko = "không"
dc = "được"
vs = "với"
```

---

## Cấu trúc dự án

- [`engine/`](engine/): Lõi xử lý ngôn ngữ Việt–Anh độc lập I/O (Telex, VNI, chuẩn dấu kiểu cũ, Gboard Bilingual Bigram/Unigram Scorer, State Machine & Property Tests).
- [`protocol/`](protocol/): Tầng giao tiếp `uinput` và giao dịch clipboard nguyên tử cho Wayland (`ext-data-control-v1` / `zwlr-data-control-v1`) & X11.
- [`daemon/`](daemon/): Dịch vụ nền `vietc-daemon` bắt phím `evdev`, quản lý trạng thái theo cửa sổ focus, phát hiện ô mật khẩu AT-SPI2 / `/proc`.
- [`ui/`](ui/): Biểu tượng khay hệ thống `vietc-tray` qua giao thức StatusNotifierItem (`ksni`).
- [`cli/`](cli/): Công cụ dòng lệnh `vietc-cli` để kiểm thử nhanh bộ gõ.

---

## Phát triển & Kiểm thử

```bash
# Định dạng mã nguồn & kiểm tra Clippy nghiêm ngặt (-D warnings)
make fmt
make lint

# Chạy toàn bộ bộ kiểm thử khói CI ở chế độ headless an toàn
make test-ci-smoke
```

## Giấy phép

Phát hành theo giấy phép [MIT](LICENSE).
