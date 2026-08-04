#!/usr/bin/env bash
# Build the frontend + Tauri app and package it as a macOS .dmg.
# Usage: scripts/build-dmg.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

command -v pnpm >/dev/null || { echo "pnpm not found in PATH" >&2; exit 1; }
command -v cargo >/dev/null || { echo "cargo not found in PATH" >&2; exit 1; }

CORE_BIN="$ROOT/src-tauri/resources/bin/darwin-arm64/sing-box"
if [[ ! -x "$CORE_BIN" ]]; then
  echo "sing-box core missing, fetching..."
  "$ROOT/scripts/fetch-bundled-core-darwin-arm64.sh"
fi

echo "Installing JS dependencies..."
pnpm install --frozen-lockfile

echo "Building app and packaging dmg..."
pnpm tauri build --bundles dmg

DMG="$(find "$ROOT/src-tauri/target/release/bundle/dmg" -name '*.dmg' -maxdepth 1 | head -1)"
if [[ -z "$DMG" ]]; then
  echo "Build finished but no .dmg found under src-tauri/target/release/bundle/dmg" >&2
  exit 1
fi

echo "DMG ready: $DMG"
