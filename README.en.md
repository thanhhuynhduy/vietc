# VietC

Vietnamese input method for Linux, supporting Wayland and X11, written in Rust.

This project is a continuation of the original VietC codebase, with comprehensive upgrades to the Vietnamese/English bilingual input engine (directly inspired by [Ngó Sen](https://github.com/ngosen/ngosen) and [bamboo-core](https://github.com/bamboo-vi/bamboo-core)), automated testing, clipboard preservation, and security hardening.

![License](https://img.shields.io/badge/license-MIT-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)

## ✨ Features

- **Wayland & X11 support** — works on all major Linux desktop environments.
- **Smart Vietnamese input engine** — supports Telex, VNI, and VIQR typing methods.
- **Bilingual Vietnamese/English handling** — context-aware recognition and dictionary prioritization.
- **Clipboard safety** — does not interfere with clipboard contents during typing operations.
- **Security hardened** — input device access restricted to active user sessions; no world-readable/writable permissions.
- **Automated testing** — extensive test suite ensures Vietnamese text processing quality.
- **Web dashboard** — visual control panel for configuration and monitoring.
- **Easy installation** — one-liner install script, Nix flake support.

## 🚀 Installation

### Quick install
```bash
curl -fsSL https://raw.githubusercontent.com/thanhhuynhduy/vietc/main/install.sh | bash
```

Or download and run:
```bash
wget https://raw.githubusercontent.com/thanhhuynhduy/vietc/main/install.sh
chmod +x install.sh
sudo ./install.sh
```

**Note:** The installer sets up udev rules and ACLs to grant input device access to the current user. After installation, reboot or run `sudo udevadm control --reload-rules && sudo udevadm trigger`.

### Manual installation
```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
cargo build --release
sudo cp target/release/vietc /usr/local/bin/
sudo cp target/release/vietc-daemon /usr/local/bin/
# Install udev rules & systemd service: run install.sh or do it manually
```

### Using Nix
```bash
nix build github:thanhhuynhduy/vietc
# or run directly
nix run github:thanhhuynhduy/vietc
```

## 🖥️ Usage

After installation, the input method starts automatically with the system. Defaults:
- **Toggle input method:** `Shift + Alt`
- **Web dashboard:** [http://localhost:1999](http://localhost:1999) (port configurable)

To run manually:
```bash
vietc-daemon
```

## 🛠️ Building from source

Requirements:
- Rust 1.75+
- `cargo` and system libraries (pkg-config, libxdo, libevdev, etc.)

```bash
git clone https://github.com/thanhhuynhduy/vietc.git
cd vietc
cargo build --release
```

Run tests:
```bash
cargo test --workspace
```

## ⚙️ Configuration

Default config file at `~/.config/vietc/config.toml`. Example:
```toml
[engine]
input_method = "Telex"          # Telex, VNI, VIQR
free_marking = true              # Allow free diacritic placement
spell_check = true               # English spell check

[shortcuts]
toggle_input = "Shift+Alt"

[web]
port = 1999
```

## 🌐 Web Dashboard

Visit [http://localhost:1999](http://localhost:1999) to view status, switch input method, and manage dictionaries.

The dashboard is built with React + TypeScript and bundled with the daemon.

## 🤝 Contributing

Contributions are welcome!
1. Fork the repo
2. Create a feature branch (`git checkout -b feature/amazing`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push (`git push origin feature/amazing`)
5. Open a Pull Request

## 📄 License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for details.

---

Built with ❤️ for the Vietnamese Linux community.
