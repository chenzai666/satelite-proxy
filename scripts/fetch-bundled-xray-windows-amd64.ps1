# Fetch and stage the bundled Xray core for Windows (amd64).
# Downloads xray.exe + geosite.dat + geoip.dat from the Xray-windows-64.zip
# release asset into src-tauri/resources/bin/windows-amd64/.
# Usage: pwsh scripts/fetch-bundled-xray-windows-amd64.ps1 [-Version 26.3.27]
[CmdletBinding()]
param(
  [string]$Version = "26.3.27",
  [string]$Proxy   = $env:HTTPS_PROXY  # e.g. http://127.0.0.1:7890
)

$ErrorActionPreference = "Stop"

$ROOT = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$DEST = Join-Path $ROOT "src-tauri\resources\bin\windows-amd64"
$TMP  = Join-Path $env:TEMP "satelite-xray-$Version"

$Url = "https://github.com/XTLS/Xray-core/releases/download/v$Version/Xray-windows-64.zip"

# Optional proxy for Invoke-WebRequest
$webParams = @{ UseBasicParsing = $true }
if ($Proxy) { $webParams.Proxy = $Proxy }

if (-not (Test-Path $DEST)) { New-Item -ItemType Directory -Path $DEST | Out-Null }

if (Test-Path (Join-Path $DEST "xray.exe")) {
  Write-Host "xray.exe already present, skipping download."
  return
}

Write-Host "Downloading Xray v$Version from $Url"
if ($Proxy) { Write-Host "(via proxy $Proxy)" }
New-Item -ItemType Directory -Path $TMP -Force | Out-Null
$Zip = Join-Path $TMP "xray.zip"
try {
  Invoke-WebRequest -Uri $Url -OutFile $Zip @webParams
} catch {
  # Fall back to curl.exe (ships with Win10+) which honours env proxies
  Write-Host "Invoke-WebRequest failed, retrying with curl.exe..."
  & curl.exe -sSL -x "$Proxy" -o "$Zip" "$Url"
  if ($LASTEXITCODE -ne 0) { throw "curl download failed (exit $LASTEXITCODE)" }
}

Write-Host "Extracting..."
Expand-Archive -Path $Zip -DestinationPath $TMP -Force

# Xray zips keep the payload at the archive root (no inner version dir).
$Exe = Join-Path $TMP "xray.exe"
if (-not (Test-Path $Exe)) { throw "xray.exe not found in archive" }
Copy-Item -Force $Exe (Join-Path $DEST "xray.exe")
# geodata ships alongside the binary: stage it so geosite:/geoip: routing works
foreach ($dat in @("geosite.dat", "geoip.dat")) {
  $src = Join-Path $TMP $dat
  if (Test-Path $src) { Copy-Item -Force $src (Join-Path $DEST $dat) }
  else { Write-Warning "$dat missing from archive; geo rules will need a runtime download" }
}

# wintun.dll powers the native tun inbound (NOT shipped in the Xray zip).
if (-not (Test-Path (Join-Path $DEST "wintun.dll"))) {
  $WintunZip = Join-Path $TMP "wintun.zip"
  $WintunUrl = "https://www.wintun.net/builds/wintun-0.14.1.zip"
  Write-Host "Downloading wintun.dll from $WintunUrl"
  try {
    Invoke-WebRequest -Uri $WintunUrl -OutFile $WintunZip @webParams
  } catch {
    & curl.exe -sSL -x "$Proxy" -o "$WintunZip" "$WintunUrl"
    if ($LASTEXITCODE -ne 0) { throw "curl wintun download failed (exit $LASTEXITCODE)" }
  }
  Expand-Archive -Path $WintunZip -DestinationPath (Join-Path $TMP "wintun") -Force
  Copy-Item -Force (Join-Path $TMP "wintun\wintun\bin\amd64\wintun.dll") (Join-Path $DEST "wintun.dll")
}

Set-Content -Path (Join-Path $DEST "xray-version.txt") -Value "v$Version" -NoNewline

Write-Host "Staged Xray v$Version -> $DEST"
Get-ChildItem $DEST | Format-Table Name, Length
