# Dboard

<p align="center">
  <strong>Pure <code>evdev</code>/<code>uinput</code> Bilingual Vietnamese–English Input Method for Linux Wayland (COSMIC, Hyprland, Sway — US QWERTY)</strong><br/>
  <a href="README.md">Tài liệu Tiếng Việt</a>
</p>

**Dboard** is a standalone Linux Vietnamese input method operating directly at the kernel input layer (`evdev` + `zwp_virtual_keyboard_v1` / `uinput`), **with zero dependency on IBus or Fcitx5**. It delivers zero-underline typing on Wayland compositors supporting `zwp_virtual_keyboard_v1` (**COSMIC Desktop, Hyprland, Sway, and wlroots compositors**) with **US QWERTY** keyboards.

> **Supported Scope:** Wayland (`COSMIC`, `Hyprland`, `Sway` / `wlroots`) + `US QWERTY` only. GNOME Wayland, KDE Plasma Wayland, X11, and non-US keyboard layouts are not supported.

Its [`dboard-engine`](engine/) core implements an Android **Gboard-inspired Dual-Hypothesis Bilingual Decoder** paired with traditional **old-style Vietnamese tone placement (`hòa`, `khỏe`, `thủy`)**.

---

## Highlights

- **Pure `evdev` + `zwp_virtual_keyboard_v1` / `uinput` Architecture (Rootless via `uaccess`):**
  - Captures hardware keystrokes via `/dev/input/event*` and commits Unicode text directly on Wayland via `zwp_virtual_keyboard_v1` (8-level US QWERTY XKB keymap, zero-clipboard, zero-underline), using Wayland atomic clipboard only for out-of-keymap symbols/emoji in user macros.
  - Uses active-seat `systemd-logind` ACLs (`TAG+="uaccess"` scoped to keyboard/mouse/touchpad `/dev/input/event*` devices, and `TAG+="uaccess", GROUP="root", MODE="0600"` for `/dev/uinput` in [`70-dboard.rules`](packaging/70-dboard.rules)) — runs as an unprivileged user service without `root`, Linux capabilities, or `input` group membership.
- **Gboard-Inspired Bilingual Vietnamese–English Engine:**
  - **Dual-Hypothesis Tracking:** Maintains the exact `raw_buffer` alongside the composed Vietnamese syllable on every keystroke.
  - **Bigram Language Context (`prev_context`) + Unigram Frequency Tiers:** Resolves valid Telex collisions between English and Vietnamese using preceding-word context:
    - `"bánh tét"` stays Vietnamese, while `"cargo test"` restores to English.
    - `"cảm ơn"` stays Vietnamese, while `"my own"` restores to English.
    - `"hai bên"` stays Vietnamese, while `"have been"` restores to English.
  - **Backspace-after-Space Auto-Restore Undo:** If an ambiguous word is auto-restored on `Space` (e.g. `"test "` instead of `"tét"`), pressing `Backspace` immediately reverts to `"tét"` and locks Vietnamese priority for the next `Space`.
  - **Technical Token, ALLCAPS Acronym & Double-Letter Protection:** Preserves `AWS`, `DDOS`, `ECS`, `ORM`, `GIF`, `OOO`, `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, `err`, `arr`, `nextjs`, `nuxtjs`, English double-letter words (`cheese`, `loose`, `keeper`, `stuff`, `business`, `address`, `IEEE`), and common Vietnamese shorthand (`đc`, `đk`, `ko`, `vs`).
- **Old-Style Tone Placement & Free-Order Telex / VNI:**
  - Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`), automatically shifting to the second vowel when closed by a final consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
  - Full **Telex** and **VNI** support including `z` / `0` tone removal, shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), `-ươu` triphthongs (`huouw` → `hươu`, `ruouwj` → `rượu`), and free-order `qu`/`gi` glides (`gifa` → `già`, `qusa` → `quá`, `quo7` → `quơ`, `dan9` → `đan`).
- **Password & Remote Desktop Safety:**
  - Automatically bypasses composition in password fields via AT-SPI2 (`ROLE_PASSWORD_TEXT`), terminal credential prompts (`sudo`, `passwd`, `pkexec`, `ssh`, `gpg`, `cryptsetup`), and remote desktop / VM / game windows.

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
| Undo Auto-Restore / Autocorrect | `Backspace` right after `Space` | e.g. `"test "` + `Backspace` → `"tét"` |
| System Tray UI (optional) | `dboard-tray` | StatusNotifierItem tray icon for quick VN/EN and Telex/VNI toggles |
| Environment Diagnostics | `dboard-daemon doctor` | Checks Wayland socket, `uaccess` ACLs, `/dev/uinput`, config & user dictionary |
| Interactive CLI Harness | `dboard-cli` | Test engine behavior directly in terminal |

---

## Development & Verification

```bash
make fmt
make lint
make test-ci-smoke
```

## Credits & Acknowledgments

This project builds upon the original [**vietc**](https://github.com/vndangkhoa/vietc) project created by [**Võ Nguyễn Đăng Khoa (@vndangkhoa)**](https://github.com/vndangkhoa). Special thanks to the original author for laying the foundation for a native Linux Vietnamese input method.

## License

Released under the [MIT License](LICENSE).
