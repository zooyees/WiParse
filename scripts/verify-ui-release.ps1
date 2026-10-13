param([Parameter(Mandatory=$true)][string]$Exe)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$exePath = (Resolve-Path -LiteralPath $Exe).Path
$auditRoot = Join-Path $repoRoot ('tmp/ui-release-smoke-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $auditRoot | Out-Null
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$apiPort = $listener.LocalEndpoint.Port
$listener.Stop()
$env:WIPARSE_APP_ROOT = $auditRoot
$env:WIPARSE_CONFIG = Join-Path $auditRoot 'config.json'
$env:WIPARSE_API_BIND = "127.0.0.1:$apiPort"
$config = Get-Content -LiteralPath (Join-Path $repoRoot 'config.default.json') -Raw | ConvertFrom-Json
$config.apps.test_tool.plugins_dir = Join-Path $repoRoot 'test-tools/plugins'
$config.apps.test_tool.data_root = $auditRoot
$config.apps.test_tool.marketplace.base_url = 'http://127.0.0.1:9'
$config.ui.debug_mode = $false
$config | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $env:WIPARSE_CONFIG -Encoding utf8
$testProcess = Start-Process -FilePath $exePath -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $auditRoot 'stdout.log') -RedirectStandardError (Join-Path $auditRoot 'stderr.log')
try {
    $base = "http://127.0.0.1:$apiPort"
    $limit = [DateTime]::UtcNow.AddSeconds(20)
    $health = $null
    while (-not $health -and [DateTime]::UtcNow -lt $limit) {
        if ($testProcess.HasExited) { throw "GUI exited early: $($testProcess.ExitCode)" }
        try { $health = Invoke-RestMethod "$base/v1/health" -TimeoutSec 1 } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $health -or -not $health.ok) { throw 'GUI health unavailable' }
    if ($health.data.version -ne '1.1.30') { throw 'Unexpected GUI API version' }
    $pages = @()
    foreach ($page in @('serial','calculator','instruments','waveform','data_analysis','test_report','test_tool')) {
        $body = @{ method = 'ui.show'; params = @{ tab = $page } } | ConvertTo-Json -Depth 4
        $reply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 10
        if (-not $reply.ok) { throw "Page switch failed: $page $($reply.error)" }
        Start-Sleep -Milliseconds 150
        if ($testProcess.HasExited) { throw "GUI exited rendering $page" }
        $pages += $page
    }
    if ($testProcess.HasExited) { throw 'GUI exited after page switches' }
    $demoRelative = 'projects/default/tests/ScopeSerialDemo/runs/20261013_231510'
    $demoDestination = Join-Path $auditRoot $demoRelative
    New-Item -ItemType Directory -Force -Path (Split-Path $demoDestination) | Out-Null
    Copy-Item -LiteralPath (Join-Path $repoRoot $demoRelative) -Destination $demoDestination -Recurse
    $artifacts = @()
    foreach ($file in @(
        'artifacts/reports/ScopeSerialDemo_summary_20261013_231510.md',
        'artifacts/docs/capture-records.json',
        'artifacts/docs/serial-monitor.log',
        'artifacts/shots/ScopeSerialDemo_PING_START_20261013_231510.png',
        'artifacts/waves/ScopeSerialDemo_PING_START_20261013_231510.isf'
    )) {
        $artifactPath = Join-Path $demoDestination $file
        $body = @{method='ui.report.open'; params=@{path=$artifactPath}} | ConvertTo-Json -Depth 5
        $reply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 20
        if (-not $reply.ok) { throw "Report artifact open failed: $file $($reply.error)" }
        if ($testProcess.HasExited) { throw 'GUI exited opening report artifact' }
        $artifacts += $file
    }
    @{ ok = $true; version = $health.data.version; product_version = (Get-Item -LiteralPath $exePath).VersionInfo.ProductVersion; pages = $pages; report_artifacts = $artifacts; exe = $exePath; root = $auditRoot } | ConvertTo-Json -Depth 4
} finally {
    if (-not $testProcess.HasExited) { Stop-Process -Id $testProcess.Id -Force }
}
