#!/usr/bin/env bash
# Download official sing-box (macOS Intel / x86_64) into app resources for bundling.
# Usage: scripts/fetch-bundled-core-darwin-amd64.sh [version]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-1.13.18}"
OUT_DIR="$ROOT/src-tauri/resources/bin/darwin-amd64"
ASSET="sing-box-${VER}-darwin-amd64.tar.gz"
URL="https://github.com/SagerNet/sing-box/releases/download/v${VER}/${ASSET}"

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL …"
curl -fL --retry 3 -o "$TMP/$ASSET" "$URL"
tar -xzf "$TMP/$ASSET" -C "$TMP"
BIN="$(find "$TMP" -type f -name sing-box | head -1)"
if [[ -z "$BIN" ]]; then
  echo "sing-box binary not found in archive" >&2
  exit 1
fi

cp "$BIN" "$OUT_DIR/sing-box"
chmod +x "$OUT_DIR/sing-box"
echo "v${VER}" > "$OUT_DIR/version.txt"

echo "Installed:"
ls -lh "$OUT_DIR/sing-box" "$OUT_DIR/version.txt"
"$OUT_DIR/sing-box" version | head -3
