#!/usr/bin/env bash
# Run fetch-bundled-* scripts with retry/backoff. GitHub-hosted runners
# occasionally fail to reach github.com for a few tens of seconds; the fetch
# scripts' own fallbacks (IWR -> curl.exe, curl --retry) fire immediately and
# cannot ride out such blips.
# Usage: bash scripts/ci-fetch-with-retry.sh <script> [<script> ...]
# .ps1 scripts are invoked with pwsh.
set -euo pipefail

run_script() {
  case "$1" in
    *.ps1) pwsh "$@" ;;
    *)     "$@" ;;
  esac
}

fetch_with_retry() {
  local script="$1" attempt=1
  until run_script "$script"; do
    if (( attempt >= 3 )); then
      echo "::error::$script failed after $attempt attempts" >&2
      return 1
    fi
    echo "$script: attempt $attempt/3 failed; retrying in 30s ..." >&2
    sleep 30
    attempt=$((attempt + 1))
  done
}

for script in "$@"; do
  fetch_with_retry "$script"
done
