# Dboard

<p align="center">
  <strong>Bộ gõ tiếng Việt &amp; song ngữ Việt–Anh thuần <code>evdev</code>/<code>uinput</code> cho Linux Wayland (COSMIC, Hyprland, Sway — US QWERTY / Dvorak / Colemak)</strong><br/>
  <a href="README.en.md">English Documentation</a>
</p>

**Dboard** là bộ gõ tiếng Việt độc lập ở tầng thiết bị Linux (`evdev` + `zwp_virtual_keyboard_v1` / `uinput`), **hoàn toàn không phụ thuộc IBus hay Fcitx5**, tối ưu cho các Wayland Compositor hỗ trợ `zwp_virtual_keyboard_v1` (**COSMIC Desktop, Hyprland, Sway và các wlroots compositor**) với bố cục bàn phím **US QWERTY, Dvorak và Colemak**, không có gạch chân preedit.

> **Phạm vi hỗ trợ:** Hỗ trợ Pure Wayland (`COSMIC`, `Hyprland`, `Sway` / `wlroots`) với bàn phím `US QWERTY`, `Dvorak`, `Colemak`. Không hỗ trợ GNOME Wayland, KDE Plasma Wayland hoặc X11.

Lõi ngôn ngữ [`dboard-engine`](engine/) mô phỏng cơ chế **Dual-Hypothesis Bilingual Decoder & Quantized Statistical Language Model** của **Gboard trên Android**, kết hợp quy tắc **đặt dấu thanh kiểu cũ (`hòa`, `khỏe`, `thủy`)**.

---

## Điểm nổi bật

- **Kiến trúc thuần `evdev` + `zwp_virtual_keyboard_v1` / `uinput` (Rootless qua `uaccess`, Zero-Sleep Fast Path):**
  - Bắt phím trực tiếp từ `/dev/input/event*` và xuất ký tự Unicode trực tiếp trên Wayland qua `zwp_virtual_keyboard_v1` (Keymap XKB 8 cấp độ cho US QWERTY / Dvorak / Colemak, không dùng clipboard, không gạch chân preedit), chỉ dùng Wayland clipboard đa MIME nguyên tử cho các ký tự đặc biệt ngoài keymap (như emoji trong macro).
  - Cấp quyền theo phiên người dùng đang ngồi tại máy (`TAG+="uaccess"` cho thiết bị bàn phím/chuột/touchpad trên `/dev/input/event*`, và `TAG+="uaccess", GROUP="root", MODE="0600"` cho `/dev/uinput` trong [`70-dboard.rules`](packaging/70-dboard.rules)) — daemon chạy ở user-space thường, không cần `root`, không cần `setcap`, không cần thêm user vào nhóm `input`.
- **Mô hình Ngôn ngữ Song ngữ Việt–Anh Thống kê Lượng tử hóa (Chuẩn Gboard Android):**
  - **Song song hai giả thuyết (Dual-Hypothesis) + Quantized 2/3/4-gram Kneser-Ney LM (`bilingual_lm.bin` 348 KB + `ngram_lm.bin` 2.63 MB):** Tra cứu $O(1)$ FNV-1a zero-allocation với độ trễ fast-path `p50 ~ 1.47 µs`, `p99 ~ 7.48 µs` (`soak_perf` 100k phím), dự đoán từ tiếp theo (**Next-Word Prediction 100% top-3**), và độ chính xác song ngữ **99.94% theo từ (7,217/7,221) / 99.22% theo câu (506/510; `pure_vn`/`pure_en`/`office`/`dev` 100%, `code_switch` 96.19%, 0 autocorrect false positives)** trên benchmark 510 câu:
    - `"bánh tét"` giữ tiếng Việt, `"cargo test"` / `"mở test.py"` khôi phục tiếng Anh.
    - `"cảm ơn"` giữ tiếng Việt, `"my own"` khôi phục tiếng Anh.
    - `"hai bên"` giữ tiếng Việt, `"have been"` khôi phục tiếng Anh; `"á"` / `"á?"` đứng riêng giữ tiếng Việt, `"as soon"` / `"run as root"` giữ tiếng Anh.
    - Sửa hồi quy 2 từ liên tiếp (`"Lát nữa"`, `"Lots of"`, `"Rust toolchain"`, `"REST API"`, `"this morning"`) kèm **One-Tap Undo** đồng bộ tuyệt đối về từ gốc khi nhấn `Backspace` ngay sau `Space`.
  - **Khôi phục tên file có dấu chấm giữa câu tiếng Việt:** Tự động nhận diện và khôi phục `mở test.py`, `xem list.txt`, `mở test.docx`, `xem list.xlsx`, `sửa host.conf`, `giải nén archive.tar.gz` mà vẫn bảo toàn 100% dấu chấm kết thúc câu tiếng Việt (`"ăn bánh tét. hôm nay"`).
  - **Weighted Spatial SymSpell (Telex & VNI + ~35k tiếng Anh) + Personal Bigrams + Smart Auto-Capitalization:** Sửa lỗi thiếu/thừa/trượt phím và tách từ dính cho cả **Telex** và **VNI** (`khoog` / `khog6` → `không`, `tieng71` → `tiếng`, `khoongbieets` / `khong6biet61` → `không biết`, `algoritm` → `algorithm`, `tomorow` → `tomorrow`) với pre-warm trên luồng nền, bảo toàn 100% định danh lập trình và từ ghép tiếng Anh (`kubectl`, `hashmap`, `hostlist`, `testlist`, `u32`, `sha256`), tự viết hoa đầu câu thông minh (`TP.`, `TS.`, `PGS.`, `Mr.`, `etc.`, `"Chào." Hôm`, `Chào. "Hôm nay"`), và học cụm 2 từ cá nhân quyền `0600` (tự động tắt học trong ô mật khẩu/terminal/bypass).
  - **Tự nhận diện token kỹ thuật, từ viết tắt ALLCAPS & camelCase:** Giữ nguyên `dOs`, `cAs`, `tOs`, `iOs`, `AWS`, `DDOS`, `QĐ-TTg`, `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, `err`, `arr`, `nextjs`, `nuxtjs`, và không nháy chữ `đ` giữa các từ tiếng Anh có `dd` (`add`, `reddit`, `hidden`, `middle`, `wedding`).
- **Quy tắc dấu câu kiểu cũ (`Kiểu cũ`) & Gõ dấu tự do (Telex / VNI):**
  - Nguyên âm đôi mở `oa`, `oe`, `uy` đặt dấu vào nguyên âm đầu: `hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`.
  - Khi thêm phụ âm cuối (âm tiết đóng), dấu tự động dịch sang nguyên âm thứ hai: `hòa` + `n` → `hoàn`, `toán`, `huỳnh`, `tuýt`.
  - Hỗ trợ đầy đủ cả **Telex** và **VNI**, gõ dấu mũ vượt phụ âm cuối kiểu Gboard (`khongo` → `không`, `khongso` → `khống`, `vietej` / `vietje` → `việt`, `tienges` / `tiengse` → `tiếng`, `tana` → `tân`), xóa dấu bằng `z` (Telex) / `0` (VNI), ghi đè mũ/móc (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), vần `-ươu` (`huouw` → `hươu`, `ruouwj` → `rượu`), và âm đệm `qu`/`gi`.
- **Thanh Gợi ý Ứng viên Nổi Tùy chọn (`dboard-popup` — Wayland Layer-Shell):**
  - Tắt mặc định (`candidate_bar = false`) hoặc bật nhanh 1 chạm bằng **Preset: Gboard Mode (All On)** trên `dboard-tray`. Giao tiếp non-blocking (`O_NONBLOCK` Unix Datagram `0600`) tới tiến trình độc lập `dboard-popup` (`zwlr_layer_shell_v1`, `KeyboardInteractivity::None`), vẽ chữ TrueType anti-aliased bằng **`swash`** có clip biên khe (tự fallback bitmap 8×16 có clip khe), bám con trỏ AT-SPI2 đa màn hình (hỗ trợ cả tọa độ âm, màn hình xoay & Fractional HiDPI qua `zxdg_output_v1`, fallback neo trên giữa **top-center**), hỗ trợ chọn ứng viên & dự đoán từ tiếp theo nối tiếp bằng chuột hoặc `Alt+1`/`Alt+2`/`Alt+3` (không cướp phím chuyển tab / `Alt+Shift+1/2/3`, tự ẩn tuyệt đối khi nhập mật khẩu) và sửa nhanh từ giữa câu (`tét` ↔ `test`, `khôg` → `không`).
- **An toàn mật khẩu 4 lớp & Đa thiết bị phần cứng (Đã kiểm chứng qua 9 đợt Audit — 0 Critical/High):**
  - Tự động tạm ngưng biến đổi phím trong ô nhập mật khẩu qua AT-SPI2 fail-closed (`ROLE_PASSWORD_TEXT` + grace window 150ms), Window Class (`polkit`, `cosmic-osd`, `pinentry`, `keepassxc`, `bitwarden`...)/Title lowercase, cờ termios cấp nhân PTY (`ICANON && !ECHO`) có làm mới tức thì sau `Enter` và sau khoảng dừng cho `sudo`/`ssh`/`git push`/`passwd`/`su`/`doas`, kiểm tra tiến trình con `/proc` (chỉ log khi chuyển trạng thái, không ghi log phím trong ô mật khẩu), reset an toàn khi chuyển sang Layer-Shell (`Polkit`/launcher) đồng bộ theo sự kiện `Done`, nhận diện Touchpad Tap-to-Click, `bypass_mode` 0ms cho game/VM và hợp nhất modifier xuyên nhiều bàn phím USB/Bluetooth.

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
| Hoàn tác Auto-Restore / Autocorrect / Bigram | `Backspace` ngay sau `Space` | Ví dụ: `"test "` + `Backspace` → `"tét"`, `"Lát nữa "` + `Backspace` → `"Last nữa"` |
| Chọn nhanh ứng viên (`candidate_bar = true`) | `Alt + 1` / `Alt + 2` / `Alt + 3` | `[1]` Tiếng Việt, `[2]` Gốc ASCII, `[3]` Gợi ý sửa lỗi |
| Khay hệ thống & Thanh gợi ý (tùy chọn) | `dboard-tray` / `dboard-popup` | Biểu tượng khay hệ thống (`ksni`) và thanh gợi ý nổi Wayland Layer-Shell |
| Chẩn đoán môi trường | `dboard-daemon doctor` | Kiểm tra Wayland socket, `uaccess`, `/dev/uinput`, xung đột `Mod3`/`Mod5`, layout & cấu hình |
| Kiểm thử CLI độc lập | `dboard-cli` | Kiểm tra trực tiếp engine từ dòng lệnh |

---

## Cấu hình (`~/.config/dboard/config.toml`)

Bản mẫu đầy đủ nằm tại [`dboard.toml`](dboard.toml):

```toml
input_method = "telex"          # "telex" hoặc "vni"
keyboard_layout = "us"          # "us" (US QWERTY), "dvorak", hoặc "colemak"
toggle_key = "space"            # Ctrl + <toggle_key>
toggle_modes = ["vn", "en"]     # Hoặc ["en", "vni", "telex"] để xoay vòng 3 chế độ
start_enabled = true
autocorrect = false             # Opt-in: sửa lỗi gõ trượt phím (Telex & VNI) + One-Tap Undo
auto_capitalize = false         # Opt-in: tự viết hoa đầu câu thông minh
candidate_bar = false           # Opt-in: phát sự kiện tới thanh gợi ý nổi dboard-popup
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

- [`engine/`](engine/): Lõi xử lý ngôn ngữ Việt–Anh độc lập I/O (Telex Recomposition, VNI Bamboo, chuẩn dấu kiểu cũ, Quantized Statistical LM `stat_lm.rs` / `bilingual_lm.bin`, Dot-Filename Restore `filename_restore.rs`, Spatial Autocorrect, Smart Auto-Cap, State Machine & Property Tests).
- [`protocol/`](protocol/): Tầng xuất phím Wayland Virtual Keyboard (`zwp_virtual_keyboard_v1` XKB 8-level cho US QWERTY / Dvorak / Colemak), `uinput` và giao dịch clipboard đa MIME nguyên tử Wayland (`ext-data-control-v1` / `zwlr-data-control-v1`).
- [`daemon/`](daemon/): Dịch vụ nền `dboard-daemon` bắt phím `evdev`, hotplug & hợp nhất modifier đa bàn phím, nhận diện Touchpad Tap-to-Click, quản lý trạng thái theo cửa sổ focus & Layer-Shell, bảo vệ mật khẩu 4 lớp và IPC non-blocking.
- [`ui/`](ui/): Biểu tượng khay hệ thống `dboard-tray` qua giao thức StatusNotifierItem (`ksni`).
- [`ui-popup/`](ui-popup/): Thanh gợi ý ứng viên nổi tùy chọn `dboard-popup` qua Wayland `zwlr_layer_shell_v1`, vẽ chữ TrueType anti-aliased qua `swash` (tự fallback font bitmap Unicode tiếng Việt 8×16 nội sinh).
- [`cli/`](cli/): Công cụ dòng lệnh `dboard-cli` để kiểm thử nhanh bộ gõ.
- [`docs/dboard-system-architecture.md`](docs/dboard-system-architecture.md) & [`docs/dboard-gboard-parity-plan.md`](docs/dboard-gboard-parity-plan.md): Tài liệu kiến trúc chuẩn 3 tầng, 6 bất biến hệ thống và lộ trình đạt chuẩn Gboard Android.

---

## Phát triển & Kiểm thử

```bash
# Định dạng mã nguồn, kiểm tra giới hạn kích thước file & Clippy nghiêm ngặt (-D warnings)
make fmt
python3 scripts/check_source_size.py
make lint

# Chạy toàn bộ bộ kiểm thử khói CI ở chế độ headless an toàn
make test-ci-smoke

# Chạy benchmark độ trễ 100,000 phím (p50 < 2us, p99 < 10us) & độ chính xác song ngữ
make test-soak
cargo test -p dboard-engine --test bilingual_accuracy_bench -- --nocapture
```

## Ghi nhận & Tác giả gốc (Credits)

Dự án được phát triển dựa trên nền tảng và cảm hứng từ dự án gốc [**vietc**](https://github.com/vndangkhoa/vietc) của tác giả [**Võ Nguyễn Đăng Khoa (@vndangkhoa)**](https://github.com/vndangkhoa). Xin gửi lời cảm ơn chân thành tới tác giả gốc vì những đóng góp đặt nền móng cho bộ gõ tiếng Việt trên Linux.

## Giấy phép

Phát hành theo giấy phép [MIT](LICENSE).
