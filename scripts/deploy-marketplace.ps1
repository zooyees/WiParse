# Deploy local Testing Hub marketplace server with seeded sample plugins.
#
#   .\scripts\deploy-marketplace.ps1
#   .\scripts\deploy-marketplace.ps1 -Foreground
#   $env:PORT=8787; $env:TOKEN='dev-token'; .\scripts\deploy-marketplace.ps1

param(
    [switch]$Foreground,
    [switch]$Clean,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

if ($Help) {
    Write-Host "Usage: .\scripts\deploy-marketplace.ps1 [-Foreground] [-Clean]"
    exit 0
}

# Repo or demo package root (parent of scripts/).
$Root = Split-Path -Parent $PSScriptRoot
$Port = if ($env:PORT) { [int]$env:PORT } else { 8787 }
$HostName = if ($env:HOST) { $env:HOST } else { "127.0.0.1" }
$Token = if ($env:TOKEN) { $env:TOKEN } else { "dev-token" }
$DataDir = if ($env:DATA_DIR) { $env:DATA_DIR } else { Join-Path $Root "services\testing-hub-marketplace\data" }
$PidFile = if ($env:PID_FILE) { $env:PID_FILE } else { Join-Path $DataDir "server.pid" }
$LogFile = if ($env:LOG_FILE) { $env:LOG_FILE } else { Join-Path $DataDir "server.log" }
$ErrFile = "$LogFile.err"
$SeedScript = Join-Path $Root "scripts\seed-marketplace-plugins.mjs"
$ServerJs = Join-Path $Root "services\testing-hub-marketplace\src\index.mjs"

if (-not (Test-Path $SeedScript)) { throw "seed script not found: $SeedScript" }
if (-not (Test-Path $ServerJs)) { throw "marketplace server not found: $ServerJs" }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { throw "node is required on PATH" }

Set-Location $Root
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

Write-Host "[deploy] seeding plugins -> $DataDir"
if ($Clean) { $env:CLEAN = "1" }
& node $SeedScript --data $DataDir
Remove-Item Env:CLEAN -ErrorAction SilentlyContinue
if ($LASTEXITCODE -ne 0) { throw "seed failed" }

if (Test-Path $PidFile) {
    $old = (Get-Content $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($old -and $old.Trim() -match '^\d+$') {
        $procOld = Get-Process -Id ([int]$old.Trim()) -ErrorAction SilentlyContinue
        if ($procOld) {
            Write-Host "[deploy] stopping previous server pid=$($old.Trim())"
            Stop-Process -Id ([int]$old.Trim()) -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 400
        }
    }
    Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
}

$env:MARKETPLACE_PUBLISH_TOKENS = $Token
$env:MARKETPLACE_DATA_DIR = $DataDir
$env:PORT = "$Port"
$env:HOST = $HostName

$nodeArgs = @(
    $ServerJs,
    "--port", "$Port",
    "--host", $HostName,
    "--data", $DataDir
)

if ($Foreground) {
    Write-Host "[deploy] listening http://${HostName}:${Port} (token=$Token) foreground"
    & node @nodeArgs
    exit $LASTEXITCODE
}

Write-Host "[deploy] starting http://${HostName}:${Port}"
if (Test-Path $LogFile) { Remove-Item $LogFile -Force -ErrorAction SilentlyContinue }
if (Test-Path $ErrFile) { Remove-Item $ErrFile -Force -ErrorAction SilentlyContinue }

$proc = Start-Process -FilePath "node" -ArgumentList $nodeArgs -WorkingDirectory $Root `
    -RedirectStandardOutput $LogFile -RedirectStandardError $ErrFile `
    -WindowStyle Hidden -PassThru
Set-Content -Path $PidFile -Value $proc.Id -Encoding ascii
Start-Sleep -Milliseconds 800

if ($proc.HasExited) {
    Write-Host "[deploy] server failed to start; see $LogFile / $ErrFile"
    if (Test-Path $LogFile) { Get-Content $LogFile }
    if (Test-Path $ErrFile) { Get-Content $ErrFile }
    exit 1
}

$healthUrl = "http://${HostName}:${Port}/v1/health"
$catalogUrl = "http://${HostName}:${Port}/v1/catalog?channel=stable"
try {
    $health = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 5
    Write-Host "[deploy] health $($health | ConvertTo-Json -Compress)"
    $catalog = Invoke-RestMethod -Uri $catalogUrl -TimeoutSec 5
    $ids = @($catalog.plugins | ForEach-Object { $_.id })
    Write-Host "[deploy] catalog plugins=$($ids -join ', ')"
} catch {
    Write-Host "[deploy] warning: health check failed: $_"
    if (Test-Path $LogFile) { Get-Content $LogFile -Tail 40 }
    if (Test-Path $ErrFile) { Get-Content $ErrFile -Tail 40 }
}

Write-Host "[deploy] server pid=$($proc.Id) url=http://${HostName}:${Port} log=$LogFile token=$Token"
Write-Host "[deploy] In WiParse: Testing Hub -> Market -> Enable -> Refresh"
