#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# VietC — Pure evdev/uinput Vietnamese Input Method Installer
set -euo pipefail

# Privileged operations must never resolve executables from user-writable paths.
export PATH="/usr/sbin:/usr/bin:/sbin:/bin"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

FROM_SOURCE=false
PREBUILT=false
LANG_CODE="en"
case "${LANG:-}" in
  vi*|*_VN|*VN*) LANG_CODE="vi" ;;
esac
LANG_NEXT=0
for arg in "$@"; do
    if [ "$LANG_NEXT" = "1" ]; then
        case "$arg" in
            vi|vi_VN|vi-VN) LANG_CODE="vi" ;;
            en) LANG_CODE="en" ;;
        esac
        LANG_NEXT=0
    elif [ "$arg" = "--from-source" ] || [ "$arg" = "--local" ]; then
        FROM_SOURCE=true
    elif [ "$arg" = "--prebuilt" ]; then
        PREBUILT=true
    elif [ "$arg" = "--lang" ]; then
        LANG_NEXT=1
    elif [ "$arg" = "--lang=vi" ] || [ "$arg" = "--lang=vi_VN" ] || [ "$arg" = "--lang=vi-VN" ]; then
        LANG_CODE="vi"
    elif [ "$arg" = "--lang=en" ]; then
        LANG_CODE="en"
    fi
done

t() {
    local key="$1"; shift
    if [ "$LANG_CODE" = "vi" ]; then
        case "$key" in
            sudo) echo -e "${RED}Vui lòng chạy với quyền sudo.${NC}" ;;
            src_detected) echo -e "${YELLOW}Đã phát hiện cây mã nguồn — đang biên dịch từ mã nguồn.${NC}" ;;
            unsupported_arch) echo -e "${RED}Kiến trúc không được hỗ trợ: $1${NC}" ;;
            detected) echo -e "Đã phát hiện: $1 ($2)" ;;
            distro_fallback) echo -e "Không nhận diện rõ ID distro. Chuyển sang trình quản lý gói: $1" ;;
            install_build_runtime_deps) echo -e "Đang cài đặt thư viện phụ thuộc biên dịch và chạy..." ;;
            install_runtime_deps) echo -e "Đang cài đặt thư viện phụ thuộc chạy..." ;;
            unsupported_distro) echo -e "${YELLOW}Không hỗ trợ tự động cài gói cho: $1. Hãy cài đặt thủ công các thư viện phụ thuộc.${NC}" ;;
            install_rust) echo -e "Đang cài đặt Rust..." ;;
            cloning) echo -e "Đang clone mã nguồn để biên dịch..." ;;
            building) echo -e "Đang biên dịch từ mã nguồn..." ;;
            fetching_release) echo -e "Đang tải thông tin bản phát hành mới nhất..." ;;
            fetch_failed) echo -e "${RED}Không thể tải thông tin bản phát hành mới nhất.${NC}" ;;
            latest_version) echo -e "Phiên bản mới nhất: v$1" ;;
            downloading_tarball) echo -e "Đang tải tệp nén tarball..." ;;
            downloading_deb) echo -e "Đang tải gói .deb..." ;;
            no_prebuilt) echo -e "${RED}Không tìm thấy tệp nhị phân dựng sẵn cho v$1 ($2).${NC}" ;;
            visit_releases) echo -e "${YELLOW}Truy cập https://github.com/thanhhuynhduy/vietc/releases${NC}" ;;
            installing_bins) echo -e "Đang cài đặt vào /usr/bin/..." ;;
            capability_removed) echo -e "${GREEN}Đã xác nhận vietc-daemon không mang Linux capability đặc quyền.${NC}" ;;
            success_title)
                echo -e "${GREEN}========================================${NC}"
                echo -e "${GREEN}  VietC đã cài đặt thành công!${NC}"
                echo -e "${GREEN}========================================${NC}" ;;
            next_steps)
                echo -e "${GREEN}► Các bước tiếp theo:${NC}"
                echo -e "  1. Dịch vụ người dùng ${GREEN}vietc.service${NC} (evdev/uinput qua systemd-logind uaccess) đã được cài đặt."
                echo -e "  2. Nhấn ${GREEN}Ctrl+Space${NC} để bật/tắt gõ tiếng Việt (Telex / VNI, chuẩn dấu kiểu cũ)."
                echo -e "  3. Tùy chọn khay hệ thống: chạy ${GREEN}vietc-tray${NC} hoặc mở VietC từ menu ứng dụng."
                echo -e "  4. Cấu hình cá nhân: ${GREEN}~/.config/vietc/config.toml${NC}" ;;
            ns_uninstall_hdr) echo -e "Để gỡ cài đặt:" ;;
            ns_uninstall) echo -e "  ${GREEN}sudo bash ./uninstall.sh${NC}" ;;
            *) echo -e "$key" ;;
        esac
    else
        case "$key" in
            sudo) echo -e "${RED}Please run with sudo.${NC}" ;;
            src_detected) echo -e "${YELLOW}Source tree detected — building from source.${NC}" ;;
            unsupported_arch) echo -e "${RED}Unsupported architecture: $1${NC}" ;;
            detected) echo -e "Detected: $1 ($2)" ;;
            distro_fallback) echo -e "Distro ID not explicitly recognized. Falling back to package manager: $1" ;;
            install_build_runtime_deps) echo -e "Installing build and runtime dependencies..." ;;
            install_runtime_deps) echo -e "Installing runtime dependencies..." ;;
            unsupported_distro) echo -e "${YELLOW}Unsupported distro: $1. Install dependencies manually.${NC}" ;;
            install_rust) echo -e "Installing Rust..." ;;
            cloning) echo -e "Cloning source to build..." ;;
            building) echo -e "Building from source..." ;;
            fetching_release) echo -e "Fetching latest release..." ;;
            fetch_failed) echo -e "${RED}Failed to fetch latest release info.${NC}" ;;
            latest_version) echo -e "Latest version: v$1" ;;
            downloading_tarball) echo -e "Downloading tarball..." ;;
            downloading_deb) echo -e "Downloading .deb package..." ;;
            no_prebuilt) echo -e "${RED}No prebuilt binary found for v$1 ($2).${NC}" ;;
            visit_releases) echo -e "${YELLOW}Visit https://github.com/thanhhuynhduy/vietc/releases${NC}" ;;
            installing_bins) echo -e "Installing to /usr/bin/..." ;;
            capability_removed) echo -e "${GREEN}Confirmed vietc-daemon has no privileged Linux capabilities.${NC}" ;;
            success_title)
                echo -e "${GREEN}========================================${NC}"
                echo -e "${GREEN}  VietC installed successfully!${NC}"
                echo -e "${GREEN}========================================${NC}" ;;
            next_steps)
                echo -e "${GREEN}► Next steps:${NC}"
                echo -e "  1. User service ${GREEN}vietc.service${NC} (pure evdev/uinput via systemd-logind uaccess) is installed."
                echo -e "  2. Press ${GREEN}Ctrl+Space${NC} to toggle Vietnamese input (Telex / VNI, old-style tone placement)."
                echo -e "  3. Optional tray icon: run ${GREEN}vietc-tray${NC} or launch VietC from your app menu."
                echo -e "  4. User configuration: ${GREEN}~/.config/vietc/config.toml${NC}" ;;
            ns_uninstall_hdr) echo -e "To uninstall:" ;;
            ns_uninstall) echo -e "  ${GREEN}sudo bash ./uninstall.sh${NC}" ;;
            *) echo -e "$key" ;;
        esac
    fi
}

if [ "$FROM_SOURCE" != true ] && [ "$PREBUILT" != true ] && [ -f Cargo.toml ]; then
    t src_detected
    FROM_SOURCE=true
fi

if [ "$EUID" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        echo -e "${YELLOW}VietC installer requires sudo permissions.${NC}"
        if [ ! -f "$0" ]; then
            echo "Refusing to re-download and execute a mutable remote script as root." >&2
            echo "Download install.sh from a GitHub release, verify it with that release's SHA256SUMS, then run: sudo bash ./install.sh" >&2
            exit 1
        fi
        exec sudo bash "$0" "$@"
    else
        t sudo
        exit 1
    fi
fi

INSTALLING_USER="${SUDO_USER:-$USER}"

cleanup_legacy_uinput_socket() {
    local user_home
    user_home="$(getent passwd "$INSTALLING_USER" 2>/dev/null | cut -d: -f6 || true)"
    [ -n "$user_home" ] || return 0

    local legacy_socket="$user_home/.vietc/uinput.sock"
    if [ -S "$legacy_socket" ]; then
        rm -f -- "$legacy_socket"
    elif [ -e "$legacy_socket" ] || [ -L "$legacy_socket" ]; then
        echo "Refusing to remove non-socket legacy path: $legacy_socket" >&2
        return 1
    fi
}

ARCH=$(uname -m)
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
    *) t unsupported_arch "$ARCH"; exit 1 ;;
esac

[ -f /etc/os-release ] && . /etc/os-release
DISTRO="${ID:-unknown}"
t detected "$DISTRO" "$ARCH"

install_deps() {
    local matched=false
    case "$DISTRO" in
        ubuntu|debian|linuxmint|mint|pop|neon|zorin|elementary|fedora|rhel|centos|arch|manjaro|cachyos|endeavouros|garuda|artix|omarchy)
            matched=true
            ;;
    esac

    if [ "$matched" = false ]; then
        if command -v pacman &>/dev/null; then
            DISTRO="arch"
            matched=true
        elif command -v apt-get &>/dev/null; then
            DISTRO="ubuntu"
            matched=true
        elif command -v dnf &>/dev/null; then
            DISTRO="fedora"
            matched=true
        fi
        if [ "$matched" = true ]; then
            t distro_fallback "$DISTRO"
        fi
    fi

    if [ "$FROM_SOURCE" = true ]; then
        t install_build_runtime_deps
        case "$DISTRO" in
            ubuntu|debian|linuxmint|mint|pop|neon|zorin|elementary)
                export DEBIAN_FRONTEND=noninteractive
                apt-get update -y
                apt-get install -y build-essential pkg-config libx11-dev libxtst-dev \
                  libdbus-1-dev libevdev-dev libwayland-dev git \
                  libevdev2 libdbus-1-3 libx11-6 libxtst6 \
                  libwayland-client0 xclip wl-clipboard wtype curl
                ;;
            fedora|rhel|centos)
                dnf install -y gcc pkgconf-pkg-config libX11-devel libXtst-devel \
                  dbus-devel libevdev-devel wayland-devel git \
                  libevdev libX11 libXtst dbus-libs libwayland-client xclip wl-clipboard wtype curl
                ;;
            arch|manjaro|cachyos|endeavouros|garuda|artix|omarchy)
                pacman -Sy --needed --noconfirm base-devel pkgconf git \
                  libevdev libx11 libxtst dbus wayland xclip wl-clipboard wtype curl
                ;;
            *)
                t unsupported_distro "$DISTRO"
                ;;
        esac
    else
        t install_runtime_deps
        case "$DISTRO" in
            ubuntu|debian|linuxmint|mint|pop|neon|zorin|elementary)
                export DEBIAN_FRONTEND=noninteractive
                apt-get update -y
                apt-get install -y libevdev2 libdbus-1-3 libx11-6 libxtst6 \
                  libwayland-client0 xclip wl-clipboard wtype curl
                ;;
            fedora|rhel|centos)
                dnf install -y libevdev libX11 libXtst dbus-libs libwayland-client xclip wl-clipboard wtype curl
                ;;
            arch|manjaro|cachyos|endeavouros|garuda|artix|omarchy)
                pacman -Sy --needed --noconfirm libevdev libx11 libxtst dbus \
                  wayland xclip wl-clipboard wtype curl
                ;;
            *)
                t unsupported_distro "$DISTRO"
                ;;
        esac
    fi
}

install_deps

TMPDIR=$(mktemp -d)
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

if [ "$FROM_SOURCE" = true ]; then
    REAL_USER="${SUDO_USER:-$USER}"
    REAL_HOME="$(getent passwd "$REAL_USER" 2>/dev/null | cut -d: -f6)"
    [ -z "$REAL_HOME" ] && REAL_HOME="$HOME"
    [ -z "$REAL_HOME" ] && REAL_HOME="/home/$REAL_USER"
    export CARGO_HOME="$REAL_HOME/.cargo"
    export RUSTUP_HOME="$REAL_HOME/.rustup"

    run_as_build_user() {
        if [ "$REAL_USER" != "root" ] && [ "$(id -u)" = "0" ]; then
            if command -v runuser >/dev/null 2>&1; then
                runuser -u "$REAL_USER" -- env PATH="$CARGO_HOME/bin:$PATH" CARGO_HOME="$CARGO_HOME" RUSTUP_HOME="$RUSTUP_HOME" "$@"
            elif command -v su >/dev/null 2>&1; then
                su "$REAL_USER" -s /bin/bash -c 'env PATH="$1/bin:$PATH" CARGO_HOME="$1" RUSTUP_HOME="$2" "${@:3}"' -- "$CARGO_HOME" "$RUSTUP_HOME" "$@"
            else
                sudo -u "$REAL_USER" env PATH="$CARGO_HOME/bin:$PATH" CARGO_HOME="$CARGO_HOME" RUSTUP_HOME="$RUSTUP_HOME" "$@"
            fi
        else
            env PATH="$CARGO_HOME/bin:$PATH" CARGO_HOME="$CARGO_HOME" RUSTUP_HOME="$RUSTUP_HOME" "$@"
        fi
    }

    CARGO_BIN=""
    if [ -x "$CARGO_HOME/bin/cargo" ]; then
        CARGO_BIN="$CARGO_HOME/bin/cargo"
        echo "Using cargo from $CARGO_BIN (user=$REAL_USER)"
    elif command -v cargo &>/dev/null; then
        CARGO_BIN="$(command -v cargo)"
    fi

    if [ -z "$CARGO_BIN" ] || ! run_as_build_user cargo --version >/dev/null 2>&1; then
        if [ -z "$CARGO_BIN" ]; then
            t install_rust
            if [ "$REAL_USER" != "root" ]; then
                run_as_build_user bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path'
            else
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
            fi
            if ! run_as_build_user cargo --version >/dev/null 2>&1; then
                echo "ERROR: cargo still not found after rustup." >&2
                exit 1
            fi
        else
            run_as_build_user rustup default stable 2>&1 || true
            if ! run_as_build_user cargo --version >/dev/null 2>&1; then
                export RUSTUP_TOOLCHAIN=stable
            fi
        fi
    fi

    # Cargo's PATH is scoped to run_as_build_user. Do not modify user-owned
    # shell profiles or environment.d as root.
    REAL_GROUP="$(id -gn "$REAL_USER" 2>/dev/null || echo "$REAL_USER")"
    if [ ! -f Cargo.toml ] || [ ! -d .git ]; then
        t cloning
        chmod 755 "$TMPDIR" 2>/dev/null || true
        mkdir -p "$TMPDIR/source"
        if [ "$REAL_USER" != "root" ] && [ "$(id -u)" = "0" ]; then
            chown "$REAL_USER:$REAL_GROUP" "$TMPDIR/source" 2>/dev/null || true
        fi
        run_as_build_user git clone https://github.com/thanhhuynhduy/vietc.git "$TMPDIR/source"
        cd "$TMPDIR/source"
    fi

    t building
    if [ "$REAL_USER" != "root" ] && [ "$(id -u)" = "0" ]; then
        [ -d target ] && chown -R "$REAL_USER:$REAL_GROUP" target 2>/dev/null || true
        [ -d ui/target ] && chown -R "$REAL_USER:$REAL_GROUP" ui/target 2>/dev/null || true
    fi
    run_as_build_user cargo build --release
    (cd ui && run_as_build_user cargo build --release)
elif [ -f "./bin/vietc-daemon" ]; then
    INSTALL_DIR="$(pwd)"
else
    t fetching_release
    RELEASE_JSON=$(curl --proto '=https' --tlsv1.2 -sSfL "https://api.github.com/repos/thanhhuynhduy/vietc/releases/latest" 2>/dev/null || echo "")
    TAG=$(echo "$RELEASE_JSON" | grep '"tag_name"' | sed 's/.*"v\(.*\)",/\1/')
    if [ -z "$TAG" ]; then
        t fetch_failed
        exit 1
    fi
    t latest_version "$TAG"

    TARBALL="vietc_${TAG}_linux_${ARCH}.tar.gz"
    TARBALL_URL="https://github.com/thanhhuynhduy/vietc/releases/download/v${TAG}/${TARBALL}"
    DEB="vietc_${TAG}-1_${ARCH}.deb"
    DEB_URL="https://github.com/thanhhuynhduy/vietc/releases/download/v${TAG}/${DEB}"
    CHECKSUMS_URL="https://github.com/thanhhuynhduy/vietc/releases/download/v${TAG}/SHA256SUMS"
    INSTALL_DIR="$TMPDIR/install"
    mkdir -p "$INSTALL_DIR"

    if ! command -v sha256sum >/dev/null 2>&1; then
        echo "sha256sum is required to verify release artifacts." >&2
        exit 1
    fi
    if ! curl --proto '=https' --tlsv1.2 -sSfL \
        -o "$TMPDIR/SHA256SUMS" "$CHECKSUMS_URL"; then
        echo "Failed to download SHA256SUMS for release v${TAG}; refusing installation." >&2
        exit 1
    fi

    verify_release_artifact() {
        local asset="$1"
        local checksum_line

        checksum_line="$(awk -v name="$asset" '
            {
                file = $2
                sub(/^\*/, "", file)
                sub(/^\.\//, "", file)
                if (file == name) {
                    print $1 "  " name
                    matches++
                }
            }
            END {
                if (matches != 1) {
                    exit 1
                }
            }
        ' "$TMPDIR/SHA256SUMS")" || {
            echo "Missing or ambiguous checksum for ${asset}; refusing installation." >&2
            return 1
        }

        printf '%s\n' "$checksum_line" |
            (cd "$TMPDIR" && sha256sum --check --strict -)
    }

    if curl --proto '=https' --tlsv1.2 -sSfL \
        -o "$TMPDIR/$TARBALL" "$TARBALL_URL" 2>/dev/null; then
        t downloading_tarball
        verify_release_artifact "$TARBALL" || exit 1
        tar -xzf "$TMPDIR/$TARBALL" -C "$INSTALL_DIR"
        BIN_DIR="$INSTALL_DIR/vietc_${TAG}_linux_${ARCH}/bin"
        PKG_DIR="$INSTALL_DIR/vietc_${TAG}_linux_${ARCH}"
    elif curl --proto '=https' --tlsv1.2 -sSfL \
        -o "$TMPDIR/$DEB" "$DEB_URL" 2>/dev/null; then
        t downloading_deb
        verify_release_artifact "$DEB" || exit 1
        if command -v dpkg-deb &>/dev/null; then
            dpkg-deb -x "$TMPDIR/$DEB" "$INSTALL_DIR"
        else
            mkdir -p "$TMPDIR/deb"
            ar x "$TMPDIR/$DEB" --output="$TMPDIR/deb" 2>/dev/null
            tar -xzf "$TMPDIR/deb/data.tar.gz" -C "$INSTALL_DIR" 2>/dev/null || \
            tar -xJf "$TMPDIR/deb/data.tar.xz" -C "$INSTALL_DIR" 2>/dev/null || \
            tar --zstd -xf "$TMPDIR/deb/data.tar.zst" -C "$INSTALL_DIR" 2>/dev/null || true
        fi
        BIN_DIR="$INSTALL_DIR/usr/bin"
        PKG_DIR="$INSTALL_DIR"
    else
        t no_prebuilt "$TAG" "$ARCH"
        t visit_releases
        exit 1
    fi
fi

# Stop running processes before replacing binaries
pkill -x vietc-tray 2>/dev/null || true
pkill -x vietc-daemon 2>/dev/null || true
pkill -x vietc 2>/dev/null || true
pkill -x vietc-cli 2>/dev/null || true
sleep 0.3
cleanup_legacy_uinput_socket

# Install binaries
t installing_bins
if [ "$FROM_SOURCE" = true ]; then
    install -m 755 target/release/vietc /usr/bin/vietc-daemon
    install -m 755 target/release/vietc-cli /usr/bin/vietc-cli
    [ -f ui/target/release/vietc-tray ] && install -m 755 ui/target/release/vietc-tray /usr/bin/vietc-tray || true
else
    install -m 755 "$BIN_DIR/vietc-daemon" /usr/bin/vietc-daemon
    install -m 755 "$BIN_DIR/vietc-cli" /usr/bin/vietc-cli
    [ -f "$BIN_DIR/vietc-tray" ] && install -m 755 "$BIN_DIR/vietc-tray" /usr/bin/vietc-tray || true
fi

# The daemon must remain unprivileged. Device access is granted exclusively to
# the active desktop seat by systemd-logind via the uaccess udev rules below.
if command -v setcap &>/dev/null; then
    setcap -r /usr/bin/vietc-daemon 2>/dev/null || true
fi
t capability_removed

# Remove stale binaries and duplicate autostart entries from older versions
rm -f /usr/local/bin/vietc /usr/local/bin/vietc-daemon /usr/local/bin/vietc-cli \
      /usr/local/bin/vietc-tray /etc/xdg/autostart/vietc-tray.desktop \
      /etc/default/vietc 2>/dev/null || true

USER_HOME="$(getent passwd "$INSTALLING_USER" 2>/dev/null | cut -d: -f6 || true)"
if [ -n "$INSTALLING_USER" ] && [ "$INSTALLING_USER" != "root" ] && [ -n "$USER_HOME" ]; then
    rm -f "$USER_HOME/.local/bin/vietc" "$USER_HOME/.local/bin/vietc-daemon" \
          "$USER_HOME/.local/bin/vietc-cli" "$USER_HOME/.local/bin/vietc-tray" \
          "$USER_HOME/.local/bin/vietc-start" \
          "$USER_HOME/.config/autostart/vietc.desktop" 2>/dev/null || true
fi

# Udev rules & uinput kernel module (active-seat uaccess; preserve distro input group on event*)
mkdir -p /etc/modules-load.d
echo "uinput" > /etc/modules-load.d/vietc.conf
modprobe uinput 2>/dev/null || true
rm -f /etc/udev/rules.d/99-vietc.rules
cat > /etc/udev/rules.d/70-vietc.rules << 'EOF'
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", GROUP="root", MODE="0600"
SUBSYSTEM=="input", KERNEL=="event*", TAG+="uaccess"
EOF
chmod o-rw /dev/uinput /dev/input/event* 2>/dev/null || true
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --action=change --subsystem-match=input 2>/dev/null || true
udevadm trigger --action=change --subsystem-match=misc 2>/dev/null || true

# Icons
for size in scalable 256x256 128x128 64x64 48x48 32x32; do
    mkdir -p "/usr/share/icons/hicolor/$size/apps"
    if [ "$FROM_SOURCE" = true ]; then
        cp packaging/icons/*.svg "/usr/share/icons/hicolor/$size/apps/" 2>/dev/null || true
    else
        if [ -d "$PKG_DIR/icons" ]; then
            cp "$PKG_DIR/icons"/*.svg "/usr/share/icons/hicolor/$size/apps/" 2>/dev/null || true
        elif [ -d "$INSTALL_DIR/usr/share/icons" ]; then
            cp -r "$INSTALL_DIR/usr/share/icons/"* /usr/share/icons/ 2>/dev/null || true
        fi
    fi
done
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true

# Desktop launcher
if [ "$FROM_SOURCE" = true ]; then
    mkdir -p /usr/share/applications
    cp packaging/deb/vietc.desktop /usr/share/applications/
else
    if [ -f "$PKG_DIR/desktop/vietc.desktop" ]; then
        mkdir -p /usr/share/applications
        cp "$PKG_DIR/desktop/vietc.desktop" /usr/share/applications/
    elif [ -f "$INSTALL_DIR/usr/share/applications/vietc.desktop" ]; then
        mkdir -p /usr/share/applications
        cp "$INSTALL_DIR/usr/share/applications/vietc.desktop" /usr/share/applications/
    fi
fi

# Systemd user service
mkdir -p /usr/lib/systemd/user
cat > /usr/lib/systemd/user/vietc.service << 'EOF'
[Unit]
Description=VietC Vietnamese Input Daemon (evdev/uinput)
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/bin/vietc-daemon
Restart=on-failure
RestartSec=3
KillMode=process
NoNewPrivileges=yes
PrivateTmp=yes
RestrictSUIDSGID=yes
LockPersonality=yes
RestrictRealtime=yes
SystemCallArchitectures=native

[Install]
WantedBy=graphical-session.target
EOF

# Configuration
mkdir -p /etc/vietc
if [ "$FROM_SOURCE" = true ]; then
    cp vietc.toml /etc/vietc/config.toml
elif [ -f "$PKG_DIR/config/config.toml" ]; then
    cp "$PKG_DIR/config/config.toml" /etc/vietc/config.toml
elif [ -f "$INSTALL_DIR/etc/vietc/config.toml" ]; then
    cp "$INSTALL_DIR/etc/vietc/config.toml" /etc/vietc/config.toml
fi

if [ -n "$USER_HOME" ] && [ "$INSTALLING_USER" != "root" ]; then
    INSTALLING_GROUP="$(id -gn "$INSTALLING_USER" 2>/dev/null || echo "$INSTALLING_USER")"
    if [ ! -f "$USER_HOME/.config/vietc/config.toml" ] && [ -f /etc/vietc/config.toml ]; then
        mkdir -p "$USER_HOME/.config/vietc"
        chmod 700 "$USER_HOME/.config/vietc" 2>/dev/null || true
        cp /etc/vietc/config.toml "$USER_HOME/.config/vietc/config.toml" 2>/dev/null || true
        chmod 600 "$USER_HOME/.config/vietc/config.toml" 2>/dev/null || true
        chown -R "$INSTALLING_USER:$INSTALLING_GROUP" "$USER_HOME/.config/vietc" 2>/dev/null || true
    fi

    rm -f "$USER_HOME/.config/systemd/user/vietc.service" \
          "$USER_HOME/.config/systemd/user/graphical-session.target.wants/vietc.service" \
          "$USER_HOME/.config/systemd/user/default.target.wants/vietc.service" 2>/dev/null || true

    DROPIN_DIR="$USER_HOME/.config/systemd/user/vietc.service.d"
    if [ -d "$DROPIN_DIR" ]; then
        for dropin in "$DROPIN_DIR"/*.conf; do
            [ -f "$dropin" ] || continue
            if grep -Eq '^[[:space:]]*ExecStart[[:space:]]*=' "$dropin"; then
                rm -f "$dropin"
            fi
        done
        rmdir "$DROPIN_DIR" 2>/dev/null || true
    fi

    mkdir -p "$USER_HOME/.config/systemd/user/graphical-session.target.wants"
    ln -sf /usr/lib/systemd/user/vietc.service \
          "$USER_HOME/.config/systemd/user/graphical-session.target.wants/vietc.service"
    chown -R "$INSTALLING_USER:$INSTALLING_GROUP" "$USER_HOME/.config/systemd/user" 2>/dev/null || true

    U_UID="$(id -u "$INSTALLING_USER" 2>/dev/null || true)"
    if command -v systemctl >/dev/null 2>&1 && [ -n "$U_UID" ]; then
        DETECTED_WAYLAND="${WAYLAND_DISPLAY:-}"
        if [ -z "$DETECTED_WAYLAND" ]; then
            for sock in "/run/user/$U_UID"/wayland-[0-9]*; do
                if [ -S "$sock" ]; then
                    DETECTED_WAYLAND="$(basename "$sock")"
                    break
                fi
            done
        fi
        DETECTED_SESSION_TYPE="${XDG_SESSION_TYPE:-}"
        if [ -z "$DETECTED_SESSION_TYPE" ] && [ -n "$DETECTED_WAYLAND" ]; then
            DETECTED_SESSION_TYPE="wayland"
        fi

        IMPORT_VARS=("XDG_RUNTIME_DIR" "DBUS_SESSION_BUS_ADDRESS")
        ENV_ARGS=(
            "XDG_RUNTIME_DIR=/run/user/$U_UID"
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$U_UID/bus"
        )
        if [ -n "${DISPLAY:-}" ]; then
            IMPORT_VARS+=("DISPLAY")
            ENV_ARGS+=("DISPLAY=$DISPLAY")
        fi
        if [ -n "$DETECTED_WAYLAND" ]; then
            IMPORT_VARS+=("WAYLAND_DISPLAY")
            ENV_ARGS+=("WAYLAND_DISPLAY=$DETECTED_WAYLAND")
        fi
        if [ -n "${XDG_CURRENT_DESKTOP:-}" ]; then
            IMPORT_VARS+=("XDG_CURRENT_DESKTOP")
            ENV_ARGS+=("XDG_CURRENT_DESKTOP=$XDG_CURRENT_DESKTOP")
        fi
        if [ -n "$DETECTED_SESSION_TYPE" ]; then
            IMPORT_VARS+=("XDG_SESSION_TYPE")
            ENV_ARGS+=("XDG_SESSION_TYPE=$DETECTED_SESSION_TYPE")
        fi

        sudo -u "$INSTALLING_USER" env "${ENV_ARGS[@]}" \
            systemctl --user import-environment "${IMPORT_VARS[@]}" 2>/dev/null || true
        sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
            systemctl --user daemon-reload 2>/dev/null || true
        sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
            systemctl --user enable vietc.service 2>/dev/null || true
        sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
            systemctl --user restart vietc.service 2>/dev/null || true
    fi
fi

echo ""
t success_title
echo ""
t next_steps
echo ""
t ns_uninstall_hdr
t ns_uninstall
