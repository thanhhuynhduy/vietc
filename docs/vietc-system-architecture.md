# TÀI LIỆU KIẾN TRÚC HỆ THỐNG TOÀN DIỆN BỘ GÕ TIẾNG VIỆT VIETC
**VietC — Pure Linux Architecture & System Specification (Authoritative Reference)**

> **TÀI LIỆU CHUẨN KIẾN TRÚC CỐT LÕI (AUTHORITATIVE SYSTEM BLUEPRINT)**  
> **Áp dụng cho:** Tất cả các kỹ sư, chuyên gia bảo trì và AI Agent phát triển dự án VietC.  
> **Quy định bắt buộc:** Bất kỳ thay đổi mã nguồn, tái cấu trúc (refactoring), sửa lỗi (bug fixing) hay bổ sung tính năng nào trong tương lai **BẮT BUỘC** phải đọc và tuân thủ tuyệt đối các phân tầng và bất biến được định nghĩa trong tài liệu này. Tuyệt đối nghiêm cấm việc sửa lỗi cục bộ kiểu "chắp vá" (patchwork) phá vỡ tính logic và làm rách nát kiến trúc hệ thống.

---

## MỤC LỤC CHI TIẾT
1. [PHẦN 1: TỔNG QUAN, TRIẾT LÝ THIẾT KẾ VÀ BỐI CẢNH HỆ THỐNG](#phần-1-tổng-quan-triết-lý-thiết-kế-và-bối-cảnh-hệ-thống)
   - 1.1. Bối cảnh lịch sử và Vấn đề cốt tử của bộ gõ trên Linux
   - 1.2. Tuyên ngôn Triết lý Thiết kế của VietC
   - 1.3. Sơ đồ Tổng thể Kiến trúc Hệ thống Phân tầng
   - 1.4. Phân chia Trách nhiệm giữa các Crate
2. [PHẦN 2: TẦNG 1 - BẮT PHÍM CẤP KERNEL & TIỀN XỬ LÝ (HARDWARE INTERCEPTION)](#phần-2-tầng-1---bắt-phím-cấp-kernel--tiền-xử-lý-hardware-interception)
   - 2.1. Độc quyền Bắt phím cấp Nhân (`evdev grab`) và Vòng lặp `libc::poll`
   - 2.2. Cơ chế Tự động Phát hiện Bàn phím Cắm ngoài (Hotplug Detection)
   - 2.3. Máy Quản lý Trạng thái Phím và Modifier (`KeyboardState`)
   - 2.4. Phím tắt Chuyển đổi Bộ gõ (`Toggle Method`) và Nuốt phím
   - 2.5. Đồng bộ Cửa sổ và Ứng dụng đích (`FocusTracker`)
   - 2.6. Nhận diện và Bảo vệ Ô nhập Mật khẩu (`PasswordDetector`)
   - 2.7. Chế độ Bỏ qua Ứng dụng Độ trễ 0ms (`App Bypass Mode`)
3. [PHẦN 3: TẦNG 2 - ĐỘNG CƠ NGÔN NGỮ TIẾNG VIỆT (LINGUISTIC ENGINE)](#phần-3-tầng-2---động-cơ-ngôn-ngữ-tiếng-việt-linguistic-engine)
   - 3.1. Thuật toán Lõi Bamboo (`bamboo.rs`) & Quy tắc Âm vần Tiếng Việt
   - 3.2. Chuẩn Đặt Dấu Thanh: Kiểu Cũ (Old-style Orthography)
   - 3.3. Bộ đệm Vòng Lịch sử Từ (`CommittedRingBuffer`) và Quản lý Ranh giới
   - 3.4. Bộ Nhận diện Ngôn ngữ và Tự động Khôi phục Tiếng Anh (`english.rs`, `spelling.rs`)
   - 3.5. Thuật toán Sửa sai Hồi quy Hai Từ Liên tiếp (`bigram.rs`)
   - 3.6. Máy Trạng thái Soạn thảo (Composition State Machine)
4. [PHẦN 4: TẦNG 3 - XUẤT PHÍM & GIAO THỨC HIỂN THỊ (INJECTION LAYER)](#phần-4-tầng-3---xuất-phím--giao-thức-hiển-thị-injection-layer)
   - 4.1. Điều phối Giao thức Kép: Wayland Virtual Keyboard vs Linux Uinput
   - 4.2. Bố cục Keymap Tĩnh XKB 8 Cấp độ (`wayland_vk_keymap.rs`)
   - 4.3. Giới hạn Dải Keycode An toàn (`1..=111`) Tránh Xung đột Trình duyệt
   - 4.4. Quy trình Thay thế Văn bản Nguyên tử (`replace_text`)
   - 4.5. Phân tích Hiện tượng Đè phím (Key Overlap) và Giải pháp Atomic Space
   - 4.6. Trọng tài Đơn điệu Thời gian (Monotonic Timestamp FIFO)
5. [PHẦN 5: BẢO VỆ GIAO DIỆN & CÁC TRƯỜNG HỢP BIÊN ĐẶC BIỆT (EDGE CASES)](#phần-5-bảo-vệ-giao-diện--các-trường-hợp-biên-đặc-biệt-edge-cases)
   - 5.1. Chrome / Chromium Omnibox Autocomplete & Selection Shield
   - 5.2. Trình soạn thảo Hiện đại (VS Code, Sublime Text, JetBrains)
   - 5.3. Trình giả lập Thiết bị đầu cuối (Terminal Emulators & Shells)
   - 5.4. Ứng dụng Nền tảng Electron / CEF / Slack / Discord
6. [PHẦN 6: BỘ BẤT BIẾN KIẾN TRÚC CỐT TỬ (ARCHITECTURAL INVARIANTS)](#phần-6-bộ-bất-biến-kiến-trúc-cốt-tử-architectural-invariants)
   - Bất biến 1: Tính Độc lập của Ranh giới Từ (Word Boundary Isolation)
   - Bất biến 2: Thứ tự Sự kiện FIFO & Timestamp Đơn điệu Tuyệt đối
   - Bất biến 3: Xuất Phím Phân cách Nguyên tử (Atomic Delimiter Dispatch)
   - Bất biến 4: Zero-Sleep Trong Luồng Xử lý Sự kiện (Non-blocking Protocol)
   - Bất biến 5: Cổng Kiểm soát Kích thước Mã nguồn (Source Size Quality Gate)
   - Bất biến 6: Nguyên tắc Bác bỏ Vá lỗi Cục bộ (Clean State Machine Transitions)
7. [PHẦN 7: QUY TRÌNH KIỂM THỬ, XÁC MINH & VẬN HÀNH (VERIFICATION RUNBOOK)](#phần-7-quy-trình-kiểm-thử-xác-minh--vận-hành-verification-runbook)
   - 7.1. Bộ Kiểm tra Chất lượng Tĩnh (Static Analysis Suite)
   - 7.2. Kiểm thử Tách biệt Môi trường Đồ họa (Headless Sandbox Testing)
   - 7.3. Kịch bản Kiểm thử Áp lực Người dùng Văn phòng (Office Stress Test Suite)

---

# PHẦN 1: TỔNG QUAN, TRIẾT LÝ THIẾT KẾ VÀ BỐI CẢNH HỆ THỐNG

### 1.1. Bối cảnh lịch sử và Vấn đề cốt tử của bộ gõ trên Linux
Hệ sinh thái bộ gõ tiếng Việt trên Linux trong nhiều thập kỷ qua chủ yếu dựa trên hai kiến trúc trung gian (Input Method Framework): **IBus** và **Fcitx5**. Dù đã có nhiều đóng góp, mô hình kiến trúc trung gian này bộc lộ những nhược điểm chí mạng không thể khắc phục triệt để:
1. **Xung đột Biến Môi trường và Deadlock DBus:**  
   Người dùng phải cấu hình thủ công các biến như `GTK_IM_MODULE=ibus`, `QT_IM_MODULE=ibus`, `XMODIFIERS=@im=ibus`. Khi một daemon gặp sự cố hoặc DBus bị nghẽn, toàn bộ việc nhập liệu trên hệ thống bị tê liệt hoặc rơi vào trạng thái deadlock.
2. **Hiện tượng Gạch chân Giật màn hình (Preedit Underline Glitch):**  
   Mô hình IBus/Fcitx đẩy chuỗi đang soạn thảo vào vùng đệm tạm thời (preedit string) có gạch chân nhấp nháy. Trong các ứng dụng soạn thảo hiện đại có tính năng gợi ý thông minh (như VS Code IntelliSense, Chrome Omnibox Autocomplete, JetBrains IDEs), preedit string liên tục kích hoạt việc tự động bôi đen, làm con trỏ nhảy loạn xạ, nuốt ký tự hoặc chèn sai vị trí.
3. **Sự cô lập của Sandbox Hiện đại (Flatpak / Snap / AppImage):**  
   Các ứng dụng chạy trong sandbox thường bị ngắt kết nối với socket DBus của bộ gõ ngoài host, khiến người dùng không thể gõ tiếng Việt trong trình duyệt hay công cụ làm việc cài từ Flatpak.

### 1.2. Tuyên ngôn Triết lý Thiết kế của VietC
VietC được xây dựng lại từ đầu với một triết lý hoàn toàn khác biệt:
- **Nói KHÔNG với IBus và Fcitx5:** VietC là một **Standalone Pure Linux IME Daemon**. Không phụ thuộc vào bất kỳ framework bộ gõ trung gian nào, không can thiệp vào biến môi trường hệ thống.
- **Trải nghiệm Zero-Underline (Chuẩn Gboard trên Android):** Chữ được gõ thẳng trực tiếp vào ứng dụng đích mà không hề có bất kỳ dấu gạch chân tiền biên tập nào. Trải nghiệm mượt mà, liền mạch như bàn phím di động cao cấp.
- **Tiếp cận Cấp Thấp (Kernel-Level Hardware Interception):** Bắt trực tiếp tín hiệu phần cứng qua Linux `evdev` và xuất phím thông qua giao thức hiện đại nhất của Wayland (`zwp_virtual_keyboard_v1`) hoặc Linux `/dev/uinput`.
- **Chuẩn Dấu Thanh Truyền Thống (Old-Style Vietnamese Orthography):** Mặc định đặt dấu thanh chuẩn văn phong tiếng Việt truyền thống (`hòa`, `thủy`, `hóa`, `quỳnh`, `tòa`), bảo tồn tính thẩm mỹ tự nhiên của ngôn ngữ.

### 1.3. Sơ đồ Tổng thể Kiến trúc Hệ thống Phân tầng
```
                 ┌────────────────────────────────────────────────────────┐
                 │             Hardware Keyboard (USB / AT)               │
                 └───────────────────────────┬────────────────────────────┘
                                             │ Raw Linux Input Events (/dev/input/event*)
                                             ▼
                 ┌────────────────────────────────────────────────────────┐
                 │  TẦNG 1: THU NHẬN SỰ KIỆN (daemon/src/evdev_loop.rs)   │
                 │  - evdev::Device::grab() bắt độc quyền cấp kernel      │
                 │  - Quản lý trạng thái vật lý (KeyboardState, dedup)    │
                 │  - Đồng bộ ứng dụng (FocusTracker, PasswordDetector)   │
                 └───────────┬────────────────────────────────┬───────────┘
                             │                                │
              Passthrough key│(None)             Replace event│(Some(Replace))
                             ▼                                ▼
                 ┌───────────────────────┐        ┌───────────────────────┐
                 │  TẦNG 3: XUẤT PHÍM    │        │  TẦNG 2: ENGINE XỬ LÝ │
                 │  (protocol/uinput)    │        │  (engine/src/)        │
                 │  - UinputInjector     │        │  - Bamboo Core        │
                 │  - zwp_virtual_kb_v1  │◄───────┤  - RingBuffer         │
                 │  - XKB Keymap 8-level │        │  - English & Bigram   │
                 └───────────┬───────────┘        └───────────────────────┘
                             │
                             ▼ Native Wayland / X11 Events
                 ┌────────────────────────────────────────────────────────┐
                 │  CLIENT APPLICATIONS (Electron, VS Code, Browser, App) │
                 └────────────────────────────────────────────────────────┘
```

### 1.4. Phân chia Trách nhiệm giữa các Crate
Hệ thống VietC được module hóa nghiêm ngặt thành các crate độc lập:
1. [`vietc-engine`](file:///home/thanhhd/Git-projects/vietc/engine): Thuần túy tính toán logic tiếng Việt, không chứa bất kỳ lời gọi hệ thống hay giao thức I/O nào. Có thể chạy độc lập trong môi trường WebAssembly hoặc Unit Test.
2. [`vietc-daemon`](file:///home/thanhhd/Git-projects/vietc/daemon): Tiến trình chạy nền (daemon service), điều phối việc bắt phím phần cứng `evdev`, đồng bộ tiêu điểm AT-SPI, bảo vệ mật khẩu và điều khiển State Machine.
3. [`vietc-protocol`](file:///home/thanhhd/Git-projects/vietc/protocol): Tầng trừu tượng hóa giao thức xuất phím, chứa bộ nạp Wayland Virtual Keyboard, cấu hình Keymap XKB 8 cấp độ và Linux Uinput.
4. [`vietc-ui`](file:///home/thanhhd/Git-projects/vietc/ui): Giao diện đồ họa cấu hình và khay hệ thống (System Tray).
5. [`vietc-cli`](file:///home/thanhhd/Git-projects/vietc/cli) / [`vietcctl`](file:///home/thanhhd/Git-projects/vietc/vietcctl): Công cụ dòng lệnh giao tiếp với daemon qua Unix Domain Socket (`/run/user/$UID/vietc.sock`).

---

# PHẦN 2: TẦNG 1 - BẮT PHÍM CẤP KERNEL & TIỀN XỬ LÝ (HARDWARE INTERCEPTION)

Vị trí mã nguồn: [`daemon/src/evdev_loop.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/evdev_loop.rs), [`daemon/src/main.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/main.rs), [`daemon/src/device.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/device.rs).

### 2.1. Độc quyền Bắt phím cấp Nhân (`evdev grab`) và Vòng lặp `libc::poll`
- **Cơ chế `grab()`:** Ngay khi khởi động, daemon mở file thiết bị tại `/dev/input/event*` và gửi ioctl `EVIOCGRAB`. Nhân Linux lập tức khóa cổng chuyển tiếp của bàn phím này: toàn bộ sự kiện phím vật lý bị chặn lại, không truyền tới X11, Wayland Compositor hay tty.
- **Tối ưu Vòng lặp I/O:** Daemon sử dụng hàm hệ thống `libc::poll` để giám sát tập hợp file descriptors của tất cả các bàn phím đồng thời. Khi có phím bấm, luồng thức dậy ngay lập tức mà không tiêu tốn chu kỳ CPU nào trong trạng thái nghỉ.
- **Ưu tiên Luồng Real-time:** Hàm [`boost_thread_priority()`](file:///home/thanhhd/Git-projects/vietc/daemon/src/main.rs#L39-L44) thiết lập mức ưu tiên `nice -10` cho tiến trình daemon, đảm bảo độ trễ phản hồi phím luôn $\le 1\text{ms}$ ngay cả khi hệ thống đang tải nặng.

### 2.2. Cơ chế Tự động Phát hiện Bàn phím Cắm ngoài (Hotplug Detection)
Vòng lặp chính duy trì một bộ hẹn giờ định kỳ kiểm tra các thiết bị mới trong `/dev/input/`. Khi phát hiện người dùng cắm thêm bàn phím cơ USB hoặc bàn phím không dây Bluetooth:
1. Xác định thiết bị có thuộc nhóm bàn phím thực hay không (thông qua cờ `EV_KEY` và các phím chữ cái tiêu chuẩn).
2. Tự động khởi tạo kết nối và gọi `grab()` để đưa vào danh sách quản lý.
3. Khi bàn phím bị rút ra, bộ điều phối tự động xóa descriptor khỏi mảng `poll` mà không gây treo hay đổ vỡ tiến trình.

### 2.3. Máy Quản lý Trạng thái Phím và Modifier (`KeyboardState`)
Mỗi sự kiện phím thô bao gồm: `keycode` (mã phím phần cứng), `value` (0 = KeyUp, 1 = KeyDown, 2 = KeyRepeat).
Module [`KeyboardState`](file:///home/thanhhd/Git-projects/vietc/daemon/src/app_state.rs) theo dõi một bitmask liên tục:
- Các phím Modifier vật lý: `Shift_L` (42), `Shift_R` (54), `Ctrl_L` (29), `Ctrl_R` (97), `Alt_L` (56), `Alt_R` (100), `Super_L` (125), `Super_R` (126).
- Trạng thái khóa: `CapsLock` (58), `NumLock` (69).
- Bộ lọc chống dội phím (Debounce/Dedup): Loại bỏ các sự kiện trùng lặp bất thường do switch bàn phím chập chờn.

### 2.4. Phím tắt Chuyển đổi Bộ gõ (`Toggle Method`) và Nuốt phím
- Phím tắt mặc định chuyển đổi giữa Tiếng Việt và Tiếng Anh là `Ctrl + Space` hoặc `Alt + Z` (có thể tùy biến trong file cấu hình `~/.config/vietc/config.toml`).
- **Quy tắc Nuốt phím (Key Consumption):** Khi phát hiện tổ hợp phím tắt được nhấn, daemon lập tức "nuốt" sự kiện này (không chuyển tiếp tới ứng dụng đích), đảo cờ chế độ gõ và phát âm thanh/hiển thị thông báo OSD nếu được kích hoạt.

### 2.5. Đồng bộ Cửa sổ và Ứng dụng đích (`FocusTracker`)
Vị trí mã nguồn: [`daemon/src/focus_tracker.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/focus_tracker.rs).
- Sử dụng giao thức AT-SPI2 DBus Accessibility và Wayland Toplevel Protocol để theo dõi cửa sổ đang nhận tiêu điểm nhập liệu (active focused window).
- **Nguyên tắc Reset Tuyệt đối:** Khi tiêu điểm chuyển từ ứng dụng này sang ứng dụng khác (ví dụ: chuyển từ VS Code sang Telegram):
  1. Daemon lập tức gọi `engine.reset_composition_state()`.
  2. Toàn bộ `raw_buffer` và chuỗi giao dịch đang soạn dở bị hủy bỏ.
  3. Đảm bảo từ đang gõ dở ở ứng dụng cũ không bao giờ bị chèn nhầm sang ứng dụng mới.

### 2.6. Nhận diện và Bảo vệ Ô nhập Mật khẩu (`PasswordDetector`)
Vị trí mã nguồn: [`daemon/src/password_detector.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/password_detector.rs).
- Khi người dùng nhấp chuột vào một ô mật khẩu trong trình duyệt hoặc màn hình đăng nhập, AT-SPI Accessibility báo hiệu thuộc tính `ROLE_PASSWORD_TEXT`.
- VietC lập tức chuyển sang chế độ **Direct Raw Pass-Through**: mọi ký tự phím thô được đẩy thẳng vào ứng dụng mà không qua bất kỳ khâu xử lý ngữ âm tiếng Việt nào, đồng thời không bao giờ lưu trữ các ký tự này trong bất kỳ bộ đệm lịch sử nào.

### 2.7. Chế độ Bỏ qua Ứng dụng Độ trễ 0ms (`App Bypass Mode`)
- Đối với các ứng dụng yêu cầu nắm bắt phím thô thời gian thực như trò chơi bắn súng góc nhìn thứ nhất (CS2, Valorant, Minecraft) hoặc phần mềm máy ảo (QEMU, VirtualBox, Remmina RDP), việc phân tích dấu tiếng Việt có thể gây ảnh hưởng đến thao tác di chuyển (`WASD`).
- Người dùng có thể khai báo danh sách process trong cấu hình. Khi ứng dụng trong danh sách nhận focus, daemon sẽ bypass toàn bộ logic tiếng Việt, gửi thẳng scancode gốc qua `uinput` với độ trễ 0ms.

---

# PHẦN 3: TẦNG 2 - ĐỘNG CƠ NGÔN NGỮ TIẾNG VIỆT (LINGUISTIC ENGINE)

Vị trí mã nguồn: [`engine/src/`](file:///home/thanhhd/Git-projects/vietc/engine/src/).

### 3.1. Thuật toán Lõi Bamboo (`bamboo.rs`) & Quy tắc Âm vần Tiếng Việt
Lõi xử lý kế thừa và phát triển các nguyên lý tối ưu từ bộ gõ Ngó Sen:
1. **Cơ chế Bỏ dấu Tự do (Free-Tone Marking):**
   Người dùng có thể gõ phím dấu thanh ở bất kỳ thời điểm nào khi đang gõ từ mà bộ gõ vẫn tính toán đặt dấu đúng trên nguyên âm chính:
   - `toans` $\rightarrow$ `toán`
   - `hoanf` $\rightarrow$ `hoàn`
   - `thuowng` $\rightarrow$ `thương`
   - `chuyeens` $\rightarrow$ `chuyến`
2. **Quy tắc Nuốt và Hoàn tác Phím Đúp (Double-Key Undo):**
   - Phím tạo mũ nguyên âm: `aa` $\rightarrow$ `â`, gõ tiếp `a` $\rightarrow$ `aa`; `ee` $\rightarrow$ `ê`, gõ tiếp `e` $\rightarrow$ `ee`; `oo` $\rightarrow$ `ô`, gõ tiếp `o` $\rightarrow$ `oo`.
   - Phím sừng và gạch ngang: `w` sau `u`/`o` biến thành `ư`/`ơ` (`uw` $\rightarrow$ `ư`, `ow` $\rightarrow$ `ơ`, `uow` $\rightarrow$ `ươ`). Gõ tiếp `w` sẽ hoàn tác về ký tự gốc (`uww` $\rightarrow$ `uw`).
   - Phím chữ `đ`: `dd` $\rightarrow$ `đ`, gõ tiếp `d` $\rightarrow$ `dd`.

### 3.2. Chuẩn Đặt Dấu Thanh: Kiểu Cũ (Old-style Orthography)
VietC tuân thủ nghiêm ngặt quy tắc đặt dấu thanh truyền thống:
- Khi một âm tiết chứa nguyên âm đôi có âm đệm `o` hoặc `u` (`oa`, `oe`, `uy`): Dấu thanh được đặt trên **nguyên âm chính** đứng sau:
  - `hòa` (KHÔNG PHẢI `hoà`)
  - `thủy` (KHÔNG PHẢI `thuỷ`)
  - `hóa` (KHÔNG PHẢI `hoá`)
  - `quỳnh` (KHÔNG PHẢI `qùynh`)
- Khi có phụ âm cuối đi kèm (`oan`, `oat`, `uyen`): Trọng tâm dấu thanh tự động dịch chuyển vào nguyên âm áp chót:
  - `hoàn` (trên `a`)
  - `khuyên` (trên `e`)
  - `thuở`, `khuỷu` (đặt chuẩn trên nguyên âm biểu thị âm sắc chính).

### 3.3. Bộ đệm Vòng Lịch sử Từ (`CommittedRingBuffer`) và Quản lý Ranh giới
Vị trí mã nguồn: [`engine/src/ring_buffer.rs`](file:///home/thanhhd/Git-projects/vietc/engine/src/ring_buffer.rs).
- `CommittedRingBuffer` lưu trữ một danh sách cố định (thường là 8-16 từ gần nhất) đã được người dùng gõ hoàn tất:
  ```rust
  pub struct CommittedWordEntry {
      pub raw_keys: String,             // Chuỗi phím thô (ví dụ: "mayf")
      pub visible_word: String,         // Chuỗi hiển thị trên màn hình (ví dụ: "mày")
      pub trailing_delimiters: String,  // Ký tự phân cách (ví dụ: " ")
      pub prior_context: LanguageContext,
      pub committed_context: LanguageContext,
      pub was_auto_restored: bool,
      pub user_override_vn: bool,
  }
  ```
- **Bất biến Phân định Ranh giới Từ:** Khi người dùng bấm phím phân cách (`is_flush_char`: Space, Enter, dấu chấm, dấu phẩy...), từ hiện tại được coi là đã hoàn tất (`Committed`) và chuyển vào Ring Buffer.
- **Quy tắc Xử lý Backspace Khi Buffer Rỗng (`on_backspace_when_idle`):**
  - Khi người dùng gõ một từ mới, `raw_buffer` khởi đầu hoàn toàn sạch.
  - Tuyệt đối không tự ý ghép nối ký tự của từ mới vào từ cũ đã nằm trong Ring Buffer.

### 3.4. Bộ Nhận diện Ngôn ngữ và Tự động Khôi phục Tiếng Anh (`english.rs`, `spelling.rs`)
- **Tập Từ điển Nhúng:**
  - `vietnamese.cm.dict`: Bảng từ điển âm tiết tiếng Việt nén tối ưu bộ nhớ.
  - `english_words.txt`: Danh mục các từ tiếng Anh và thuật ngữ lập trình phổ biến.
- **Bảo vệ Từ Lập trình và Từ Mượn:**
  - Nhận diện các từ tiếng Anh có chứa các tổ hợp phím gõ dấu tiếng Việt: `test` (chứa `s`), `windows` (chứa `w`, `s`), `push` (chứa `s`), `git`, `server`, `code`, `commit`.
  - Khi người dùng gõ xong các từ này và nhấn Space, bộ gõ tự động giữ nguyên định dạng tiếng Anh gốc thay vì tạo ra từ tiếng Việt vô nghĩa (như `tét`, `ưindoưs`).

### 3.5. Thuật toán Sửa sai Hồi quy Hai Từ Liên tiếp (`bigram.rs`)
- Trong trường hợp một từ đứng một mình có thể là cả tiếng Anh lẫn tiếng Việt (ví dụ: `Last` vs `Lát`):
  - Khi người dùng gõ `Last` + Space, ban đầu có thể tạm thời hiển thị `Last`.
  - Nhưng khi người dùng gõ tiếp từ thứ hai là `nuwax` (`nữa`), thuật toán Bigram nhận diện cụm `Lát nữa` là một ngữ cảnh tiếng Việt có độ tin cậy tuyệt đối:
    1. Phát lệnh xóa lùi qua ký tự Space và từ trước.
    2. Viết lại từ thứ nhất thành `Lát`.
    3. Thêm lại Space và chèn từ thứ hai `nữa`.
  - Toàn bộ thao tác diễn ra trong chớp mắt, tạo cảm giác bộ gõ "thấu hiểu" ý định của người viết.

### 3.6. Máy Trạng thái Soạn thảo (Composition State Machine)
```
                  Gặp ký tự phân cách (Space, Enter, Punctuation)
         ┌──────────────────────────────────────────────────────────────┐
         │                                                              ▼
   ┌───────────┐         Ký tự chữ cái         ┌───────────┐     Phím dấu / Mũ      ┌───────────┐
   │   IDLE    │ ────────────────────────────► │ COMPOSING │ ─────────────────────► │  REPLACE  │
   └───────────┘                               └───────────┘                        └─────┬─────┘
         ▲                                           │                                    │
         │             Phím xóa Backspace            │               Phát sự kiện         │
         └───────────────────────────────────────────┴────────────────────────────────────┘
```
- Khi ở trạng thái `COMPOSING`, mỗi phím biến đổi dấu sẽ sinh ra một cấu trúc:
  ```rust
  pub struct Replacement {
      pub delete_before_cursor: usize, // Số ký tự cần xóa lùi
      pub insert: String,              // Chuỗi ký tự mới cần chèn vào
      pub cursor_offset: isize,        // Vị trí con trỏ sau chèn
  }
  ```

---

# PHẦN 4: TẦNG 3 - XUẤT PHÍM & GIAO THỨC HIỂN THỊ (INJECTION LAYER)

Vị trí mã nguồn: [`protocol/src/wayland_vk.rs`](file:///home/thanhhd/Git-projects/vietc/protocol/src/wayland_vk.rs), [`protocol/src/wayland_vk_keymap.rs`](file:///home/thanhhd/Git-projects/vietc/protocol/src/wayland_vk_keymap.rs), [`protocol/src/uinput_monitor.rs`](file:///home/thanhhd/Git-projects/vietc/protocol/src/uinput_monitor.rs).

### 4.1. Điều phối Giao thức Kép: Wayland Virtual Keyboard vs Linux Uinput
VietC áp dụng mô hình phối hợp giao thức linh hoạt:
1. **Wayland Virtual Keyboard (`zwp_virtual_keyboard_v1`):**  
   Được sử dụng làm kênh xuất phím mặc định và ưu tiên cao nhất trên môi trường Wayland (COSMIC Desktop, GNOME, KDE, Hyprland). Kết nối trực tiếp với Wayland Compositor qua Unix Socket, cho phép chèn ký tự Unicode chính xác tuyệt đối mà không cần can thiệp Clipboard.
2. **Linux `/dev/uinput` Fallback:**  
   Được sử dụng khi chạy trong môi trường X11 truyền thống, phiên làm việc Linux Console (TTY), hoặc khi Wayland Compositor không hỗ trợ virtual keyboard protocol.

### 4.2. Bố cục Keymap Tĩnh XKB 8 Cấp độ (`wayland_vk_keymap.rs`)
Đây là phát minh kỹ thuật đột phá của VietC nhằm loại bỏ hoàn toàn hiện tượng nhấp nháy gạch chân:
- **Nguyên lý Upload Duy nhất 1 Lần:** Keymap XKB hoàn chỉnh được biên dịch thành chuỗi định dạng XKB v1 và đẩy lên Compositor thông qua shared memory (`memfd_create`) **duy nhất một lần** trong suốt vòng đời của daemon.
- **Cấu trúc 8 Cấp độ (`VIETC_EIGHT_LEVEL`):**
  Mỗi phím chữ cái vật lý (từ `KEY_Q` đến `KEY_M`) được định nghĩa 8 ký tự tương ứng với các trạng thái tổ hợp modifier:
  - Cấp 1: Ký tự chữ thường cơ bản (Base, ví dụ: `a`)
  - Cấp 2: Ký tự chữ hoa cơ bản (`Shift`, ví dụ: `A`)
  - Cấp 3: Ký tự tiếng Việt thường nhóm 1 (`Mod5`, ví dụ: `á`)
  - Cấp 4: Ký tự tiếng Việt hoa nhóm 1 (`Shift + Mod5`, ví dụ: `Á`)
  - Cấp 5: Ký tự tiếng Việt thường nhóm 2 (`Mod3`, ví dụ: `à`)
  - Cấp 6: Ký tự tiếng Việt hoa nhóm 2 (`Shift + Mod3`, ví dụ: `À`)
  - Cấp 7: Ký tự tiếng Việt thường nhóm 3 (`Mod5 + Mod3`, ví dụ: `ả`)
  - Cấp 8: Ký tự tiếng Việt hoa nhóm 3 (`Shift + Mod5 + Mod3`, ví dụ: `Ả`)
- Nhờ cấu trúc này, mọi ký tự tiếng Việt có sẵn dấu thanh đều có thể được phát ra tức thì bằng cách kích hoạt phím vật lý cùng mặt nạ modifier tương ứng, hoàn toàn không cần giả lập Unicode Hex gõ chậm.

### 4.3. Giới hạn Dải Keycode An toàn (`1..=111`) Tránh Xung đột Trình duyệt
- **Vấn đề Kỹ thuật:** Trong nhân Linux, các keycode $\ge 112$ tương ứng với các phím đặc biệt (ví dụ: 127 = `KEY_COMPOSE`/`ContextMenu`, 131 = `KEY_UNDO`, 133 = `KEY_COPY`, 137 = `KEY_CUT`, 135 = `KEY_PASTE`). Trình duyệt Chromium, Electron (VS Code, Slack) lắng nghe các keycode này ở cấp độ sâu và sẽ tự động kích hoạt menu ngữ cảnh hoặc thực hiện lệnh Paste ngoài ý muốn.
- **Quy tắc Bất biến:** Toàn bộ keymap ảo của VietC chỉ sử dụng các keycode trong dải an toàn từ `1` đến `111` (bao gồm các phím số, chữ cái và ký tự đặc biệt tiêu chuẩn trên bàn phím ANSI/ISO 104-key).

### 4.4. Quy trình Thay thế Văn bản Nguyên tử (`replace_text`)
Mỗi giao dịch thay thế ký tự diễn ra tuần tự và nguyên tử qua 6 bước khép kín:
1. **Đóng băng Modifier Vật lý:** Tạm thời nhả trạng thái `Shift` hoặc `CapsLock` của người dùng để tránh làm biến dạng thao tác xóa lùi.
2. **Phát chuỗi Backspace:** Gửi chính xác $N$ sự kiện `KEY_BACKSPACE` (mỗi sự kiện gồm KeyDown và KeyUp liên tục) để xóa đúng số lượng ký tự hiển thị cũ.
3. **Flush Xóa:** Đồng bộ sự kiện xóa tới Compositor để đảm bảo con trỏ đã lùi về vị trí mong muốn.
4. **Phát ký tự Mới:** Thiết lập bitmask modifier tương ứng với Level của ký tự tiếng Việt, gửi sự kiện KeyDown + KeyUp của phím vật lý mang ký tự đó.
5. **Khôi phục Modifier Vật lý:** Trả lại trạng thái `Shift`/`CapsLock` ban đầu của người dùng.
6. **Flush Giao dịch:** Hoàn tất transaction, giải phóng lock.

### 4.5. Phân tích Hiện tượng Đè phím (Key Overlap) và Giải pháp Atomic Space
- **Hiện tượng Dính chữ (`báovới`, `màyvừasửa`):**  
  Khi gõ nhanh (80 - 120 WPM), ngón tay người dùng bấm phím `Space` và ngay lập tức bấm tiếp ký tự đầu tiên của từ tiếp theo (ví dụ: chữ `v` trong `vừa`) trước khi phím `Space` kịp nhả lên.
  - Nếu hệ thống gửi phím `Space` theo cơ chế phân tách (KeyDown khi bấm, KeyUp khi nhả), sự kiện KeyDown của phím `v` sẽ xen vào giữa:
    $$\text{KeyDown}(\text{Space}) \rightarrow \text{KeyDown}('v') \rightarrow \text{KeyUp}(\text{Space}) \rightarrow \text{KeyUp}('v')$$
  - Một số ứng dụng (đặc biệt là Electron, Chromium) sẽ tối ưu hóa việc phân tách từ hoặc bỏ qua khoảng trắng bị kẹp giữa hai lần bấm ký tự, dẫn đến việc khoảng trắng bị nuốt chửng và hai từ dính liền thành một khối.
- **Giải pháp Atomic Space:**  
  Khi phát hiện ký tự phân cách (`KEY_SPACE`), daemon luôn phát **nguyên tử cả cụm KeyDown và KeyUp liên tiếp ngay lập tức** trong cùng một transaction. Nhờ đó, khoảng trắng luôn được chèn hoàn tất vào tài liệu trước khi ký tự của từ tiếp theo bắt đầu xuất hiện.

### 4.6. Trọng tài Đơn điệu Thời gian (Monotonic Timestamp FIFO)
- Mọi sự kiện phím ảo phát ra từ VietC đều được gắn timestamp tăng đơn điệu:
  $$t_{\text{event}_{i+1}} \ge t_{\text{event}_i}$$
- Nghiêm cấm tình trạng sự kiện chèn ký tự có timestamp nhỏ hơn hoặc bằng sự kiện xóa Backspace đứng trước nó, ngăn ngừa triệt để lỗi Chromium sắp xếp lại sự kiện và làm đảo lộn trật tự văn bản.

---

# PHẦN 5: BẢO VỆ GIAO DIỆN & CÁC TRƯỜNG HỢP BIÊN ĐẶC BIỆT (EDGE CASES)

### 5.1. Chrome / Chromium Omnibox Autocomplete & Selection Shield
Vị trí mã nguồn: [`daemon/src/focus_tracker.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/focus_tracker.rs), [`daemon/src/focus.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/focus.rs).
- **Vấn đề:** Khi gõ vào thanh địa chỉ của Google Chrome hoặc Microsoft Edge, trình duyệt tự động gợi ý URL và bôi đen phần gợi ý (ví dụ: gõ `h` $\rightarrow$ gợi ý `h[ttps://google.com]`). Nếu người dùng gõ tiếp `o` để tạo thành `hòa`, việc gửi phím `Backspace` sẽ xóa sạch toàn bộ vùng bôi đen thay vì chỉ xóa chữ `o`.
- **Cơ chế Omnibox Shield:** VietC lắng nghe sự kiện `TextSelectionChanged` từ AT-SPI. Khi phát hiện vùng bôi đen nội tuyến (inline selection) được kích hoạt bởi trình duyệt, daemon sẽ phát phím mũi tên phải (`KEY_RIGHT`) để giải phóng vùng chọn trước khi thực thi việc thay thế ký tự tiếng Việt.

### 5.2. Trình soạn thảo Hiện đại (VS Code, Sublime Text, JetBrains)
- Các IDE hiện đại thường có tính năng tự động đóng ngoặc (`Auto Closing Quotes / Brackets`) và hỗ trợ nhiều con trỏ đồng thời (Multi-Cursor).
- VietC đảm bảo không sử dụng phím giả lập di chuyển con trỏ bất thường, duy trì tính nguyên tử trong mỗi lệnh thay thế để không làm gãy ngữ cảnh của các extension phân tích cú pháp (Language Server Protocol - LSP).

### 5.3. Trình giả lập Thiết bị đầu cuối (Terminal Emulators & Shells)
- Các terminal như Alacritty, Foot, Kitty, GNOME Terminal hoạt động ở chế độ ký tự thô (Raw Mode/Readline).
- VietC tôn trọng hoàn toàn các tổ hợp phím điều khiển hệ thống (`Ctrl+C`, `Ctrl+D`, `Ctrl+Z`, `Ctrl+L`) và chế độ phím `vi-mode` trong Zsh/Bash, chuyển tiếp trực tiếp các lệnh này mà không can thiệp.

### 5.4. Ứng dụng Nền tảng Electron / CEF / Slack / Discord
- Ứng dụng Electron chạy một vòng lặp sự kiện đồ họa riêng biệt và rất nhạy cảm với tốc độ đẩy phím ảo.
- Việc loại bỏ hoàn toàn các lệnh `thread::sleep` chặn luồng và thay thế bằng cơ chế `connection.flush()` của Wayland đảm bảo các thông điệp luôn được xử lý mượt mà, không gây hiện tượng "chữ bay" hay lag giao diện.

---

# PHẦN 6: BỘ BẤT BIẾN KIẾN TRÚC CỐT TỬ (ARCHITECTURAL INVARIANTS)

Bất kỳ lập trình viên nào khi chỉnh sửa mã nguồn VietC đều phải tuân thủ nghiêm ngặt 6 Bất biến sau đây. Vi phạm bất kỳ bất biến nào đều bị coi là lỗi kiến trúc nghiêm trọng:

### ⚠️ BẤT BIẾN 1: TÍNH ĐỘC LẬP CỦA RANH GIỚI TỪ (WORD BOUNDARY ISOLATION)
- Khi một từ được kết thúc bằng phím phân cách (`Space`, Enter, dấu câu), từ đó thuộc về văn bản của tài liệu.
- **CẤM:** Tuyệt đối không tự ý "hồi sinh" (rehydrate) từ cũ vào `raw_buffer` khi người dùng đang bắt đầu gõ một từ mới.
- Một từ mới phải được khởi tạo với trạng thái hoàn toàn độc lập, không cho phép ký tự của từ mới bị ghép nối ngầm vào chuỗi phím thô của từ trước đó.

### ⚠️ BẤT BIẾN 2: THỨ TỰ SỰ KIỆN FIFO & TIMESTAMP ĐƠN ĐIỆU TUYỆT ĐỐI
- Mọi phím ảo phát ra phải tuân thủ thứ tự FIFO và có timestamp tăng dần đều.
- Không bao giờ cho phép sự kiện ký tự chèn đến trước sự kiện xóa Backspace trong cùng một transaction.

### ⚠️ BẤT BIẾN 3: XUẤT PHÍM PHÂN CÁCH NGUYÊN TỬ (ATOMIC DELIMITER DISPATCH)
- Phím `KEY_SPACE` và các ký tự phân cách phải luôn được phát nguyên tử (KeyDown + KeyUp lập tức) trên cùng một kênh xuất phím với các ký tự văn bản.
- Loại bỏ hoàn toàn khả năng phím Space bị giữ lại khi phím ký tự tiếp theo đã được nhấn xuống.

### ⚠️ BẤT BIẾN 4: ZERO-SLEEP TRONG LUỒNG XỬ LÝ SỰ KIỆN (NON-BLOCKING PROTOCOL)
- Nghiêm cấm việc đặt các lệnh tạm dừng luồng như `std::thread::sleep` trong luồng xử lý chính `evdev_loop` hoặc giao tiếp Wayland.
- Mọi thao tác chờ đồng bộ phải sử dụng cơ chế Non-blocking I/O hoặc Socket Flush của giao thức.

### ⚠️ BẤT BIẾN 5: CỔNG KIỂM SOÁT KÍCH THƯỚC MÃ NGUỒN (SOURCE SIZE QUALITY GATE)
- Để tránh việc mã nguồn phình to mất kiểm soát dẫn đến code bẩn khó bảo trì, hệ thống duy trì công cụ kiểm tra tự động `scripts/check_source_size.py`:
  - [`daemon/src/app_state.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/app_state.rs) $\le$ 1,239 dòng.
  - [`daemon/src/evdev_loop.rs`](file:///home/thanhhd/Git-projects/vietc/daemon/src/evdev_loop.rs) $\le$ 1,402 dòng.
  - Mọi module/file Rust khác $\le$ 1,000 dòng.
- Khi cần bổ sung logic phức tạp, phải tách module chức năng độc lập thay vì nhồi nhét vào các file điều phối chính.

### ⚠️ BẤT BIẾN 6: NGUYÊN TẮC BÁC BỎ VÁ LỖI CỤC BỘ (CLEAN STATE MACHINE TRANSITIONS)
- Tuyệt đối cấm kiểu sửa lỗi chắp vá: chèn các câu lệnh kiểm tra ngoại lệ `if` rải rác để che giấu triệu chứng mà không giải quyết tận gốc nguyên nhân.
- Mọi cách khắc phục lỗi phải dựa trên việc chuẩn hóa các chuyển trạng thái (State Transitions) của State Machine và bảo toàn tính toàn vẹn của dữ liệu.

---

# PHẦN 7: QUY TRÌNH KIỂM THỬ, XÁC MINH & VẬN HÀNH (VERIFICATION RUNBOOK)

Trước khi nghiệm thu bất kỳ thay đổi nào vào nhánh chính (`main`), kỹ sư bắt buộc phải thực hiện đầy đủ quy trình kiểm thử 3 bước sau:

### 7.1. Bộ Kiểm tra Chất lượng Tĩnh (Static Analysis Suite)
Chạy lần lượt các lệnh sau từ thư mục gốc của repository:
```bash
# 1. Kiểm tra giới hạn kích thước dòng code
python3 scripts/check_source_size.py

# 2. Kiểm tra định dạng chuẩn Rust
cargo fmt --check

# 3. Kiểm tra cảnh báo linter Clippy nghiêm ngặt
cargo clippy --all-targets -- -D warnings
```
Yêu cầu: Cả 3 lệnh phải trả về exit code `0` mà không có bất kỳ warning hay violation nào.

### 7.2. Kiểm thử Tách biệt Môi trường Đồ họa (Headless Sandbox Testing)
Để ngăn các test case mở kết nối tới Wayland/X11 thật của máy phát triển gây nhiễu, bắt buộc phải unset toàn bộ biến môi trường đồ họa:
```bash
env -u DISPLAY -u WAYLAND_DISPLAY -u VIETC_RUNTIME_TESTS -u VIETC_CONTROLLED_DISPLAY \
  cargo test --workspace
```
Yêu cầu: Toàn bộ Unit Test và Integration Test của tất cả các crate (`vietc-engine`, `vietc-protocol`, `vietc-daemon`) phải vượt qua 100%.

### 7.3. Kịch bản Kiểm thử Áp lực Người dùng Văn phòng (Office Stress Test Suite)
Bộ test case giả lập hành vi gõ thực tế của người dùng văn phòng với tốc độ cao và sửa lỗi liên tục:
1. **Kịch bản Gõ Dấu Tự Do và Dấu Kiểu Cũ:**
   - Chuỗi gõ: `hoaf` $\rightarrow$ Kết quả: `hòa`
   - Chuỗi gõ: `thuyr` $\rightarrow$ Kết quả: `thủy`
   - Chuỗi gõ: `toans hocj` $\rightarrow$ Kết quả: `toán học`
2. **Kịch bản Ngăn Ngừa Dính Chữ (Anti-Sticky Words):**
   - Chuỗi gõ: `mayf vuwaf suwar` $\rightarrow$ Kết quả: `mày vừa sửa ` (Đảm bảo đầy đủ dấu cách giữa các từ, không bị `màyvừasửa`).
   - Chuỗi gõ: `thoong baos vowis mayf` $\rightarrow$ Kết quả: `thông báo với mày ` (Không bị dính `báovới`).
3. **Kịch bản Nhận diện Tiếng Anh Không Bị Biến Dạng:**
   - Chuỗi gõ: `open windows ` $\rightarrow$ Kết quả: `open windows ` (Không bị thành `ưindoưs`).
   - Chuỗi gõ: `git push origin main ` $\rightarrow$ Kết quả: `git push origin main ` (Không bị thành `púh`).
4. **Kịch bản Sửa Lỗi Typo và Xóa Lùi (Backspace Cascade):**
   - Gõ `vieetj`, nhấn Backspace 1 lần $\rightarrow$ `việt` biến thành `việ`, gõ tiếp `c` $\rightarrow$ `việc`.
   - Gõ từ hoàn chỉnh, gõ Space, nhấn Backspace xóa Space, gõ tiếp ký tự $\rightarrow$ Chữ cái được nối mượt mà hoặc bắt đầu từ mới độc lập mà không làm hỏng dữ liệu hiển thị.

---
*(Hết tài liệu kiến trúc hệ thống VietC)*
