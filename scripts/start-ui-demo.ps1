# Open the packaged UI with isolated simulated-device settings.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
if (-not (Test-Path -LiteralPath (Join-Path $here 'bin/WiParse.exe'))) {
    throw 'Run the copy of start-ui-demo.ps1 in the extracted release folder.'
}
$previewRoot = Join-Path $here 'ui-demo'
New-Item -ItemType Directory -Force -Path $previewRoot | Out-Null
$previewConfig = @{
    ui = @{ language = 'zh'; theme = 'dark'; debug_mode = $true }
    apps = @{
        project = @{ active_id = 'default' }
        test_tool = @{
            plugins_dir = (Join-Path $here 'test-tools/plugins')
            cli_path = (Join-Path $here 'bin/WiParse-CLI.exe')
            data_root = $here
            marketplace = @{ enabled = $false }
        }
    }
}
$configPath = Join-Path $previewRoot 'config.json'
[IO.File]::WriteAllText($configPath, ($previewConfig | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$previewPort = $listener.LocalEndpoint.Port
$listener.Stop()
$env:WIPARSE_API_BIND = "127.0.0.1:$previewPort"
$env:WIPARSE_APP_ROOT = $previewRoot
$env:WIPARSE_CONFIG = $configPath
$env:WCM_CONFIG = $configPath
$env:WIPARSE_PROJECT_ROOT = $here
Start-Process -FilePath (Join-Path $here 'bin/WiParse.exe') -WorkingDirectory $here
$base = "http://127.0.0.1:$previewPort"
$limit = [DateTime]::UtcNow.AddSeconds(15)
$genericRequested = $false
do {
    try {
        $body = @{ method='ui.show'; params=@{tab='instruments'} } | ConvertTo-Json -Depth 4
        $null = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 1
        $body = @{ method='instrument.overview'; params=@{} } | ConvertTo-Json -Depth 4
        $inventory = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 1
        if (@($inventory.data.devices).Count -ge 7 -and -not $genericRequested) {
            $body = @{ method='instrument.connect'; params=@{resource='DEMO::Generic';kind='generic'} } | ConvertTo-Json -Depth 4
            $reply = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 1
            $genericRequested = $reply.ok
        }
        if (@($inventory.data.devices).Count -ge 8) {
            $body = @{ method='ui.instrument.select'; params=@{kind='dc_source'} } | ConvertTo-Json -Depth 4
            $null = Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 1
            break
        }
    } catch { }
    Start-Sleep -Milliseconds 150
} while ([DateTime]::UtcNow -lt $limit)
