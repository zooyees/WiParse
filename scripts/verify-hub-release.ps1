param([Parameter(Mandatory=$true)][string]$Exe, [string]$AssetsRoot, [string]$ExpectedVersion='1.2.2', [switch]$AutoRoot, [string]$DemoRoot, [string]$CliPath)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
if (-not $AssetsRoot) { $AssetsRoot=$repoRoot }
$AssetsRoot=(Resolve-Path -LiteralPath $AssetsRoot).Path
$exePath=(Resolve-Path -LiteralPath $Exe).Path
$relativeRoot='tmp/hub-runtime-'+[Guid]::NewGuid().ToString('N')
$auditRoot=Join-Path $AssetsRoot $relativeRoot
New-Item -ItemType Directory -Force -Path $auditRoot | Out-Null
$demoSource=if($DemoRoot){Join-Path $DemoRoot 'recipes/fpga-firmware-test'}else{Join-Path $AssetsRoot 'recipes/fpga-firmware-test'}
if (-not (Test-Path -LiteralPath $demoSource)) { $demoSource=Join-Path $repoRoot 'tmp/fpga-release-demo-final3/recipes/fpga-firmware-test' }
New-Item -ItemType Directory -Force -Path (Join-Path $auditRoot 'recipes') | Out-Null
Copy-Item -LiteralPath $demoSource -Destination (Join-Path $auditRoot 'recipes/fpga-firmware-test') -Recurse
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start(); $port=$listener.LocalEndpoint.Port; $listener.Stop()
if($AutoRoot){
    Remove-Item Env:WIPARSE_PROJECT_ROOT,Env:WIPARSE_APP_ROOT,Env:WCM_PROJECT_ROOT -ErrorAction SilentlyContinue
}else{
    $env:WIPARSE_PROJECT_ROOT=$AssetsRoot
    $env:WIPARSE_APP_ROOT=$auditRoot
}
$env:WIPARSE_CONFIG=Join-Path $auditRoot 'config.json'
$env:WIPARSE_API_BIND="127.0.0.1:$port"
$config=Get-Content -LiteralPath (Join-Path $AssetsRoot 'config.default.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$config.apps.test_tool.plugins_dir='test-tools/plugins'
$config.apps.test_tool.cli_path=if($CliPath){$CliPath}elseif(Test-Path -LiteralPath (Join-Path $AssetsRoot 'bin/WiParse-CLI.exe')){'bin/WiParse-CLI.exe'}else{'target/release/wiparse.exe'}
$config.apps.test_tool.data_root=$relativeRoot
$config.apps.test_tool.marketplace.base_url='http://127.0.0.1:9'
[IO.File]::WriteAllText($env:WIPARSE_CONFIG,($config|ConvertTo-Json -Depth 40),[Text.UTF8Encoding]::new($false))
$env:WIPARSE_TEST_NODE_DIAGNOSTIC=Join-Path $auditRoot 'runner-stderr.log'
$diagnostic=Join-Path $auditRoot 'capture-stderr.cjs'
[IO.File]::WriteAllText($diagnostic,'const fs=require("node:fs"),write=process.stderr.write.bind(process.stderr);process.stderr.write=(chunk,...args)=>{try{fs.appendFileSync(process.env.WIPARSE_TEST_NODE_DIAGNOSTIC,chunk)}catch{}return write(chunk,...args)};',[Text.UTF8Encoding]::new($false))
$env:NODE_OPTIONS='--require="'+$diagnostic.Replace('\','/')+'"'
$process=Start-Process -FilePath $exePath -WorkingDirectory $env:TEMP -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $auditRoot 'stdout.log') -RedirectStandardError (Join-Path $auditRoot 'stderr.log')
$base="http://127.0.0.1:$port"
function Invoke-Hub($method,$parameters) {
    $body=@{method=$method;params=$parameters}|ConvertTo-Json -Depth 40 -Compress
    for($retry=0;;$retry++) {
        try {
            $reply=Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 70
            break
        }catch{
            $readOnly=$method -in @('ui.test_tool.config.list','ui.test_tool.preflight.state','ui.test_tool.matrix.get')
            if(-not $readOnly -or $retry -ge 9 -or $_.ErrorDetails.Message -notmatch '"code"\s*:\s*"CONFIG.BUSY"'){throw}
            Start-Sleep -Milliseconds 75
        }
    }
    if(-not $reply.ok){throw "$method failed: $($reply|ConvertTo-Json -Depth 10 -Compress)"}
    return $reply.data
}
function Wait-Hub {
    $deadline=[DateTime]::UtcNow.AddSeconds(60)
    do {
        if($process.HasExited){throw 'GUI exited during plugin execution'}
        $state=Invoke-Hub 'ui.state' @{}
        if(-not $state.test_tool.running){return $state.test_tool}
        Start-Sleep -Milliseconds 150
    }while([DateTime]::UtcNow -lt $deadline)
    throw 'Plugin execution timed out'
}
try {
    $deadline=[DateTime]::UtcNow.AddSeconds(25); $health=$null
    while(-not $health -and [DateTime]::UtcNow -lt $deadline){
        if($process.HasExited){throw 'GUI exited before health'}
        try{$health=Invoke-RestMethod "$base/v1/health" -TimeoutSec 1}catch{Start-Sleep -Milliseconds 100}
    }
    if(-not $health.ok -or $health.data.version -ne $ExpectedVersion){throw 'Unexpected GUI version/health'}
    $null=Invoke-Hub 'ui.show' @{tab='test_tool'}
    $items=Invoke-Hub 'ui.test_tool.config.list' @{plugin='fpga-firmware-test';limit=500}
    $expectedCount=@(Get-ChildItem -LiteralPath (Join-Path $demoSource 'cases') -File -Filter '*.json').Count
    if(@($items.items).Count -ne $expectedCount){throw "Expected $expectedCount configurations: $($items|ConvertTo-Json -Depth 5 -Compress)"}
    $parameters=@{execution_mode='simulation';config_dir='recipes/fpga-firmware-test/cases'}
    $sourceProject=if($DemoRoot){Join-Path $DemoRoot 'projects/default/tests/FpgaFirmwareTest/.hub/fpga-firmware-test/project-config.json'}else{$null}
    if($sourceProject -and (Test-Path -LiteralPath $sourceProject)){
        $projectSource=Get-Content -LiteralPath $sourceProject -Raw -Encoding UTF8|ConvertFrom-Json
        $current=Invoke-Hub 'ui.test_tool.project.get' @{plugin='fpga-firmware-test'}
        $null=Invoke-Hub 'ui.test_tool.project.save' @{plugin='fpga-firmware-test';value=$projectSource.value;expected_revision=$current.revision;expected_sha256=$current.sha256}
    }else{
        $migration=Invoke-Hub 'ui.test_tool.project.migration.preview' @{plugin='fpga-firmware-test'}
        if(-not $migration.ok -or $migration.conflicts.Count -gt 0){throw 'Isolated runtime migration conflicts'}
        $null=Invoke-Hub 'ui.test_tool.project.migration.commit' @{plugin='fpga-firmware-test';plan_sha256=$migration.plan_sha256}
    }

    $ids=@($items.items|Where-Object {$_.enabled -ne $false}|ForEach-Object {$_.id})
    $null=Invoke-Hub 'ui.test_tool.run' @{id='fpga-firmware-test';preflight=$true;params=$parameters;test_matrix=@{selected_item_ids=$ids}}
    $preflightJob=Wait-Hub
    $preflight=Invoke-Hub 'ui.test_tool.preflight.state' @{plugin='fpga-firmware-test'}
    if(-not $preflight.valid){throw "Preflight invalid: $($preflight|ConvertTo-Json -Depth 8 -Compress); job=$($preflightJob|ConvertTo-Json -Depth 5 -Compress)"}
    $null=Invoke-Hub 'ui.test_tool.run' @{id='fpga-firmware-test';preflight=$false;params=$parameters;test_matrix=@{config_manifest_sha256=$preflight.manifest_sha256;preflight_revision=$preflight.revision;selected_item_ids=$ids}}
    $runJob=Wait-Hub
    $matrix=Invoke-Hub 'ui.test_tool.matrix.get' @{plugin='fpga-firmware-test';limit=500}
    if($matrix.summary.total -ne $expectedCount -or $matrix.summary.pass -ne $ids.Count){throw "Unexpected matrix: $($matrix|ConvertTo-Json -Depth 10 -Compress); job=$($runJob|ConvertTo-Json -Depth 5 -Compress)"}
    $runDirectory=Get-ChildItem -LiteralPath (Join-Path $auditRoot 'projects/default/tests/FpgaFirmwareTest/runs') -Directory|Sort-Object Name -Descending|Select-Object -First 1
    $run=Get-Content -LiteralPath (Join-Path $runDirectory.FullName 'run.json') -Raw -Encoding UTF8|ConvertFrom-Json
    if(-not $run.simulation_only){throw 'Simulation marker missing'}
    $opened=@()
    foreach($pattern in @('summary.md','*.png','*.csv','*.log','result.json')){
        $artifact=Get-ChildItem -LiteralPath $runDirectory.FullName -Recurse -File -Filter $pattern|Select-Object -First 1
        if(-not $artifact){throw "Missing artifact: $pattern"}
        $null=Invoke-Hub 'ui.report.open' @{path=$artifact.FullName}
        $opened+=$artifact.Name
    }
    for($retry=0;;$retry++) {
        try {$saved=Get-Content -LiteralPath $env:WIPARSE_CONFIG -Raw -Encoding UTF8|ConvertFrom-Json;break}
        catch{if($retry -ge 9){throw};Start-Sleep -Milliseconds 100}
    }
    if([IO.Path]::IsPathRooted($saved.apps.test_tool.plugins_dir) -or [IO.Path]::IsPathRooted($saved.apps.test_tool.cli_path) -or $saved.apps.test_tool.data_root -ne $relativeRoot){throw 'Relative portable configuration was rewritten'}
    if($process.HasExited){throw 'GUI exited opening evidence'}
    @{ok=$true;version=$health.data.version;configs=$expectedCount;summary=$matrix.summary;opened=$opened;simulation_only=$true;relative_paths_preserved=$true;foreign_working_directory=$true;auto_detected_root=$AutoRoot.IsPresent;exe_sha256=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash;root=$auditRoot}|ConvertTo-Json -Depth 8
}finally{
    if(-not $process.HasExited){Stop-Process -Id $process.Id -Force}
}
