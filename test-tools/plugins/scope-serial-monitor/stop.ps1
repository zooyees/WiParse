<#
.SYNOPSIS
  Stop scope-serial-monitor via runner.mjs (writes {test_dir}/run.stop).
  Prefer: wiparse ui test-tool stop
  Test case dir is sanitize(file_prefix), not plugin id.
#>
param(
  [string]$DataRoot = "",
  [string]$Project = "",
  [string]$FilePrefix = "",
  [string]$TestId = "",
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

function Get-SanitizedTestId {
  param([string]$Raw, [string]$Fallback)
  if (-not $Raw) { return $Fallback }
  $slug = ($Raw.Trim() -replace "\s+", "_")
  $slug = $slug -replace "[^a-zA-Z0-9._-]+", "_"
  $slug = $slug -replace "^[^a-zA-Z0-9]+", ""
  $slug = $slug -replace "[._-]+$", ""
  if ($slug -match "^[a-zA-Z0-9][a-zA-Z0-9._-]*$") { return $slug }
  return $Fallback
}

function Get-DefaultFilePrefix {
  $station = Join-Path $pluginDir "station.json"
  if (Test-Path -LiteralPath $station) {
    try {
      $j = Get-Content -LiteralPath $station -Raw -Encoding UTF8 | ConvertFrom-Json
      if ($j.paths.file_prefix) { return [string]$j.paths.file_prefix }
    } catch {}
  }
  return "ScopeSerial"
}

if (-not $DataRoot) {
  if ($env:WIPARSE_DATA_ROOT) { $DataRoot = $env:WIPARSE_DATA_ROOT }
  else { $DataRoot = $repoRoot }
}
if (-not $Project) {
  if ($env:WIPARSE_PROJECT) { $Project = $env:WIPARSE_PROJECT }
  else { $Project = "default" }
}
if (-not $FilePrefix) {
  if ($env:WIPARSE_TEST_ID) { $FilePrefix = $env:WIPARSE_TEST_ID }
  else { $FilePrefix = Get-DefaultFilePrefix }
}
if (-not $TestId) {
  $TestId = Get-SanitizedTestId -Raw $FilePrefix -Fallback $pluginId
}

$testDir = Join-Path $DataRoot ("projects\" + $Project + "\tests\" + $TestId)
$env:WIPARSE_PROJECT = $Project
$env:WIPARSE_TEST_ID = $TestId
$node = Find-Node -Hint $NodeExe
if (Test-Path -LiteralPath $runner) {
  Write-Host ("Requesting graceful stop via runner (test_id={0})..." -f $TestId)
  & $node $runner --plugin $pluginId --lifecycle stop --data-root $DataRoot --project $Project --test-id $TestId -- --file_prefix $FilePrefix
} else {
  $stopFile = Join-Path $testDir "run.stop"
  New-Item -ItemType Directory -Force -Path $testDir | Out-Null
  Set-Content -LiteralPath $stopFile -Value "host stop" -Encoding utf8
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

$lock = Join-Path $testDir "run.lock"
if (Test-Path -LiteralPath $lock) {
  Remove-Item -LiteralPath $lock -Force -ErrorAction SilentlyContinue
}
$stopLocal = Join-Path $testDir "run.stop"
if (Test-Path -LiteralPath $stopLocal) {
  Remove-Item -LiteralPath $stopLocal -Force -ErrorAction SilentlyContinue
}

Write-Host "Loop stopped."
