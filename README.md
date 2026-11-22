# Dboard

<p align="center">
  <strong>Bộ gõ tiếng Việt &amp; song ngữ Việt–Anh thuần <code>evdev</code>/<code>uinput</code> cho Linux Wayland (COSMIC, Hyprland, Sway — US QWERTY)</strong><br/>
  <a href="README.en.md">English Documentation</a>
</p>

**Dboard** là bộ gõ tiếng Việt độc lập ở tầng thiết bị Linux (`evdev` + `zwp_virtual_keyboard_v1` / `uinput`), **hoàn toàn không phụ thuộc IBus hay Fcitx5**, tối ưu cho các Wayland Compositor hỗ trợ `zwp_virtual_keyboard_v1` (**COSMIC Desktop, Hyprland, Sway và các wlroots compositor**) với bố cục bàn phím **US QWERTY**, không có gạch chân preedit.

> **Phạm vi hỗ trợ:** Chỉ hỗ trợ Wayland (`COSMIC`, `Hyprland`, `Sway` / `wlroots`) và bàn phím `US QWERTY`. Không hỗ trợ GNOME Wayland, KDE Plasma Wayland, X11 hoặc các bố cục bàn phím ngoài US QWERTY.

Lõi ngôn ngữ [`dboard-engine`](engine/) mô phỏng cơ chế **Dual-Hypothesis Bilingual Decoder** của **Gboard trên Android**, kết hợp quy tắc **đặt dấu thanh kiểu cũ (`hòa`, `khỏe`, `thủy`)**.

---

## Điểm nổi bật

- **Kiến trúc thuần `evdev` + `zwp_virtual_keyboard_v1` / `uinput` (Rootless qua `uaccess`):**
  - Bắt phím trực tiếp từ `/dev/input/event*` và xuất ký tự Unicode trực tiếp trên Wayland qua `zwp_virtual_keyboard_v1` (Keymap XKB 8 cấp độ US QWERTY, không dùng clipboard, không gạch chân preedit), chỉ dùng Wayland clipboard nguyên tử cho các ký tự đặc biệt ngoài keymap (như emoji trong macro).
  - Cấp quyền theo phiên người dùng đang ngồi tại máy (`TAG+="uaccess"` cho thiết bị bàn phím/chuột/touchpad trên `/dev/input/event*`, và `TAG+="uaccess", GROUP="root", MODE="0600"` cho `/dev/uinput` trong [`70-dboard.rules`](packaging/70-dboard.rules)) — daemon chạy ở user-space thường, không cần `root`, không cần `setcap`, không cần thêm user vào nhóm `input`.
- **Cơ chế song ngữ Việt–Anh học từ Gboard Android:**
  - **Song song hai giả thuyết (Dual-Hypothesis):** Luôn lưu chuỗi phím thô (`raw_buffer`) song song với âm tiết tiếng Việt đang dựng.
  - **Ngữ cảnh Bigram (`prev_context`) + Phân tầng từ vựng Unigram:** Tự động phân giải các từ trùng phím Telex giữa tiếng Việt và tiếng Anh dựa trên ngôn ngữ của từ đứng ngay trước:
    - `"bánh tét"` giữ tiếng Việt, `"cargo test"` khôi phục tiếng Anh.
    - `"cảm ơn"` giữ tiếng Việt, `"my own"` khôi phục tiếng Anh.
    - `"hai bên"` giữ tiếng Việt, `"have been"` khôi phục tiếng Anh.
  - **Nhấn `Backspace` ngay sau `Space` để hoàn tác Auto-Restore:** Nếu gõ `"test"` + `Space` tự động đổi thành `"test "` nhưng bạn thực sự muốn gõ `"tét"`, chỉ cần bấm `Backspace` một lần để quay lại `"tét"` và khóa ưu tiên tiếng Việt cho lần nhấn `Space` kế tiếp.
  - **Tự nhận diện token kỹ thuật, từ viết tắt ALLCAPS & chữ hoa hỗn hợp:** Giữ nguyên `AWS`, `DDOS`, `ECS`, `ORM`, `GIF`, `OOO`, `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, `err`, `arr`, `nextjs`, `nuxtjs`, các từ tiếng Anh có phụ âm/nguyên âm đôi (`cheese`, `loose`, `keeper`, `stuff`, `business`, `address`, `IEEE`) và từ viết tắt tiếng Việt thông dụng (`đc`, `đk`, `ko`, `vs`).
- **Quy tắc dấu câu kiểu cũ (`Kiểu cũ`) & Gõ dấu tự do (Telex / VNI):**
  - Nguyên âm đôi mở `oa`, `oe`, `uy` đặt dấu vào nguyên âm đầu: `hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`.
  - Khi thêm phụ âm cuối (âm tiết đóng), dấu tự động dịch sang nguyên âm thứ hai: `hòa` + `n` → `hoàn`, `toán`, `huỳnh`, `tuýt`.
  - Hỗ trợ đầy đủ cả **Telex** và **VNI**, xóa dấu bằng `z` (Telex) / `0` (VNI), ghi đè mũ/móc (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), vần `-ươu` (`huouw` → `hươu`, `ruouwj` → `rượu`), và gõ tự do với âm đệm `qu`/`gi` (`gifa` → `già`, `qusa` → `quá`, `quo7` → `quơ`, `dan9` → `đan`).
- **An toàn mật khẩu & ứng dụng từ xa:**
  - Tự động tạm ngưng biến đổi phím trong ô nhập mật khẩu qua AT-SPI2 (`ROLE_PASSWORD_TEXT`), cờ termios cấp nhân PTY (`ICANON && !ECHO`) cho `sudo`/`ssh`/`passwd`/`su`/`doas` trên mọi Wayland compositor, kiểm tra tiến trình con `/proc`, và tự động bypass độ trễ 0ms cho Remote Desktop / VM / Game.

---

## Cài đặt

### Cách 1: Cài đặt từ mã nguồn (Khuyên dùng)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git dboard
cd dboard
sudo bash ./install.sh --from-source
```

Sau khi cài đặt lần đầu, đăng xuất rồi đăng nhập lại để `systemd-logind` áp dụng ACL `uaccess` cho phiên làm việc. Dịch vụ `dboard.service` sẽ tự khởi chạy cùng phiên đồ họa.

### Cách 2: Đóng gói và cài đặt gói `.deb` (Ubuntu / Debian / Mint)

```bash
make deb
sudo apt install ./packaging/deb/dboard_*.deb
```

*(Ngoài ra có thể cài từ gói release tarball đã giải nén bằng `sudo bash ./install.sh` — script tự nhận diện binary cục bộ `./bin/dboard-daemon` để cài offline, hoặc truyền `--prebuilt` để tải bản phát hành mới nhất có kiểm tra `SHA256SUMS`.)*

### Gỡ cài đặt

```bash
sudo bash ./uninstall.sh
```

---

## Phím tắt & Sử dụng

| Thao tác | Mặc định | Ghi chú |
|---|---|---|
| Chuyển chế độ (`VN` ↔ `EN`) | `Ctrl + Space` | Mặc định `toggle_modes = ["vn", "en"]` (có thể đổi sang xoay vòng 3 chế độ `["en", "vni", "telex"]` và tùy chỉnh `toggle_key` trong `config.toml`); khớp chính xác modifier |
| Xóa thanh điệu đang có | `z` (Telex) / `0` (VNI) | Ví dụ: `toansz` → `toan`, `toan10` → `toan` |
| Hoàn tác Auto-Restore / Autocorrect | `Backspace` ngay sau `Space` | Ví dụ: `"test "` + `Backspace` → `"tét"` |
| Khay hệ thống (tùy chọn) | `dboard-tray` | Biểu tượng khay hệ thống bật/tắt VN–EN và chuyển Telex/VNI |
| Chẩn đoán môi trường | `dboard-daemon doctor` | Kiểm tra Wayland socket, `uaccess`, `/dev/uinput`, cấu hình & từ điển cá nhân |
| Kiểm thử CLI độc lập | `dboard-cli` | Kiểm tra trực tiếp engine từ dòng lệnh |

---

## Cấu hình (`~/.config/dboard/config.toml`)

Bản mẫu đầy đủ nằm tại [`dboard.toml`](dboard.toml):

```toml
input_method = "telex"          # "telex" hoặc "vni"
toggle_key = "space"            # Ctrl + <toggle_key>
toggle_modes = ["vn", "en"]     # Hoặc ["en", "vni", "telex"] để xoay vòng 3 chế độ
start_enabled = true
autocorrect = false             # Opt-in: sửa lỗi gõ trượt phím US QWERTY + One-Tap Undo
auto_capitalize = false         # Opt-in: tự viết hoa đầu câu
grab = true
debug = false
deduplicate_keys = false
deduplicate_window_ms = 35

[auto_restore]
enabled = true
trigger_keys = ["space", "escape"]

[app_state]
enabled = true
terminal_input_method = "vni"
english_apps = ["code", "jetbrains", "intellij", "pycharm", "webstorm", "vim", "nvim"]
vietnamese_apps = ["telegram", "discord", "slack", "firefox", "chromium", "gedit", "gnome-text-editor"]
terminal_apps = ["terminal", "kitty", "alacritty", "foot", "ghostty", "cosmic-term", "wezterm", "konsole", "gnome-terminal"]
bypass_apps = ["steam", "dota", "csgo", "cs2", "minecraft", "remmina", "rustdesk", "virt-manager"]

[macros]
ko = "không"
dc = "được"
bt = "biết"
```

---

## Cấu trúc dự án

- [`engine/`](engine/): Lõi xử lý ngôn ngữ Việt–Anh độc lập I/O (Telex, VNI, chuẩn dấu kiểu cũ, Gboard Bilingual Bigram/Unigram Scorer, State Machine & Property Tests).
- [`protocol/`](protocol/): Tầng xuất phím Wayland Virtual Keyboard (`zwp_virtual_keyboard_v1` XKB 8-level US QWERTY), `uinput` và giao dịch clipboard nguyên tử Wayland (`ext-data-control-v1` / `zwlr-data-control-v1`).
- [`daemon/`](daemon/): Dịch vụ nền `dboard-daemon` bắt phím `evdev`, quản lý trạng thái theo cửa sổ focus, phát hiện ô mật khẩu AT-SPI2 / `/proc`.
- [`ui/`](ui/): Biểu tượng khay hệ thống `dboard-tray` qua giao thức StatusNotifierItem (`ksni`).
- [`cli/`](cli/): Công cụ dòng lệnh `dboard-cli` để kiểm thử nhanh bộ gõ.
- [`docs/dboard-system-architecture.md`](docs/dboard-system-architecture.md): Tài liệu kiến trúc chuẩn 3 tầng & 6 bất biến hệ thống.

---

## Phát triển & Kiểm thử

```bash
# Định dạng mã nguồn & kiểm tra Clippy nghiêm ngặt (-D warnings)
make fmt
make lint

# Chạy toàn bộ bộ kiểm thử khói CI ở chế độ headless an toàn
make test-ci-smoke
```

## Ghi nhận & Tác giả gốc (Credits)

Dự án được phát triển dựa trên nền tảng và cảm hứng từ dự án gốc [**vietc**](https://github.com/vndangkhoa/vietc) của tác giả [**Võ Nguyễn Đăng Khoa (@vndangkhoa)**](https://github.com/vndangkhoa). Xin gửi lời cảm ơn chân thành tới tác giả gốc vì những đóng góp đặt nền móng cho bộ gõ tiếng Việt trên Linux.

## Giấy phép

Phát hành theo giấy phép [MIT](LICENSE).
