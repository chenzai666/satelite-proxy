# Build the frontend + Tauri app and package it as a Windows installer.
# Usage:
#   pwsh scripts/build-windows.ps1                 # NSIS (.exe) setup, default
#   pwsh scripts/build-windows.ps1 -Bundle msi     # MSI installer
#   pwsh scripts/build-windows.ps1 -Bundle nsis    # NSIS explicitly
#   pwsh scripts/build-windows.ps1 -Proxy http://127.0.0.1:7890
[CmdletBinding()]
param(
  [ValidateSet("nsis", "msi")]
  [string]$Bundle = "nsis",
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

# --- 3. Frontend deps --------------------------------------------------------
Write-Host "Installing JS dependencies..."
pnpm install --frozen-lockfile

# --- 4. Build + bundle -------------------------------------------------------
Write-Host "Building app and packaging $Bundle installer..."
pnpm tauri build --bundles $Bundle

# --- 5. Locate artifact ------------------------------------------------------
$OutDir = Join-Path $ROOT "src-tauri\target\release\bundle\$Bundle"
if (-not (Test-Path $OutDir)) {
  Write-Error "Build finished but no $Bundle output under $OutDir"
  exit 1
}
$Artifact = Get-ChildItem $OutDir -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $Artifact) {
  Write-Error "No artifact found in $OutDir"
  exit 1
}

Write-Host ""
Write-Host "==========================================" -ForegroundColor Green
Write-Host "$Bundle installer ready:" -ForegroundColor Green
Write-Host "  $($Artifact.FullName)" -ForegroundColor Green
Write-Host "  $([math]::Round($Artifact.Length / 1MB, 1)) MB" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green
