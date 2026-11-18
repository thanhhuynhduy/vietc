#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

PACKAGE="${1:-}"
[ -f "$PACKAGE" ] || {
    echo "usage: $0 path/to/vietc.deb" >&2
    exit 2
}

PACKAGE="$(realpath "$PACKAGE")"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

cp "$PACKAGE" "$WORK_DIR/vietc.deb"

docker run --rm \
    --volume "$WORK_DIR:/artifacts:ro" \
    ubuntu:24.04 \
    bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive

        apt-get update
        apt-get install --yes ca-certificates dbus libdbus-1-3 libevdev2 \
            libwayland-client0 libx11-6 libxtst6 systemd

        dpkg-deb --info /artifacts/vietc.deb
        dpkg-deb --contents /artifacts/vietc.deb

        dpkg --install /artifacts/vietc.deb

        test -x /usr/bin/vietc-daemon
        test -x /usr/bin/vietc-cli
        test -x /usr/bin/vietc-tray
        test -f /usr/lib/systemd/user/vietc.service
        test -f /lib/udev/rules.d/70-vietc.rules
        test -f /etc/vietc/config.toml

        ldd /usr/bin/vietc-daemon >/dev/null
        ldd /usr/bin/vietc-tray >/dev/null

        dpkg --remove vietc
        test ! -e /usr/bin/vietc-daemon
        test ! -e /usr/bin/vietc-cli
        test ! -e /usr/bin/vietc-tray
        test ! -e /usr/lib/systemd/user/vietc.service
        test ! -e /lib/udev/rules.d/70-vietc.rules

        dpkg --purge vietc
        test ! -e /etc/vietc/config.toml
        test ! -e /etc/vietc
    '