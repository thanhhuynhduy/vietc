#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

PACKAGE="${1:-}"
[ -f "$PACKAGE" ] || {
    echo "usage: $0 path/to/dboard.deb" >&2
    exit 2
}

PACKAGE="$(realpath "$PACKAGE")"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

cp "$PACKAGE" "$WORK_DIR/dboard.deb"

docker run --rm \
    --volume "$WORK_DIR:/artifacts:ro" \
    ubuntu:24.04 \
    bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive

        apt-get update
        apt-get install --yes ca-certificates dbus libdbus-1-3 libevdev2 \
            libwayland-client0 systemd

        dpkg-deb --info /artifacts/dboard.deb
        dpkg-deb --contents /artifacts/dboard.deb

        dpkg --install /artifacts/dboard.deb

        test -x /usr/bin/dboard-daemon
        test -x /usr/bin/dboard-cli
        test -x /usr/bin/dboard-tray
        test -f /usr/lib/systemd/user/dboard.service
        test -f /lib/udev/rules.d/70-dboard.rules
        test -f /etc/dboard/config.toml

        ! ldd /usr/bin/dboard-daemon | grep -q "not found"
        ! ldd /usr/bin/dboard-tray | grep -q "not found"

        dpkg --remove dboard
        test ! -e /usr/bin/dboard-daemon
        test ! -e /usr/bin/dboard-cli
        test ! -e /usr/bin/dboard-tray
        test ! -e /usr/lib/systemd/user/dboard.service
        test ! -e /lib/udev/rules.d/70-dboard.rules

        dpkg --purge dboard
        test ! -e /etc/dboard/config.toml
        test ! -e /etc/dboard
    '