#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Dboard — Vietnamese Input Method Uninstaller
# Usage: verify the release checksum, then run sudo bash ./uninstall.sh
set -euo pipefail

export PATH="/usr/sbin:/usr/bin:/sbin:/bin"

clear_device_acls() {
    if ! command -v setfacl >/dev/null 2>&1; then
        echo "WARNING: setfacl is unavailable; device session ACLs could not be revoked. Install the acl package and retry cleanup." >&2
        return 0
    fi
    local device
    for device in "$@"; do
        [ -e "$device" ] || continue
        # ACL entries do not record their origin. Clear extended access ACLs,
        # then let the distribution rules restore legitimate session access.
        if ! setfacl -b -- "$device"; then
            echo "WARNING: could not clear device ACLs on $device; session access may remain." >&2
        fi
    done
    return 0
}

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

# Language: auto-detect from $LANG, override with --lang vi|en
LANG_CODE="vi"
case "${LANG:-}" in
  en*|C*|POSIX*) LANG_CODE="en" ;;
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
    elif [ "$arg" = "--lang" ]; then
        LANG_NEXT=1
    elif [ "$arg" = "--lang=vi" ] || [ "$arg" = "--lang=vi_VN" ] || [ "$arg" = "--lang=vi-VN" ]; then
        LANG_CODE="vi"
    elif [ "$arg" = "--lang=en" ]; then
        LANG_CODE="en"
    fi
done

# t KEY — print translated message
t() {
    local key="$1"
    if [ "$LANG_CODE" = "vi" ]; then
        case "$key" in
            sudo) echo -e "${RED}Vui lòng chạy với quyền sudo.${NC}" ;;
            uninstaller_header) echo -e "${YELLOW}=== Bắt đầu gỡ cài đặt Dboard ===${NC}" ;;
            stopping_services) echo -e "Đang dừng các dịch vụ và tiến trình Dboard..." ;;
            removing_files) echo -e "Đang xóa các tệp nhị phân, cấu hình và icon..." ;;
            removed) echo -e "${GREEN}=== Đã gỡ cài đặt hoàn toàn Dboard khỏi hệ thống! ===${NC}" ;;
            *) echo -e "$key" ;;
        esac
    else
        case "$key" in
            sudo) echo -e "${RED}Please run with sudo.${NC}" ;;
            uninstaller_header) echo -e "${YELLOW}=== Starting Dboard Uninstaller ===${NC}" ;;
            stopping_services) echo -e "Stopping Dboard services and processes..." ;;
            removing_files) echo -e "Removing binaries, configuration, and icons..." ;;
            removed) echo -e "${GREEN}=== Dboard has been completely removed from your system! ===${NC}" ;;
            *) echo -e "$key" ;;
        esac
    fi
}

# Auto-elevate with sudo if not root
if [ "$EUID" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        echo -e "${YELLOW}Dboard uninstaller requires sudo permissions.${NC}"
        if [ ! -f "$0" ]; then
            echo "Refusing to re-download and execute a mutable remote script as root." >&2
            echo "Download uninstall.sh from a GitHub release, verify it with that release's SHA256SUMS, then run: sudo bash ./uninstall.sh" >&2
            exit 1
        fi
        exec sudo bash "$0" "$@"
    else
        t sudo
        exit 1
    fi
fi

t uninstaller_header

INSTALLING_USER="${SUDO_USER:-$USER}"
if [ -z "$INSTALLING_USER" ] || [ "$INSTALLING_USER" = "root" ]; then
    if command -v logname &>/dev/null; then
        INSTALLING_USER="$(logname 2>/dev/null || true)"
    fi
fi

USER_HOME=""
U_UID=""
if [ -n "$INSTALLING_USER" ] && [ "$INSTALLING_USER" != "root" ]; then
    USER_HOME="$(getent passwd "$INSTALLING_USER" 2>/dev/null | cut -d: -f6 || true)"
    U_UID="$(id -u "$INSTALLING_USER" 2>/dev/null || true)"
fi

t stopping_services

# Stop and disable systemd user services
if [ -n "$INSTALLING_USER" ] && [ -n "$U_UID" ]; then
    sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
        systemctl --user stop dboard.service dboard-tray.service 2>/dev/null || true
    sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
        systemctl --user disable dboard.service dboard-tray.service 2>/dev/null || true
fi

# Gracefully stop all running processes first so evdev devices are cleanly ungrabbed
pkill -TERM -x dboard 2>/dev/null || true
pkill -TERM -x dboard-daemon 2>/dev/null || true
pkill -TERM -x dboard-tray 2>/dev/null || true
pkill -TERM -x dboard-popup 2>/dev/null || true
sleep 0.2
pkill -9 -x dboard 2>/dev/null || true
pkill -9 -x dboard-daemon 2>/dev/null || true
pkill -9 -x dboard-tray 2>/dev/null || true
pkill -9 -x dboard-popup 2>/dev/null || true
pkill -9 -x dboardctl 2>/dev/null || true
pkill -9 -x dboard-uinputd 2>/dev/null || true
pkill -9 -x dboard-cli 2>/dev/null || true

t removing_files

# Remove system binaries
rm -f /usr/bin/dboard /usr/bin/dboard-daemon /usr/bin/dboardctl /usr/bin/dboard-tray \
      /usr/bin/dboard-popup /usr/bin/dboard-uinputd /usr/bin/dboard-cli /usr/bin/dboard-xrecord
rm -f /usr/local/bin/dboard /usr/local/bin/dboard-daemon /usr/local/bin/dboardctl \
      /usr/local/bin/dboard-tray /usr/local/bin/dboard-popup /usr/local/bin/dboard-uinputd \
      /usr/local/bin/dboard-cli /usr/local/bin/dboard-xrecord

# Remove user local binaries if present
if [ -n "$USER_HOME" ]; then
    LEGACY_UINPUT_SOCKET="$USER_HOME/.dboard/uinput.sock"
    if [ -S "$LEGACY_UINPUT_SOCKET" ]; then
        rm -f -- "$LEGACY_UINPUT_SOCKET"
    elif [ -e "$LEGACY_UINPUT_SOCKET" ] || [ -L "$LEGACY_UINPUT_SOCKET" ]; then
        echo "Refusing to remove non-socket legacy path: $LEGACY_UINPUT_SOCKET" >&2
    fi

    rm -f "$USER_HOME/.local/bin/dboard" "$USER_HOME/.local/bin/dboard-daemon" \
          "$USER_HOME/.local/bin/dboardctl" "$USER_HOME/.local/bin/dboard-tray" \
          "$USER_HOME/.local/bin/dboard-popup" "$USER_HOME/.local/bin/dboard-uinputd" \
          "$USER_HOME/.local/bin/dboard-cli"
fi
if [ -n "${U_UID:-}" ]; then
    rm -f "/run/user/$U_UID/dboard-candidates.sock" "/run/user/$U_UID/dboard-select.sock"
fi

# Remove systemd service files
rm -f /usr/lib/systemd/user/dboard.service /usr/lib/systemd/user/dboard-tray.service \
      /usr/lib/systemd/user/graphical-session.target.wants/dboard.service
rm -f /etc/systemd/user/dboard.service /etc/systemd/user/dboard-tray.service
if [ -n "$USER_HOME" ]; then
    rm -f "$USER_HOME/.config/systemd/user/dboard.service" \
          "$USER_HOME/.config/systemd/user/dboard-tray.service" \
          "$USER_HOME/.config/systemd/user/graphical-session.target.wants/dboard.service" \
          "$USER_HOME/.config/systemd/user/default.target.wants/dboard.service"
fi

# Remove udev rules and kernel module load drop-in
rm -f /etc/udev/rules.d/70-dboard.rules /etc/udev/rules.d/99-dboard.rules /etc/udev/rules.d/99-uinput.rules \
      /lib/udev/rules.d/70-dboard.rules /usr/lib/udev/rules.d/70-dboard.rules
rm -f /etc/modules-load.d/dboard.conf /usr/lib/modules-load.d/dboard.conf /etc/default/dboard

# Remove legacy component XML if present from older versions
rm -f /usr/share/ibus/component/dboard.xml

# Remove system desktop autostart & icons
rm -f /usr/share/applications/dboard*.desktop /etc/xdg/autostart/dboard*.desktop
rm -f /usr/share/icons/hicolor/*/apps/dboard*.svg /usr/share/icons/dboard*.svg
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true

# Remove user configs, user autostart & icons
rm -rf /etc/dboard
if [ -n "$USER_HOME" ]; then
    rm -rf "$USER_HOME/.config/dboard" "$USER_HOME/.local/state/dboard"
    rm -f "$USER_HOME/.config/autostart/dboard*.desktop"
    rm -f "$USER_HOME/.local/share/icons/dboard*.svg"
    rm -f "$USER_HOME/.local/share/icons/hicolor/*/apps/dboard*.svg"
fi

# Clean up status, lock, and socket files
if [ -n "$U_UID" ]; then
    rm -f "/tmp/dboard-daemon-$U_UID.lock" "/tmp/dboard-tray-$U_UID.lock" \
          "/tmp/dboard-status-$U_UID" "/tmp/dboard-method-$U_UID" \
          "/tmp/dboard-$U_UID.log" "/tmp/dboard-$U_UID.log.old"
    rm -f "/run/user/$U_UID/dboard-daemon.lock" "/run/user/$U_UID/dboard-tray.lock" \
          "/run/user/$U_UID/dboard-popup.lock"
    rm -rf "/run/user/$U_UID/dboard"
fi

# Revoke legacy world and named-user access before restoring distro policy.
clear_device_acls /dev/uinput /dev/input/event*
for device in /dev/uinput /dev/input/event*; do
    [ -e "$device" ] || continue
    if ! chmod o-rw -- "$device"; then
        echo "WARNING: could not revoke world access on $device." >&2
    fi
done
if ! udevadm control --reload-rules; then
    echo "WARNING: could not reload udev rules; distribution device policy may not be restored." >&2
fi
if ! udevadm trigger --action=change --subsystem-match=input; then
    echo "WARNING: could not reapply distribution input device policy." >&2
fi
if ! udevadm trigger --action=change --subsystem-match=misc; then
    echo "WARNING: could not reapply distribution misc device policy." >&2
fi
if ! udevadm settle --timeout=10; then
    echo "WARNING: udev did not settle; device policy restoration is not confirmed." >&2
fi

# Reload systemd
if [ -n "$INSTALLING_USER" ] && [ -n "$U_UID" ]; then
    sudo -u "$INSTALLING_USER" XDG_RUNTIME_DIR="/run/user/$U_UID" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$U_UID/bus" \
        systemctl --user daemon-reload 2>/dev/null || true
fi
if command -v systemctl &>/dev/null; then
    systemctl --global daemon-reload 2>/dev/null || true
fi

t removed

