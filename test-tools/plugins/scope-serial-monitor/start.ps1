<#
.SYNOPSIS
  Legacy/CLI helper. Prefer Testing Hub → Run.

  Starts scope-serial-monitor via test-tools/runner.mjs so path templates
  ({data_root}/{product}/{plugin_dir}) match the GUI path.
#>
param(
  [string]$DataRoot = "",
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

$node = Find-Node -Hint $NodeExe
if (-not $DataRoot) {
  if ($env:WIPARSE_DATA_ROOT) { $DataRoot = $env:WIPARSE_DATA_ROOT }
  else { $DataRoot = $repoRoot }
}

$existing = @(Get-LoopProcesses)
if ($existing.Count -gt 0 -and -not $Force) {
  $ids = ($existing | ForEach-Object { $_.ProcessId }) -join ","
  Write-Host ("Already running PID={0}. Use stop.ps1 or -Force." -f $ids)
  exit 0
}

if ($Force -and $existing.Count -gt 0) {
  Write-Host "Force restart..."
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $pluginDir "stop.ps1")
  Start-Sleep -Seconds 1
}

$life = if ($Preflight) { "preflight" } else { "run" }
$nodeArgs = @(
  $runner,
  "--plugin", $pluginId,
  "--lifecycle", $life,
  "--data-root", $DataRoot
)

if ($Hud -and -not $Preflight) {
  $hud = Join-Path $pluginDir "hud.ps1"
  if (Test-Path -LiteralPath $hud) {
    # Status path is expanded by the plugin; HUD needs a concrete file.
    # Prefer default under data_root after a short wait once the loop writes it.
    $statusGuess = Join-Path $DataRoot "instrument_data\ScopeSerial\_loop_status.json"
    Start-Process -FilePath "powershell.exe" -WindowStyle Hidden -ArgumentList @(
      "-NoProfile", "-STA", "-ExecutionPolicy", "Bypass",
      "-File", $hud, "-StatusFile", $statusGuess
    ) | Out-Null
  }
}

Write-Host ("Starting {0} via runner ({1})..." -f $pluginId, $life)
if ($Preflight) {
  & $node @nodeArgs
  exit $LASTEXITCODE
}

$logDir = Join-Path $pluginDir "logs"
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
Write-Host "Prefer Testing Hub for params + in-panel HUD. Use -Hud only if you need the floating overlay."
