<#
.SYNOPSIS
  Stop scope-serial-monitor via runner.mjs (writes expanded stop_file).
#>
param(
  [string]$DataRoot = "",
  [string]$NodeExe = "",
  [int]$WaitSeconds = 30
)

$ErrorActionPreference = "Continue"
$pluginDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolsDir = (Resolve-Path (Join-Path $pluginDir "..\..")).Path
$repoRoot = (Resolve-Path (Join-Path $toolsDir "..")).Path
$runner = Join-Path $toolsDir "runner.mjs"
$pluginId = "scope-serial-monitor"

function Find-Node {
  param([string]$Hint)
  if ($Hint -and (Test-Path -LiteralPath $Hint)) { return $Hint }
  if ($env:NODE_EXE -and (Test-Path -LiteralPath $env:NODE_EXE)) { return $env:NODE_EXE }
  foreach ($p in @("C:\Program Files\nodejs\node.exe", "D:\SOFTWARE\NodeJS\node.exe")) {
    if (Test-Path -LiteralPath $p) { return $p }
  }
  $cmd = Get-Command node -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return "node"
}

function Get-LoopProcesses {
  Get-CimInstance Win32_Process | Where-Object {
    $cl = [string]$_.CommandLine
    if (-not $cl) { return $false }
    return ($cl.IndexOf("scope-serial-monitor") -ge 0) -and (
      ($cl.IndexOf("runner.mjs") -ge 0) -or ($cl.IndexOf("loop.mjs") -ge 0)
    )
  }
}

if (-not $DataRoot) {
  if ($env:WIPARSE_DATA_ROOT) { $DataRoot = $env:WIPARSE_DATA_ROOT }
  else { $DataRoot = $repoRoot }
}

$node = Find-Node -Hint $NodeExe
if (Test-Path -LiteralPath $runner) {
  Write-Host "Requesting graceful stop via runner..."
  & $node $runner --plugin $pluginId --lifecycle stop --data-root $DataRoot
} else {
  # Fallback: write plugin-local stop file (templates not expanded).
  $stopFile = Join-Path $pluginDir "run.stop"
  New-Item -ItemType File -Force -Path $stopFile | Out-Null
  Write-Host "Wrote $stopFile (runner.mjs missing)"
}

$deadline = (Get-Date).AddSeconds($WaitSeconds)
while ((Get-Date) -lt $deadline) {
  if ((@(Get-LoopProcesses)).Count -eq 0) { break }
  Start-Sleep -Seconds 1
}

foreach ($proc in @(Get-LoopProcesses)) {
  Write-Host ("Force stop PID {0}" -f $proc.ProcessId)
  Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
}

$lock = Join-Path $pluginDir "run.lock"
if (Test-Path -LiteralPath $lock) {
  Remove-Item -LiteralPath $lock -Force -ErrorAction SilentlyContinue
}
$stopLocal = Join-Path $pluginDir "run.stop"
if (Test-Path -LiteralPath $stopLocal) {
  Remove-Item -LiteralPath $stopLocal -Force -ErrorAction SilentlyContinue
}

Write-Host "Loop stopped."
