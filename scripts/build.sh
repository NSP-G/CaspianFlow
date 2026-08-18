#!/usr/bin/env bash
# P33 · Local build helper (HOST platform only).
#
# Builds the native installer for the machine you run this on. Cross-building
# all three platforms from one OS is fragile (needs mingw / osxcross), so the
# supported path for the full set is .github/workflows/release.yml (one GitHub
# runner per OS). Use this script for a fast local smoke-test of your own platform.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Installing frontend deps"
pnpm install

echo "==> Building frontend (vite -> dist)"
pnpm build

echo "==> Building Tauri app (host target, --features tauri)"
pnpm tauri build --features tauri

echo "==> Done. Artifacts in: src-tauri/target/release/bundle/"
