param(
  [string]$Config = "",
  [string]$NodeExe = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Config) { $Config = Join-Path $here "station.json" }
$cfg = Get-Content -LiteralPath $Config -Raw -Encoding UTF8 | ConvertFrom-Json

function Find-Node {
  param([string]$Hint)
  if ($Hint -and (Test-Path -LiteralPath $Hint)) { return $Hint }
  if ($env:NODE_EXE -and (Test-Path -LiteralPath $env:NODE_EXE)) { return $env:NODE_EXE }
  foreach ($p in @("D:\SOFTWARE\NodeJS\node.exe", "C:\Program Files\nodejs\node.exe")) {
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
    $prod = ($cl.IndexOf("tianshu-xinwei-ask02") -ge 0) -and ($cl.IndexOf("loop.mjs") -ge 0)
    $legacy = $cl.IndexOf("_loop_ask02_stress.mjs") -ge 0
    return ($prod -or $legacy)
  }
}

$node = Find-Node -Hint $NodeExe
$loop = Join-Path $here "loop.mjs"
$hud = Join-Path $here "hud.ps1"
if (-not (Test-Path -LiteralPath $hud)) {
  $hud = Join-Path $here "..\..\mcp\wiparse\_loop_ask02_hud.ps1"
}
$statusFile = [string]$cfg.paths.status_file

function Start-Hud {
  Start-Process -FilePath "powershell.exe" -ArgumentList @(
    "-NoProfile", "-STA", "-ExecutionPolicy", "Bypass",
    "-File", $hud, "-StatusFile", $statusFile
  ) | Out-Null
}

$existing = @(Get-LoopProcesses)
if ($existing.Count -gt 0 -and -not $Force) {
  $ids = ($existing | ForEach-Object { $_.ProcessId }) -join ","
  Write-Host ("Already running PID={0}. Did not start a second copy." -f $ids)
  Write-Host "Status: wait / cycle in _loop_status.json"
  Write-Host "Restart: stop.ps1 then start.ps1   (or start.ps1 -Force)"
  Start-Hud
  exit 0
}

if ($Force -and $existing.Count -gt 0) {
  Write-Host "Force restart..."
  $stopScript = Join-Path $here "stop.ps1"
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $stopScript
  Start-Sleep -Seconds 1
}

Write-Host "Preflight..."
& $node $loop "--config=$Config" --preflight
if ($LASTEXITCODE -ne 0) { throw "Preflight failed." }

$stopFile = [string]$cfg.paths.stop_file
if (Test-Path -LiteralPath $stopFile) { Remove-Item -LiteralPath $stopFile -Force }

Start-Hud

$logDir = Join-Path $here "logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
$log = Join-Path $logDir ("loop_" + $stamp + ".log")
$repo = (Resolve-Path (Join-Path $here "..\..")).Path
$errLog = $log + ".err"
$cfgArg = "--config=" + $Config
$p = Start-Process -FilePath $node -ArgumentList @($loop, $cfgArg) -WorkingDirectory $repo -RedirectStandardOutput $log -RedirectStandardError $errLog -PassThru -WindowStyle Hidden
Write-Host ("Started PID={0}  {1} Rev {2}" -f $p.Id, $cfg.sop.id, $cfg.sop.rev)
Write-Host ("HUD: {0}" -f $statusFile)
Write-Host ("Log: {0}" -f $log)
Write-Host "HUD color: green=wait  orange=capture  blue=done  red=stopped"
