#!/usr/bin/env bash
# Download official sing-box (macOS Apple Silicon) into app resources for bundling.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-1.13.18}"
OUT_DIR="$ROOT/src-tauri/resources/bin/darwin-arm64"
ASSET="sing-box-${VER}-darwin-arm64.tar.gz"
URL="https://github.com/SagerNet/sing-box/releases/download/v${VER}/${ASSET}"

mkdir -p "$OUT_DIR"

# Skip when the exact pinned version is already staged (keeps CI cache hits
# and repeat local builds download-free; a version bump refreshes it).
if [[ -f "$OUT_DIR/sing-box" && "$(cat "$OUT_DIR/version.txt" 2>/dev/null)" == "v${VER}" ]]; then
  echo "sing-box v${VER} already staged, skipping download."
  exit 0
fi

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
