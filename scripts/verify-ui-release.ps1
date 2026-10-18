param([Parameter(Mandatory=$true)][string]$Exe, [string]$ExpectedVersion = '1.1.34', [switch]$InstrumentDemo)
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
$config = Get-Content -LiteralPath (Join-Path $repoRoot 'config.default.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$config.apps.test_tool.plugins_dir = Join-Path $repoRoot 'test-tools/plugins'
$config.apps.test_tool.data_root = $auditRoot
$config.apps.test_tool.marketplace.base_url = 'http://127.0.0.1:9'
$config.ui.debug_mode = $InstrumentDemo.IsPresent
[IO.File]::WriteAllText($env:WIPARSE_CONFIG, ($config | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
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
    if ($health.data.version -ne $ExpectedVersion) { throw 'Unexpected GUI API version' }
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

    $overviewBody = @{method='ui.instrument.select';params=@{overview=$true}} | ConvertTo-Json -Depth 4
    $overviewReply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $overviewBody -TimeoutSec 10
    if (-not $overviewReply.ok) { throw 'Device overview unavailable in this startup mode' }
    $demoDevices = @()
    if ($InstrumentDemo) {
        $body = @{method='ui.show';params=@{tab='instruments'}} | ConvertTo-Json -Depth 4
        $null = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 10
        $body = @{method='instrument.connect';params=@{resource='DEMO::Generic';kind='generic'}} | ConvertTo-Json -Depth 4
        $genericReply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 10
        if (-not $genericReply.ok) { throw 'Generic simulated session could not be created' }
        $demoLimit = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $inventoryBody = @{method='instrument.overview';params=@{}} | ConvertTo-Json -Depth 4
            $inventory = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $inventoryBody -TimeoutSec 10
            if (@($inventory.data.devices).Count -ge 8) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $demoLimit)
        if (@($inventory.data.devices).Count -lt 8) { throw 'Simulated devices did not initialize' }
        foreach ($device in $inventory.data.devices) {
            $body = @{method='ui.instrument.select';params=@{device_id=$device.device_id}} | ConvertTo-Json -Depth 4
            $reply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 10
            if (-not $reply.ok) { throw "Device page failed: $($device.kind)" }
            Start-Sleep -Milliseconds 200
            if ($testProcess.HasExited) { throw 'GUI exited rendering a simulated instrument' }
            $demoDevices += $device.kind
        }
    }

    $demoRelative = 'projects/default/tests/ScopeSerialDemo/runs/20261016_120000'
    $demoDestination = Join-Path $auditRoot $demoRelative
    New-Item -ItemType Directory -Force -Path (Split-Path $demoDestination) | Out-Null
    Copy-Item -LiteralPath (Join-Path $repoRoot $demoRelative) -Destination $demoDestination -Recurse
    $artifacts = @()
    foreach ($file in @(
        'artifacts/reports/ScopeSerialDemo_summary_20261016_120000.md',
        'artifacts/docs/capture-records.json',
        'artifacts/docs/serial-monitor.log',
        'artifacts/shots/ScopeSerialDemo_PING_START_20261016_120000.png',
        'artifacts/waves/ScopeSerialDemo_PING_START_20261016_120000.isf'
    )) {
        $artifactPath = Join-Path $demoDestination $file
        $body = @{method='ui.report.open'; params=@{path=$artifactPath}} | ConvertTo-Json -Depth 5
        $reply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 20
        if (-not $reply.ok) { throw "Report artifact open failed: $file $($reply.error)" }
        if ($testProcess.HasExited) { throw 'GUI exited opening report artifact' }
        $artifacts += $file
    }
    if ((Get-Content -LiteralPath (Join-Path $auditRoot 'stderr.log') -Raw -ErrorAction SilentlyContinue) -match 'Configuration load failed') { throw 'Startup configuration was rejected' }
    @{ ok = $true; version = $health.data.version; product_version = (Get-Item -LiteralPath $exePath).VersionInfo.ProductVersion; pages = $pages; report_artifacts = $artifacts; simulated_devices = $demoDevices; overview = "passed"; exe = $exePath; root = $auditRoot } | ConvertTo-Json -Depth 4
} finally {
    if (-not $testProcess.HasExited) { Stop-Process -Id $testProcess.Id -Force }
}
