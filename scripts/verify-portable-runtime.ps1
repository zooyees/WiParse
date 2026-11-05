param([Parameter(Mandatory=$true)][string]$PackageRoot)
$ErrorActionPreference='Stop'
$root=(Resolve-Path -LiteralPath $PackageRoot).Path
$repo=Split-Path -Parent $PSScriptRoot
$audit=Join-Path $repo ('tmp/portable-runtime-check-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $audit | Out-Null
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start();$port=$listener.LocalEndpoint.Port;$listener.Stop()
foreach($name in @('WIPARSE_APP_ROOT','WIPARSE_PROJECT_ROOT','WCM_PROJECT_ROOT','WCM_CONFIG')) {
    Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
}
$env:WIPARSE_API_BIND="127.0.0.1:$port"
$env:WIPARSE_CONFIG=Join-Path $audit 'config.json'
Copy-Item -LiteralPath (Join-Path $root 'config.json') -Destination $env:WIPARSE_CONFIG
$env:TEMP=$audit;$env:TMP=$audit
# Deliberately exclude system Node/npm and all development toolchains.
$env:PATH=Join-Path $env:SystemRoot 'System32'
$gui=Start-Process -FilePath (Join-Path $root 'bin/WiParse.exe') -WorkingDirectory $audit -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $audit 'gui.stdout.log') -RedirectStandardError (Join-Path $audit 'gui.stderr.log')
try {
    $base="http://127.0.0.1:$port";$health=$null;$deadline=[DateTime]::UtcNow.AddSeconds(25)
    while(-not $health -and [DateTime]::UtcNow -lt $deadline) {
        if($gui.HasExited){throw "GUI exited: $($gui.ExitCode)"}
        try {$health=Invoke-RestMethod "$base/v1/health" -TimeoutSec 1} catch {Start-Sleep -Milliseconds 100}
    }
    if(-not $health.ok -or $health.data.version -ne '1.2.2'){throw 'GUI health/version failed'}
    foreach($tab in @('serial','test_tool','test_report')) {
        $reply=Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body (@{method='ui.show';params=@{tab=$tab}}|ConvertTo-Json -Depth 4)
        if(-not $reply.ok){throw "UI page failed: $tab"}
    }
    $cli=& (Join-Path $root 'bin/WiParse-CLI.exe') --url $base api health
    if($LASTEXITCODE -ne 0 -or -not (($cli -join "`n")|ConvertFrom-Json).ok){throw 'Bundled CLI could not reach GUI'}
    & (Join-Path $root 'bin/WiParse-Service.exe') --help *> (Join-Path $audit 'service-help.log')
    if($LASTEXITCODE -ne 0){throw 'Bundled Service could not start'}
    $node=Join-Path $root 'runtime/node/node.exe'
    & $node (Join-Path $root 'test-tools/runner.mjs') --list *> (Join-Path $audit 'plugins.log')
    if($LASTEXITCODE -ne 0){throw 'Bundled Node plugin discovery failed'}
    $ps=Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    & $ps -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'mcp/wiparse/setup-mcp.ps1') -VerifyOnly -SkipNpm -Url $base *> (Join-Path $audit 'mcp-setup.log')
    if($LASTEXITCODE -ne 0){throw 'Offline MCP setup failed'}
    $registration=Get-Content -LiteralPath (Join-Path $root 'mcp/wiparse/cursor.mcp.generated.json') -Raw -Encoding UTF8|ConvertFrom-Json
    if($registration.mcpServers.wiparse.command -ne $node.Replace('\','/')){throw 'MCP did not use the relocated bundled Node'}
    if($registration.mcpServers.wiparse.args[0] -ne (Join-Path $root 'mcp/wiparse/dist/index.js').Replace('\','/')){throw 'MCP points to the wrong machine/path'}
    if((Get-Content -LiteralPath (Join-Path $audit 'gui.stderr.log') -Raw -ErrorAction SilentlyContinue) -match 'Configuration load failed'){throw 'Portable configuration rejected'}
    @{ok=$true;root=$root;version=$health.data.version;gui='passed';cli='passed';service='passed';plugin_discovery='passed';mcp_offline_setup='passed';system_node_available=$false;user_cursor_modified=$false;audit=$audit}|ConvertTo-Json
} finally {
    if(-not $gui.HasExited){Stop-Process -Id $gui.Id -Force}
}
