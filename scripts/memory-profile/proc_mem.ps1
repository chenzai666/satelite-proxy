# Sum WebView2 process-tree memory belonging to a root app process.
# usage: powershell -NoProfile -ExecutionPolicy Bypass -File proc_mem.ps1 [-RootName satelite]
param([string]$RootName = "satelite")
$ErrorActionPreference = "SilentlyContinue"
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine, WorkingSetSize, PrivatePageCount
$byPid = @{}
foreach ($p in $all) { $byPid[[uint32]$p.ProcessId] = $p }
function Test-DescendantOf($proc, [uint32]$rootPid) {
  $cur = $proc
  for ($i = 0; $i -lt 24; $i++) {
    if ($null -eq $cur) { return $false }
    if ([uint32]$cur.ProcessId -eq $rootPid) { return $true }
    $parent = $byPid[[uint32]$cur.ParentProcessId]
    if ($null -eq $parent) { return $false }
    $cur = $parent
  }
  return $false
}
$roots = @($all | Where-Object { $_.Name -like "*$RootName*" })
$ts = Get-Date -Format "HH:mm:ss"
foreach ($root in $roots) {
  $wv = @($all | Where-Object { $_.Name -eq "msedgewebview2.exe" -and (Test-DescendantOf $_ ([uint32]$root.ProcessId)) })
  $totalWs = ($wv | Measure-Object WorkingSetSize -Sum).Sum
  $totalPriv = ($wv | Measure-Object PrivatePageCount -Sum).Sum
  if ($null -eq $totalWs) { $totalWs = 0 }
  if ($null -eq $totalPriv) { $totalPriv = 0 }
  Write-Output ("PROCSNAP {0} root={1}({2}) wv_count={3} ws_total={4:N1}MB priv_total={5:N1}MB" -f $ts, $root.Name, $root.ProcessId, $wv.Count, ($totalWs/1MB), ($totalPriv/1MB))
  foreach ($p in $wv) {
    $cl = [string]$p.CommandLine
    $type = "browser"
    if ($cl -match "--type=([a-z-]+)") { $type = $Matches[1] }
    Write-Output ("  pid={0,-8} type={1,-12} ws={2,8:N1}MB priv={3,8:N1}MB" -f $p.ProcessId, $type, ($p.WorkingSetSize/1MB), ($p.PrivatePageCount/1MB))
  }
  Write-Output ("ROOTMEM {0} pid={1} ws={2:N1}MB priv={3:N1}MB" -f $root.Name, $root.ProcessId, ($root.WorkingSetSize/1MB), ($root.PrivatePageCount/1MB))
}
