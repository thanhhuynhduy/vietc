# KẾ HOẠCH HOÀN THIỆN & NÂNG CẤP DBOARD ĐẠT CHUẨN GBOARD ANDROID
**Dboard Implementation Blueprint & Agent Execution Plan (Post-Audit & Hardened Roadmap)**

> **Mục đích tài liệu:** Đây là bản kế hoạch kỹ thuật chuẩn xác sau khi audit toàn bộ 5 crate (`engine/`, `daemon/`, `protocol/`, `cli/`, `ui/`), có thể giao trực tiếp cho bất kỳ kỹ sư hoặc AI coding agent nào để thực thi tuần tự mà **không đi lệch kiến trúc, không tạo bug mới/bug tiềm ẩn, không phá vỡ bảo mật**.
> **Tài liệu gốc bắt buộc đọc trước:** [`docs/dboard-system-architecture.md`](./dboard-system-architecture.md)

---

## 0. QUY TẮC BẮT BUỘC CHO AGENT THỰC THI (GUARDRAILS)

Bất kỳ AI Agent hoặc lập trình viên nào thực hiện các task trong tài liệu này **BẮT BUỘC** phải tuân thủ:

1. **Đọc trước khi sửa:** Đọc toàn bộ 6 Bất biến trong [`docs/dboard-system-architecture.md`](./dboard-system-architecture.md) (Phần 6).
2. **Không vi phạm giới hạn kích thước file (Bất biến 5 — Source Size Gate):**
   - Các file cốt lõi hiện đã sát ngưỡng cứng của `scripts/check_source_size.py`:
     - `engine/src/engine.rs`: `995 / 1,000` dòng (chỉ còn **5 dòng**)
     - `daemon/src/evdev_loop.rs`: `1,365 / 1,402` dòng (chỉ còn **37 dòng**)
     - `protocol/src/wayland_vk.rs`: `933 / 1,000` dòng (chỉ còn **67 dòng**)
     - `protocol/src/uinput_monitor/clipboard.rs`: `1,222 / 1,229` dòng (chỉ còn **7 dòng**)
     - `protocol/src/wayland_clipboard.rs`: `1,790 / 1,791` dòng (chỉ còn **1 dòng**)
   - **CẤM** nhồi thêm logic phức tạp hoặc unit test dài vào các file trên — phải tách helper/submodule riêng (ví dụ `engine/src/filename_restore.rs`, `engine/src/stat_lm.rs`, `daemon/src/hotplug.rs`, `protocol/src/wayland_vk_connect.rs`) trước khi thêm tính năng.
   - Mọi module Rust sản xuất mới phải $\le 1,000$ dòng và phải pass `python3 scripts/check_source_size.py`.
3. **Zero-Sleep & Non-Blocking trên Fast Path (Bất biến 4):**
   - Tuyệt đối không thêm `std::thread::sleep` vào `daemon/src/evdev_loop.rs` hoặc `protocol/src/wayland_vk.rs`.
   - Tuyệt đối không thêm blocking disk `fsync` hoặc `poll` timeout dài ($> 50\text{ms}$) lên luồng `evdev_loop`.
4. **Cấm sửa lỗi kiểu chắp vá (Bất biến 6):**
   - Mọi fix phải dựa trên chuyển trạng thái sạch (State Machine), quy tắc âm vị học, hoặc bảng dữ liệu có cấu trúc; KHÔNG thêm chuỗi `if word == "..."` rải rác trong `engine.rs`.
   - Không làm vỡ các hợp đồng hành vi hiện có (ví dụ: chuẩn đặt dấu kiểu cũ `hòa`/`thủy`/`khỏe`/`quỳnh`/`hươu`, chữ viết tắt `QĐ-TTg`, double-vowel continuation `teele` $\rightarrow$ `tele`, `extended_english_lexicon_is_sorted_and_collision_free_with_telex`).
5. **Bất biến Bảo mật & An toàn Tiến trình (Security Invariants):**
   - **4 lớp bảo vệ mật khẩu** (AT-SPI2 `ROLE_PASSWORD_TEXT` + grace window 150ms, Window Class, Window Title lowercase, Kernel PTY `ICANON && !ECHO` / `/proc` descendant) **KHÔNG BAO GIỜ** được suy giảm; mọi bề mặt layer-shell (Polkit, launcher) không được kế thừa cấu hình `bypass_apps`/`english_apps` của cửa sổ cũ.
   - **`UserDict`:** Tuyệt đối chỉ lưu token ASCII chữ cái thuần (`2..=32` ký tự), quyền file `0600`, không bao giờ lưu chuỗi chứa dấu cách, số, ký tự đặc biệt hay mật khẩu.
   - **Không tín hiệu nhầm PID:** Mọi `SIGKILL` trên tiến trình con phải kiểm tra quyền sở hữu `DAEMON_PID` trước khi bắn tín hiệu.
   - **Không amplification phím ảo:** Mọi giao dịch Backspace/Replace không được khuếch đại $O(N)$ khi xóa 1 ký tự từ chuỗi dán dài.
   - **Không thêm phụ thuộc IBus, Fcitx5, hay `wtype`.**
6. **Cổng nghiệm thu bắt buộc sau MỖI Task (không dồn nhiều task rồi mới test):**
   ```bash
   make fmt
   python3 scripts/check_source_size.py
   make lint
   make test-ci-smoke
   make test-soak
   ```
   Và tại ranh giới **MỖI Milestone (`M0`, `M1`, `M2`, `M3`, `M4`)** chạy thêm:
   ```bash
   make test-properties-exhaustive
   cargo audit --deny warnings
   cargo deny check
   cargo deny --manifest-path ui/Cargo.toml check
   ```
   Toàn bộ lệnh phải trả về exit code `0`, `0 warnings`, và `soak` giữ `p50 < 2.0us`, `p99 < 10.0us`.

---

## PHASE 0: NỀN TẢNG BẢO MẬT, ỔN ĐỊNH & GIẢI PHÓNG DUNG LƯỢNG (MILESTONE M0)

### Task P0.1: Hotfix Bảo mật & Ổn định Khẩn cấp (ĐÃ HOÀN THÀNH)
Các lỗi nghiêm trọng phát hiện trong phiên audit đã được vá và kiểm chứng qua 5/5 cổng nghiệm thu:
- [x] Chuẩn hóa lowercase cho tiêu đề cửa sổ trong `daemon/src/password_detector.rs` (`record_compositor_window_title`, `record_background_window_security`) để Lớp 3 bắt đúng tiêu đề viết hoa trên Wayland (`"Password"`, `"Mật Khẩu"`).
- [x] Giữ grace window 150ms cho cờ mật khẩu AT-SPI2 trong `clear_atspi_password_flag_if_non_atspi`.
- [x] Chặn rollover gõ nhanh `Space -> Ctrl+C` và `z -> Alt+Tab` kích hoạt nhầm toggle trong `daemon/src/event.rs` (`is_toggle_key`).
- [x] Chặn race `SIGKILL` nhầm PID tái sử dụng trong `ui/src/main.rs` (`stop_daemon`).
- [x] Bổ sung `!self.is_in_mod_release_cooldown()` và `Ordering::Acquire` trong `protocol/src/uinput_monitor.rs` (`send_backspace`) để đảm bảo FIFO với `uinput` `Ctrl` release.
- [x] Khôi phục Key Repeat (`value == 2`) cho mọi phím phân cách (`Enter`, `Shift+Enter`, `-`, `=`, `.`, `_`, `Shift+Space`) trong `daemon/src/evdev_loop.rs`.
- [x] Chuyển Backspace trên `pasted_prefix` sang $O(1)$ (`Replacement(1, "")`) trong `engine/src/engine.rs`, tránh khuếch đại $O(2N-1)$.
- [x] Giới hạn `prev_word` trước `word_at_caret` trong `daemon/src/surrounding_context.rs` chỉ nhận đúng 1 dấu cách `' '` để chống đếm lệch khi sửa hồi quy 2 từ.

### Task P0.2: Refactor Giải phóng Headroom & Khắc phục Nợ Kỹ thuật Audit
- **Mục tiêu:** Tạo khoảng trống an toàn cho Invariant 5 trước khi bước vào Phase A, đồng thời xử lý dứt điểm các lỗi tiềm ẩn mức Trung bình phát hiện khi audit.
- **Yêu cầu cụ thể:**
  1. **Tách module cho các file sát trần:**
     - Tách helper flush/retroactive/seed từ `engine/src/engine.rs` sang `engine/src/engine_helpers.rs` hoặc module con để đưa `engine.rs` xuống $\le 850$ dòng.
     - Tách logic hotplug & cross-device modifier từ `daemon/src/evdev_loop.rs` sang `daemon/src/hotplug.rs` để đưa `evdev_loop.rs` xuống $\le 1,200$ dòng.
     - Tách kết nối socket & discovery từ `protocol/src/wayland_vk.rs` sang `protocol/src/wayland_vk_connect.rs` để đưa `wayland_vk.rs` xuống $\le 800$ dòng.
  2. **Loại bỏ blocking disk I/O & lock thừa trên `evdev_loop`:**
     - Chuyển `save_overrides()` (`f.sync_all()`) và `write_status()`/`write_method_status()` ra luồng nền hoặc coalesce bất đồng bộ để luồng `evdev_loop` không bao giờ chạm disk I/O.
     - Hợp nhất việc đọc `shared_focus` giữa `sync_app_class` và `focus_sync.sync_on_key_press` (chỉ lock 1 lần/phím và chỉ cập nhật `injector.set_*` khi cửa sổ/trạng thái thực sự thay đổi).
     - Chuyển `dedup.observe(...)` chạy **sau** kiểm tra `if !grabbed && i != 0 { continue; }` để tránh bàn phím phụ chưa grab làm bẩn trạng thái dedup.
  3. **Sửa lỗi Config, Doctor, UserDict Shutdown & UI IPC:**
     - `daemon/src/config.rs` & `daemon/src/doctor.rs`: log cảnh báo rõ ràng khi `config.toml` lỗi cú pháp TOML (và `dboard doctor` phải báo `[WARN]`/`[FAIL]` thay vì `[OK]`); sửa macro mặc định không bị xóa trắng khi `config.toml` tồn tại không có `[macros]`; loại bỏ macro không thể khớp `"ko dc"`; sửa chuỗi hiển thị toggle `"Ctrl+alt+z"` trong `doctor.rs:207` & `main.rs:100`.
     - `daemon/src/user_dict_store.rs`: hỗ trợ flush & join gọn luồng `"dboard-dict-save"` khi daemon nhận `SIGTERM`/`SIGINT`.
     - `ui/src/main.rs` & `ui/src/tray.rs` & `ui/src/config.rs`: loại bỏ race `flock` trong `is_daemon_running()`, gọi `shutdown_handle.shutdown()` gọn khi `Quit` (không `exit(0)` đột ngột giữa D-Bus callback), xử lý `IN_IGNORED`/`IN_DELETE_SELF` cho `inotify`, escape `Exec` và ghi file nguyên tử trong `install_autostart()`.
     - `protocol/src/wayland_vk.rs:656-672`: xử lý `libc::write(...) == 0` trong `create_keymap_memfd` (tránh vòng lặp vô hạn); dùng `next_timestamp_ms()` riêng cho `KEY_LEFTSHIFT` và `KEY_RIGHTSHIFT` trong `handoff_shift_to_uinput`/`adopt_shift_from_uinput`.
- **Bộ Test bắt buộc (P0.2):**
  - Config TOML lỗi cú pháp không bị nuốt im lặng; `dboard doctor` phát hiện và báo lỗi chính xác.
  - `user_dict_store` flush sạch hàng đợi khi shutdown (không mất từ vừa One-Tap Undo).
  - `evdev_loop` chỉ gọi `injector.set_active_window` khi đổi cửa sổ (không gọi lại mỗi phím).

---

## PHASE A: KHẮC PHỤC TRIỆT ĐỂ CÁC THIẾU SÓT TỪ PHIÊN RETEST (STABILITY v1.0)

### Task A1: Khôi phục tên file có dấu chấm giữa câu tiếng Việt (`mở test.py`, `xem list.txt`, `sửa host.conf`, `test.tar.gz`)
- **Hiện tượng từ Retest:** Gõ `mowr test.py xem list.txt suwar host.conf` hiện ra `mở tét.py xem lít.txt sửa hót.còn` vì tại thời điểm gõ dấu `.`, engine chưa biết sau `.` là phần mở rộng file hay dấu chấm kết thúc câu.
- **Bất biến phải giữ:**
  - Dấu chấm kết thúc câu tiếng Việt **BẮT BUỘC** giữ nguyên (`"ăn bánh tét. hôm nay"` $\rightarrow$ `"ăn bánh tét. hôm nay"`, KHÔNG biến thành `"ăn bánh test."`).
  - **Không vi phạm Bất biến 1 (`ring_buffer`):** `engine.rs:765-779` hiện xóa sạch `ring_buffer` với mọi ký tự khác `' '` để đảm bảo `on_backspace_when_idle()` không bao giờ rehydrate vượt qua dấu `.`. **KHÔNG** đẩy entry có `trailing_delimiters == "."` vào `ring_buffer` mà không cô lập!
- **Vị trí sửa:**
  - Tạo module mới `engine/src/filename_restore.rs` (đăng ký trong `engine/src/lib.rs`).
  - Thêm 1 trường chuyên biệt `last_dot_stem: Option<DotStemEntry>` trong `Engine` (chỉ giữ stem ngay trước dấu `.`, tự xóa khi gặp khoảng trắng, ký tự khác, Backspace, hoặc focus change).
- **Thiết kế kỹ thuật chi tiết:**
  1. Định nghĩa hàm `is_known_file_extension(ext_lower: &str) -> bool` gồm các đuôi file dev/office phổ biến:
     `"py" | "rs" | "ts" | "tsx" | "js" | "jsx" | "txt" | "md" | "json" | "toml" | "yaml" | "yml" | "conf" | "cfg" | "ini" | "sh" | "bash" | "zsh" | "css" | "html" | "xml" | "sql" | "env" | "lock" | "log" | "csv" | "pdf" | "png" | "jpg" | "jpeg" | "webp" | "svg" | "gif" | "mp4" | "mp3" | "zip" | "tar" | "gz" | "deb" | "rpm" | "exe" | "so" | "dll" | "c" | "h" | "cpp" | "hpp" | "go" | "java" | "kt" | "rb" | "php"`.
  2. Khi flush một từ tại dấu `.` (với `!had_pasted_prefix`): lưu `DotStemEntry { raw_keys, visible_word, user_override_vn }` vào `self.last_dot_stem` (chỉ lưu khi `!user_override_vn`). Với mọi flush char khác `.` hoặc Backspace/Reset: xóa `self.last_dot_stem`.
  3. Khi flush một từ `ext` (tại `' '`, `Enter`, dấu câu, **hoặc một dấu `.` tiếp theo** cho tên file nhiều chấm như `test.tar.gz`) mà:
     - `self.last_dot_stem` là `Some(stem)` và `!stem.user_override_vn && !had_pasted_prefix`
     - `is_known_file_extension(&raw.to_ascii_lowercase()) == true`
     - `stem.visible_word != stem.raw_keys || previous != raw`
     - `dict.is_english_word(&stem_raw_lower) || is_tech_token(&stem_raw_lower) || is_alphanumeric_tech_token(&stem.raw_keys)`
     $\Rightarrow$ Thực hiện **Retroactive Dot-Filename Restore**:
     - `total_delete = stem.visible_word.chars().count() + 1 + prev_len` (dùng `prev_len` đồng bộ với `recovered_screen_len`, `1` là dấu `.`).
     - `insert = format!("{}.{}", stem.raw_keys, raw)` (khôi phục đồng thời cả stem và extension như `host.conf`).
     - Nếu `ch == '.'` (chuỗi nhiều chấm `test.tar.gz`), cập nhật `self.last_dot_stem` thành `ext` đã khôi phục; ngược lại xóa `self.last_dot_stem` và đặt `self.prev_context = LanguageContext::English` (trừ khi `is_sentence_boundary(ch)`).
- **Bộ Test vét cạn bắt buộc (`engine/tests/auto_restore.rs`):**
  - Chuỗi gốc: `"mowr test.py xem list.txt suwar host.conf "` $\rightarrow$ `"mở test.py xem list.txt sửa host.conf "`
  - Tên file nhiều chấm: `"giai nen archive.tar.gz "` / `"test.tar.gz "` $\rightarrow$ `"giải nén archive.tar.gz "` / `"test.tar.gz "`
  - Bảo toàn dấu câu tiếng Việt: `"aan basnh test. hoom nay "` $\rightarrow$ `"ăn bánh tét. hôm nay "`, `"ddepj qusa. "` $\rightarrow$ `"đẹp quá. "`, `"aan basnh test. so "` $\rightarrow$ `"ăn bánh tét. so "` (có dấu cách sau `.` nên tuyệt đối không coi là đuôi `.so`).
  - Bảo toàn user override VN: gõ `"tét"` với override (ví dụ `testz` hoặc One-Tap Undo về tiếng Việt) rồi `.py` không bị tự ý ghi đè trái ý user.
  - Bảo toàn Bất biến 1: `"test."` + `Backspace` + `"s"` không rehydrate sai qua dấu `.`.
  - Trường hợp dán (`pasted_prefix`) + `.py`: không xóa lệch.

---

### Task A2: Sửa lỗi camelCase 3 ký tự (`dOs`, `cAs`, `tOs`) & nháy `đ` giữa từ tiếng Anh (`add`, `reddit`, `hidden`)
- **Hiện tượng từ Retest:**
  1. `dOs`, `cAs`, `tOs` bị hạ chữ thường thành `dó`, `cá`, `tó`.
  2. Khi đang gõ tiếng Anh `add`, `reddit`, `hidden`, chữ `đ` nháy thoáng qua trước khi bấm Space.
- **Bẫy kỹ thuật đã phát hiện & cách tránh:**
  - **Bẫy 1 (`QDD-TTg`):** Cấm `dd -> đ` cứng ở index `0` sẽ làm gãy test hiện có `QDD-TTg -> QĐ-TTg` (`engine/tests/auto_restore.rs:305`) và các chữ viết tắt tiếng Việt `VNDD`, `HDD`, `NDD`.
  - **Bẫy 2 (`ddd` undo):** `telex/mark.rs:310-325` quét `raw.windows(3)` toàn từ; nếu chặn `dd` sau nguyên âm mà không đồng bộ guard cho `310-325`, gõ `addd` sẽ bị ăn mất 1 chữ `d`! Lưu ý: test `engine/src/tests.rs:103-105` (`addd -> add`) tồn tại do trước đây `add` bị biến thành `ađ`; khi `add` không bao giờ biến thành `ađ` nữa, `addd` phải giữ nguyên `"addd"` (cập nhật test sang `"ddd" -> "dd"` và `"dddi" -> "ddi"` để kiểm tra đúng hành vi `dd` undo).
  - **Bẫy 3 (`vieeTj`, `nguowIf`, `dduowNgf`):** `english.rs:258-288` cho phép 1 chữ hoa early-Shift nằm giữa từ tiếng Việt dài ($\ge 5$ ký tự). Nếu sửa `match_casing` quá hẹp sẽ làm hỏng nhóm ca early-Shift này.
- **Vị trí sửa & Thiết kế kỹ thuật:**
  1. [`engine/src/telex/mark.rs:109-117`](../engine/src/telex/mark.rs#L109-L117) & [`310-325`](../engine/src/telex/mark.rs#L310-L325):
     - Định nghĩa helper `can_apply_dd_at(raw: &[char], d_first_idx: usize) -> bool`: trả về `true` khi `!raw[..d_first_idx].iter().any(|&c| is_base_vowel(c))` (không có nguyên âm nào đứng trước cặp `dd`).
     - Áp dụng `can_apply_dd_at` cho **cả** biến đổi `dd -> đ` (`109-117`) **lẫn** hoàn tác `ddd -> dd` (`310-325`).
     - Kết quả: `ddi` $\rightarrow$ `đi`, `ddos` $\rightarrow$ `đó`, `DDANG` $\rightarrow$ `ĐANG`, `QDD-TTg` $\rightarrow$ `QĐ-TTg`, `ddd` $\rightarrow$ `dd`; còn `add`, `odd`, `reddit`, `hidden`, `middle`, `wedding`, `addd` giữ nguyên 100% ASCII, không bao giờ nháy `đ`.
  2. [`engine/src/engine_helpers.rs:299-308`](../engine/src/engine_helpers.rs#L299-L308) (`match_casing`) & [`engine/src/english.rs:258-288`](../engine/src/english.rs#L258-L288) (`has_mixed_internal_caps`):
     - Với từ ngắn $\le 4$ ký tự có chữ hoa ở giữa (không phải ký tự đầu và không phải ký tự modifier Telex ở vị trí cuối cùng, như `dOs`, `cAs`, `tOs`, `iOs`, `mAc`): `has_mixed_internal_caps` trả về `true` và `match_casing` KHÔNG hạ chữ thường.
     - Vẫn bảo toàn 100% early-Shift cho phím modifier cuối (`chaoF` $\rightarrow$ `chào`, `VieetJ` $\rightarrow$ `Việt`) và từ tiếng Việt dài (`vieeTj` $\rightarrow$ `việt`, `nguowIf` $\rightarrow$ `người`, `dduowNgf` $\rightarrow$ `đường`).
- **Bộ Test vét cạn bắt buộc (`engine/tests/gboard_parity.rs`, `engine/src/telex/tests.rs`, `engine/tests/auto_restore.rs`):**
  - 3-char camelCase: `dOs `, `cAs `, `tOs `, `iOs `, `mAc ` $\rightarrow$ giữ nguyên 100%.
  - Early-Shift VN: `chaoF ` $\rightarrow$ `chào `, `VieetJ ` $\rightarrow$ `Việt `, `cHAOF ` $\rightarrow$ `cHÀO `, `vieeTj ` $\rightarrow$ `việt `, `nguowIf ` $\rightarrow$ `người `, `dduowNgf ` $\rightarrow$ `đường `.
  - `dd` onset & viết tắt: `ddi` $\rightarrow$ `đi`, `ddos` $\rightarrow$ `đó`, `DDANG` $\rightarrow$ `ĐANG`, `ddd` $\rightarrow$ `dd`, `QDD-TTg ` $\rightarrow$ `QĐ-TTg `.
  - `dd` giữa từ tiếng Anh (không nháy `đ` ngay cả trước Space): `resolve_and_render("add") == "add"`, `"odd"`, `"reddit"`, `"hidden"`, `"middle"`, `"wedding"`, `"addd" == "addd"`.

---

### Task A3: Sửa lỗi hiển thị ký tự phân cách trong `dboard-cli` & Thống nhất Single Source of Truth `is_flush_char`
- **Hiện tượng từ Retest:** Khi chạy `/usr/bin/dboard-cli`, gõ `raw_buf` hoặc `[test]` bị hiển thị thiếu `_` và `]` trên dòng `Screen:` do `cli/src/main.rs:350-352` tự định nghĩa `is_flush_char` thiếu ký tự, `input.trim()` cắt mất dấu cách cuối, `AutoRestore` thiếu push delimiter, và `flush()` bỏ qua `EngineEvent::Replace`.
- **Vị trí sửa:**
  - [`engine/src/engine_helpers.rs:199`](../engine/src/engine_helpers.rs#L199) & [`engine/src/lib.rs`](../engine/src/lib.rs): export công khai `pub fn is_flush_char(ch: char) -> bool` làm **Single Source of Truth**.
  - [`daemon/src/event.rs`](../daemon/src/event.rs): gọi lại `dboard_engine::is_flush_char` (loại bỏ bản sao cục bộ dễ lệch).
  - [`cli/src/main.rs`](../cli/src/main.rs):
    - Đổi `input.trim()` (`line 54`) $\rightarrow$ `input.trim_end_matches(['\r', '\n'])` để bảo toàn dấu cách cuối dòng.
    - Khi nhận `EngineEvent::Replace(rep)` hoặc `EngineEvent::AutoRestore(word)` tại `dboard_engine::is_flush_char(ch)`: sau khi áp dụng thay thế, bắt buộc `output.push(ch)`.
    - Xử lý đầy đủ `EngineEvent::Replace` trong nhánh `state.engine.flush()` (`lines 140-148`).
- **Bộ Test vét cạn bắt buộc (`cli/src/main.rs`):**
  - `raw_buf [test] (nhuw) foo-bar/baz.py ` $\rightarrow$ `raw_buf [test] (như) foo-bar/baz.py `
  - Chuỗi có dấu cách cuối dòng (`"xin chaof "`), Backspace giữa từ, macro flush, và ký tự Unicode đa byte.

---

### Task A4: Đồng bộ Telex / VNI Parity & Loại bỏ lệch trạng thái `bamboo` vs `telex`
- **Các lỗi cần xử lý triệt để:**
  1. **Lệch undo `ddsd` (`engine/src/engine.rs:879-900`):**
     - Khi `self.uses_recomposition() == true`, KHÔNG gọi `self.bamboo.undo_telex_transform()` trước `telex::resolve_and_render`.
     - Cho `telex` trả về metadata `mark_undone` / `tone_undone` (từ `ResolveResult`) làm nguồn chân lý duy nhất để `Engine` phân biệt chính xác `ReplacementReason::UndoTransform` vs `ApplyTransform`.
  2. **`uses_recomposition()` khi `BoundaryState::Invalidated` (`engine/src/engine.rs:157-161`):**
     - Tách helper `uses_recomposition_backend(&self) -> bool` (chỉ kiểm tra `method == Telex && telex_backend == Recomposition`, không phụ thuộc `boundary != Invalidated`) cho `compose_candidate()` và `seed_surrounding_context()`.
  3. **VNI Parity (`engine/src/engine.rs:923-948`, `engine/src/engine_helpers.rs:16-90`):**
     - Cập nhật `active_transform` và `generation` cho `InputMethod::Vni` (`'1'..='9'`), với điều kiện `!visible.ends_with(trigger)` để phân biệt VNI undo (`a11`, `a66`, `d99` xóa `active_transform`) với đổi dấu (`a1` $\rightarrow$ `a2` phát `ReplaceTransform`).
     - **Sửa lỗi `!cased.is_ascii()` tại `engine.rs:943`:** Đổi điều kiện thành `!previous.is_ascii()` và bật `self.override_keep_vietnamese = true` cho cả `InputMethod::Telex && ch.eq_ignore_ascii_case(&'z')` **lẫn** `InputMethod::Vni && ch == '0'` (vì sau khi xóa dấu, `toan10` $\rightarrow$ `toan` và `toansz` $\rightarrow$ `toan` đều là chuỗi ASCII!).
  4. **Telex bỏ dấu mũ vượt phụ âm cuối (`engine/src/telex/mark.rs:123-140`) — Thiết kế An toàn Không Phá Tiếng Anh:**
     - Hỗ trợ gõ tự do kiểu Gboard: `khongo` $\rightarrow$ `không`, `viete` $\rightarrow$ `viêt`, `vietej` $\rightarrow$ `việt`, `tienges` $\rightarrow$ `tiếng`, `tana` $\rightarrow$ `tân` khi phần giữa 2 nguyên âm trùng là phụ âm cuối tiếng Việt hợp lệ (`n, m, ng, nh, c, ch, t, p`).
     - **3 Bẫy bắt buộc phải chặn trong thiết kế:**
       - *(a) Bảo toàn `banana`/`canada`:* Chỉ áp dụng mũ vượt phụ âm cuối khi không có nguyên âm đứng sau trigger và khi `positions.len() == 2` (hoặc kiểm tra lại undo `mark.rs:263-284` để không ăn mất chữ `'a'` thứ 2 thành `"banna"`, đồng thời giữ nguyên test `teele -> tele`).
       - *(b) Không check `vietnamese.cm.dict` thô trong `mark::resolve` trước `tone::resolve`:* Với vần đóng tắc (`-t, -c, -ch, -p`), âm tiết thanh ngang như `"viêt"` không nằm trong từ điển (chỉ `"viết"`/`"việt"` có) — phải validate bằng bộ âm vị học (`is_valid_vietnamese_syllable` / kiểm tra họ thanh sắc-nặng cho vần tắc) hoặc kiểm tra ở tầng tổng hợp sau `tone::resolve`.
       - *(c) Không phá `extended_english_lexicon_is_sorted_and_collision_free_with_telex` (`english.rs:655-686`):* Các từ tiếng Anh như `mono`, `tenet`, `benefit`, `manage`, `banana`, `delete`, `choose` phải được bảo toàn 100% qua bộ lọc tiếng Anh / auto-restore và KHÔNG làm fail test collision-free.
- **Bộ Test vét cạn bắt buộc (`engine/tests/telex_backend_contract.rs`, `engine/tests/gboard_parity.rs`, `engine/tests/auto_restore.rs`):**
  - `ddsd`, `ddsf`, `dds` không lệch trạng thái, không kẹt `undone_in_word`.
  - `seed_surrounding_context` ngay sau `Invalidated` chạy đúng `Recomposition`.
  - VNI: `a1` $\rightarrow$ `a2` phát `ReplaceTransform`; `a11`/`a66`/`d99` undo sạch; `toan10 ` giữ nguyên `"toan "` (`override_keep_vietnamese = true`); Telex `toansz ` giữ nguyên `"toan "`.
  - Telex mũ vượt phụ âm cuối: `khongo ` $\rightarrow$ `không `, `vietej ` $\rightarrow$ `việt `, `tienges ` $\rightarrow$ `tiếng `, `tana ` $\rightarrow$ `tân `, `viets ` $\rightarrow$ `viết `.
  - Bảo toàn tiếng Anh & contract cũ: `banana `, `canada `, `delete `, `choose `, `mono `, `tenet `, `benefit `, `manage `, `teele` $\rightarrow$ `tele`, và pass 100% `extended_english_lexicon_is_sorted_and_collision_free_with_telex`.

---

### Task A5: Gia cố Tầng 1 (`daemon/`) — Touchpad Tap-to-Click, Layer-Shell, Hotplug & Multi-Keyboard
- **Lưu ý Invariant 5:** Phải thực hiện sau P0.2 (đã tách `daemon/src/hotplug.rs`) để giữ `daemon/src/evdev_loop.rs` $\le 1,402$ dòng.
- **Vị trí sửa & Thiết kế an toàn:**
  1. **Touchpad Tap-to-Click (`daemon/src/pointer_watcher.rs:19-30`):**
     - Mở rộng `is_pointer_device` chấp nhận thiết bị có `BTN_LEFT || BTN_TOUCH` (tránh bỏ sót touchpad/touchscreen chỉ phát `BTN_TOUCH`).
     - Theo dõi timestamp nhấn `BTN_TOUCH` theo từng thiết bị (dọn sạch khi thiết bị ngắt kết nối tại `dead` removal).
     - Chỉ kích hoạt tap (`< 200ms`) khi **không** có chuyển động vượt ngưỡng (tránh palm brush/scroll flick qua `ABS_MT_POSITION_X/Y` hoặc `BTN_TOOL_DOUBLETAP`) và chống bắn đúp khi clickpad phát đồng thời cả `BTN_TOUCH` lẫn `BTN_LEFT`.
  2. **Layer-Shell Surfaces (`daemon/src/toplevel_tracker.rs:96-111`, `daemon/src/focus.rs:144-150`, `daemon/src/main.rs:148-186`):**
     - Thêm `Publish::FocusLost` và method `FocusSnapshot::observe_focus_lost()` (bump `focus.generation`, reset composition).
     - **Chống 3 bẫy đã audit:**
       - *(a) Alt+Tab transient `None`:* Chỉ phát `FocusLost` sau khi nhận chuỗi sự kiện đồng bộ (`Done`) mà thực sự không có toplevel nào còn `Activated`.
       - *(b) Bảo mật Polkit/Launcher:* Khi `FocusLost`, KHÔNG kế thừa `bypass_apps`/`english_apps`/`terminal_apps` từ ứng dụng trước đó (đưa về chế độ mặc định an toàn + giữ nguyên lớp bảo vệ mật khẩu AT-SPI2/title).
       - *(c) Chống thread 250ms ghi đè:* Giữ `identity = Some("toplevel:focus-lost")` để vòng lặp nền 250ms nhận biết nguồn authoritative từ Wayland.
  3. **Hotplug Retry & Composite `uniq` (`daemon/src/device.rs:65-71`, `daemon/src/hotplug.rs`):**
     - Ghép định danh `uniq:{uniq}|phys:{phys}` thay vì chỉ `uniq:{uniq}` để bàn phím composite USB/BT nhiều interface không nuốt mất interface gõ chính.
     - Khi `dev.grab()` thất bại tạm thời lúc hotplug (udev chưa kịp gán `uaccess`): **xóa `pid` khỏi `known_phys_ids`** (vì `scan_keyboard_dir` đã chèn trước đó), không push vào `devices`, và cho phép retry tối đa 5 lần (dọn bộ đếm retry khi device biến mất).
     - Chuyển chu kỳ hotplug scan sang dựa trên `Instant::now()` (mỗi 2 giây) thay vì phụ thuộc `poll_ret == 0` (xóa hoàn toàn lỗi starvation khi gõ liên tục).
  4. **Cross-Device Modifier (`daemon/src/evdev_loop.rs` & `daemon/src/hotplug.rs`):**
     - Tính trạng thái `Shift`/`Ctrl`/`Alt`/`Super` bằng phép hợp (OR) giữa `key_state` cục bộ (đang `std::mem::take` tại dòng 431) và `device_states[j].keys` với `j != i`; truyền tập modifier hợp nhất cho `execute_with_shift_released` và `is_toggle_combination_state`.
     - Giữ `device_states[i].keys` tách riêng cho `release_disconnected_device_keys`, và chỉ nhả modifier khi rút phím nếu **không còn bàn phím kết nối nào khác** đang giữ modifier đó.
  5. **Sửa fallback `wlrctl` (`daemon/src/app_state.rs:244-263`):**
     - Gọi `wlrctl toplevel list state:focused` (không dùng flag `--format` không phổ biến giữa các distro) và parse định dạng chuẩn `<app_id>: <title>`.
- **Bộ Test vét cạn bắt buộc (`daemon/src/` unit tests & `daemon/tests/`):**
  - `pointer_watcher`: tap ngắn `< 200ms` kích hoạt; giữ kéo `>= 200ms`, cuộn 2 ngón, palm/move vượt ngưỡng KHÔNG kích hoạt; clickpad `BTN_TOUCH + BTN_LEFT` chỉ kích hoạt 1 lần; rút thiết bị dọn sạch state.
  - `toplevel_tracker`: `FocusLost` bump generation, không kế thừa `bypass_apps` của game/VM sang Polkit/launcher, không bị Alt+Tab transient làm loạn.
  - `device` / `hotplug`: composite USB/BT cùng `uniq` khác `phys` được mở đủ; `grab()` lỗi tạm thời retry thành công ở lần sau; ngắt kết nối khi bàn phím thứ 2 vẫn giữ `Shift` không nhả `Shift` sớm.
  - `app_state`: parse chuẩn output `wlrctl toplevel list state:focused`.

---

### Task A6: Gia cố Tầng 3 (`protocol/`) — Rich Clipboard, `GlobalRemove`, `CapsLock` LED & `Mod3`/`Mod5`
- **Lưu ý Invariant 5:** Phải tách helper/test ra submodule (`wayland_vk_connect.rs`, `clipboard_backup.rs`, ...) vì `wayland_vk.rs` (933/1000), `clipboard.rs` (1222/1229), và `wayland_clipboard.rs` (1790/1791) đều sát trần.
- **Vị trí sửa & Thiết kế an toàn:**
  1. **Bảo toàn Rich Clipboard (`protocol/src/uinput_monitor/clipboard_backup.rs`, `clipboard.rs`, `wayland_clipboard.rs`):**
     - Lưu danh sách các cặp `(mime, bytes)` ưu tiên (ví dụ `text/html` + `text/plain;charset=utf-8`, hoặc `image/png`, `text/uri-list`) trong giới hạn `MAX_BINARY_CLIPBOARD_BYTES`.
     - **Bẫy `wl-copy`:** `wl-copy` chỉ set được 1 MIME/lần gọi (nếu chỉ restore `text/html` sẽ hỏng paste vào terminal). Phải restore đa MIME qua kênh native `ext_data_control` / `zwlr_data_control` của `wayland_clipboard.rs` (chỉ fallback `wl-copy` khi không có data-control).
     - Xóa lỗi gọi `clipboard_types()` 2 lần liên tiếp (TOCTOU) trong `snapshot_wayland_clipboard()`.
  2. **Xử lý `GlobalRemove` & Stale Wayland Socket (`protocol/src/wayland_vk_connect.rs`, `wayland_vk.rs`, `wayland_clipboard.rs`):**
     - Lưu `seat_global_name: Option<u32>`, `manager_global_name: Option<u32>` và `global_removed: bool` trong `VkDiscovery`; khi `wl_registry::Event::GlobalRemove { name }` khớp 1 trong 2 ID, đặt `global_removed = true` và `VkSession::flush_and_drain()` đánh dấu `healthy = false` để daemon reconnect sạch.
     - Gộp bước probe socket + connect thành **1 kết nối duy nhất** (tránh tạo kết nối rác tới compositor).
     - Xử lý `EINPROGRESS` trong `connect_unix_nonblocking()` bằng `libc::poll(POLLOUT, 50ms)` (**tuyệt đối KHÔNG dùng 500ms** vì chạy trên luồng `evdev_loop`, 500ms × 10 socket sẽ làm đơ bàn phím 5 giây!) + `getsockopt(SO_ERROR)`. Đồng bộ fix tương tự cho `protocol/src/wayland_clipboard.rs:252-311`.
  3. **Tránh nháy LED `CapsLock` & gói `modifiers` thừa (`protocol/src/wayland_vk.rs:463-500`):**
     - Chỉ tắt `XKB_MOD_LOCK` khi `sess.caps_lock == true` **và** `text` thực sự chứa ký tự chịu ảnh hưởng bởi `Lock` (nhóm `ALPHABETIC` / `DBOARD_EIGHT_LEVEL`, không áp dụng cho `text.is_empty()` như `send_backspace()`).
     - Guard lệnh gửi `vk.modifiers` bằng `if sess.phys_depressed() != 0 || edit_locked != sess.phys_locked()` để loại bỏ hoàn toàn gói `modifiers` thừa trên 99% đường chạy bình thường (`caps_lock == false && shift == false`).
  4. **Giảm xung đột `Mod3`/`Mod5` trên XWayland / Kitty (`protocol/src/wayland_vk_keymap.rs:265-344`, `daemon/src/doctor.rs`):**
     - Bổ sung đầy đủ `interpret` / `virtualModifier` cho `Alt`, `Super`, `Meta`, `Caps_Lock` trong `xkb_compatibility`.
     - Xác minh bằng test động `libxkbcommon` (`wayland_vk_keymap.rs:440-549`) rằng chỉ số bit của `Mod1..Mod5` (`3..=7`, đặc biệt `XKB_MOD_MOD3 = 1<<5`, `XKB_MOD_MOD5 = 1<<7`) giữ nguyên 100% và toàn bộ 134 ký tự tiếng Việt vẫn ánh xạ đúng.
     - Bổ sung cảnh báo trong `dboard doctor` nếu phát hiện cấu hình bind phím tắt compositor với `Mod3`/`Mod5`.
- **Bộ Test vét cạn bắt buộc (`protocol/`):**
  - Rich clipboard: roundtrip đa MIME (`text/html` + `text/plain`), ảnh `image/png`, file `text/uri-list`, không gọi `clipboard_types()` 2 lần, giới hạn `MAX_BINARY_CLIPBOARD_BYTES` an toàn.
  - Wayland socket: bỏ qua stale socket file chết, xử lý `EINPROGRESS` trong $\le 50\text{ms}$, `GlobalRemove` đánh dấu `healthy = false` và tự phục hồi.
  - `CapsLock`: không gửi thừa `vk.modifiers` khi `caps_lock == false` hoặc `send_backspace()`; khi `caps_lock == true` gõ tiếng Việt hoa/thường chính xác không mất trạng thái `CapsLock`/`NumLock`.
  - `libxkbcommon`: kiểm tra `vmodmap` trả về giá trị khác `0` cho `Alt`/`Super`/`Meta`/`NumLock` và toàn bộ 134 ký tự tiếng Việt trong dải keycode an toàn `1..=111`.

---

## PHASE B: NÂNG CẤP ĐẠT CHUẨN TƯƠNG ĐƯƠNG GBOARD ANDROID

### Task B1: Quantized Statistical Bigram/Trigram Language Model (Thay thế bảng rule cứng ~250 cặp)
- **Mục tiêu:** Đưa độ chính xác song ngữ Việt–Anh của Dboard lên tiệm cận Gboard bằng mô hình xác suất thống kê thực sự, trong khi vẫn **zero-allocation, $O(1)$ lookup, binary `< 350KB`, `p50 < 2.0us`, `p99 < 10.0us`**.
- **Kiến trúc đề xuất:**
  1. Tạo script offline [`scripts/build_lm.py`](../scripts/build_lm.py) (chạy lúc phát triển, deterministic/reproducible, KHÔNG chạy ở runtime):
     - Đầu vào: Corpus tiếng Việt (Wikipedia VI + báo chí + văn phòng) + Corpus tiếng Anh/Dev (Wikipedia EN + tài liệu lập trình/Linux).
     - Xuất ra file nhúng nhị phân `engine/src/bilingual_lm.bin` (tối đa ~250–350 KB):
       - Header magic + version + checksum + table sizes (validate lúc load trong test).
       - **Unigram quantized log-prob (`u8`)** cho các âm tiết va chạm Việt–Anh (~2,000 từ giao thoa): `(P_vn(w), P_en(w))`.
       - **Bigram quantized log-prob (`u8`)** lưu bằng open-addressing hash table dùng FNV-1a 64-bit key `hash(w1, w2)`:
         - `P_vn(w2 | w1)` cho top ~30,000 cặp tiếng Việt phổ biến.
         - `P_en(w2 | w1)` cho top ~20,000 cặp tiếng Anh/Dev phổ biến.
         - `P_code_switch(en_w2 | vn_w1)` cho top ~5,000 cặp Việt–Anh phổ biến (`chạy test`, `mở file`, `đọc docs`, `cài app`, `push code`...).
  2. Tạo module mới `engine/src/stat_lm.rs` ($\le 400$ dòng, zero `unsafe`, zero heap allocation ở hot path):
     - Nạp `static LM_BYTES: &[u8] = include_bytes!("bilingual_lm.bin");` (zero startup parse time!).
     - Hàm `score_bilingual(prev2, prev1, cand_vn, cand_en) -> i32`: tính hiệu điểm log-prob $\Delta = \log P_{EN} - \log P_{VN}$ (dương $\Rightarrow$ khôi phục tiếng Anh, âm $\Rightarrow$ giữ tiếng Việt).
     - Giữ tầng ưu tiên 1 (deterministic high-precision overrides trong `lm.rs`/`bigram.rs`) và dùng `stat_lm.rs` làm tầng ưu tiên 2 cho toàn bộ phần còn lại.
- **Bộ Test & Benchmark vét cạn bắt buộc (`engine/tests/bilingual_accuracy_bench.rs`):**
  - $\ge 500$ câu song ngữ Việt–Anh thực tế: văn phòng, lập trình (Linux/Git/Rust/JS/Docker), chat đời thường, chuyển mã giữa câu (code-switching), từ đồng âm va chạm (`test/tét`, `last/lát`, `more/mỏ`, `list/lít`, `host/hót`, `case/cáe`, `push/púh`, `server/sẻver`...).
  - Yêu cầu độ chính xác $\ge 97\%$, `bilingual_lm.bin < 350KB`, zero regression trên toàn bộ test hiện có, và `make test-soak` giữ `p50 < 2.0us`, `p99 < 10.0us`.

---

### Task B2: Hoàn thiện Spatial Autocorrect cho VNI & One-Tap Undo cho Retroactive 2-Word Rewrite
- **Vị trí sửa:** [`engine/src/autocorrect.rs`](../engine/src/autocorrect.rs), [`engine/src/engine.rs`](../engine/src/engine.rs) (qua helper trong `engine_helpers.rs` để giữ Invariant 5).
- **Thiết kế kỹ thuật (đã khắc phục 2 bẫy từ audit):**
  1. Trong `autocorrect.rs`:
     - Mở rộng `key_coord(ch: char)` thêm hàng phím số US QWERTY: `'1' => (-1.0, 0.0)` đến `'0' => (-1.0, 9.0)`.
     - Tạo bảng `QWERTY_VNI_DIGITS: [char; 9] = ['1', '2', '3', '4', '5', '6', '7', '8', '9']` (loại trừ `'0'` vì `'0'` là phím xóa dấu VNI).
     - **Chặn bẫy cross-row chữ $\leftrightarrow$ số:** Vì hàng số (`row -1.0`) cách hàng chữ `q..p` (`row 0.0`) đúng `1.0 <= 1.25`, `try_vn_spatial_substitution` **BẮT BUỘC** tách bạch: chỉ sửa `orig.is_ascii_digit()` sang `QWERTY_VNI_DIGITS` (khi `method == InputMethod::Vni`, `key_distance <= 1.25`: `6` $\leftrightarrow$ `7`, `1` $\leftrightarrow$ `2`, `5` $\leftrightarrow$ `6`, `8` $\leftrightarrow$ `9`), và chỉ sửa `orig.is_ascii_alphabetic()` sang `QWERTY_ALPHA`. Tuyệt đối không biến chữ thành số hay số thành chữ.
  2. Trong `engine.rs` (nhánh `evaluate_retroactive_first_word` dòng `701-762`) & `handle_backspace` (`384-411`):
     - Mở rộng `AutoRestoreUndo` thành enum 2 biến thể sạch: `SingleWord { ... }` và `RetroactivePair { first_original, first_rewritten, second_entry }` (không nhét chuỗi chứa dấu cách vào `raw_buffer` hay `user_dict`!).
     - Khi user bấm `Backspace` ngay sau `Space` của một sửa hồi quy 2 từ (`"Lats nuwax "` $\rightarrow$ `"Lát nữa "`): xóa cặp 2 từ trên màn hình, viết lại `"Last nữa"`, khôi phục đúng `w1` (`"Last"`) vào `ring_buffer` và đặt con trỏ sạch ở cuối `w2` (`"nữa"`), **không** ghi chuỗi có dấu cách vào `user_dict`.
- **Bộ Test vét cạn bắt buộc (`engine/tests/gboard_parity.rs`, `engine/src/autocorrect.rs`):**
  - VNI spatial autocorrect: trượt phím số dấu (`6`/`7`, `1`/`2`, `5`/`6`, `8`/`9`) sửa chuẩn + One-Tap Undo về gốc khi bấm `Backspace`.
  - Cross-row safety: phím chữ hàng trên (`q, w, e, r, t, y, u, i, o, p`) KHÔNG bao giờ bị sửa thành số (`1..9`) và ngược lại.
  - Retroactive 2-word One-Tap Undo: `"Lats nuwax "` $\rightarrow$ `"Lát nữa "` + `Backspace` $\rightarrow$ `"Last nữa"` (và gõ tiếp từ thứ 3 không lệch buffer, `user_dict` không chứa chuỗi có dấu cách).

---

### Task B3: Hoàn thiện Smart Auto-Capitalization (Abbreviation Guard & Closing Quotes)
- **Vị trí sửa:** [`engine/src/user_dict.rs:208-269`](../engine/src/user_dict.rs#L208-L269) (`AutoCapState`).
- **Thiết kế kỹ thuật:**
  1. **Closing Quotes/Brackets (kể cả dấu ngoặc thông minh Unicode):** Khi `pending_sentence_end == true` và gặp `'"' | '\'' | ')' | ']' | '»' | '”' | '’'`, giữ nguyên `pending_sentence_end = true` (KHÔNG `reset()`), để `"Chào." hôm` và `“Chào.” hôm` đều tự viết hoa thành `"Chào." Hôm` / `“Chào.” Hôm`.
  2. **Abbreviation Guard:** Truyền từ vừa commit trước dấu `.` vào `on_flush_char(ch, prev_word)`: nếu `ch == '.'` và `prev_word.to_lowercase()` nằm trong bảng viết tắt (`"mr"`, `"mrs"`, `"ms"`, `"dr"`, `"prof"`, `"st"`, `"vs"`, `"etc"`, `"eg"`, `"ie"`, `"tp"`, `"ts"`, `"ths"`, `"pgs"`, `"gs"`, `"ks"`, `"bs"`, `"v"`, `"vv"`), KHÔNG bật `pending_sentence_end = true`.
- **Bộ Test vét cạn bắt buộc (`engine/src/user_dict.rs`, `engine/tests/gboard_parity.rs`):**
  - `"\"Chào.\" hôm "` $\rightarrow$ `"\"Chào.\" Hôm "`
  - `"“Chào.” hôm "` $\rightarrow$ `"“Chào.” Hôm "`
  - `"(Xong rồi!) tiếp "` $\rightarrow$ `"(Xong rồi!) Tiếp "`
  - `"TP. hcm "`, `"Mr. nam "`, `"TS. nguyễn "`, `"PGS. trần "`, `"etc. tiếp "` KHÔNG tự viết hoa từ sau dấu chấm (trừ khi user tự nhấn Shift).
  - Sau `Enter` vẫn tự viết hoa đầu dòng mới bình thường.

---

### Task B4: Thanh Gợi ý Ứng viên Tùy chọn (Optional Floating Candidate Strip — Wayland Layer-Shell)
- **Nguyên tắc kiến trúc:**
  - **Tắt mặc định** (`candidate_bar = false`) để giữ 100% trải nghiệm Zero-Underline tối giản.
  - **Không nhúng UI vào `dboard-daemon`:** Tách thành binary/crate độc lập `ui-popup/` dùng `zwlr_layer_shell_v1` (`keyboard_interactivity = None`, tuyệt đối không cướp keyboard focus).
  - Giao tiếp 1 chiều non-blocking từ `dboard-daemon` sang `dboard-popup` qua Unix Datagram Socket trong `$XDG_RUNTIME_DIR/dboard-candidates.sock` (chế độ `O_NONBLOCK`, quyền socket `0600`, gửi mất gói thì bỏ qua, **tuyệt đối không chặn `evdev_loop`**).
  - **Bảo mật:** Khi đang ở ô mật khẩu (`password_detection`) hoặc app bypass (`bypass_apps`), gửi gói ẩn thanh gợi ý ngay lập tức và **tuyệt đối không gửi nội dung buffer** qua socket.
  - Hiển thị top 3 ứng viên: `[1] từ tiếng Việt | [2] từ gốc ASCII | [3] gợi ý sửa lỗi/từ ghép`, cho phép chọn nhanh bằng phím tắt cấu hình hoặc tắt hẳn.
- **Bộ Test vét cạn bắt buộc:**
  - `candidate_bar = false`: zero socket syscall, zero latency impact.
  - Non-blocking IPC: socket đầy hoặc popup treo/chết KHÔNG làm chậm hay block `evdev_loop`.
  - Bảo mật: khi bật chế độ mật khẩu, popup nhận lệnh ẩn và không bao giờ nhận ký tự mật khẩu.

---

### Task B5: Mở rộng Đa Bố cục Bàn phím (US QWERTY / Dvorak / Colemak)
- **Đa bố cục bàn phím (Dvorak / Colemak / US QWERTY):**
  - Mở rộng [`protocol/src/layout.rs`](../protocol/src/layout.rs) thành enum `KeyboardLayout { UsQwerty, Dvorak, Colemak }` cấu hình qua `keyboard_layout = "us" | "dvorak" | "colemak"` trong `dboard.toml`.
  - Đồng bộ tọa độ `key_coord_for_layout()` trong `autocorrect.rs` và bảng `build_xkb_keymap_for_layout()` trong `wayland_vk_keymap.rs` (kiểm chứng động qua `libxkbcommon` cho cả 3 layout và xử lý `Lock` cho toàn bộ phím `DBOARD_EIGHT_LEVEL`); cập nhật `dboard doctor` đọc layout thực tế thay vì hardcode `[OK] us`.

---

## PHASE C: NÂNG CẤP TIỆM CẬN 1-1 VỚI GBOARD DESKTOP (MILESTONE M5 & M6)

> **Lưu ý kiến trúc (2 Tầng Tốc độ — Giữ vững Bất biến 4):**
> - **Fast Path (mỗi phím gõ trong từ):** Giữ nguyên độ trễ `p50 < 2.0us`, `p99 < 10.0us`, zero-allocation, zero-sleep.
> - **Smart Path (chỉ chạy tại ranh giới `Space` hoặc trên luồng nền không chặn `evdev_loop`):** Cho phép mô hình ngôn ngữ giàu hơn và SymSpell chạy trong `< 300us` mà không bao giờ làm chậm cảm giác gõ phím.
> - **Trade-off dung lượng:** Các mô hình 3-gram/4-gram lượng tử hóa và font vector có thể nâng dung lượng data gói cài đặt từ ~350 KB lên ~5–10 MB (vẫn cực nhẹ so với Gboard ~150 MB).

### Task C0: Hoàn thiện Đóng gói, Hot-Reload Layout & Điều khiển trên `dboard-tray` (Post-Audit Polish)
- **Mục tiêu:** Khắc phục các điểm thiếu sót nhỏ phát hiện trong đợt audit sau M4 để trải nghiệm cài đặt & sử dụng thực tế liền mạch 100%.
- **Yêu cầu cụ thể:**
  - [ ] **Hot-reload & cảnh báo `keyboard_layout` ([`daemon/src/daemon.rs`](../daemon/src/daemon.rs)):** Cập nhật `self.engine.set_keyboard_layout_name(&new_config.keyboard_layout)` trong `reload_config()` và log cảnh báo rõ ràng (hoặc tái tạo XKB keymap) khi người dùng đổi `keyboard_layout` lúc daemon đang chạy.
  - [ ] **Đóng gói `dboard-popup`:** Tích hợp build/install `dboard-popup` vào [`install.sh`](../install.sh), `packaging/deb/build-deb.sh`, `packaging/build-tarball.sh`, và tùy chọn quản lý/tự khởi động khi `candidate_bar = true`.
  - [ ] **Điều khiển nhanh trên [`dboard-tray`](../ui/src/tray.rs):** Bổ sung menu bật/tắt trực tiếp `Autocorrect`, `Auto-Capitalize`, `Candidate Bar`, và chọn `Keyboard Layout` (`US QWERTY` / `Dvorak` / `Colemak`) ngay trên khay hệ thống (ghi nguyên tử vào `~/.config/dboard/config.toml`).
- **Bộ Test bắt buộc:**
  - `reload_config()` đồng bộ `keyboard_layout` sang `Engine`; `test_installer.py` & `package-smoke-test.sh` xác minh sự hiện diện của `dboard-popup`.

---

### Task C1: Nâng cấp Language Model (Quantized 3-gram/4-gram hoặc Tiny Int8 Neural LM) & Next-Word Prediction
- **Mục tiêu:** Đưa độ chính xác song ngữ Việt–Anh lên `> 99%`, chấm điểm ngữ cảnh câu thực sự cho Autocorrect và mở khóa **Dự đoán Từ Tiếp Theo (Next-Word Prediction)** chuẩn Gboard.
- **Yêu cầu cụ thể:**
  - [ ] Mở rộng [`scripts/build_lm.py`](../scripts/build_lm.py) xuất mô hình **Quantized Kneser-Ney 3-gram/4-gram** (nén trie ~3–8 MB) hoặc **Tiny Int8 GRU/Transformer** pure-Rust SIMD, vẫn giữ checksum & integrity gate.
  - [ ] Tách module mới (ví dụ `engine/src/ngram_lm.rs` $\le 1,000$ dòng, giữ vững Bất biến 5) cung cấp 2 API:
    - `score_context(prev_words, candidate) -> i32`: chấm điểm xếp hạng ứng viên theo ngữ cảnh câu.
    - `predict_next_words(prev_words, limit: 3) -> Vec<String>`: gợi ý top 3 từ tiếp theo ngay sau khi nhấn `Space` (ví dụ `xin ` $\rightarrow$ `chào | lỗi | cảm`), đẩy lên `dboard-popup` để chọn nhanh bằng `Tab` hoặc `Alt+1/2/3`.
  - [ ] Đảm bảo Fast Path khi gõ từng phím vẫn `p50 < 2.0us`, `p99 < 10.0us`, và tổng thời gian chạy tại `Space` `< 300us`.
- **Bộ Test bắt buộc:**
  - `bilingual_accuracy_bench` đạt `>= 99.0%` câu; benchmark Next-Word Prediction top-3 hit-rate trên corpus tiếng Việt văn phòng/chat; `soak_perf` không thoái lui.

---

### Task C2: Weighted Spatial SymSpell Autocorrect (Đa Lỗi & Tách Từ Dính)
- **Mục tiêu:** Vượt qua giới hạn chỉ sửa đơn ứng viên của [`engine/src/autocorrect.rs`](../engine/src/autocorrect.rs), xử lý được các lỗi gõ nhanh phổ biến nhất như Gboard nhưng vẫn giữ nguyên kỷ luật Zero-False-Positive cho lập trình viên.
- **Yêu cầu cụ thể:**
  - [ ] Tách module `engine/src/symspell.rs` ($\le 1,000$ dòng) hỗ trợ tra cứu edit distance `1..=2` trong $O(1)$ kết hợp trọng số khoảng cách phím vật lý (`key_coord_for_layout`) và điểm ngữ cảnh từ Task C1:
    - **Thiếu ký tự (Deletion):** `khog` $\rightarrow$ `không`, `nguoi` $\rightarrow$ `người`, `vietnma` $\rightarrow$ `vietnam`.
    - **Thừa ký tự (Insertion):** `khonhgs` $\rightarrow$ `không`.
    - **Nhầm 2 phím gần nhau** trên US QWERTY / Dvorak / Colemak.
    - **Tách từ dính (Missing Space):** `khongbiet` $\rightarrow$ `không biết`, `xinchao` $\rightarrow$ `xin chào` khi vượt ngưỡng tin cậy cao.
  - [ ] **Giữ chặt Zero-False-Positive Guard:** Không sửa token trong `UserDict`, từ điển tiếng Anh/Việt hợp lệ, ALL_CAPS, chuỗi có số/ký hiệu code, hoặc khi đang ở `terminal_apps`/`english_apps`; mọi sửa lỗi bắt buộc đi kèm **One-Tap Undo** qua `Backspace`.
- **Bộ Test bắt buộc:**
  - Bộ test đa lỗi (thiếu/thừa/sai 2 phím/tách từ dính) + One-Tap Undo + tập adversarial code/identifier (`kubectl`, `systemd`, `hashmap`, `serde`) phải giữ nguyên 100%.

---

### Task C3: Nâng cấp `dboard-popup` Thành Floating Bar Chuẩn Gboard (Bám Caret, Vector Font & Click Chuột)
- **Mục tiêu:** Biến [`ui-popup/`](../ui-popup/) từ thanh pill bitmap cố định ở đáy màn hình thành thanh gợi ý nổi sắc nét, bám sát con trỏ văn bản và tương tác được bằng chuột.
- **Yêu cầu cụ thể:**
  - [ ] **Caret Tracking qua AT-SPI2:** Mở rộng [`daemon/src/focus_tracker.rs`](../daemon/src/focus_tracker.rs) gọi `Text::GetCharacterExtents(caret_offset, SCREEN_COORDS)` (non-blocking trên luồng AT-SPI2 nền) và đính kèm tọa độ tùy chọn `"caret":{"x":..,"y":..,"h":..}` vào gói JSON của [`candidate_ipc.rs`](../daemon/src/candidate_ipc.rs).
  - [ ] **Neo thông minh trong [`ui-popup/src/wayland_layer.rs`](../ui-popup/src/wayland_layer.rs):** Khi có tọa độ caret hợp lệ, đặt `Anchor::Top | Anchor::Left` + `set_margin(y + h + 6, 0, 0, x)` để nổi ngay dưới chữ đang gõ (tự kẹp trong biên màn hình); tự động fallback về `Anchor::Bottom` khi ứng dụng không cung cấp tọa độ caret.
  - [ ] **Vector Font & HiDPI:** Thay bitmap 8×16 bằng renderer nhẹ (`fontdue`/`ab_glyph` + `tiny-skia`, tuyệt đối không phụ thuộc GTK/Qt), hỗ trợ HiDPI fractional scaling (`wp_fractional_scale_v1`).
  - [ ] **Chọn ứng viên bằng chuột (`wl_pointer`):** Cho phép click trực tiếp vào khe `[1]`/`[2]`/`[3]` trên popup (gửi phản hồi an toàn về daemon qua socket cục bộ `0600`) mà vẫn giữ `KeyboardInteractivity::None`.
- **Bộ Test bắt buộc:**
  - Parse gói JSON có/không có `caret`; clamping tọa độ không tràn màn hình; giữ nguyên bảo mật `0600` và ẩn ngay lập tức khi nhập mật khẩu.

---

### Task C4: Cá nhân hóa Sâu — Personal Bigram & Adaptive Ranking (`UserDict`)
- **Mục tiêu:** Giúp bộ gõ thích nghi với văn phong, tên riêng người Việt và thuật ngữ công việc của từng người dùng như Gboard.
- **Yêu cầu cụ thể:**
  - [ ] Mở rộng [`engine/src/user_dict.rs`](../engine/src/user_dict.rs) hỗ trợ học **cụm 2 từ cá nhân** (Personal Bigram, ví dụ tên riêng tiếng Việt, cụm thuật ngữ hay gõ) với giới hạn dung lượng cứng (LRU/FIFO eviction) và lưu nguyên tử quyền `0600` qua [`user_dict_store.rs`](../daemon/src/user_dict_store.rs).
  - [ ] Tích hợp trọng số Personal Bigram vào xếp hạng Autocorrect (Task C2) và Next-Word Prediction (Task C1).
  - [ ] **Bất biến Bảo mật:** Tuyệt đối không học từ/cụm từ khi `password_detection` đang bật, trong `bypass_apps`, `terminal_apps`, hoặc chuỗi chứa số/ký tự đặc biệt.
- **Bộ Test bắt buộc:**
  - Học & ưu tiên cụm 2 từ cá nhân sau `>= 2` lần dùng; kiểm chứng eviction khi đầy bộ nhớ; kiểm chứng không bao giờ lưu mật khẩu/số/ký tự đặc biệt.

---

### Task C5: Sửa Nhanh Từ Giữa Câu Khi Quay Lại Con Trỏ (Mid-Sentence Re-Selection)
- **Mục tiêu:** Mang trải nghiệm "chạm vào từ cũ để đổi ứng viên" của Gboard lên desktop.
- **Yêu cầu cụ thể:**
  - [ ] Mở rộng [`daemon/src/surrounding_context.rs`](../daemon/src/surrounding_context.rs) và `Engine`: khi người dùng click chuột hoặc dùng phím mũi tên đưa con trỏ vào một từ đã commit trước đó (và `candidate_bar = true`), daemon tính toán các ứng viên thay thế hợp lý (ví dụ `tét` $\leftrightarrow$ `test`, `lát` $\leftrightarrow$ `last`, hoặc gợi ý SymSpell) và đẩy lên `dboard-popup`.
  - [ ] Cho phép bấm `Alt+1/2/3` (hoặc click trên popup) để thay thế ngay từ dưới con trỏ mà không cần xóa gõ lại.
- **Bộ Test bắt buộc:**
  - Kiểm chứng thay thế chính xác khi con trỏ ở cuối từ/giữa từ, không lệch ký tự khi gặp dấu câu hoặc Unicode đa byte, và tự ẩn sạch khi rời khỏi từ hoặc vào ô mật khẩu.

---

## MA TRẬN KIỂM THỬ VÉT CẠN & KẾT QUẢ NGHIỆM THU (VERIFIED EXECUTION STATUS)

| Milestone | Bao gồm Tasks | Trạng thái | Kết quả Nghiệm thu Thực tế |
|---|---|---|---|
| **M0 — Security & Headroom Baseline** | `P0.1`, `P0.2` | ✅ **HOÀN THÀNH** | 5 cổng nghiệm thu PASS + `cargo audit`/`deny` sạch + `engine.rs = 844/850`, `evdev_loop.rs = 948/1200`, `wayland_vk.rs = 766/800` |
| **M1 — Retest Zero-Defect** | `A1`, `A2`, `A3` | ✅ **HOÀN THÀNH** | `filename_restore.rs` (`test.py`, `host.conf`, `test.tar.gz`), `can_apply_dd_at` (`add`/`reddit` không nháy `đ`, giữ `QĐ-TTg`), 3-char camelCase (`dOs`/`cAs`/`tOs`), SSOT `is_flush_char` & `dboard-cli` PASS 100% |
| **M2 — Engine & Hardware Rock-Solid (v1.0)** | `A4`, `A5`, `A6` | ✅ **HOÀN THÀNH** | Telex/VNI parity + mũ vượt phụ âm cuối (`khongo -> không`, `vietej -> việt`), Touchpad Tap-to-Click, Layer-Shell `FocusLost` reset, hotplug retry + composite `uniq|phys`, cross-device modifier, rich clipboard đa MIME, `GlobalRemove`, 50ms `EINPROGRESS`, Dvorak+CapsLock unlock, `libxkbcommon` 134 ký tự PASS 100% |
| **M3 — Gboard Intelligence** | `B1`, `B2`, `B3` | ✅ **HOÀN THÀNH** | `stat_lm.rs` + `bilingual_lm.bin` (`348,184 B < 350 KB`, Leipzig Corpora): **97.25% câu (496/510), 99.79% từ (7206/7221)**; VNI spatial autocorrect + `RetroactivePair` One-Tap Undo; Smart Auto-Cap; `soak_perf` `p50 = 1.41us`, `p99 = 6.63us` |
| **M4 — Gboard UX & Multi-Layout** | `B4`, `B5` | ✅ **HOÀN THÀNH** | `ui-popup` (`dboard-popup` Wayland Layer-Shell + font Unicode tiếng Việt 8×16 nội sinh `font.rs` + 3 khe ứng viên `Alt+1`/`Alt+2`/`Alt+3` nhả `Alt` an toàn + ẩn tuyệt đối khi nhập mật khẩu); hỗ trợ đầy đủ `US QWERTY`, `Dvorak`, `Colemak` |
| **M5 — Gboard Desktop UX, Caret Tracking & Packaging** | `C0`, `C3`, `C5` | ⏳ **CHƯA THỰC HIỆN (TODO)** | Hot-reload `keyboard_layout`, đóng gói `dboard-popup` + menu `dboard-tray`, popup bám caret (AT-SPI2) + vector font + click chuột, sửa nhanh từ giữa câu |
| **M6 — Gboard Next-Gen Intelligence (1-1 Parity)** | `C1`, `C2`, `C4` | ⏳ **CHƯA THỰC HIỆN (TODO)** | Quantized 3-gram/4-gram (hoặc Tiny Int8 LM) + Next-Word Prediction (`>= 99%` song ngữ), Weighted Spatial SymSpell đa lỗi & tách từ dính, Personal Bigram trong `UserDict` |
