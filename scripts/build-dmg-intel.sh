#!/usr/bin/env bash
# Build an Intel (x86_64) macOS .dmg. Run on macOS.
# Usage: scripts/build-dmg-intel.sh [sing-box-version]
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/build-dmg.sh" --arch intel "$@"
