#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -1)
VERSION="${1:-$DEFAULT_VERSION}"
PACKAGE="vietc_${VERSION}-1_amd64"
STAGING="$SCRIPT_DIR/$PACKAGE"

echo "=== Building VietC .deb package v${VERSION} ==="

echo "[1/5] Building binaries..."
cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml"
(cd "$PROJECT_ROOT/ui" && cargo build --release)
echo "  Done."

echo "[2/5] Creating staging directory..."
rm -rf "$STAGING"
mkdir -p "$STAGING/DEBIAN"
mkdir -p "$STAGING/usr/bin"
mkdir -p "$STAGING/usr/lib/systemd/user/graphical-session.target.wants"
mkdir -p "$STAGING/usr/lib/modules-load.d"
mkdir -p "$STAGING/etc/vietc"
mkdir -p "$STAGING/usr/share/applications"
mkdir -p "$STAGING/usr/share/doc/vietc"
mkdir -p "$STAGING/usr/share/metainfo"
mkdir -p "$STAGING/lib/udev/rules.d"

echo "[3/5] Installing binaries and assets..."
install -m 755 "$PROJECT_ROOT/target/release/vietc" "$STAGING/usr/bin/vietc-daemon"
install -m 755 "$PROJECT_ROOT/target/release/vietc-cli" "$STAGING/usr/bin/vietc-cli"
install -m 755 "$PROJECT_ROOT/ui/target/release/vietc-tray" "$STAGING/usr/bin/vietc-tray"

for size in scalable 256x256 128x128 64x64 48x48 32x32; do
    mkdir -p "$STAGING/usr/share/icons/hicolor/$size/apps"
    cp "$PROJECT_ROOT/packaging/icons/"*.svg "$STAGING/usr/share/icons/hicolor/$size/apps/"
done

cp "$SCRIPT_DIR/vietc.desktop" "$STAGING/usr/share/applications/"
cp "$PROJECT_ROOT/packaging/70-vietc.rules" "$STAGING/lib/udev/rules.d/70-vietc.rules"
echo "uinput" > "$STAGING/usr/lib/modules-load.d/vietc.conf"
cp "$PROJECT_ROOT/vietc.service" "$STAGING/usr/lib/systemd/user/vietc.service"
ln -sf ../vietc.service "$STAGING/usr/lib/systemd/user/graphical-session.target.wants/vietc.service"
cp "$PROJECT_ROOT/README.md" "$STAGING/usr/share/doc/vietc/"
cp "$PROJECT_ROOT/LICENSE" "$STAGING/usr/share/doc/vietc/"
cp "$PROJECT_ROOT/vietc.toml" "$STAGING/etc/vietc/config.toml"

cat > "$STAGING/usr/share/metainfo/io.github.thanhhuynhduy.vietc.appdata.xml" << 'XML'
<?xml version="1.0" encoding="UTF-8"?>
<component type="console-application">
  <id>io.github.thanhhuynhduy.vietc</id>
  <name>VietC</name>
  <summary>Pure evdev/uinput Bilingual Vietnamese Input Method for Linux</summary>
  <description>
    <p>Zero-underline bilingual Vietnamese and English input method for Linux Wayland and X11 powered by pure evdev/uinput capture, Gboard-inspired bilingual context disambiguation, Telex and VNI input, and old-style tone placement.</p>
  </description>
  <metadata_license>MIT</metadata_license>
  <project_license>MIT</project_license>
  <url type="homepage">https://github.com/thanhhuynhduy/vietc</url>
  <provides><binary>vietc-daemon</binary></provides>
  <categories><category>Utility</category></categories>
  <keywords><keyword>vietnamese</keyword><keyword>evdev</keyword><keyword>ime</keyword><keyword>telex</keyword><keyword>vni</keyword></keywords>
</component>
XML

mkdir -p "$STAGING/usr/share/lintian/overrides"
cat > "$STAGING/usr/share/lintian/overrides/vietc" << 'LINTIAN'
vietc: no-manual-page *
vietc: no-systemd-service-file-outside-lib
vietc: wrong-section-according-to-package-name
LINTIAN

echo "[4/5] Creating Debian control and non-interactive maintainer scripts..."
cat > "$STAGING/DEBIAN/control" << 'CONTROL'
Package: vietc
Version: VERSION_PLACEHOLDER
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.31), libevdev2 (>= 1.9.0), libdbus-1-3, libwayland-client0 (>= 1.20), libx11-6, libxtst6
Recommends: wl-clipboard, xclip, wtype, libnotify-bin
Homepage: https://github.com/thanhhuynhduy/vietc
Maintainer: VietC Contributors <https://github.com/thanhhuynhduy/vietc>
Description: VietC — Pure evdev/uinput Vietnamese Input Method for Linux
 Bilingual Vietnamese and English input method supporting Telex and VNI
 with old-style tone placement and Gboard-inspired context disambiguation.
 Uses systemd-logind uaccess ACLs for rootless active-seat evdev/uinput.
CONTROL
sed -i "s/VERSION_PLACEHOLDER/$VERSION/" "$STAGING/DEBIAN/control"

echo "/etc/vietc/config.toml" > "$STAGING/DEBIAN/conffiles"

cat > "$STAGING/DEBIAN/postinst" << 'POSTINST'
#!/bin/sh
set -e

case "$1" in
  configure)
    pkill -x vietc-tray 2>/dev/null || true
    pkill -x vietc-daemon 2>/dev/null || true
    pkill -x vietc 2>/dev/null || true

    rm -f /usr/local/bin/vietc-tray /usr/local/bin/vietc /usr/local/bin/vietc-daemon \
          /usr/local/bin/vietc-cli /etc/udev/rules.d/99-vietc.rules 2>/dev/null || true

    if command -v setcap >/dev/null 2>&1; then
      setcap -r /usr/bin/vietc-daemon >/dev/null 2>&1 || true
    fi

    chmod o-rw /dev/uinput /dev/input/event* 2>/dev/null || true

    if command -v modprobe >/dev/null 2>&1; then
      modprobe uinput >/dev/null 2>&1 || true
    fi

    if command -v systemctl >/dev/null 2>&1; then
      systemctl --global daemon-reload >/dev/null 2>&1 || true
    fi

    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
      gtk-update-icon-cache -f /usr/share/icons/hicolor/ >/dev/null 2>&1 || true
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
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --global daemon-reload >/dev/null 2>&1 || true
    fi
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
      rm -rf /etc/vietc 2>/dev/null || true
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
fakeroot dpkg-deb --build "$STAGING" "$SCRIPT_DIR/vietc_${VERSION}-1_amd64.deb"
rm -rf "$STAGING"
echo ""
echo "=== Package built: packaging/deb/vietc_${VERSION}-1_amd64.deb ==="
