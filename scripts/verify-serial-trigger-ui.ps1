param([Parameter(Mandatory=$true)][string]$PreviewRoot)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
$preview=(Resolve-Path -LiteralPath $PreviewRoot).Path
$exePath=Join-Path $preview 'bin/WiParse.exe'
$auditRoot=Join-Path $repoRoot ('tmp/serial-trigger-runtime-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $auditRoot | Out-Null
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start()
$apiPort=$listener.LocalEndpoint.Port
$listener.Stop()
$env:WIPARSE_PROJECT_ROOT=$preview
$env:WIPARSE_APP_ROOT=$preview
$env:WIPARSE_CONFIG=Join-Path $auditRoot 'config.json'
$env:WIPARSE_API_BIND="127.0.0.1:$apiPort"
$config=@{ui=@{language='zh';debug_mode=$false};apps=@{test_tool=@{
    plugins_dir=(Join-Path $preview 'test-tools/plugins');data_root=$auditRoot;
    marketplace=@{enabled=$false};node_path='node';cli_path=(Join-Path $preview 'bin/WiParse-CLI.exe')
}}}
[IO.File]::WriteAllText($env:WIPARSE_CONFIG,($config|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
$base="http://127.0.0.1:$apiPort"
function Invoke-Preview($method,$parameters) {
    $body=@{method=$method;params=$parameters}|ConvertTo-Json -Depth 12
    try {
        Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 10
    } catch {
        # Expected validation failures use a non-2xx HTTP status with a JSON
        # error envelope. Preserve that envelope for the assertions below.
        if($_.ErrorDetails.Message){return ($_.ErrorDetails.Message|ConvertFrom-Json)}
        throw
    }
}
# All run requests below are invalid drafts. They must fail before starting a
# runner or opening any hardware. Pointer editing is covered by egui tests.
$testProcess=Start-Process -FilePath $exePath -WorkingDirectory $auditRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $auditRoot 'stdout.log') -RedirectStandardError (Join-Path $auditRoot 'stderr.log')
try {
    $limit=[DateTime]::UtcNow.AddSeconds(20)
    $health=$null
    do {
        try {$health=Invoke-RestMethod "$base/v1/health" -TimeoutSec 1} catch {Start-Sleep -Milliseconds 100}
        if($testProcess.HasExited){throw 'Preview exited before API readiness'}
    } while(-not $health -and [DateTime]::UtcNow -lt $limit)
    if(-not $health.ok){throw 'API unavailable'}
    $cases=@(
        @{name='empty';rules=@();expected='Add and enable'},
        @{name='blank_pattern';rules=@(@{id='T1';type='contains';pattern=''});expected='enter match text'},
        @{name='duplicate_slug';rules=@(@{id='a b';type='contains';pattern='ERROR'},@{id='a?b';type='contains';pattern='timeout'});expected='duplicate IDs'},
        @{name='malformed';rules='not JSON';expected='Cannot read triggers'}
    )
    $checks=@()
    foreach($case in $cases) {
        $reply=Invoke-Preview 'ui.test_tool.run' @{id='scope-serial-monitor';preflight=$true;params=@{triggers=$case.rules}}
        if($reply.ok -or $reply.error.message -notlike "*$($case.expected)*") {
            throw "Draft gate $($case.name) failed: $($reply|ConvertTo-Json -Depth 8 -Compress)"
        }
        $checks+=@{name=$case.name;passed=$true;error=$reply.error.message}
    }
    $snapshot=Invoke-Preview 'ui.state' @{}
    if(-not $snapshot.ok){throw "Snapshot unavailable: $($snapshot.error)"}
    if($snapshot.data.test_tool.running){throw 'An invalid draft started a runner'}
    $plugin=@($snapshot.data.test_tool.plugins | Where-Object id -eq 'scope-serial-monitor')
    $expectedPluginVersion=(Get-Content -LiteralPath (Join-Path $preview 'test-tools/plugins/scope-serial-monitor/plugin.json') -Raw -Encoding UTF8|ConvertFrom-Json).version
    if($plugin.Count -ne 1 -or $plugin[0].version -ne $expectedPluginVersion){throw 'Updated plugin not loaded'}
    $result=@{kind='serial_trigger_exe_smoke';passed=$true;version=$health.data.version;
        exe_sha256=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant();
        plugin_version=$plugin[0].version;invalid_draft_gates=$checks;runner_started=$false;hardware_test=$false}
    $result|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $auditRoot 'result.json') -Encoding UTF8
    Write-Output "PASS: updated plugin loaded; four invalid drafts blocked before runner start. Evidence: $auditRoot"
} finally {
    if(-not $testProcess.HasExited){Stop-Process -Id $testProcess.Id -ErrorAction SilentlyContinue}
}
