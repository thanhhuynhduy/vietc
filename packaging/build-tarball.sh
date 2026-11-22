#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Dboard — Generic Linux Tarball Packager
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -1)

case "$(uname -m)" in
    aarch64|arm64) TAR_ARCH="arm64" ;;
    *) TAR_ARCH="amd64" ;;
esac
PACKAGE_NAME="dboard_${VERSION}_linux_${TAR_ARCH}"
DIST_DIR="$PROJECT_ROOT/target/dist"
STAGING="$DIST_DIR/$PACKAGE_NAME"

echo "=== Building Dboard release tarball v${VERSION} (${TAR_ARCH}) ==="

echo "[1/4] Compiling components in release mode..."
cargo build --release --locked --manifest-path "$PROJECT_ROOT/Cargo.toml"
(cd "$PROJECT_ROOT/ui" && cargo build --release --locked)

echo "[2/4] Assembling package structure..."
rm -rf "$DIST_DIR"
mkdir -p "$STAGING/bin"
mkdir -p "$STAGING/udev"
mkdir -p "$STAGING/desktop"
mkdir -p "$STAGING/icons"
mkdir -p "$STAGING/config"
mkdir -p "$STAGING/systemd"

install -m 755 "$PROJECT_ROOT/target/release/dboard" "$STAGING/bin/dboard-daemon"
install -m 755 "$PROJECT_ROOT/target/release/dboard-cli" "$STAGING/bin/dboard-cli"
install -m 755 "$PROJECT_ROOT/ui/target/release/dboard-tray" "$STAGING/bin/dboard-tray"

cp "$PROJECT_ROOT/packaging/70-dboard.rules" "$STAGING/udev/"
cp "$PROJECT_ROOT/packaging/deb/dboard.desktop" "$STAGING/desktop/"
cp "$PROJECT_ROOT/dboard.service" "$STAGING/systemd/"
cp "$PROJECT_ROOT/dboard.toml" "$STAGING/config/config.toml"
cp "$PROJECT_ROOT/packaging/icons/"*.svg "$STAGING/icons/"

cp "$PROJECT_ROOT/install.sh" "$STAGING/"
cp "$PROJECT_ROOT/uninstall.sh" "$STAGING/"
chmod +x "$STAGING/install.sh" "$STAGING/uninstall.sh"

cp "$PROJECT_ROOT/README.md" "$STAGING/"
cp "$PROJECT_ROOT/LICENSE" "$STAGING/"

echo "[3/4] Creating tarball archive..."
(cd "$DIST_DIR" && tar -czf "${PACKAGE_NAME}.tar.gz" "$PACKAGE_NAME")

rm -rf "$STAGING"

echo -e "\n=== Package successfully built: ==="
echo "target/dist/${PACKAGE_NAME}.tar.gz"
