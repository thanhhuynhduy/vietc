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
  - **Dual-Hypothesis + Quantized 2/3/4-gram Kneser-Ney LM (`bilingual_lm.bin` 348 KB + `ngram_lm.bin` 2.63 MB):** $O(1)$ zero-allocation FNV-1a lookup (`p50 ~ 1.48 µs`, `p99 ~ 8.04 µs`), **Next-Word Prediction (100% top-3)**, and **99.92% word accuracy (99.02% sentence accuracy; 100% `pure_vn`/`pure_en`/`office`/`dev`, 95.24% `code_switch`)** across the 510-sentence bilingual benchmark:
    - `"bánh tét"` stays Vietnamese, while `"cargo test"` / `"mở test.py"` restores to English.
    - `"cảm ơn"` stays Vietnamese, while `"my own"` restores to English.
    - `"hai bên"` stays Vietnamese, while `"have been"` restores to English.
    - Retroactive 2-word disambiguation (`"Lát nữa"`, `"Lots of"`, `"Rust toolchain"`, `"REST API"`, `"this morning"`) with **One-Tap Undo** via `Backspace` immediately after `Space`.
  - **Mid-Sentence Dot-Filename Restoration:** Automatically restores `test.py`, `list.txt`, `host.conf`, `archive.tar.gz` typed mid-sentence while preserving Vietnamese sentence-ending periods (`"ăn bánh tét. hôm nay"`).
  - **Weighted Spatial SymSpell (Telex & VNI + ~35k English) + Personal Bigrams + Smart Auto-Capitalization:** Fixes missing/extra/adjacent-key typos and splits missing-space compounds for both **Telex** and **VNI** (`khoog` / `khog6` → `không`, `khoongbieets` / `khong6biet61` → `không biết`, `algoritm` → `algorithm`, `tomorow` → `tomorrow`) with background startup pre-warming, zero false positives on code/dev identifiers (`kubectl`, `hashmap`, `u32`, `sha256`), Smart Auto-Cap (`TP.`, `TS.`, `PGS.`, `Mr.`, `etc.`, `"Chào." Hôm`), and adaptive `0600` Personal Bigrams.
  - **Technical Token, ALLCAPS Acronym & camelCase Protection:** Preserves `dOs`, `cAs`, `tOs`, `iOs`, `AWS`, `DDOS`, `QĐ-TTg`, `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, `err`, `arr`, `nextjs`, `nuxtjs`, and prevents `đ` flash on English words containing medial `dd` (`add`, `reddit`, `hidden`, `middle`, `wedding`).
- **Old-Style Tone Placement & Free-Order Telex / VNI:**
  - Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`), automatically shifting to the second vowel when closed by a final consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
  - Full **Telex** and **VNI** support including cross-coda circumflex (`khongo` → `không`, `vietej` → `việt`, `tienges` → `tiếng`, `tana` → `tân`), `z` / `0` tone removal, shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), `-ươu` triphthongs (`huouw` → `hươu`, `ruouwj` → `rượu`), and `qu`/`gi` glides.
- **Optional Floating Candidate Strip (`dboard-popup` — Wayland Layer-Shell):**
  - Disabled by default (`candidate_bar = false`) or toggled in 1 click via **Preset: Gboard Mode (All On)** in `dboard-tray`. Uses non-blocking (`O_NONBLOCK` Unix Datagram `0600`) IPC to the standalone `dboard-popup` (`zwlr_layer_shell_v1`, `KeyboardInteractivity::None`) with anti-aliased TrueType rendering via pure-Rust **`swash`** (with 8×16 bitmap fallback), multi-monitor + HiDPI AT-SPI2 caret tracking, mouse click selection (`wl_pointer`), chained next-word prediction, mid-sentence re-selection (`tét` ↔ `test`), and tab-safe `Alt+1`/`Alt+2`/`Alt+3` shortcuts (automatically hidden in password fields).
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
