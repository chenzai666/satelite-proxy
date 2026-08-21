#!/usr/bin/env bash
# Download the built-in remote rule sets (binary .srs) into app resources
# for bundling. Run by the build scripts, or manually once before
# `pnpm tauri dev` (otherwise the app falls back to downloading the sets
# from their URLs on first launch).
#
# Usage:
#   scripts/fetch-bundled-rule-sets.sh            # fetch missing files
#   scripts/fetch-bundled-rule-sets.sh --force    # refresh all files
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/src-tauri/resources/rule-sets"

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
  FORCE=1
elif [[ -n "${1:-}" ]]; then
  echo "unknown option: $1 (only --force is supported)" >&2
  exit 1
fi

# Keep in sync with BUILTIN_REMOTE_RULE_SETS in src-tauri/src/domain/rule.rs.
NAMES=(
  "builtin-remote-geolocation-not-cn.srs"
  "builtin-remote-geoip-cn.srs"
  "builtin-remote-geosite-cn.srs"
)
URLS=(
  "https://cdn.jsdelivr.net/gh/SagerNet/sing-geosite@rule-set/geosite-geolocation-!cn.srs"
  "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geoip/cn.srs"
  "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geosite/cn.srs"
)

mkdir -p "$OUT_DIR"
status=0
for i in "${!NAMES[@]}"; do
  name="${NAMES[$i]}"
  url="${URLS[$i]}"
  out="$OUT_DIR/$name"
  if [[ $FORCE -eq 0 && -f "$out" ]] && [[ "$(head -c 3 "$out")" == "SRS" ]]; then
    echo "rule set $name already present"
    continue
  fi
  echo "fetching $name ..."
  if curl -fsSL --retry 3 --connect-timeout 15 -o "$out.tmp" "$url"; then
    if [[ "$(head -c 3 "$out.tmp")" == "SRS" ]]; then
      mv "$out.tmp" "$out"
      echo "  ok ($(wc -c < "$out" | tr -d ' ') bytes)"
    else
      rm -f "$out.tmp"
      echo "  ERROR: $name is not a binary SRS (bad URL or HTML error page)" >&2
      status=1
    fi
  else
    rm -f "$out.tmp"
    echo "  ERROR: download failed: $url" >&2
    status=1
  fi
done
exit $status
