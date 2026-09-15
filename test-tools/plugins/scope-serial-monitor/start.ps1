<#
.SYNOPSIS
  Legacy helper. Prefer:
    wiparse ui test-tool run --id scope-serial-monitor
    wiparse ui test-tool run --id scope-serial-monitor --preflight true

  Starts the plugin via test-tools/runner.mjs with --project so paths match Hub.
  Test case dir is sanitize(file_prefix), not plugin id.
#>
param(
  [string]$DataRoot = "",
  [string]$Project = "",
  [string]$FilePrefix = "",
  [string]$TestId = "",
  [string]$NodeExe = "",
  [switch]$Force,
  [switch]$Hud,
  [switch]$Preflight
)

$ErrorActionPreference = "Stop"
$pluginDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolsDir = (Resolve-Path (Join-Path $pluginDir "..\..")).Path
$repoRoot = (Resolve-Path (Join-Path $toolsDir "..")).Path
$runner = Join-Path $toolsDir "runner.mjs"
$pluginId = "scope-serial-monitor"

if (-not (Test-Path -LiteralPath $runner)) {
  throw "runner.mjs not found: $runner"
}

function Find-Node {
  param([string]$Hint)
  if ($Hint -and (Test-Path -LiteralPath $Hint)) { return $Hint }
  if ($env:NODE_EXE -and (Test-Path -LiteralPath $env:NODE_EXE)) { return $env:NODE_EXE }
  foreach ($p in @("C:\Program Files\nodejs\node.exe", "D:\SOFTWARE\NodeJS\node.exe")) {
    if (Test-Path -LiteralPath $p) { return $p }
  }
  $cmd = Get-Command node -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  throw "Node.exe not found. Set NODE_EXE or install Node.js."
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

$node = Find-Node -Hint $NodeExe
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

$existing = @(Get-LoopProcesses)
if ($existing.Count -gt 0 -and -not $Force) {
  $ids = ($existing | ForEach-Object { $_.ProcessId }) -join ","
  Write-Host ("Already running PID={0}. Use stop.ps1 or -Force." -f $ids)
  exit 0
}

if ($Force -and $existing.Count -gt 0) {
  Write-Host "Force restart..."
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pluginDir "stop.ps1") -DataRoot $DataRoot -Project $Project -FilePrefix $FilePrefix -TestId $TestId
  Start-Sleep -Seconds 1
}

$life = if ($Preflight) { "preflight" } else { "run" }
$nodeArgs = @(
  $runner,
  "--plugin", $pluginId,
  "--lifecycle", $life,
  "--data-root", $DataRoot,
  "--project", $Project,
  "--test-id", $TestId,
  "--",
  "--file_prefix", $FilePrefix
)

if ($Hud -and -not $Preflight) {
  $hud = Join-Path $pluginDir "hud.ps1"
  if (Test-Path -LiteralPath $hud) {
    $statusGuess = Join-Path $testDir "status.json"
    Start-Process -FilePath "powershell.exe" -WindowStyle Hidden -ArgumentList @(
      "-NoProfile", "-STA", "-ExecutionPolicy", "Bypass",
      "-File", $hud, "-StatusFile", $statusGuess
    ) | Out-Null
  }
}

Write-Host ("Starting {0} via runner ({1}) project={2} test_id={3}..." -f $pluginId, $life, $Project, $TestId)
if ($Preflight) {
  & $node @nodeArgs
  exit $LASTEXITCODE
}

New-Item -ItemType Directory -Force -Path $testDir | Out-Null
$logDir = Join-Path $testDir "helper_logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
$log = Join-Path $logDir ("loop_" + $stamp + ".log")
$errLog = $log + ".err"
$p = Start-Process -FilePath $node -ArgumentList $nodeArgs `
  -WorkingDirectory $toolsDir `
  -RedirectStandardOutput $log -RedirectStandardError $errLog `
  -PassThru -WindowStyle Hidden
Write-Host ("Started PID={0}" -f $p.Id)
Write-Host ("Log: {0}" -f $log)
Write-Host ("HUD/lock: {0}" -f $testDir)
Write-Host "Prefer: wiparse ui test-tool run --id scope-serial-monitor"
