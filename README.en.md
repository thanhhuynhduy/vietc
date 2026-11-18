# VietC

<p align="center">
  <strong>Pure <code>evdev</code>/<code>uinput</code> Bilingual Vietnamese–English Input Method for Linux (Wayland &amp; X11)</strong><br/>
  <a href="README.md">Tài liệu Tiếng Việt</a>
</p>

**VietC** is a standalone Linux Vietnamese input method operating directly at the kernel input layer (`evdev` + `uinput`), **with zero dependency on IBus or Fcitx5**. It delivers zero-underline typing across Wayland (GNOME, KDE Plasma, COSMIC, Hyprland, Sway) and X11.

Its [`vietc-engine`](engine/) core implements an Android **Gboard-inspired Dual-Hypothesis Bilingual Decoder** paired with traditional **old-style Vietnamese tone placement (`hòa`, `khỏe`, `thủy`)**.

---

## Highlights

- **Pure `evdev` + `uinput` Architecture (Rootless via `uaccess`):**
  - Captures hardware keystrokes via `/dev/input/event*` and commits text via `/dev/uinput` plus atomic clipboard transactions (`ext-data-control-v1` / `zwlr-data-control-v1` on Wayland, X11 Selection on X11).
  - Uses active-seat `systemd-logind` ACLs (`TAG+="uaccess", GROUP="root", MODE="0600"` in [`70-vietc.rules`](packaging/70-vietc.rules)) — runs as an unprivileged user service without `root`, Linux capabilities, or `input` group membership.
- **Gboard-Inspired Bilingual Vietnamese–English Engine:**
  - **Dual-Hypothesis Tracking:** Maintains the exact `raw_buffer` alongside the composed Vietnamese syllable on every keystroke.
  - **Bigram Language Context (`prev_context`) + Unigram Frequency Tiers:** Resolves valid Telex collisions between English and Vietnamese using preceding-word context:
    - `"bánh tét"` stays Vietnamese, while `"cargo test"` restores to English.
    - `"cảm ơn"` stays Vietnamese, while `"my own"` restores to English.
    - `"hai bên"` stays Vietnamese, while `"have been"` restores to English.
  - **Backspace-after-Space Auto-Restore Undo:** If an ambiguous word is auto-restored on `Space` (e.g. `"test "` instead of `"tét"`), pressing `Backspace` immediately reverts to `"tét"` and locks Vietnamese priority for the next `Space`.
  - **Technical Token & Double-Letter Protection:** Preserves `DDoS`, `GraphQL`, `iPhone`, `u32`, `i64`, `mp4`, `sha256`, English double-letter words (`cheese`, `loose`, `keeper`, `stuff`, `business`, `address`, `IEEE`), and common Vietnamese shorthand (`đc`, `đk`, `ko`, `vs`).
- **Old-Style Tone Placement & Free-Order Telex / VNI:**
  - Open diphthongs `oa`, `oe`, `uy` place tones on the first vowel (`hòa`, `hóa`, `khỏe`, `xòe`, `thủy`, `túy`), automatically shifting to the second vowel when closed by a final consonant (`hoàn`, `toán`, `huỳnh`, `tuýt`).
  - Full **Telex** and **VNI** support including `z` / `0` tone removal, shape overrides (`aaw` → `ă`, `oow` → `ơ`, `a68` → `ă`, `o67` → `ơ`), `-ươu` triphthongs (`huouw` → `hươu`, `ruouwj` → `rượu`), and free-order `qu`/`gi` glides (`gifa` → `già`, `qusa` → `quá`, `quo7` → `quơ`, `dan9` → `đan`).
- **Password & Remote Desktop Safety:**
  - Automatically bypasses composition in password fields via AT-SPI2 (`ROLE_PASSWORD_TEXT`), terminal credential prompts (`sudo`, `passwd`, `pkexec`, `ssh`, `gpg`, `cryptsetup`), and remote desktop / VM / game windows.

---

## Installation

### From Source (Recommended)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
sudo bash ./install.sh --from-source
```

Log out and log back in once after initial installation so `systemd-logind` applies `uaccess` device ACLs to your desktop session.

### Debian / Ubuntu `.deb` Package

```bash
make deb
sudo apt install ./packaging/deb/vietc_0.2.0-1_amd64.deb
```

### Uninstall

```bash
sudo bash ./uninstall.sh
```

---

## Shortcuts & Usage

| Action | Default | Notes |
|---|---|---|
| Toggle Vietnamese / English | `Ctrl + Space` | Configurable via `toggle_key` in `config.toml` |
| Switch Input Method (`Telex` ↔ `VNI`) | `Ctrl + Left Shift` | Configurable via `toggle_method_key` in `config.toml` |
| Strip Active Tone | `z` (Telex) / `0` (VNI) | e.g. `toansz` → `toan`, `toan10` → `toan` |
| Undo Auto-Restore | `Backspace` right after `Space` | e.g. `"test "` + `Backspace` → `"tét"` |
| System Tray UI (optional) | `vietc-tray` | StatusNotifierItem tray icon for quick toggles |
| Interactive CLI Harness | `vietc-cli` | Test engine behavior directly in terminal |

---

## Development & Verification

```bash
make fmt
make lint
make test-ci-smoke
```

## License

Released under the [MIT License](LICENSE).
