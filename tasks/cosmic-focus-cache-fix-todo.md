# Checklist sửa lỗi stale Telex cache trên COSMIC Wayland

## Phase 1: Focus identity foundation

- [x] Task 1.1: Tách parser/validator cho active window ID
- [x] Task 1.2: Từ chối `0x0`, `0`, chuỗi rỗng và root/no-window sentinel
- [x] Task 1.3: Thêm unit tests cho output `xprop` hợp lệ và `0x0`
- [x] Task 1.4: Xác nhận COSMIC log không còn nhận `0x0` là window transition

### Checkpoint 1

- [x] `cargo test -p vietc-daemon app_state` pass
- [x] Strict Clippy cho `vietc-daemon` pass
- [x] Hyprland, Sway, GNOME và X11 identity hợp lệ không regression

## Phase 2: Focus generation

- [x] Task 2.1: Định nghĩa `FocusSnapshot`
- [x] Task 2.2: Thêm `identity`, `class`, `generation` và reliability
- [x] Task 2.3: Chỉ tăng generation khi có bằng chứng focus thay đổi
- [x] Task 2.4: Không tạo transition giả khi chỉ class metadata đổi
- [x] Task 2.5: Thay shared `active_window`/`window_class` bằng shared snapshot

### Checkpoint 2

- [x] Known identity change tăng generation đúng một lần
- [x] Unknown focus không tạo identity giả
- [x] Unit tests focus generation pass

## Phase 3: Composition invalidation

- [x] Task 3.1: Tạo helper reset toàn bộ composition state
- [x] Task 3.2: Helper reset engine buffer
- [x] Task 3.3: Helper clear replay event store và screen output
- [x] Task 3.4: Gọi helper khi focus generation thay đổi
- [x] Task 3.5: Reset focus-independent, không phụ thuộc `app_state.enabled`
- [x] Task 3.6: App policy evaluation vẫn chỉ chạy khi app-state bật

### Checkpoint 3

- [x] Target A composition không tồn tại sau generation change
- [x] `visible_revision`/replacement cũ không áp dụng vào target mới
- [x] Injection transaction tests pass

## Phase 4: Unknown-focus fallback

- [x] Task 4.1: Thêm named constant `FOCUS_UNKNOWN_COMPOSITION_TTL = 750 ms`
- [x] Task 4.2: Track thời điểm physical key cuối
- [x] Task 4.3: Reset composition trước phím mới nếu focus unknown và idle vượt TTL
- [x] Task 4.4: Viết fake-clock test cho buffer `hoor` hết hạn
- [x] Task 4.5: Viết test gõ liên tục dưới TTL không bị reset

### Checkpoint 4

- [x] Slow app switch không thể phát lại stale buffer
- [x] Continuous typing trong cùng từ không regression
- [x] Tests không dùng sleep thật

## Phase 5: COSMIC fast-switch signal

- [x] Task 5.1: Thêm AT-SPI focus watcher event-driven
- [x] Task 5.2: Chỉ theo dõi focus/application identity tối thiểu
- [x] Task 5.3: Không DFS Chromium accessibility tree
- [x] Task 5.4: Deduplicate event storm
- [x] Task 5.5: Watcher failure/unavailable degrade về unknown-focus TTL
- [x] Task 5.6: Không log text field hoặc nội dung người dùng

### Checkpoint 5

- [x] Edge → Telegram dưới 750 ms tạo generation mới khi AT-SPI khả dụng
- [x] Tắt AT-SPI không làm daemon crash
- [x] Unknown-focus fallback vẫn hoạt động

## Phase 6: Regression và runtime verification

- [x] Task 6.1: Automated two-target stale-composition regression test
- [x] Task 6.2: Runtime fixture Edge `con hổ` không Space → Telegram `xin chào`
- [x] Task 6.3: Kiểm tra switch nhanh dưới 750 ms
- [x] Task 6.4: Kiểm tra switch chậm trên 750 ms
- [x] Task 6.5: Lặp mỗi trường hợp tối thiểu 20 lần
- [x] Task 6.6: Edge core delta bằng 0
- [x] Task 6.7: Clipboard preservation pass
- [x] Task 6.8: Modifier release pass
- [x] Task 6.9: Chromium, Chrome, GTK, Qt và terminal runtime pass
- [x] Task 6.10: Cập nhật tài liệu investigation và testing dictionary

### Evidence Phase 6

- Fast switch 100 ms: 20/20 vòng pass, không stale transaction, không injection failure, core delta bằng 0.
- Slow switch 900 ms: 20/20 vòng pass, 39 lần TTL reset, không stale transaction, không injection failure, core delta bằng 0.
- Daemon giữ `ActiveState=active`, `SubState=running`, `NRestarts=0`.
- Chromium, Google Chrome, GTK, Qt và terminal đều pass với process healthy, clipboard preserved và modifiers released.
- Electron được ghi nhận `UNAVAILABLE`, không được tính là pass.

## Final quality gate

- [x] `cargo fmt --all -- --check`
- [x] `cargo test --workspace --all-targets`
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [x] Không còn log `window ID '' -> '0x0'`
- [x] Không còn output `hoorxin chào`
- [x] Không tái phát Edge SIGILL
- [ ] Working tree sạch sau commit