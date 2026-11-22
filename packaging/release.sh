#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Dboard — Release Packaging Script (.deb + Linux Tarball)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION="${1:-}"
if [ -z "$VERSION" ] || [[ "$VERSION" == --* ]]; then
    VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -1)
fi

echo "========================================================"
echo "  Dboard Release Packaging — v${VERSION}"
echo "========================================================"

echo -e "\n[1/2] Building Debian/Ubuntu .deb package..."
bash "$SCRIPT_DIR/deb/build-deb.sh" "$VERSION"

echo -e "\n[2/2] Building generic Linux release tarball..."
bash "$SCRIPT_DIR/build-tarball.sh"

echo -e "\n========================================================"
echo "  Release v${VERSION} Packaging Completed!"
echo "========================================================"
echo "Generated Artifacts:"
echo "  - Debian .deb:  packaging/deb/dboard_${VERSION}-1_amd64.deb"
echo "  - Tarball:      target/dist/dboard_${VERSION}_linux_amd64.tar.gz"
echo "========================================================"
