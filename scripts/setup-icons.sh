#!/usr/bin/env bash
# P33 · Generate the full platform icon set from a single source image.
#
# Tauri needs icon.icns (macOS), icon.ico (Windows) and PNGs (Linux) in
# src-tauri/icons/. `tauri icon` expands a 1024x1024 app-icon.png into all of
# them. The CI release workflow runs this automatically; run it once locally
# after adding/updating app-icon.png.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -f app-icon.png ]; then
  echo "error: place a 1024x1024 'app-icon.png' at the repo root first." >&2
  exit 1
fi

pnpm tauri icon
echo "==> Icon set regenerated under src-tauri/icons/"
