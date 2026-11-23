# Dboard

<p align="center">
  <strong>Pure <code>evdev</code>/<code>uinput</code> Bilingual Vietnamese–English Input Method for Linux Wayland (COSMIC, Hyprland, Sway — US QWERTY / Dvorak / Colemak)</strong><br/>
  <a href="README.md">Tài liệu Tiếng Việt</a>
</p>

**Dboard** is a standalone Linux Vietnamese input method operating directly at the kernel input layer (`evdev` + `zwp_virtual_keyboard_v1` / `uinput`), **with zero dependency on IBus or Fcitx5**. It delivers zero-underline typing on Wayland compositors supporting `zwp_virtual_keyboard_v1` (**COSMIC Desktop, Hyprland, Sway, and wlroots compositors**) with **US QWERTY, Dvorak, and Colemak** keyboards.

> **Supported Scope:** Pure Wayland (`COSMIC`, `Hyprland`, `Sway` / `wlroots`) + `US QWERTY`, `Dvorak`, `Colemak`. GNOME Wayland, KDE Plasma Wayland, and X11 are not supported.

Its [`dboard-engine`](engine/) core implements an Android **Gboard-inspired Dual-Hypothesis Bilingual Decoder & Quantized Statistical Language Model** paired with traditional **old-style Vietnamese tone placement (`hòa`, `khỏe`, `thủy`)**.

---

## Highlights

- **Pure `evdev` + `zwp_virtual_keyboard_v1` / `uinput` Architecture (Rootless via `uaccess`, Zero-Sleep Fast Path):**
  - Captures hardware keystrokes via `/dev/input/event*` and commits Unicode text directly on Wayland via `zwp_virtual_keyboard_v1` (8-level XKB keymap for US QWERTY / Dvorak / Colemak, zero-clipboard, zero-underline), using native multi-MIME Wayland atomic clipboard only for out-of-keymap symbols/emoji in user macros.
  - Uses active-seat `systemd-logind` ACLs (`TAG+="uaccess"` scoped to keyboard/mouse/touchpad `/dev/input/event*` devices, and `TAG+="uaccess", GROUP="root", MODE="0600"` for `/dev/uinput` in [`70-dboard.rules`](packaging/70-dboard.rules)) — runs as an unprivileged user service without `root`, Linux capabilities, or `input` group membership.
- **Gboard-Grade Quantized Statistical Bilingual Engine:**
  - **Dual-Hypothesis + Quantized Statistical LM (`bilingual_lm.bin`, ~340 KB):** Trained on Leipzig Corpora (Vietnamese/English Wikipedia + News) and dev/office code-switching seeds with $O(1)$ zero-allocation FNV-1a lookup (`p50 ~ 1.4 µs`, `p99 ~ 6.6 µs`), achieving **99.79% word accuracy (97.25% sentence accuracy)** across the 510-sentence bilingual benchmark:
    - `"bánh tét"` stays Vietnamese, while `"cargo test"` / `"mở test.py"` restores to English.
    - `"cảm ơn"` stays Vietnamese, while `"my own"` restores to English.
    - `"hai bên"` stays Vietnamese, while `"have been"` restores to English.
    - Retroactive 2-word disambiguation (`"Lát nữa"`, `"Lots of"`, `"Mã số"`, `"Too much"`) with **One-Tap Undo** via `Backspace` immediately after `Space`.
  - **Mid-Sentence Dot-Filename Restoration:** Automatically restores `test.py`, `list.txt`, `host.conf`, `archive.tar.gz` typed mid-sentence while preserving Vietnamese sentence-ending periods (`"ăn bánh tét. hôm nay"`).
  - **Spatial Autocorrect (Telex & VNI) + Smart Auto-Capitalization + Adaptive User Dictionary:** Fixes physical key-proximity slips (including VNI tone digits `1..9` with strict cross-row safety), auto-capitalizes sentences (with abbreviation guard for `TP.`, `TS.`, `PGS.`, `Mr.`, `etc.` and closing quotes `"Chào." Hôm`), and persists user vocabulary with `0600` permissions.
  - **Technical Token, ALLCAPS Acronym & camelCase Protection:** Preserves `dOs`, `cAs`, `tOs`, `iOs`, `AWS`, `DDOS`, `QĐ-TTg`, `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, `err`, `arr`, `nextjs`, `nuxtjs`, and prevents `đ` flash on English words containing medial `dd` (`add`, `reddit`, `hidden`, `middle`, `wedding`).
- **Old-Style Tone Placement & Free-Order Telex / VNI:**
  - Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`), automatically shifting to the second vowel when closed by a final consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
  - Full **Telex** and **VNI** support including cross-coda circumflex (`khongo` → `không`, `vietej` → `việt`, `tienges` → `tiếng`, `tana` → `tân`), `z` / `0` tone removal, shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), `-ươu` triphthongs (`huouw` → `hươu`, `ruouwj` → `rượu`), and `qu`/`gi` glides.
- **Optional Floating Candidate Strip (`dboard-popup` — Wayland Layer-Shell):**
  - Disabled by default (`candidate_bar = false`) to preserve pure zero-underline typing; when enabled, broadcasts non-blocking (`O_NONBLOCK` Unix Datagram `0600`) packets to the standalone `dboard-popup` (`zwlr_layer_shell_v1`, `KeyboardInteractivity::None`) rendering all 134 Vietnamese Unicode characters across 3 slots `[1] Vietnamese | [2] raw ASCII | [3] typo suggestion` (`Alt+1` / `Alt+2` / `Alt+3`, automatically hidden in password fields).
- **4-Layer Password Safety & Hardware Resilience:**
  - Automatically bypasses composition in password fields via AT-SPI2 (`ROLE_PASSWORD_TEXT` + 150ms grace window), lowercase Window Class/Title matching, kernel PTY termios (`ICANON && !ECHO`) for `sudo`/`ssh`/`passwd`/`su`/`doas`, `/proc` descendant inspection, safe Layer-Shell `FocusLost` reset (`Polkit`/launchers), Touchpad Tap-to-Click detection, and cross-device modifier merging across multiple USB/Bluetooth keyboards.

---

## Installation

### From Source (Recommended)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git dboard
cd dboard
sudo bash ./install.sh --from-source
```

Log out and log back in once after initial installation so `systemd-logind` applies `uaccess` device ACLs to your desktop session.

### Debian / Ubuntu `.deb` Package

```bash
make deb
sudo apt install ./packaging/deb/dboard_*.deb
```

*(You can also install offline from an extracted release tarball via `sudo bash ./install.sh`, or pass `--prebuilt` to download and verify the latest release against `SHA256SUMS`.)*

### Uninstall

```bash
sudo bash ./uninstall.sh
```

---

## Shortcuts & Usage

| Action | Default | Notes |
|---|---|---|
| Toggle Mode (`VN` ↔ `EN`) | `Ctrl + Space` | Defaults to `toggle_modes = ["vn", "en"]` (configurable to 3-way `["en", "vni", "telex"]` and custom `toggle_key` in `config.toml`); exact modifier matching |
| Strip Active Tone | `z` (Telex) / `0` (VNI) | e.g. `toansz` → `toan`, `toan10` → `toan` |
| Undo Auto-Restore / Autocorrect / Bigram | `Backspace` right after `Space` | e.g. `"test "` + `Backspace` → `"tét"`, `"Lát nữa "` + `Backspace` → `"Last nữa"` |
| Quick Candidate Select (`candidate_bar = true`) | `Alt + 1` / `Alt + 2` / `Alt + 3` | `[1]` Vietnamese, `[2]` Raw ASCII, `[3]` Autocorrect suggestion |
| System Tray & Candidate Strip (optional) | `dboard-tray` / `dboard-popup` | StatusNotifierItem tray icon and Wayland Layer-Shell floating candidate pill |
| Environment Diagnostics | `dboard-daemon doctor` | Checks Wayland socket, `uaccess` ACLs, `/dev/uinput`, `Mod3`/`Mod5` binds, layout & config |
| Interactive CLI Harness | `dboard-cli` | Test engine behavior directly in terminal |

---

## Development & Verification

```bash
make fmt
python3 scripts/check_source_size.py
make lint
make test-ci-smoke
make test-soak
```

## Credits & Acknowledgments

This project builds upon the original [**vietc**](https://github.com/vndangkhoa/vietc) project created by [**Võ Nguyễn Đăng Khoa (@vndangkhoa)**](https://github.com/vndangkhoa). Special thanks to the original author for laying the foundation for a native Linux Vietnamese input method.

## License

Released under the [MIT License](LICENSE).
