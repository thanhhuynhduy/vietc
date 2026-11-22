#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

OUTPUT="${1:-dist/RELEASE-EVIDENCE.md}"
CHANNEL="${RELEASE_CHANNEL:-unknown}"
TAG="${RELEASE_TAG:-${GITHUB_REF_NAME:-untagged}}"
COMMIT="${GITHUB_SHA:-$(git rev-parse HEAD)}"
RUN_URL=""

if [ -n "${GITHUB_SERVER_URL:-}" ] && [ -n "${GITHUB_REPOSITORY:-}" ] && [ -n "${GITHUB_RUN_ID:-}" ]; then
    RUN_URL="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
fi

mkdir -p "$(dirname "$OUTPUT")"

RUST_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"
KERNEL="$(uname -srmo)"
DISTRO="$(. /etc/os-release 2>/dev/null && printf '%s %s' "${NAME:-unknown}" "${VERSION_ID:-unknown}")"

cat > "$OUTPUT" <<EOF
# Dboard Release Evidence

- Channel: ${CHANNEL}
- Tag: ${TAG}
- Commit: ${COMMIT}
- Generated at UTC: $(date -u '+%Y-%m-%dT%H:%M:%SZ')
- Workflow run: ${RUN_URL:-local}
- Rust: ${RUST_VERSION}
- Cargo: ${CARGO_VERSION}
- Build host: ${DISTRO}
- Kernel: ${KERNEL}

## Automated quality gates

- cargo fmt --all -- --check
- cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
- make test-ci-smoke CARGO_FLAGS=--locked
- Debian package clean-container install/remove/purge smoke test
- SHA-256 verification for published artifacts

## Artifact checksums

\`\`\`text
$(if [ -f "$(dirname "$OUTPUT")/SHA256SUMS" ]; then cat "$(dirname "$OUTPUT")/SHA256SUMS"; else echo "Generated after evidence collection."; fi)
\`\`\`

## Manual compatibility evidence

Complete before promoting a candidate to stable:

| Environment | Fresh install | Upgrade | Telex | VNI | Password/PIN bypass | Clipboard restore | Rollback |
|---|---|---|---|---|---|---|---|
| GNOME Wayland | Pending | Pending | Pending | Pending | Pending | Pending | Pending |
| COSMIC Wayland | Pending | Pending | Pending | Pending | Pending | Pending | Pending |
| X11 | Pending | Pending | Pending | Pending | Pending | Pending | Pending |

## Promotion decision

- Candidate approved by:
- Candidate tag:
- Stable tag:
- Known limitations:
- Rollback artifact/checksum:
EOF