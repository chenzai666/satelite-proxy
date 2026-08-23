# Fetch and stage the bundled meow core for Windows (amd64).
# Downloads meow.exe + wintun.dll from the meow-v{ver}-x86_64-pc-windows-msvc.zip
# release asset plus MetaCubeX geodata (Country.mmdb + mrs geosite.dat) into
# src-tauri/resources/bin/windows-amd64/ (meow-geodata/ subdir).
# Usage: pwsh scripts/fetch-bundled-meow-windows-amd64.ps1 [-Version 0.21.0] [-Proxy http://127.0.0.1:7890]
[CmdletBinding()]
param(
  [string]$Version = "0.21.0",
  [string]$Proxy   = $env:HTTPS_PROXY  # e.g. http://127.0.0.1:7890
)

$ErrorActionPreference = "Stop"

$ROOT = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$DEST = Join-Path $ROOT "src-tauri\resources\bin\windows-amd64"
$GEO  = Join-Path $DEST "meow-geodata"
$TMP  = Join-Path $env:TEMP "satelite-meow-$Version"

$Url = "https://github.com/madeye/meow-rs/releases/download/v$Version/meow-v$Version-x86_64-pc-windows-msvc.zip"

# Optional proxy for Invoke-WebRequest
$webParams = @{ UseBasicParsing = $true }
if ($Proxy) { $webParams.Proxy = $Proxy }

if (-not (Test-Path $DEST)) { New-Item -ItemType Directory -Path $DEST | Out-Null }
if (-not (Test-Path $GEO))  { New-Item -ItemType Directory -Path $GEO  | Out-Null }

if (Test-Path (Join-Path $DEST "meow.exe")) {
  Write-Host "meow.exe already present, skipping download."
} else {
  Write-Host "Downloading meow v$Version from $Url"
  if ($Proxy) { Write-Host "(via proxy $Proxy)" }
  New-Item -ItemType Directory -Path $TMP -Force | Out-Null
  $Zip = Join-Path $TMP "meow.zip"
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
  # meow zips keep the payload inside a version dir.
  $Exe = Get-ChildItem -Path $TMP -Recurse -Filter "meow.exe" | Select-Object -First 1
  if (-not $Exe) { throw "meow.exe not found in archive" }
  Copy-Item -Force $Exe.FullName (Join-Path $DEST "meow.exe")
  # wintun.dll ships beside the binary (Windows tun adapter). Staged under a
  # meow-specific name so it never clobbers Xray's own wintun.dll.
  $Dll = Get-ChildItem -Path $TMP -Recurse -Filter "wintun.dll" | Select-Object -First 1
  if ($Dll) { Copy-Item -Force $Dll.FullName (Join-Path $DEST "meow-wintun.dll") }
  else { Write-Warning "wintun.dll missing from archive; meow tun will rely on its embedded copy" }

  Set-Content -Path (Join-Path $DEST "meow-version.txt") -Value "v$Version" -NoNewline
}

# meow geodata: Country.mmdb (MaxMind) + geosite.dat (MetaCubeX .mrs) — same
# source meow itself auto-downloads; staged so offline first starts work.
# NOTE: meow's geosite.dat is .mrs format and lives in meow-geodata/, never
# next to Xray's v2ray-format geosite.dat of the same name.
foreach ($file in @(@{ Name = "Country.mmdb"; Url = "https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download/country.mmdb" },
                    @{ Name = "geosite.dat"; Url = "https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download/geosite.dat" })) {
  $target = Join-Path $GEO $file.Name
  if (Test-Path $target) { continue }
  Write-Host "Downloading $($file.Url)"
  try {
    Invoke-WebRequest -Uri $file.Url -OutFile $target @webParams
  } catch {
    & curl.exe -sSL -x "$Proxy" -o "$target" "$($file.Url)"
    if ($LASTEXITCODE -ne 0) { throw "curl $($file.Name) download failed (exit $LASTEXITCODE)" }
  }
}

Write-Host "Staged meow v$Version -> $DEST"
Get-ChildItem $DEST -Filter "meow*" | Format-Table Name, Length
Get-ChildItem $GEO | Format-Table Name, Length
