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

- [ ] Task 2.1: Định nghĩa `FocusSnapshot`
- [ ] Task 2.2: Thêm `identity`, `class`, `generation` và reliability
- [ ] Task 2.3: Chỉ tăng generation khi có bằng chứng focus thay đổi
- [ ] Task 2.4: Không tạo transition giả khi chỉ class metadata đổi
- [ ] Task 2.5: Thay shared `active_window`/`window_class` bằng shared snapshot

### Checkpoint 2

- [ ] Known identity change tăng generation đúng một lần
- [ ] Unknown focus không tạo identity giả
- [ ] Unit tests focus generation pass

## Phase 3: Composition invalidation

- [ ] Task 3.1: Tạo helper reset toàn bộ composition state
- [ ] Task 3.2: Helper reset engine buffer
- [ ] Task 3.3: Helper clear replay event store và screen output
- [ ] Task 3.4: Gọi helper khi focus generation thay đổi
- [ ] Task 3.5: Reset focus-independent, không phụ thuộc `app_state.enabled`
- [ ] Task 3.6: App policy evaluation vẫn chỉ chạy khi app-state bật

### Checkpoint 3

- [ ] Target A composition không tồn tại sau generation change
- [ ] `visible_revision`/replacement cũ không áp dụng vào target mới
- [ ] Injection transaction tests pass

## Phase 4: Unknown-focus fallback

- [ ] Task 4.1: Thêm named constant `FOCUS_UNKNOWN_COMPOSITION_TTL = 750 ms`
- [ ] Task 4.2: Track thời điểm physical key cuối
- [ ] Task 4.3: Reset composition trước phím mới nếu focus unknown và idle vượt TTL
- [ ] Task 4.4: Viết fake-clock test cho buffer `hoor` hết hạn
- [ ] Task 4.5: Viết test gõ liên tục dưới TTL không bị reset

### Checkpoint 4

- [ ] Slow app switch không thể phát lại stale buffer
- [ ] Continuous typing trong cùng từ không regression
- [ ] Tests không dùng sleep thật

## Phase 5: COSMIC fast-switch signal

- [ ] Task 5.1: Thêm AT-SPI focus watcher event-driven
- [ ] Task 5.2: Chỉ theo dõi focus/application identity tối thiểu
- [ ] Task 5.3: Không DFS Chromium accessibility tree
- [ ] Task 5.4: Deduplicate event storm
- [ ] Task 5.5: Watcher failure/unavailable degrade về unknown-focus TTL
- [ ] Task 5.6: Không log text field hoặc nội dung người dùng

### Checkpoint 5

- [ ] Edge → Telegram dưới 750 ms tạo generation mới khi AT-SPI khả dụng
- [ ] Tắt AT-SPI không làm daemon crash
- [ ] Unknown-focus fallback vẫn hoạt động

## Phase 6: Regression và runtime verification

- [ ] Task 6.1: Automated two-target stale-composition regression test
- [ ] Task 6.2: Runtime fixture Edge `con hổ` không Space → Telegram `xin chào`
- [ ] Task 6.3: Kiểm tra switch nhanh dưới 750 ms
- [ ] Task 6.4: Kiểm tra switch chậm trên 750 ms
- [ ] Task 6.5: Lặp mỗi trường hợp tối thiểu 20 lần
- [ ] Task 6.6: Edge core delta bằng 0
- [ ] Task 6.7: Clipboard preservation pass
- [ ] Task 6.8: Modifier release pass
- [ ] Task 6.9: Chromium, Chrome, GTK, Qt và terminal runtime pass
- [ ] Task 6.10: Cập nhật tài liệu investigation và testing dictionary

## Final quality gate

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --workspace --all-targets`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] Không còn log `window ID '' -> '0x0'`
- [ ] Không còn output `hoorxin chào`
- [ ] Không tái phát Edge SIGILL
- [ ] Working tree sạch sau commit