#!/usr/bin/env bash
# P33 · Cut a release. Tags and pushes, which triggers the GitHub release
# workflow (.github/workflows/release.yml) to build all three platforms.
#
# Usage: scripts/release.sh v0.2.0
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?usage: scripts/release.sh v0.2.0}"
case "$VERSION" in
  v*) ;;
  *) echo "error: version must start with 'v' (e.g. v0.2.0)" >&2; exit 1 ;;
esac

git tag "$VERSION"
git push origin "$VERSION"
echo "==> Tagged $VERSION — the release workflow will build Windows/macOS/Linux."
