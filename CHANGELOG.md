# Changelog

## 0.2.0 — Pure `evdev` Architecture & Gboard Bilingual Engine

### Security
- Removed unverified third-party binary auto-updater (`git.khoavo.myds.me`) and AppImage self-overwrite code from `ui/src/tray.rs`.
- Standardized purely on `systemd-logind` active-seat `uaccess` rules (`70-vietc.rules` with `MODE="0600", GROUP="root"`); removed `input` group modifications (`usermod`/`adduser`) from installer and Debian packaging.
- Rewrote AT-SPI2 password field detection (`daemon/src/password_detector.rs`) to traverse the accessible tree and check `ROLE_PASSWORD_TEXT` accurately, and extended `/proc` descendant inspection for `sudo`, `passwd`, `pkexec`, `su`, `doas`, `ssh`, `gpg`, and `cryptsetup`.
- Removed plaintext keystroke and pasted-text logging from `evdev_loop.rs` and `uinput_monitor.rs`.
- Moved daemon single-instance lockfile to `$XDG_RUNTIME_DIR/vietc-daemon.lock` so `PrivateTmp=yes` services and manual invocations share the same lock namespace.

### Added
- **Wayland Virtual Keyboard (`zwp_virtual_keyboard_v1`) Zero-Clipboard Backend (`vietc-protocol`):**
  - Persistent 8-level XKB keymap (`VIETC_EIGHT_LEVEL`) uploaded once via `memfd_create`, mapping all 134 Vietnamese Unicode characters within safe evdev keycodes `1..=111` (eliminating Chromium/Electron DomCode collisions and clipboard churn).
  - Monotonic timestamp FIFO, Shift handoff between `zwp_virtual_keyboard_v1` and `uinput`, and Atomic Space delimiter dispatch to prevent sticky words at high WPM.
  - Chrome/Edge Omnibox Inline Autocomplete Selection Shield (`KEY_DELETE` pre-clear on both typing transforms and `Backspace` edits).
- **Gboard-inspired Dual-Hypothesis Bilingual Engine (`vietc-engine`):**
  - Preceding-word Bigram context (`LanguageContext`), retroactive 2-word disambiguation (`"Lát nữa"`, `"Lots of"`, `"Mã số"`, `"Too much"`), and Unigram collision tiers (`english_dominant_collisions` vs `vietnamese_dominant_collisions`).
  - Guarded `Backspace`-across-`Space` rehydration (`CommittedRingBuffer`) and Gboard Space-Undo after auto-restore.
  - Character-granular coda replacement on mid-word `Backspace` after free-order tone triggers (`vieetj` + `Backspace` + `c` → `việc`, `dduowngf` + `Backspace*2` + `cj` → `được`).
  - Full Telex `z` and VNI `0` tone removal, `-ươu` triphthong composition (`huouw` → `hươu`, `ruouwj` → `rượu`), free-order `qu`/`gi`/`oe` glides (`gifa` → `già`, `qusa` → `quá`, `khore` → `khỏe`, `quo7` → `quơ`, `dan9` → `đan`), and circumflex/breve/horn shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`).
  - Expanded English double-letter (`dd`, `ee`, `oo`, `ss`, `ff`, `rr`), `w`-onset, path/URL token, and technical/early-Shift protection (`DDoS`, `add`, `wedding`, `worse`, `wonderful`, `geese`, `vieeTj` → `việt`, `u32`, `mp4`, `sha256`), plus Vietnamese `Đ/đ` abbreviation whitelist (`QĐ`, `HĐ`, `đc`, `đk`).

### Changed
- **Old-Style Vietnamese Tone Placement (`Kiểu cũ`):** Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `khỏe`, `thủy`) and automatically shift to the second vowel when closed by a coda consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
- Unified pure `evdev` + `zwp_virtual_keyboard_v1` / `uinput` architecture; removed all IBus, Fcitx5, Wayland IM (`zwp_input_method_v2`), X11 XRecord, `vietcctl`, `uinputd`, `vk`, `web`, and legacy PPA/AUR packaging code.

### Fixed
- **Exact Toggle Chord Matching (`daemon/src/event.rs`):** `Ctrl+Space` / `Alt+Z` now require an exact modifier set, preventing `Ctrl+Shift+Space` (VS Code Parameter Hints, LibreOffice NBSP), `Ctrl+Alt+Space`, and `Ctrl+Super+Space` from being hijacked.
- **Consumed-Key Lifecycle under Modifier Overlap (`daemon/src/evdev_loop.rs`):** Handled `KeyUp`/`KeyRepeat` cleanup for `consumed_keys` prior to modifier passthrough, eliminating stale keycode leaks and phantom repeats when releasing a consumed key while holding `Ctrl`/`Alt`/`Meta`.
- **Wayland Virtual Keyboard Rollover Release Absorption (`protocol/src/wayland_vk.rs`):** Tracked `early_released_bitmap` in `VkSession` so physical `KeyUp` events for rollover keys proactively released by `replace_text` are absorbed cleanly on VK instead of falling through to `/dev/uinput`.
- **Digraph Coda Backspace, Acronyms, Multi-Tone Overwrite & Telex `z` Guard (`vietc-engine`):**
  - Extended mid-word coda replacement to 2-character digraph codas `ng`/`nh`/`ch` (`tieengs` + `Backspace` + `t` → `tiết`).
  - Preserved ALLCAPS office/tech acronyms (`AWS`, `DDOS`, `ECS`, `ORM`, `GIF`, `TAR`, `RAR`, `EOF`, `COO`, `OOO`, `POS`, `EST`, `TOS`, `SOW`, `ATS`), double-key undo tech words (`err`, `arr`), multi-tone-overwrite tokens (`nextjs`, `nuxtjs`), and short programming identifiers (`var`, `usr`, `res`, `ref`, `pos`, `eof`).
  - Guarded intentional Telex `z` tone removal against `AutoRestore` on `Space` (`vieetjz ` → `viêt `) without affecting English words containing literal `z` (`setWindowSize`).
- **Chatter-Safe Key Deduplication Default (`daemon/src/config.rs`):** Lowered `default_dedup_window_ms` from `120ms` to `35ms` so mechanical switch chatter is filtered without dropping legitimate fast Telex double-taps (`aa`, `ee`, `oo`, `dd`).
