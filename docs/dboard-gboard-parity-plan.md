# KẾ HOẠCH HOÀN THIỆN & NÂNG CẤP DBOARD ĐẠT CHUẨN GBOARD ANDROID
**Dboard Implementation Blueprint & Agent Execution Plan (Post-Retest Roadmap)**

> **Mục đích tài liệu:** Đây là bản kế hoạch kỹ thuật chi tiết, có thể giao trực tiếp cho bất kỳ kỹ sư hoặc AI coding agent nào để thực thi tuần tự mà **không đi lệch kiến trúc**.
> **Tài liệu gốc bắt buộc đọc trước:** [`docs/dboard-system-architecture.md`](./dboard-system-architecture.md)

---

## 0. QUY TẮC BẮT BUỘC CHO AGENT THỰC THI (GUARDRAILS)

Bất kỳ AI Agent hoặc lập trình viên nào thực hiện các task trong tài liệu này **BẮT BUỘC** phải tuân thủ:

1. **Đọc trước khi sửa:** Đọc toàn bộ 6 Bất biến trong [`docs/dboard-system-architecture.md`](./dboard-system-architecture.md) (Phần 6).
2. **Không vi phạm giới hạn kích thước file (Bất biến 5):**
   - `engine/src/engine.rs` hiện đã rất sát ngưỡng `1,000` dòng. **CẤM** nhồi thêm logic phức tạp vào `engine.rs` — phải tách helper/module riêng (ví dụ `engine/src/engine_helpers.rs`, `engine/src/filename_restore.rs`, `engine/src/stat_lm.rs`).
   - Mọi module Rust sản xuất mới phải $\le 1,000$ dòng và phải pass `python3 scripts/check_source_size.py`.
3. **Zero-Sleep trên Fast Path (Bất biến 4):** Tuyệt đối không thêm `std::thread::sleep` vào `daemon/src/evdev_loop.rs` hoặc `protocol/src/wayland_vk.rs`.
4. **Cấm sửa lỗi kiểu chắp vá (Bất biến 6):** Mọi fix phải dựa trên chuyển trạng thái sạch (State Machine) hoặc bảng dữ liệu có cấu trúc, KHÔNG thêm chuỗi `if word == "..."` rải rác trong `engine.rs`.
5. **Cấm phá vỡ các bất biến đã đạt được:**
   - KHÔNG đổi chuẩn đặt dấu kiểu cũ (`hòa`, `thủy`, `khỏe`, `quỳnh`, `hươu`).
   - KHÔNG thêm phụ thuộc IBus, Fcitx5, hay `wtype`.
   - KHÔNG nới lỏng quy tắc bảo mật của `UserDict` (tuyệt đối không lưu token chứa ký tự đặc biệt/mật khẩu).
6. **Cổng nghiệm thu bắt buộc sau MỖI Task (không dồn nhiều task rồi mới test):**
   ```bash
   make fmt
   python3 scripts/check_source_size.py
   make lint
   make test-ci-smoke
   make test-soak
   ```
   Toàn bộ 5 lệnh phải trả về exit code `0`, `0 warnings`, và `soak` giữ `p50 < 2.0us`, `p99 < 10.0us`.

---

## PHASE A: KHẮC PHỤC TRIỆT ĐỂ CÁC THIẾU SÓT TỪ PHIÊN RETEST (STABILITY v1.0)

### Task A1: Khôi phục tên file có dấu chấm giữa câu tiếng Việt (`mở test.py`, `xem list.txt`, `sửa host.conf`)
- **Hiện tượng từ Retest:** Gõ `mowr test.py xem list.txt suwar host.conf` hiện ra `mở tét.py xem lít.txt sửa hót.còn` vì tại thời điểm gõ dấu `.`, engine chưa biết sau `.` là phần mở rộng file hay dấu chấm kết thúc câu.
- **Bất biến phải giữ:** Dấu chấm kết thúc câu tiếng Việt **BẮT BUỘC** giữ nguyên (`"ăn bánh tét. hôm nay"` $\rightarrow$ `"ăn bánh tét. hôm nay"`, KHÔNG biến thành `"ăn bánh test."`).
- **Vị trí sửa:**
  - Tạo helper mới trong [`engine/src/engine_helpers.rs`](../engine/src/engine_helpers.rs) (hoặc module mới `engine/src/filename_restore.rs` đăng ký trong `engine/src/lib.rs`).
  - Gọi từ nhánh flush trong [`engine/src/engine.rs`](../engine/src/engine.rs) (tối đa +5 dòng trong `engine.rs`).
- **Thiết kế kỹ thuật chi tiết:**
  1. Định nghĩa hàm `is_known_file_extension(ext_lower: &str) -> bool` gồm các đuôi file phổ biến không trùng âm tiết tiếng Việt thông dụng hoặc thuộc nhóm dev/office:
     `"py" | "rs" | "ts" | "tsx" | "js" | "jsx" | "txt" | "md" | "json" | "toml" | "yaml" | "yml" | "conf" | "cfg" | "ini" | "sh" | "bash" | "zsh" | "css" | "html" | "xml" | "sql" | "env" | "lock" | "log" | "csv" | "pdf" | "png" | "jpg" | "jpeg" | "webp" | "svg" | "gif" | "mp4" | "mp3" | "zip" | "tar" | "gz" | "deb" | "rpm" | "exe" | "so" | "dll" | "c" | "h" | "cpp" | "hpp" | "go" | "java" | "kt" | "rb" | "php"`.
  2. Khi flush một từ `ext` (ví dụ `py`, `txt`, `conf`) mà:
     - `prev_boundary == Some('.')`
     - `ring_buffer.last()` có `trailing_delimiters == "."` (từ ngay trước dấu chấm là `prev_entry`, ví dụ `raw_keys = "test"`, `visible_word = "tét"`)
     - `is_known_file_extension(&raw.to_ascii_lowercase()) == true`
     - `prev_entry.visible_word != prev_entry.raw_keys` và `(dict.is_english_word(&prev_raw_lower) || is_tech_token(&prev_raw_lower) || is_alphanumeric_tech_token(&prev_entry.raw_keys))`
     $\Rightarrow$ Thực hiện **Retroactive Dot-Filename Restore**:
     - `total_delete = prev_entry.visible_word.chars().count() + 1 + previous.chars().count()` (1 là dấu `.`).
     - `insert = format!("{}.{}", prev_entry.raw_keys, raw)` (khôi phục cả stem lẫn extension như `host.conf`).
     - Cập nhật `prev_entry.visible_word = prev_entry.raw_keys.clone()` và `prev_entry.was_auto_restored = true` trong `ring_buffer`.
  3. Lưu ý trường hợp `host.conf` ở cuối dòng (khi gõ `Space` hoặc `Enter` sau `conf`): bước 2 xử lý trọn vẹn cả `host` và `conf` trong cùng 1 transaction nguyên tử!
- **Unit Test bắt buộc (thêm vào `engine/tests/auto_restore.rs`):**
  - `"mowr test.py xem list.txt suwar host.conf "` $\rightarrow$ `"mở test.py xem list.txt sửa host.conf "`
  - `"aan basnh test. hoom nay "` $\rightarrow$ `"ăn bánh tét. hôm nay "` (KHÔNG phá dấu chấm cuối câu!)
  - `"ddepj qusa. "` $\rightarrow$ `"đẹp quá. "`

---

### Task A2: Sửa lỗi camelCase 3 ký tự (`dOs`, `cAs`, `tOs`) & nháy `đ` giữa từ tiếng Anh (`add`, `reddit`, `hidden`)
- **Hiện tượng từ Retest:**
  1. `dOs`, `cAs`, `tOs` bị hạ chữ thường thành `dó`, `cá`, `tó`.
  2. Khi đang gõ tiếng Anh `add`, `reddit`, `hidden`, chữ `đ` nháy thoáng qua trước khi bấm Space.
- **Vị trí sửa:**
  1. [`engine/src/engine_helpers.rs:299-308`](../engine/src/engine_helpers.rs#L299-L308) (`match_casing`):
     - Hiện tại kiểm tra `alpha[1..].iter().filter(|c| c.is_uppercase()).count() == 1` — chấp nhận cả chữ hoa ở vị trí giữa (`alpha[1]` trong `dOs`).
     - **Sửa thành:** Chỉ cho phép heuristic early-Shift khi chữ hoa duy nhất nằm ở **ký tự cuối cùng** (`alpha[alpha.len() - 1].is_uppercase()` và `alpha[1..alpha.len() - 1].iter().all(|c| c.is_lowercase())`), và ký tự cuối phải là phím modifier Telex (`s, f, r, x, j, w, a, e, o, d`). Như vậy `chaoF` $\rightarrow$ `chào` vẫn chạy, nhưng `dOs`, `cAs`, `tOs` bị loại khỏi early-Shift.
  2. [`engine/src/english.rs:245-271`](../engine/src/english.rs#L245-L271) (`has_mixed_internal_caps`):
     - Đảm bảo `dOs` (chữ thường đầu + chữ hoa giữa + chữ thường cuối) trả về `true` để auto-restore bảo toàn nguyên bản `dOs`.
  3. [`engine/src/telex/mark.rs:109-117`](../engine/src/telex/mark.rs#L109-L117):
     - Chỉ áp dụng biến đổi `dd` $\rightarrow$ `đ` khi `d` đầu tiên nằm ở **vị trí phụ âm đầu (onset, index 0)** của âm tiết (không có ký tự chữ cái nào đứng trước `dd`, trừ trường hợp dấu/nhánh hợp lệ).
     - Kiểm tra: `ddi` $\rightarrow$ `đi`, `ddos` $\rightarrow$ `đó`, `ddang` $\rightarrow$ `đang`, `ddd` $\rightarrow$ `dd` (undo); còn `add` $\rightarrow$ `add`, `reddit` $\rightarrow$ `reddit`, `hidden` $\rightarrow$ `hidden`, `middle` $\rightarrow$ `middle` (không bao giờ nháy `đ`).
- **Unit Test bắt buộc (thêm vào `engine/tests/gboard_parity.rs` & `engine/src/telex/tests.rs`):**
  - `dOs `, `cAs `, `tOs ` $\rightarrow$ `dOs `, `cAs `, `tOs `
  - `chaoF `, `VieetJ `, `cHAOF ` $\rightarrow$ `chào `, `Việt `, `cHÀO `
  - `resolve_and_render("add") == "add"`, `resolve_and_render("reddit") == "reddit"`, `resolve_and_render("hidden") == "hidden"`, `resolve_and_render("ddi") == "đi"`, `resolve_and_render("ddd") == "dd"`.

---

### Task A3: Sửa lỗi hiển thị ký tự phân cách trong `dboard-cli`
- **Hiện tượng từ Retest:** Khi chạy `/usr/bin/dboard-cli`, gõ `raw_buf` hoặc `[test]` bị hiển thị thiếu `_` và `]` trên dòng `Screen:` (dù `dboard-daemon` chạy đúng).
- **Vị trí sửa:** [`cli/src/main.rs`](../cli/src/main.rs) (vòng lặp mô phỏng `Screen:` trong `run()`).
- **Thiết kế kỹ thuật:** Khi `engine.process_key(ch)` trả về `Some(EngineEvent::Replace(rep))` và `ch` là ký tự phân cách (không phải chữ cái/chữ số biến đổi, cụ thể là `dboard_engine::is_flush_char(ch)` hoặc `!ch.is_alphanumeric()`), sau khi áp dụng `rep.delete_before_cursor` và `rep.insert`, **bắt buộc** `screen.push(ch)` giống hệt logic trong `daemon/src/evdev_loop.rs`.
- **Unit Test bắt buộc (thêm vào `cli/src/main.rs`):** Test `raw_buf [test]` ra đúng `Screen: raw_buf [test]`.

---

### Task A4: Đồng bộ Telex / VNI Parity & Loại bỏ lệch trạng thái `bamboo` vs `telex`
- **Các lỗi cần xử lý triệt để:**
  1. **Lệch undo `ddsd` (`engine/src/engine.rs:878-899`):** Khi `self.uses_recomposition() == true`, KHÔNG gọi `self.bamboo.undo_telex_transform()` trước `telex::resolve_and_render`. Hãy để `telex::resolve_and_render` làm nguồn chân lý duy nhất (Single Source of Truth) cho cả compose lẫn undo; so sánh kết quả trước/sau của `telex::resolve_and_render` để quyết định `UndoTransform` hay `ApplyTransform`.
  2. **`uses_recomposition()` khi `BoundaryState::Invalidated` (`engine/src/engine.rs:155-159`):** Khi `seed_surrounding_context()` chạy ngay sau khi focus/click (`Invalidated`), nó phải validate bằng `telex::resolve_and_render` (nếu `method == Telex && telex_backend == Recomposition`), không rơi nhầm về `bamboo`. Tách helper `uses_recomposition_backend(&self) -> bool` (không kiểm tra `boundary != Invalidated`) cho `compose_candidate()`.
  3. **VNI Parity (`engine/src/engine.rs:923-947`, `engine/src/engine_helpers.rs:16-90`):**
     - Cập nhật `active_transform` và `generation` cho cả `InputMethod::Vni` (nhận diện phím dấu `'1'..='9'`), để đổi dấu `a1` $\rightarrow$ `a2` phát đúng `ReplacementReason::ReplaceTransform`.
     - Dòng `940-947`: bật `self.override_keep_vietnamese = true` cho cả `InputMethod::Vni && ch == '0'` (tương đương `z` của Telex).
  4. **Telex bỏ dấu mũ vượt phụ âm cuối (`engine/src/telex/mark.rs:123-140`):**
     - Hỗ trợ gõ tự do kiểu Gboard: `khongo` $\rightarrow$ `không`, `viete` $\rightarrow$ `viêt`, `vietej` $\rightarrow$ `việt`, `tienges` $\rightarrow$ `tiếng`, `tana` $\rightarrow$ `tân` (cho phép ký tự giữa 2 nguyên âm trùng là phụ âm cuối tiếng Việt hợp lệ `n, m, ng, nh, c, ch, t, p` khi kết quả nằm trong `vietnamese.cm.dict`, và vẫn giữ double-key undo + auto-restore tiếng Anh như `banana`, `delete`, `choose` không bị hỏng!).
- **Unit Test bắt buộc:**
  - `ddsd` không lệch giữa `bamboo` và `telex`, không kẹt `undone_in_word`.
  - VNI `a1` $\rightarrow$ `a2` phát `ReplaceTransform`; `toan10 ` giữ nguyên `toan ` (`override_keep_vietnamese = true`).
  - Telex `khongo `, `vietej `, `tienges ` $\rightarrow$ `không `, `việt `, `tiếng `; đồng thời `banana `, `delete `, `choose ` vẫn auto-restore thành tiếng Anh.

---

### Task A5: Gia cố Tầng 1 (`daemon/`) — Touchpad Tap-to-Click, Layer-Shell, Hotplug & Multi-Keyboard
- **Vị trí sửa & yêu cầu cụ thể:**
  1. **Touchpad Tap-to-Click (`daemon/src/pointer_watcher.rs:19-30`):**
     - Hiện tại chỉ theo dõi `BTN_LEFT | BTN_RIGHT | BTN_MIDDLE`. Cần hỗ trợ thiết bị touchpad có `BTN_TOUCH` nhưng phân biệt **tap ngắn** (`BTN_TOUCH` press $\rightarrow$ release trong `< 200ms` mà không phải giữ kéo) để gọi `record_pointer_click(&shared_focus)` và tăng `pointer_activity`, giúp tap-to-click trên laptop reset composition chuẩn xác.
  2. **Layer-Shell Surfaces (`daemon/src/toplevel_tracker.rs:94-109`):**
     - Khi `active == None` (tất cả toplevel đều mất `State::Activated`, ví dụ người dùng vừa mở `cosmic-launcher`, `wofi`, `rofi`, `fuzzel`, hoặc hộp thoại Polkit):
     - Thêm biến thể `Publish::FocusLost` khi `last.serial.is_some() && active.is_none()`. Khi `Publish::FocusLost` xảy ra, bump `focus.generation` và reset `composition` (nhưng giữ `class` dự phòng cho đến khi có toplevel mới) để chữ đang gõ dở không bao giờ rò sang launcher/Polkit!
  3. **Hotplug Retry & Composite `uniq` (`daemon/src/device.rs:65-71`, `daemon/src/evdev_loop.rs:350-390`):**
     - Trong `device.rs:65-71`: ghép `uniq` + `phys` (ví dụ `uniq:{uniq}|phys:{phys}`) thay vì chỉ `uniq:{uniq}` để bàn phím composite USB/BT có nhiều interface không bị nuốt mất interface gõ chính.
     - Trong `evdev_loop.rs:371-388`: nếu `dev.grab()` thất bại tạm thời lúc hotplug (udev chưa kịp gán `uaccess`), KHÔNG thêm vào `known_paths` vĩnh viễn; cho phép retry tối đa 5 lần ở các chu kỳ scan sau.
     - Chuyển bộ đếm hotplug scan dựa trên `Instant::now()` (mỗi 2 giây) thay vì chỉ chạy khi `poll_ret == 0` để tránh bị đói (starvation) khi gõ liên tục.
  4. **Cross-Device Modifier (`daemon/src/evdev_loop.rs:159-164`):**
     - Tính trạng thái `Shift`/`Ctrl`/`Alt`/`Super` bằng phép hợp (OR) trên tất cả `device_states` thay vì chỉ thiết bị `i`, để bàn phím tách đôi (split keyboard) và numpad rời hoạt động đồng bộ.
  5. **Sửa fallback `wlrctl` (`daemon/src/app_state.rs:246-259`):**
     - Truyền thêm bộ lọc `state:focused` vào `wlrctl toplevel list state:focused` thay vì lấy dòng đầu tiên tùy tiện.

---

### Task A6: Gia cố Tầng 3 (`protocol/`) — Rich Clipboard, `GlobalRemove`, `CapsLock` LED & `Mod3`/`Mod5`
- **Vị trí sửa & yêu cầu cụ thể:**
  1. **Bảo toàn Rich Clipboard (`protocol/src/uinput_monitor/clipboard_backup.rs:28-36`):**
     - Trong `classify_clipboard_types()`, nếu danh sách MIME chứa kiểu giàu định dạng (`text/html`, `text/uri-list`, `image/*`, `application/*`) đi kèm `text/plain`, **ưu tiên backup kiểu MIME chính** (hoặc lưu nhiều cặp `(mime, bytes)` tối đa `MAX_BINARY_CLIPBOARD_BYTES`) thay vì hạ xuống chỉ còn `text/plain`.
  2. **Xử lý `GlobalRemove` & Stale Wayland Socket (`protocol/src/wayland_vk.rs:61-85, 688-746`):**
     - Lưu `seat_global_name: Option<u32>` và `manager_global_name: Option<u32>` trong `VkDiscovery`. Khi nhận `wl_registry::Event::GlobalRemove { name }` khớp 1 trong 2 ID này, đánh dấu `healthy = false` để daemon tự reconnect sạch.
     - Trong `resolve_wayland_socket_path()`: probe kết nối Unix socket thực tế thay vì chỉ kiểm tra `path.exists()`, tránh kẹt vào socket chết sau khi compositor crash.
     - Trong `connect_unix_nonblocking()`: xử lý `EINPROGRESS` bằng `libc::poll(POLLOUT, 500ms)` + `getsockopt(SO_ERROR)`.
  3. **Tránh nháy LED `CapsLock` không cần thiết (`protocol/src/wayland_vk.rs:463-500`):**
     - Chỉ gửi `vk.modifiers(..., edit_locked, 0)` tắt `XKB_MOD_LOCK` khi `sess.caps_lock == true` **và** chuỗi `insert` thực sự cần phát ký tự chịu ảnh hưởng của `CapsLock`; nếu `sess.caps_lock == false`, không gửi thừa gói `modifiers` thay đổi lock.
  4. **Giảm xung đột `Mod3`/`Mod5` trên XWayland / Kitty (`protocol/src/wayland_vk_keymap.rs:265-331`):**
     - Khai báo đầy đủ `vmodmap` cho `Alt`, `Super`, `Meta`, `Caps_Lock` trong `xkb_compatibility` để toolkit/client truy vấn virtual modifier không nhận giá trị `0`.
     - Ghi rõ trong tài liệu & `dboard doctor` cảnh báo nếu người dùng bind phím tắt compositor với `Mod3`/`Mod5`.

---

## PHASE B: NÂNG CẤP ĐẠT CHUẨN TƯƠNG ĐƯƠNG GBOARD ANDROID

### Task B1: Quantized Statistical Bigram/Trigram Language Model (Thay thế bảng rule cứng ~250 cặp)
- **Mục tiêu:** Đưa độ chính xác song ngữ Việt–Anh của Dboard lên tiệm cận Gboard bằng mô hình xác suất thống kê thực sự, trong khi vẫn **zero-allocation, $O(1)$ lookup, < 300KB binary, `p50 < 2µs`**.
- **Kiến trúc đề xuất:**
  1. Tạo script offline [`scripts/build_lm.py`](../scripts/build_lm.py) (chạy lúc phát triển, KHÔNG chạy ở runtime):
     - Đầu vào: Corpus tiếng Việt (Wikipedia VI + báo chí + văn phòng) + Corpus tiếng Anh/Dev (Wikipedia EN + tài liệu lập trình/Linux).
     - Xuất ra file nhúng nhị phân `engine/src/bilingual_lm.bin` (tối đa ~250–350 KB):
       - Header magic + version + table sizes.
       - **Unigram quantized log-prob (`u8`)** cho các âm tiết va chạm Việt–Anh (~2,000 từ giao thoa): `(P_vn(w), P_en(w))`.
       - **Bigram quantized log-prob (`u8`)** lưu bằng open-addressing hash table dùng FNV-1a 64-bit key `hash(w1, w2)`:
         - `P_vn(w2 | w1)` cho top ~30,000 cặp tiếng Việt phổ biến.
         - `P_en(w2 | w1)` cho top ~20,000 cặp tiếng Anh/Dev phổ biến.
         - `P_code_switch(en_w2 | vn_w1)` cho top ~5,000 cặp Việt–Anh phổ biến (`chạy test`, `mở file`, `đọc docs`, `cài app`, `push code`...).
  2. Tạo module mới `engine/src/stat_lm.rs` (< 400 dòng):
     - Nạp `static LM_BYTES: &[u8] = include_bytes!("bilingual_lm.bin");` (zero startup parse time!).
     - Hàm `score_bilingual(prev2, prev1, cand_vn, cand_en) -> i32`: tính hiệu điểm log-prob $\Delta = \log P_{EN} - \log P_{VN}$ (dương $\Rightarrow$ khôi phục tiếng Anh, âm $\Rightarrow$ giữ tiếng Việt).
     - Giữ lại các high-precision override trong `lm.rs`/`bigram.rs` làm tầng ưu tiên 1 (deterministic rules), và dùng `stat_lm.rs` làm tầng ưu tiên 2 cho toàn bộ phần còn lại (thay thế hoàn toàn việc phải viết tay hàng nghìn cặp từ).
  3. **Tiêu chí nghiệm thu:**
     - Tạo bộ benchmark đánh giá `engine/tests/bilingual_accuracy_bench.rs` gồm $\ge 500$ câu song ngữ Việt–Anh thực tế (văn phòng, lập trình, chat đời thường).
     - Yêu cầu độ chính xác $\ge 97\%$ (so với ~88% hiện tại với rule cứng) và `make test-soak` giữ `p50 < 2.0us`.

---

### Task B2: Hoàn thiện Spatial Autocorrect cho VNI & One-Tap Undo cho Retroactive 2-Word Rewrite
- **Vị trí sửa:** [`engine/src/autocorrect.rs`](../engine/src/autocorrect.rs), [`engine/src/engine.rs`](../engine/src/engine.rs).
- **Thiết kế kỹ thuật:**
  1. Trong `autocorrect.rs`:
     - Mở rộng `key_coord(ch: char)` thêm hàng phím số US QWERTY: `'1' => (-1.0, 0.0)` đến `'0' => (-1.0, 9.0)`.
     - Tạo bảng `QWERTY_VNI_DIGITS: [char; 10] = ['0'..'9']`; khi `method == InputMethod::Vni`, `try_vn_spatial_substitution` cho phép sửa lỗi trượt phím giữa các phím số dấu liền kề (`6` $\leftrightarrow$ `7`, `1` $\leftrightarrow$ `2`, `5` $\leftrightarrow$ `6`, `8` $\leftrightarrow$ `9`) với ngưỡng `key_distance <= 1.25`.
  2. Trong `engine.rs` (nhánh `evaluate_retroactive_first_word` dòng `694-755`):
     - Trước khi trả về `Replacement` cho sửa hồi quy 2 từ, lưu trạng thái vào `self.pending_restore_undo` (mở rộng `AutoRestoreUndo` để hỗ trợ khôi phục cả cặp 2 từ nếu người dùng bấm `Backspace` ngay sau `Space` — One-Tap Undo).
- **Unit Test bắt buộc:**
  - VNI autocorrect: trượt phím số dấu VNI sửa đúng + undo về gốc khi bấm `Backspace`.
  - Retroactive 2-word One-Tap Undo: gõ `"Lats nuwax "` $\rightarrow$ `"Lát nữa "`, nếu người dùng thực sự muốn `"Last nữa"` chỉ cần bấm `Backspace` 1 lần.

---

### Task B3: Hoàn thiện Smart Auto-Capitalization (Abbreviation Guard & Closing Quotes)
- **Vị trí sửa:** [`engine/src/user_dict.rs:196-258`](../engine/src/user_dict.rs#L196-L258) (`AutoCapState`).
- **Thiết kế kỹ thuật:**
  1. **Closing Quotes/Brackets:** Khi `pending_sentence_end == true` và gặp `'"' | '\'' | ')' | ']' | '»'`, giữ nguyên `pending_sentence_end = true` (KHÔNG `reset()`), để chuỗi `"Chào." hôm` tự viết hoa thành `"Chào." Hôm`.
  2. **Abbreviation Guard:** Truyền từ vừa commit trước dấu `.` vào `on_flush_char(ch, prev_word)`: nếu `ch == '.'` và `prev_word` nằm trong danh sách viết tắt (`"mr"`, `"mrs"`, `"ms"`, `"dr"`, `"prof"`, `"st"`, `"vs"`, `"etc"`, `"eg"`, `"ie"`, `"tp"`, `"ts"`, `"ths"`, `"pgs"`, `"gs"`, `"ks"`, `"bs"`, `"v"`), KHÔNG bật `pending_sentence_end = true`.
- **Unit Test bắt buộc:**
  - `"\"Chào.\" hôm "` $\rightarrow$ `"\"Chào.\" Hôm "`
  - `"TP. hcm "`, `"Mr. nam "`, `"TS. nguyễn "` KHÔNG tự viết hoa từ sau dấu chấm nếu user gõ chữ thường (trừ khi user tự nhấn Shift).

---

### Task B4: Thanh Gợi ý Ứng viên Tùy chọn (Optional Floating Candidate Strip — Wayland Layer-Shell)
- **Nguyên tắc kiến trúc:**
  - **Tắt mặc định** (`candidate_bar = false`) để giữ 100% trải nghiệm Zero-Underline tối giản.
  - **Không nhúng UI vào `dboard-daemon`:** Tách thành binary/crate độc lập `ui-popup/` (hoặc mở rộng `ui/`) dùng `zwlr_layer_shell_v1` (hiển thị overlay không cướp keyboard focus — `keyboard_interactivity = None`).
  - Giao tiếp 1 chiều non-blocking từ `dboard-daemon` sang `dboard-popup` qua Unix Datagram Socket trong `$XDG_RUNTIME_DIR/dboard-candidates.sock` (chế độ `O_NONBLOCK`, gửi mất gói thì bỏ qua, **tuyệt đối không chặn `evdev_loop`**).
  - Hiển thị top 3 ứng viên: `[1] từ tiếng Việt | [2] từ gốc ASCII | [3] gợi ý sửa lỗi/từ ghép`, cho phép chọn nhanh bằng phím tắt cấu hình hoặc tắt hẳn.

---

### Task B5: Mở rộng Hệ sinh thái (Đa Layout & Compositor — Lộ trình v2.0)
- **Đa bố cục bàn phím (Dvorak / Colemak / ISO):**
  - Mở rộng [`protocol/src/layout.rs`](../protocol/src/layout.rs) thành trait/enum `KeyboardLayout { UsQwerty, Dvorak, Colemak }` cấu hình qua `keyboard_layout = "us"` trong `dboard.toml`.
  - Đồng bộ tọa độ `key_coord()` trong `autocorrect.rs` và bảng `build_xkb_keymap()` trong `wayland_vk_keymap.rs` từ cùng 1 nguồn `layout.rs`.
- **Hỗ trợ GNOME Wayland / KDE Plasma Wayland (v2.0):**
  - Không thay đổi kiến trúc lõi `zwp_virtual_keyboard_v1` cho COSMIC/Hyprland/Sway.
  - Nghiên cứu backend thứ 3 độc lập trong `protocol/` sử dụng `zwp_input_method_v2` / `xdg-desktop-portal` RemoteDesktop Input (tách feature flag riêng để không làm bẩn lõi Pure Wayland hiện tại).

---

## LỘ TRÌNH THỰC THI THEO MILESTONE (EXECUTION ORDER)

| Milestone | Bao gồm Tasks | Mục tiêu đạt được | Tiêu chí Go/No-Go |
|---|---|---|---|
| **M1 — Retest Zero-Defect** | `A1`, `A2`, `A3` | Sửa sạch 100% lỗi thấy trong phiên Retest (`test.py`, `dOs`, `add` nháy `đ`, `dboard-cli`) | `make test-ci-smoke` + `dboard-cli` retest sạch 100% |
| **M2 — Engine & Hardware Rock-Solid** | `A4`, `A5`, `A6` | Đạt chuẩn **General Stable v1.0** cho COSMIC/Hyprland/Sway + cả Telex & VNI | Zero regression trên `test-properties-exhaustive` & `soak_perf` |
| **M3 — Gboard Intelligence** | `B1`, `B2`, `B3` | Mô hình ngôn ngữ xác suất thực sự ($\ge 97\%$ song ngữ), VNI autocorrect, Auto-Cap chuẩn | Accuracy benchmark $\ge 97\%$, `p50 < 2µs`, binary LM `< 350KB` |
| **M4 — Gboard UX & Ecosystem** | `B4`, `B5` | Floating candidate strip tùy chọn, đa layout, mở rộng compositor | Non-blocking IPC verified, zero impact khi `candidate_bar = false` |
