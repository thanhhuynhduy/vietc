#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

REPOSITORY="${1:-thanhhuynhduy/vietc}"
BRANCH="${2:-main}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

gh auth status >/dev/null
gh api \
    --method PUT \
    --input "$ROOT/.github/branch-protection-main.json" \
    "repos/${REPOSITORY}/branches/${BRANCH}/protection"

echo "Configured branch protection for ${REPOSITORY}:${BRANCH}"