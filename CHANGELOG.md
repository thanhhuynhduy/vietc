# Changelog

## 0.2.0 — Pure `evdev` Architecture & Gboard Bilingual Engine

### Security
- Removed unverified third-party binary auto-updater (`git.khoavo.myds.me`) and AppImage self-overwrite code from `ui/src/tray.rs`.
- Standardized purely on `systemd-logind` active-seat `uaccess` rules (`70-vietc.rules` with `MODE="0600", GROUP="root"`); removed `input` group modifications (`usermod`/`adduser`) from installer and Debian packaging.
- Rewrote AT-SPI2 password field detection (`daemon/src/password_detector.rs`) to traverse the accessible tree and check `ROLE_PASSWORD_TEXT` accurately, and extended `/proc` descendant inspection for `sudo`, `passwd`, `pkexec`, `su`, `doas`, `ssh`, `gpg`, and `cryptsetup`.
- Removed plaintext keystroke and pasted-text logging from `evdev_loop.rs` and `uinput_monitor.rs`.
- Moved daemon single-instance lockfile to `$XDG_RUNTIME_DIR/vietc-daemon.lock` so `PrivateTmp=yes` services and manual invocations share the same lock namespace.

### Added
- **Gboard-inspired Dual-Hypothesis Bilingual Engine (`vietc-engine`):**
  - Preceding-word Bigram context (`LanguageContext`) + Unigram collision tiers (`english_dominant_collisions` vs `vietnamese_dominant_collisions`) to disambiguate bilingual sentences (`"bánh tét"` vs `"cargo test"`, `"cảm ơn"` vs `"my own"`, `"hai bên"` vs `"have been"`).
  - Gboard Backspace-after-Space undo: pressing `Backspace` immediately after an ambiguous auto-restore reverts to the Vietnamese syllable and locks Vietnamese priority for the next word boundary.
  - Full Telex `z` and VNI `0` tone removal, `-ươu` triphthong composition (`huouw` → `hươu`, `ruouwj` → `rượu`), free-order `qu`/`gi`/`oe` glides (`gifa` → `già`, `qusa` → `quá`, `khore` → `khỏe`, `quo7` → `quơ`, `dan9` → `đan`), and circumflex/breve/horn shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`).
  - Expanded English double-letter & technical token protection (`cheese`, `loose`, `keeper`, `stuff`, `business`, `address`, `DDoS`, `u32`, `i64`, `mp4`, `sha256`) and Vietnamese shorthand whitelist (`đc`, `đk`, etc.).

### Changed
- **Old-Style Vietnamese Tone Placement (`Kiểu cũ`):** Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `khỏe`, `thủy`) and automatically shift to the second vowel when closed by a coda consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
- Unified pure `evdev` + `uinput` architecture; removed all IBus, Fcitx5, Wayland IM (`zwp_input_method_v2`), X11 XRecord, `vietcctl`, `uinputd`, `vk`, `web`, and legacy PPA/AUR packaging code.
