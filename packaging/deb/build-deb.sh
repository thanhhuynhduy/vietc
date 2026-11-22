#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -1)
VERSION="${1:-$DEFAULT_VERSION}"
if command -v dpkg >/dev/null 2>&1; then
    DEB_ARCH="$(dpkg --print-architecture)"
else
    case "$(uname -m)" in
        aarch64|arm64) DEB_ARCH="arm64" ;;
        *) DEB_ARCH="amd64" ;;
    esac
fi
PACKAGE="dboard_${VERSION}-1_${DEB_ARCH}"
STAGING="$SCRIPT_DIR/$PACKAGE"

echo "=== Building Dboard .deb package v${VERSION} (${DEB_ARCH}) ==="

echo "[1/5] Building binaries..."
cargo build --release --locked --manifest-path "$PROJECT_ROOT/Cargo.toml"
(cd "$PROJECT_ROOT/ui" && cargo build --release --locked)
echo "  Done."

echo "[2/5] Creating staging directory..."
rm -rf "$STAGING"
mkdir -p "$STAGING/DEBIAN"
mkdir -p "$STAGING/usr/bin"
mkdir -p "$STAGING/usr/lib/systemd/user/graphical-session.target.wants"
mkdir -p "$STAGING/usr/lib/modules-load.d"
mkdir -p "$STAGING/etc/dboard"
mkdir -p "$STAGING/usr/share/applications"
mkdir -p "$STAGING/usr/share/doc/dboard"
mkdir -p "$STAGING/usr/share/metainfo"
mkdir -p "$STAGING/lib/udev/rules.d"

echo "[3/5] Installing binaries and assets..."
install -m 755 "$PROJECT_ROOT/target/release/dboard" "$STAGING/usr/bin/dboard-daemon"
install -m 755 "$PROJECT_ROOT/target/release/dboard-cli" "$STAGING/usr/bin/dboard-cli"
install -m 755 "$PROJECT_ROOT/ui/target/release/dboard-tray" "$STAGING/usr/bin/dboard-tray"

for size in scalable 256x256 128x128 64x64 48x48 32x32; do
    mkdir -p "$STAGING/usr/share/icons/hicolor/$size/apps"
    cp "$PROJECT_ROOT/packaging/icons/"*.svg "$STAGING/usr/share/icons/hicolor/$size/apps/"
done

cp "$SCRIPT_DIR/dboard.desktop" "$STAGING/usr/share/applications/"
cp "$PROJECT_ROOT/packaging/70-dboard.rules" "$STAGING/lib/udev/rules.d/70-dboard.rules"
echo "uinput" > "$STAGING/usr/lib/modules-load.d/dboard.conf"
cp "$PROJECT_ROOT/dboard.service" "$STAGING/usr/lib/systemd/user/dboard.service"
ln -sf ../dboard.service "$STAGING/usr/lib/systemd/user/graphical-session.target.wants/dboard.service"
cp "$PROJECT_ROOT/README.md" "$STAGING/usr/share/doc/dboard/"
cp "$PROJECT_ROOT/LICENSE" "$STAGING/usr/share/doc/dboard/"
cp "$PROJECT_ROOT/dboard.toml" "$STAGING/etc/dboard/config.toml"

cat > "$STAGING/usr/share/metainfo/io.github.thanhhuynhduy.dboard.appdata.xml" << 'XML'
<?xml version="1.0" encoding="UTF-8"?>
<component type="console-application">
  <id>io.github.thanhhuynhduy.dboard</id>
  <name>Dboard</name>
  <summary>Pure evdev/uinput Bilingual Vietnamese Input Method for Linux</summary>
  <description>
    <p>Zero-underline bilingual Vietnamese and English input method for Linux Wayland (COSMIC, Hyprland, Sway / wlroots, US QWERTY) powered by pure evdev/uinput and zwp_virtual_keyboard_v1, Gboard-inspired bilingual context disambiguation, Telex and VNI input, and old-style tone placement.</p>
  </description>
  <metadata_license>MIT</metadata_license>
  <project_license>MIT</project_license>
  <url type="homepage">https://github.com/thanhhuynhduy/vietc</url>
  <provides><binary>dboard-daemon</binary></provides>
  <categories><category>Utility</category></categories>
  <keywords><keyword>vietnamese</keyword><keyword>evdev</keyword><keyword>ime</keyword><keyword>telex</keyword><keyword>vni</keyword></keywords>
</component>
XML

mkdir -p "$STAGING/usr/share/lintian/overrides"
cat > "$STAGING/usr/share/lintian/overrides/dboard" << 'LINTIAN'
dboard: no-manual-page *
dboard: no-systemd-service-file-outside-lib
dboard: wrong-section-according-to-package-name
LINTIAN

echo "[4/5] Creating Debian control and non-interactive maintainer scripts..."
cat > "$STAGING/DEBIAN/control" << 'CONTROL'
Package: dboard
Version: VERSION_PLACEHOLDER
Section: utils
Priority: optional
Architecture: ARCH_PLACEHOLDER
Depends: libc6 (>= 2.31), libevdev2 (>= 1.9.0), libdbus-1-3, libwayland-client0 (>= 1.20)
Recommends: wl-clipboard, libnotify-bin
Homepage: https://github.com/thanhhuynhduy/vietc
Maintainer: Dboard Contributors <https://github.com/thanhhuynhduy/vietc>
Description: Dboard — Pure evdev/uinput Vietnamese Input Method for Linux
 Bilingual Vietnamese and English input method supporting Telex and VNI
 with old-style tone placement and Gboard-inspired context disambiguation.
 Uses systemd-logind uaccess ACLs for rootless active-seat evdev/uinput.
CONTROL
sed -i "s/VERSION_PLACEHOLDER/$VERSION/" "$STAGING/DEBIAN/control"
sed -i "s/ARCH_PLACEHOLDER/$DEB_ARCH/" "$STAGING/DEBIAN/control"

echo "/etc/dboard/config.toml" > "$STAGING/DEBIAN/conffiles"

cat > "$STAGING/DEBIAN/postinst" << 'POSTINST'
#!/bin/sh
set -e

case "$1" in
  configure)
    pkill -x dboard-tray 2>/dev/null || true
    pkill -x dboard-daemon 2>/dev/null || true
    pkill -x dboard 2>/dev/null || true

    rm -f /usr/local/bin/dboard-tray /usr/local/bin/dboard /usr/local/bin/dboard-daemon \
          /usr/local/bin/dboard-cli /etc/udev/rules.d/99-dboard.rules 2>/dev/null || true

    if command -v setcap >/dev/null 2>&1; then
      setcap -r /usr/bin/dboard-daemon >/dev/null 2>&1 || true
    fi

    chmod o-rw /dev/uinput /dev/input/event* 2>/dev/null || true

    if command -v modprobe >/dev/null 2>&1; then
      modprobe uinput >/dev/null 2>&1 || true
    fi

    if command -v systemctl >/dev/null 2>&1; then
      systemctl --global daemon-reload >/dev/null 2>&1 || true
    fi

    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
      gtk-update-icon-cache -f -t /usr/share/icons/hicolor/ >/dev/null 2>&1 || true
    fi

    if command -v udevadm >/dev/null 2>&1; then
      udevadm control --reload-rules >/dev/null 2>&1 || true
      udevadm trigger --action=change --subsystem-match=input >/dev/null 2>&1 || true
      udevadm trigger --action=change --subsystem-match=misc >/dev/null 2>&1 || true
    fi
    ;;
esac
POSTINST
chmod 755 "$STAGING/DEBIAN/postinst"

cat > "$STAGING/DEBIAN/prerm" << 'PRERM'
#!/bin/sh
set -e
case "$1" in
  remove|upgrade|deconfigure)
    pkill -TERM -x dboard-tray 2>/dev/null || true
    pkill -TERM -x dboard-daemon 2>/dev/null || true
    pkill -TERM -x dboard 2>/dev/null || true
    ;;
esac
PRERM
chmod 755 "$STAGING/DEBIAN/prerm"

cat > "$STAGING/DEBIAN/postrm" << 'POSTRM'
#!/bin/sh
set -e
case "$1" in
  remove|purge)
    if [ "$1" = "purge" ]; then
      rm -rf /etc/dboard 2>/dev/null || true
    fi
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --global daemon-reload >/dev/null 2>&1 || true
    fi
    if command -v setfacl >/dev/null 2>&1; then
      setfacl -b /dev/uinput /dev/input/event* >/dev/null 2>&1 || true
    fi
    chmod o-rw /dev/uinput /dev/input/event* 2>/dev/null || true
    if command -v udevadm >/dev/null 2>&1; then
      udevadm control --reload-rules >/dev/null 2>&1 || true
      udevadm trigger --action=change --subsystem-match=input >/dev/null 2>&1 || true
      udevadm trigger --action=change --subsystem-match=misc >/dev/null 2>&1 || true
    fi
    ;;
esac
POSTRM
chmod 755 "$STAGING/DEBIAN/postrm"

echo "[5/5] Building .deb package..."
fakeroot dpkg-deb --build "$STAGING" "$SCRIPT_DIR/dboard_${VERSION}-1_${DEB_ARCH}.deb"
rm -rf "$STAGING"
echo ""
echo "=== Package built: packaging/deb/dboard_${VERSION}-1_${DEB_ARCH}.deb ==="
