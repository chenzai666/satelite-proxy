#!/usr/bin/env bash
# Download official meow (macOS Intel) into app resources for bundling.
# Stages meow + meow-version.txt + meow-geodata/ (Country.mmdb + mrs
# geosite.dat from MetaCubeX — the same source meow auto-downloads).
# Usage: scripts/fetch-bundled-meow-darwin-amd64.sh [version]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-0.21.0}"
TRIPLE="x86_64-apple-darwin"
OUT_DIR="$ROOT/src-tauri/resources/bin/darwin-amd64"
ASSET="meow-v${VER}-${TRIPLE}.tar.gz"
URL="https://github.com/madeye/meow-rs/releases/download/v${VER}/${ASSET}"

mkdir -p "$OUT_DIR/meow-geodata"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [[ ! -f "$OUT_DIR/meow" ]]; then
  echo "Downloading $URL …"
  curl -fL --retry 3 -o "$TMP/$ASSET" "$URL"
  tar -xzf "$TMP/$ASSET" -C "$TMP"
  BIN="$(find "$TMP" -type f -name meow | head -1)"
  if [[ -z "$BIN" ]]; then
    echo "meow binary not found in archive" >&2
    exit 1
  fi
  cp "$BIN" "$OUT_DIR/meow"
  chmod +x "$OUT_DIR/meow"
  echo "v${VER}" > "$OUT_DIR/meow-version.txt"
else
  echo "meow already present, skipping download."
fi

# meow geodata: Country.mmdb (MaxMind) + geosite.dat (MetaCubeX .mrs) — kept
# in meow-geodata/, never next to Xray's v2ray-format geosite.dat.
for pair in "Country.mmdb country.mmdb" "geosite.dat geosite.dat"; do
  set -- $pair
  local_name="$1"; remote_name="$2"
  if [[ -f "$OUT_DIR/meow-geodata/$local_name" ]]; then continue; fi
  url="https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download/${remote_name}"
  echo "Downloading $url …"
  curl -fL --retry 3 -o "$OUT_DIR/meow-geodata/$local_name" "$url"
done

echo "Installed:"
ls -lh "$OUT_DIR"/meow "$OUT_DIR"/meow-version.txt "$OUT_DIR"/meow-geodata/* 2>/dev/null || true
"$OUT_DIR/meow" -v | head -2
