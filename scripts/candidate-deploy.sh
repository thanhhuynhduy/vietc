#!/usr/bin/env bash
# Deploy/rollback a locally built vietc-daemon candidate for the *current user's*
# systemd service. Run as the desktop user (NOT as root); sudo is requested only
# for the /usr/bin replacement and setcap. Nothing runs automatically.
#
#   scripts/candidate-deploy.sh backup     # save installed binary, caps, config
#   scripts/candidate-deploy.sh install    # backup (if needed) + install + restart
#   scripts/candidate-deploy.sh rollback [backup-dir]
#   scripts/candidate-deploy.sh status
set -euo pipefail

CANDIDATE="${VIETC_CANDIDATE:-target/release/vietc}"
INSTALLED=/usr/bin/vietc-daemon
SERVICE=vietc.service
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/vietc/config.toml"
LOG="${XDG_CONFIG_HOME:-$HOME/.config}/vietc/vietc.log"
BACKUP_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/vietc-rollback"

die() { echo "error: $*" >&2; exit 1; }
[ "$(id -u)" -ne 0 ] || die "run as your desktop user, not root (the service is a user service)"

latest_backup() { ls -1d "$BACKUP_ROOT"/*/ 2>/dev/null | sort | tail -n 1 | sed 's:/$::'; }

do_backup() {
    [ -x "$INSTALLED" ] || die "$INSTALLED not found"
    local dir="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$dir"
    chmod 700 "$BACKUP_ROOT" "$dir"
    cp -p "$INSTALLED" "$dir/vietc-daemon"
    getcap "$INSTALLED" | awk '{print $2}' > "$dir/caps.txt"
    [ -f "$CONFIG" ] && cp -p "$CONFIG" "$dir/config.toml"
    sha256sum "$dir/vietc-daemon" > "$dir/SHA256SUMS"
    echo "backup: $dir"
    echo "  binary sha256: $(cut -d' ' -f1 "$dir/SHA256SUMS")"
    echo "  caps: $(cat "$dir/caps.txt")"
}

replace_binary() { # $1 = source binary, $2 = capabilities string (may be empty)
    local src="$1" caps="$2" tmp="$INSTALLED.new"
    sudo install -m 755 "$src" "$tmp"
    if [ -n "$caps" ]; then sudo setcap "$caps" "$tmp"; fi
    sudo mv -f "$tmp" "$INSTALLED"   # atomic rename; the running process keeps its old inode
}

restart_and_check() {
    local mark; mark="$(date '+%Y-%m-%d %H:%M:%S')"
    systemctl --user restart "$SERVICE"
    sleep 3
    echo "service: $(systemctl --user is-active "$SERVICE")"
    echo "--- startup log since $mark ---"
    awk -v m="[$mark" '$0 >= m' "$LOG" 2>/dev/null \
        | grep -E 'Using uinput|Grabbed keyboard|Event loop started|not available|failed|error' \
        | tail -n 12 || true
    echo "expected: 'Using uinput injection', 'Grabbed keyboard device', 'Event loop started'"
}

case "${1:-}" in
backup)
    do_backup ;;
install)
    [ -f "$CANDIDATE" ] || die "candidate $CANDIDATE missing (cargo build --release -p vietc-daemon --locked --offline)"
    [ -n "$(latest_backup)" ] || do_backup
    BK="$(latest_backup)"
    echo "rollback point: $BK"
    echo "candidate sha256: $(sha256sum "$CANDIDATE" | cut -d' ' -f1)"
    echo "Restarting the service stops Vietnamese input for ~3 seconds; the keyboard keeps working."
    read -r -p "Install candidate and restart $SERVICE now? [y/N] " ok
    [ "$ok" = y ] || die "aborted, nothing changed"
    replace_binary "$CANDIDATE" "$(cat "$BK/caps.txt")"
    restart_and_check ;;
rollback)
    BK="${2:-$(latest_backup)}"
    [ -n "$BK" ] && [ -f "$BK/vietc-daemon" ] || die "no backup found in $BACKUP_ROOT"
    (cd "$BK" && sha256sum -c SHA256SUMS >/dev/null) || die "backup checksum mismatch: $BK"
    replace_binary "$BK/vietc-daemon" "$(cat "$BK/caps.txt")"
    restart_and_check
    echo "rolled back to $BK (config is untouched; saved copy: $BK/config.toml)" ;;
status)
    echo "installed:  $(sha256sum "$INSTALLED" | cut -d' ' -f1)  $(getcap "$INSTALLED" | awk '{print $2}')"
    [ -f "$CANDIDATE" ] && echo "candidate:  $(sha256sum "$CANDIDATE" | cut -d' ' -f1)"
    echo "backup:     $(latest_backup || true)"
    echo "service:    $(systemctl --user is-active "$SERVICE" || true)" ;;
*)
    sed -n '2,10p' "$0"; exit 2 ;;
esac
