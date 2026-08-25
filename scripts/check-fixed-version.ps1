$ErrorActionPreference = "Stop"

$expectedVersion = "1.0.16"
$repoRoot = Split-Path -Parent $PSScriptRoot

$packageVersion = (Get-Content -LiteralPath (Join-Path $repoRoot "package.json") -Raw |
    ConvertFrom-Json).version
$metadata = cargo metadata --no-deps --format-version 1 `
    --manifest-path (Join-Path $repoRoot "src-tauri\Cargo.toml") | ConvertFrom-Json
$cargoVersion = ($metadata.packages |
    Where-Object { $_.name -eq "satelite-proxy" } |
    Select-Object -First 1).version

$lockText = Get-Content -LiteralPath (Join-Path $repoRoot "src-tauri\Cargo.lock") -Raw
$lockMatch = [regex]::Match(
    $lockText,
    '(?ms)^\[\[package\]\]\r?\nname = "satelite-proxy"\r?\nversion = "([^"]+)"'
)
$lockVersion = if ($lockMatch.Success) { $lockMatch.Groups[1].Value } else { $null }

$versions = [ordered]@{
    "package.json" = $packageVersion
    "Cargo.toml"   = $cargoVersion
    "Cargo.lock"   = $lockVersion
}

$invalid = @($versions.GetEnumerator() | Where-Object { $_.Value -ne $expectedVersion })
if ($invalid.Count -gt 0) {
    $details = ($invalid | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ", "
    throw "应用版本已固定为 $expectedVersion，不允许修改：$details"
}

Write-Host "固定版本检查通过：$expectedVersion"
