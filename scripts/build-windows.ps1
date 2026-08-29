# Build the frontend + Tauri app and package it as a Windows installer.
# Usage:
#   pwsh scripts/build-windows.ps1                 # NSIS (.exe) setup, sing-box core only (default)
#   pwsh scripts/build-windows.ps1 -Bundle msi     # MSI installer
#   pwsh scripts/build-windows.ps1 -Bundle portable # extract-and-run zip
#   pwsh scripts/build-windows.ps1 -AllCores       # also bundle the Xray + mihomo cores
#   pwsh scripts/build-windows.ps1 -Proxy http://127.0.0.1:7890
[CmdletBinding()]
param(
  [ValidateSet("nsis", "msi", "portable")]
  [string]$Bundle = "nsis",
  [switch]$AllCores,
  [string]$Proxy  = $env:HTTPS_PROXY,
  [string]$CoreVersion = "1.13.15"
)

$ErrorActionPreference = "Stop"

$ROOT = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $ROOT

function Test-Cmd([string]$n) { return [bool](Get-Command $n -ErrorAction SilentlyContinue) }

# --- 0. Toolchain checks -----------------------------------------------------
foreach ($c in @("node", "pnpm", "cargo", "rustc")) {
  if (-not (Test-Cmd $c)) {
    Write-Error "'$c' not found in PATH. Install Node.js (winget OpenJS.NodeJS), pnpm (npm i -g pnpm), and Rust stable-msvc (https://win.rustup.rs)."
    exit 1
  }
}

# MSVC link.exe is required by the stable-msvc Rust target (Tauri can't link without it).
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$msvcOk = $false
if (Test-Path $vswhere) {
  $msvcOk = [bool](& $vswhere -latest -products '*' `
      -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
      -property installationPath 2>$null)
}
if (-not $msvcOk) {
  Write-Warning "MSVC C++ build tools not detected. The build will likely fail at link time.`n`nInstall 'Desktop development with C++' via Visual Studio Installer, or:`n  winget install Microsoft.VisualStudio.2022.BuildTools --override '--add Microsoft.VisualStudio.Workload.VCTools --quiet'"
}

# --- 1. Proxy env (lets Tauri fetch WiX / NSIS tooling from GitHub) -----------
if ($Proxy) {
  Write-Host "Using proxy: $Proxy"
  $env:HTTPS_PROXY = $Proxy
  $env:HTTP_PROXY  = $Proxy
  $env:ALL_PROXY   = $Proxy
}

# --- 2. Stage bundled sing-box core -----------------------------------------
$CoreExe = Join-Path $ROOT "src-tauri\resources\bin\windows-amd64\sing-box.exe"
if (-not (Test-Path $CoreExe)) {
  Write-Host "sing-box core missing, fetching..."
  & (Join-Path $PSScriptRoot "fetch-bundled-core-windows-amd64.ps1") -Version $CoreVersion -Proxy $Proxy
}

# --- 2b. Extra cores (Xray + mihomo) or the sing-box-only config overlay -----
$TauriConfigArgs = @()
if ($AllCores) {
  Write-Host "AllCores: bundling Xray + mihomo alongside sing-box..."
  $XrayExe = Join-Path $ROOT "src-tauri\resources\bin\windows-amd64\xray.exe"
  if (-not (Test-Path $XrayExe)) {
    Write-Host "xray core missing, fetching..."
    & (Join-Path $PSScriptRoot "fetch-bundled-xray-windows-amd64.ps1") -Proxy $Proxy
  }
  $MihomoExe = Join-Path $ROOT "src-tauri\resources\bin\windows-amd64\mihomo.exe"
  if (-not (Test-Path $MihomoExe)) {
    Write-Host "mihomo core missing, fetching..."
    & (Join-Path $PSScriptRoot "fetch-bundled-mihomo-windows-amd64.ps1") -Proxy $Proxy
  }
} else {
  # The base config lists the Xray/mihomo resources too — a missing file
  # fails the bundler, so switch to the sing-box-only overlay instead.
  Write-Host "Bundling sing-box only (pass -AllCores to include Xray + mihomo)."
  $TauriConfigArgs = @("--config", (Join-Path $ROOT "src-tauri\tauri.singbox-windows.conf.json"))
}

# --- 2c. Stage built-in remote rule sets -------------------------------------
# Keep in sync with BUILTIN_REMOTE_RULE_SETS in src-tauri/src/domain/rule.rs.
$RuleSets = @(
  @{ Name = "system-geolocation-not-cn.srs"; Url = "https://cdn.jsdelivr.net/gh/SagerNet/sing-geosite@rule-set/geosite-geolocation-!cn.srs" },
  @{ Name = "system-geoip-cn.srs";           Url = "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geoip/cn.srs" },
  @{ Name = "system-geosite-cn.srs";         Url = "https://testingcf.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geosite/cn.srs" }
)
$RuleSetsDir = Join-Path $ROOT "src-tauri\resources\rule-sets"
New-Item -ItemType Directory -Force -Path $RuleSetsDir | Out-Null
foreach ($set in $RuleSets) {
  $out = Join-Path $RuleSetsDir $set.Name
  if (Test-Path $out) { continue }
  Write-Host "fetching $($set.Name)..."
  $downloadArgs = @{ Uri = $set.Url; OutFile = $out; UseBasicParsing = $true }
  if ($Proxy) { $downloadArgs.Proxy = $Proxy }
  Invoke-WebRequest @downloadArgs
  $magic = [System.Text.Encoding]::ASCII.GetString((Get-Content $out -AsByteStream -TotalCount 3))
  if ($magic -ne "SRS") {
    Remove-Item $out -ErrorAction SilentlyContinue
    Write-Error "$($set.Name) is not a binary SRS (bad URL or HTML error page)"
    exit 1
  }
}

# --- 3. Frontend deps --------------------------------------------------------
Write-Host "Installing JS dependencies..."
pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}

# --- 4. Build + bundle -------------------------------------------------------
if ($Bundle -eq "portable") {
  Write-Host "Building app for portable zip..."
  pnpm tauri build --no-bundle @TauriConfigArgs
} else {
  Write-Host "Building app and packaging $Bundle installer..."
  $BuildArgs = @("--bundles", $Bundle) + $TauriConfigArgs
  pnpm tauri build @BuildArgs
}
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}

# --- 5. Locate artifact ------------------------------------------------------
if ($Bundle -eq "portable") {
  $Version = (Get-Content (Join-Path $ROOT "package.json") -Raw | ConvertFrom-Json).version
  $StageRoot = Join-Path $ROOT "src-tauri\target\release\portable-stage"
  $Stage = Join-Path $StageRoot "Satelite-Portable"
  if (Test-Path $StageRoot) { Remove-Item -Recurse -Force $StageRoot }
  New-Item -ItemType Directory -Force -Path $Stage | Out-Null
  $Exe = Join-Path $ROOT "src-tauri\target\release\satelite-proxy.exe"
  if (-not (Test-Path $Exe)) {
    Write-Error "Build finished but $Exe is missing"
    exit 1
  }
  Copy-Item $Exe $Stage
  $ConfName = if ($AllCores) { "tauri.windows.conf.json" } else { "tauri.singbox-windows.conf.json" }
  $Conf = Get-Content (Join-Path $ROOT "src-tauri\$ConfName") -Raw | ConvertFrom-Json
  foreach ($Entry in @($Conf.bundle.resources)) {
    $Src = Join-Path $ROOT "src-tauri\$Entry"
    if (-not (Test-Path $Src)) {
      Write-Error "Resource listed in $ConfName is missing: $Entry"
      exit 1
    }
    $Dest = Join-Path $Stage $Entry
    New-Item -ItemType Directory -Force -Path (Split-Path $Dest -Parent) | Out-Null
    Copy-Item -Recurse -Force $Src $Dest
  }
  Set-Content -Path (Join-Path $Stage "portable.flag") -Value "Satelite portable marker - keep this file next to satelite-proxy.exe." -NoNewline
  Set-Content -Path (Join-Path $Stage "README.txt") -Encoding utf8 -Value @"
Satelite 便携版 $Version
========================

解压后直接运行 satelite-proxy.exe，无需安装。

- 数据、内核、配置、日志和 WebView2 缓存均保存在此目录；请保留 portable.flag。
- 删除 portable.flag 后将回退为安装版行为，数据会写入 AppData。
- 需要 Microsoft Edge WebView2 运行时；Windows 10/11 通常已经自带。
- 便携版与安装版不能同时运行。
"@
  $OutDir = Join-Path $ROOT "src-tauri\target\release\bundle\portable"
  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  $Artifact = Join-Path $OutDir "Satelite_${Version}_x64_portable.zip"
  if (Test-Path $Artifact) { Remove-Item -Force $Artifact }
  Compress-Archive -Path $Stage -DestinationPath $Artifact -CompressionLevel Optimal
} else {
  $OutDir = Join-Path $ROOT "src-tauri\target\release\bundle\$Bundle"
  if (-not (Test-Path $OutDir)) {
    Write-Error "Build finished but no $Bundle output under $OutDir"
    exit 1
  }
  $Artifact = (Get-ChildItem $OutDir -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
  if (-not $Artifact) {
    Write-Error "No artifact found in $OutDir"
    exit 1
  }
}

Write-Host ""
Write-Host "==========================================" -ForegroundColor Green
Write-Host "$Bundle artifact ready:" -ForegroundColor Green
Write-Host "  $Artifact" -ForegroundColor Green
Write-Host "  $([math]::Round((Get-Item $Artifact).Length / 1MB, 1)) MB" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green
