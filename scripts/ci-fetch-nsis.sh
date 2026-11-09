#!/usr/bin/env bash
# 仅 Actions Windows runner：通过官方资产 API 预取锁定 CLI 所需的打包工具。
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_OS:-}" == Windows ]]
[[ -n "${GH_TOKEN:-}" ]]
tools="$(cygpath -u "$LOCALAPPDATA")/tauri"
temp_root="$(cygpath -u "$RUNNER_TEMP")"
archive=$(mktemp "$temp_root/satelite-nsis-archive.XXXXXX")
plugin=$(mktemp "$temp_root/satelite-nsis-plugin.XXXXXX")
trap 'rm -f -- "$archive" "$plugin"' EXIT

fetch_verified() {
  local asset=$1 file=$2 digest=$3 attempt
  for attempt in 1 2 3; do
    if gh api "$asset" -H 'Accept: application/octet-stream' > "$file" &&
      printf '%s  %s\n' "$digest" "$file" | sha256sum --check --status; then
      return 0
    fi
    if [[ "$attempt" != 3 ]]; then sleep "$((attempt * 2))"; fi
  done
  echo '官方 NSIS 资产下载失败或 SHA-256 不符' >&2
  return 1
}

# 来源：tauri-cli-v2.11.4 的 NSIS 3.11 / nsis_tauri_utils 0.5.3。
# 升级锁文件中的 CLI 时须同步审计版本、资产 ID 和摘要。
fetch_verified repos/tauri-apps/binary-releases/releases/assets/317051073 "$archive" c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1
fetch_verified repos/tauri-apps/nsis-tauri-utils/releases/assets/345882097 "$plugin" 5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709
[[ ! -e "$tools/NSIS" && ! -e "$tools/nsis-3.11" ]] || {
  echo '预取需要全新的 runner 工具目录，保留现有目录并停止' >&2; exit 1;
}
mkdir -p "$tools"
7z x -y "$archive" "-o$tools" > /dev/null
[[ -f "$tools/nsis-3.11/makensis.exe" ]]
mv -- "$tools/nsis-3.11" "$tools/NSIS"
mkdir -p "$tools/NSIS/Plugins/x86-unicode/additional"
cp -- "$plugin" "$tools/NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll"
echo 'NSIS 工具与插件已通过 SHA-256 校验并就绪'
