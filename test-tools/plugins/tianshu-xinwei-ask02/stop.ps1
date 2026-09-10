param(
  [string]$Config = "",
  [int]$WaitSeconds = 12
)

$ErrorActionPreference = "Continue"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Config) { $Config = Join-Path $here "station.json" }
$cfg = Get-Content -LiteralPath $Config -Raw -Encoding UTF8 | ConvertFrom-Json
$stopFile = [string]$cfg.paths.stop_file
$legacyStop = "D:\SOFTWARE\WiParse-Rust\mcp\wiparse\_loop_ask02_stress.stop"

New-Item -ItemType File -Force -Path $stopFile | Out-Null
New-Item -ItemType File -Force -Path $legacyStop | Out-Null
Write-Host "Stop file written, waiting..."

function Get-LoopProcesses {
  Get-CimInstance Win32_Process | Where-Object {
    $cl = [string]$_.CommandLine
    if (-not $cl) { return $false }
    $prod = ($cl.IndexOf("tianshu-xinwei-ask02") -ge 0) -and ($cl.IndexOf("loop.mjs") -ge 0)
    $legacy = $cl.IndexOf("_loop_ask02_stress.mjs") -ge 0
    return ($prod -or $legacy)
  }
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

try {
  $url = ($cfg.gui.api.TrimEnd("/") + "/v1/invoke")
  Invoke-RestMethod -Method Post -Uri $url -ContentType "application/json" -Body '{"method":"test.abort","params":{"reason":"production stop"}}' | Out-Null
} catch {}

$lock = [string]$cfg.paths.lock_file
if (Test-Path -LiteralPath $lock) { Remove-Item -LiteralPath $lock -Force -ErrorAction SilentlyContinue }
if (Test-Path -LiteralPath $stopFile) { Remove-Item -LiteralPath $stopFile -Force -ErrorAction SilentlyContinue }
if (Test-Path -LiteralPath $legacyStop) { Remove-Item -LiteralPath $legacyStop -Force -ErrorAction SilentlyContinue }

Write-Host "Loop stopped. GUI and scope left as-is."
