#!/usr/bin/env bash
# Viet+ — Linux Mint / Ubuntu test VM setup script
# Usage: curl -fsSL <url> | bash
#   or: bash scripts/setup-test-vm.sh
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

echo -e "${GREEN}=== Viet+ Test VM Setup ===${NC}"

# 1. Install system deps
echo -e "${YELLOW}[1/5] Installing system dependencies...${NC}"
sudo apt update -y
sudo apt install -y build-essential pkg-config libx11-dev libxtst-dev \
  libdbus-1-dev libevdev-dev libwayland-dev curl git \
  libevdev2 libdbus-1-3 libx11-6 libxtst6 libwayland-client0 \
  wl-clipboard xclip wtype xdotool zenity qttools5-dev-tools xterm

# 2. Install Rust if missing
echo -e "${YELLOW}[2/5] Installing Rust...${NC}"
if ! command -v cargo &>/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
else
  echo "  Rust already installed."
fi

# 3. Clone and build
echo -e "${YELLOW}[3/5] Cloning and building...${NC}"
if [ ! -d vietc ]; then
  git clone https://github.com/vndangkhoa/vietc.git
fi
cd vietc && git checkout staging && cargo build --release

# 4. Install
echo -e "${YELLOW}[4/5] Installing...${NC}"
sudo ./install.sh

# 5. Done
echo -e "${YELLOW}[5/5] Setup complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Reboot to apply group + udev changes  ${NC}"
echo -e "${GREEN}  Then: vietc-tray &                     ${NC}"
echo -e "${GREEN}  Or:   sudo vietc-daemon                ${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Quick test:"
echo "  cargo run --bin vietc-cli"
echo ""
echo "Deterministic Phase 6 integration harness:"
echo "  cargo test -p vietc-daemon --test daemon_suite -- --test-threads=1"
echo ""
echo "Privileged runtime verification (run inside the target desktop session):"
echo "  VIETC_RUNTIME_TESTS=1 cargo test -p vietc-daemon --test runtime \\"
echo "    privileged_runtime_matrix_records_exact_ready_and_skip_evidence \\"
echo "    -- --ignored --nocapture --test-threads=1"
echo "  VIETC_RUNTIME_TESTS=1 VIETC_CONTROLLED_DISPLAY=1 cargo test -p vietc-daemon --test runtime \\"
echo "    privileged_runtime_targets_execute_controlled_scenarios \\"
echo "    -- --ignored --nocapture --test-threads=1"
echo "  VIETC_RUNTIME_TESTS=1 cargo test -p vietc-daemon --test runtime \\"
echo "    wayland_wtype_success_is_observable_at_runtime \\"
echo "    -- --ignored --nocapture --test-threads=1"
echo ""
echo "Run the privileged matrix separately in X11, COSMIC Wayland, GNOME Wayland, and KDE Wayland."
echo "Each Chromium, Google Chrome, Electron, GTK, Qt, terminal, and desktop capability"
echo "is recorded as PASS, SKIPPED, UNAVAILABLE, or FAILED with a reason and owner."
echo "Real desktop apps are never launched until the controlled fixture is validated."
echo "Profiles and artifacts stay isolated under target/runtime-fixtures/."
echo "Artifacts are written under target/runtime-artifacts/."
echo ""
echo "Terminal typing (VNI mode auto-enabled in terminals):"
echo "  cha2o       -> chào"
echo "  ba5n        -> bạn"
echo "  to6i te6n la2 Khoa3 -> tôi tên là Khỏa"
echo "  d9o7i       -> đời"
echo ""
echo "Telex (Ctrl+LeftShift to switch):"
echo "  chaof       -> chào"
echo "  banj        -> bạn"
echo "  tooi teen laf Khoar -> tôi tên là Khỏa"
echo "  ddoi        -> đời"
echo ""
echo "Macros:"
echo "  ko[space]   -> không"
echo "  dc[space]   -> được"
echo "  vs[space]   -> với"
