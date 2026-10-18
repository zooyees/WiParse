# Build WiParse (Windows) + marketplace demo bundle for local testing.
#
#   .\scripts\package-marketplace-demo.ps1
# Output:
#   dist\wiparse-win-marketplace-demo\
#   dist\wiparse-win-marketplace-demo.zip
#   dist\WiParse.exe / dist\WiParse-CLI.exe (runtime copy)

param(
    [switch]$SkipBuild,
    [string]$FpgaDemoRoot = "",
    [string]$VerificationDir = "evidence/release-1.2.1-2026-10-18"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Out = if ($env:OUT) { $env:OUT } else { Join-Path $Root "dist\wiparse-win-marketplace-demo" }
$Archive = if ($env:ARCHIVE) { $env:ARCHIVE } else { Join-Path $Root "dist\wiparse-win-marketplace-demo.zip" }
$Token = if ($env:TOKEN) { $env:TOKEN } else { "dev-token" }
$Port = if ($env:PORT) { $env:PORT } else { "8787" }

Set-Location $Root

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, $Text, $utf8)
}

function Copy-Tree([string]$Src, [string]$Dst) {
    New-Item -ItemType Directory -Force -Path $Dst | Out-Null
    $robocopy = Get-Command robocopy -ErrorAction SilentlyContinue
    if ($robocopy) {
        & robocopy $Src $Dst /E /NFL /NDL /NJH /NJS /nc /ns /np `
            /XD logs node_modules .git `
            /XF run.stop run.lock _loop_status.json _loop_hint.txt | Out-Null
        if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE): $Src -> $Dst" }
    } else {
        Copy-Item -Path (Join-Path $Src "*") -Destination $Dst -Recurse -Force
    }
}

if (-not $SkipBuild) {
    Write-Host "[package] cargo release build"
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
    & cargo build --release -p wiparse-gui -p wiparse-cli -p wiparse-service
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$GuiExe = Join-Path $Root "target\release\wiparse-gui.exe"
$CliExe = Join-Path $Root "target\release\wiparse.exe"
$ServiceExe = Join-Path $Root "target\release\wiparse-service.exe"
$GuiPdb = Join-Path $Root "target\release\wiparse_gui.pdb"
if (-not (Test-Path $GuiExe)) { throw "missing $GuiExe" }
if (-not (Test-Path $CliExe)) { throw "missing $CliExe" }
if (-not (Test-Path $ServiceExe)) { throw "missing $ServiceExe" }

# Check resolved targets before deleting a previous package.
$RootPrefix = [IO.Path]::GetFullPath((Join-Path $Root 'dist')).TrimEnd('\') + '\'
$Out = [IO.Path]::GetFullPath($Out)
$Archive = [IO.Path]::GetFullPath($Archive)
if (-not $Out.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Package output must be a child directory below workspace dist' }
if (-not $Archive.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive must stay within workspace' }

if (Test-Path $Out) { Remove-Item $Out -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $Out "bin") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Out "scripts") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Out "marketplace") | Out-Null

Copy-Item -Force $GuiExe (Join-Path $Out "bin\WiParse.exe")
Copy-Item -Force $CliExe (Join-Path $Out "bin\WiParse-CLI.exe")
Copy-Item -Force $ServiceExe (Join-Path $Out "bin\WiParse-Service.exe")
Copy-Tree (Join-Path $Root "docs\specs\wireless-test") (Join-Path $Out "docs\specs\wireless-test")
Copy-Item (Join-Path $Root "docs\AUTOMATION-SERVICE.md") (Join-Path $Out "docs\AUTOMATION-SERVICE.md") -Force
Copy-Item (Join-Path $Root "docs\CHANGELOG.md") (Join-Path $Out "docs\CHANGELOG.md") -Force
if (Test-Path -LiteralPath (Join-Path $Root 'evidence\software-spec-2026-10-11\verification-summary.json')) {
    New-Item -ItemType Directory -Force -Path (Join-Path $Out 'docs\verification') | Out-Null
    Copy-Item -LiteralPath (Join-Path $Root 'evidence\software-spec-2026-10-11\verification-summary.json') -Destination (Join-Path $Out 'docs\verification\verification-summary.json') -Force
}
if (Test-Path -LiteralPath (Join-Path $Root 'docs/ui-layout-audit-2026-10-11.md')) {
    Copy-Item -LiteralPath (Join-Path $Root 'docs/ui-layout-audit-2026-10-11.md') -Destination (Join-Path $Out 'docs/ui-layout-audit-2026-10-11.md') -Force
}
if (Test-Path -LiteralPath (Join-Path $Root 'evidence/ui-layout-2026-10-11/verification-summary.json')) {
    New-Item -ItemType Directory -Force -Path (Join-Path $Out 'docs/verification') | Out-Null
    Copy-Item -LiteralPath (Join-Path $Root 'evidence/ui-layout-2026-10-11/verification-summary.json') -Destination (Join-Path $Out 'docs/verification/ui-layout-verification.json') -Force
}
if (Test-Path -LiteralPath (Join-Path $Root 'evidence/wireless-plugin-2026-10-11/verification-summary.json')) {
    New-Item -ItemType Directory -Force -Path (Join-Path $Out 'docs/verification') | Out-Null
    Copy-Item -LiteralPath (Join-Path $Root 'evidence/wireless-plugin-2026-10-11/verification-summary.json') -Destination (Join-Path $Out 'docs/verification/wireless-plugin-verification.json') -Force
}
New-Item -ItemType Directory -Force -Path (Join-Path $Out 'symbols') | Out-Null
foreach ($symbolName in @('wiparse_gui','wiparse_service','wiparse')) {
    $symbolPath = Join-Path $Root "target\release\$symbolName.pdb"
    if (Test-Path -LiteralPath $symbolPath) { Copy-Item -LiteralPath $symbolPath -Destination (Join-Path $Out "symbols\$symbolName.pdb") -Force }
}

# Also refresh the daily dist/ runtime copy (skip if the running GUI has the file mapped).
New-Item -ItemType Directory -Force -Path (Join-Path $Root "dist") | Out-Null
foreach ($pair in @(
    @{ Src = $GuiExe; Dst = Join-Path $Root "dist\WiParse.exe" },
    @{ Src = $CliExe; Dst = Join-Path $Root "dist\WiParse-CLI.exe" },
    @{ Src = $ServiceExe; Dst = Join-Path $Root "dist\WiParse-Service.exe" }
)) {
    try {
        Copy-Item -Force $pair.Src $pair.Dst
    } catch {
        Write-Host "[package] skip $($pair.Dst) (in use): $($_.Exception.Message)"
    }
}
$DistGuiExe = Join-Path $Root "dist\WiParse.exe"
$RuntimeGuiMatches = (Test-Path -LiteralPath $DistGuiExe) -and
    ((Get-FileHash -LiteralPath $DistGuiExe -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $GuiExe -Algorithm SHA256).Hash)
if ((Test-Path -LiteralPath $GuiPdb) -and $RuntimeGuiMatches) {
    $DistPdb = Join-Path $Root "dist\WiParse.pdb"
    Copy-Item -LiteralPath $GuiPdb -Destination $DistPdb -Force
    $symbolManifest = @{
        exe_sha256 = (Get-FileHash -LiteralPath $GuiExe -Algorithm SHA256).Hash
        pdb_sha256 = (Get-FileHash -LiteralPath $GuiPdb -Algorithm SHA256).Hash
        note = "Keep this PDB with the EXE build identified by exe_sha256. It cannot symbolize a different build."
    } | ConvertTo-Json
    Write-Utf8NoBom (Join-Path $Root "dist\WiParse.symbols.json") $symbolManifest
    Write-Host "[package] saved matching GUI symbols -> $DistPdb"
}
if ($RuntimeGuiMatches) {
    Copy-Tree (Join-Path $Root "test-tools") (Join-Path $Root "dist\test-tools")
} else {
    Write-Host '[package] preserve daily runtime plugins while its GUI executable is in use or differs'
}

Copy-Tree (Join-Path $Root "test-tools") (Join-Path $Out "test-tools")
$HubDestinations=@((Join-Path $Out 'test-tools/node_modules'))
if ($RuntimeGuiMatches) { $HubDestinations += (Join-Path $Root 'dist/test-tools/node_modules') }
foreach ($hubDestination in $HubDestinations) {
    & robocopy (Join-Path $Root 'test-tools/node_modules') $hubDestination /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
    if ($LASTEXITCODE -ge 8) { throw 'Copy Hub offline schema dependencies failed; run npm ci --prefix test-tools' }
}
if (-not (Test-Path -LiteralPath (Join-Path $Root 'mcp/wiparse/dist/index.js'))) { throw 'Build MCP before packaging: npm run build --prefix mcp/wiparse' }
Copy-Tree (Join-Path $Root 'mcp/wiparse') (Join-Path $Out 'mcp/wiparse')
# Generated registrations contain this workstation's paths and are not portable.
Remove-Item -LiteralPath (Join-Path $Out 'mcp/wiparse/cursor.mcp.generated.json') -Force -ErrorAction SilentlyContinue
& robocopy (Join-Path $Root 'mcp/wiparse/node_modules') (Join-Path $Out 'mcp/wiparse/node_modules') /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
if ($LASTEXITCODE -ge 8) { throw 'Copy MCP offline dependencies failed' }
foreach ($document in @('DEPLOY_MCP.md','DEPLOY_API.md','CLI_REFERENCE.md','WORKSTATION_CLOSED_LOOP.md','UPDATE.md','AI_INTERFACE.md','TESTING_HUB_WORKSPACE.md','HUB_UI_FIELDS_CONTRACT.md','FPGA_PLUGIN_UPGRADE_0.4.0.md','FPGA_PLUGIN_INTEGRATION_REVIEW.md')) {
    Copy-Item -LiteralPath (Join-Path $Root "docs/$document") -Destination (Join-Path $Out "docs/$document") -Force
}
New-Item -ItemType Directory -Force -Path (Join-Path $Out 'docs/reviews') | Out-Null
Copy-Item -LiteralPath (Join-Path $Root 'docs/reviews/fpga-testing-hub-integration-result-2026-10-18.md') -Destination (Join-Path $Out 'docs/reviews/fpga-testing-hub-integration-result-2026-10-18.md') -Force
Copy-Item -LiteralPath (Join-Path $Root 'docs/DEPLOY_MCP.md') -Destination (Join-Path $Out 'DEPLOY_MCP.md') -Force
Copy-Item -LiteralPath (Join-Path $Root 'docs/DEPLOY_API.md') -Destination (Join-Path $Out 'DEPLOY_API.md') -Force
Copy-Tree (Join-Path $Root 'docs/releases') (Join-Path $Out 'docs/releases')
Copy-Tree (Join-Path $Root 'docs/ui-testing-hub-1.2.1') (Join-Path $Out 'docs/ui-testing-hub-1.2.1')
Copy-Tree (Join-Path $Root 'docs/ui-testing-hub-1.2.0') (Join-Path $Out 'docs/ui-testing-hub-1.2.0')
Copy-Tree (Join-Path $Root 'docs/ui-ai-2026-10-18') (Join-Path $Out 'docs/ui-ai-2026-10-18')
Copy-Tree (Join-Path $Root 'docs/ui-ai-assistant-2026-10-18') (Join-Path $Out 'docs/ui-ai-assistant-2026-10-18')
Copy-Tree (Join-Path $Root 'docs/ui-release-1.1.36') (Join-Path $Out 'docs/ui-release-1.1.36')
Copy-Item -LiteralPath (Join-Path $Root 'docs/ui-flat-instruments-1.1.33.md') -Destination (Join-Path $Out 'docs/ui-flat-instruments-1.1.33.md') -Force
Copy-Tree (Join-Path $Root 'docs/ui-flat-instruments-1.1.33') (Join-Path $Out 'docs/ui-flat-instruments-1.1.33')
Copy-Item -LiteralPath (Join-Path $Root 'docs/ui-instrument-workspace-1.1.34.md') -Destination (Join-Path $Out 'docs/ui-instrument-workspace-1.1.34.md') -Force
Copy-Tree (Join-Path $Root 'docs/ui-instrument-workspace-1.1.34') (Join-Path $Out 'docs/ui-instrument-workspace-1.1.34')
Copy-Item -LiteralPath (Join-Path $Root 'docs/ui-all-channels-2026-10-17.md') -Destination (Join-Path $Out 'docs/ui-all-channels-2026-10-17.md') -Force
Copy-Tree (Join-Path $Root 'docs/ui-all-channels-2026-10-17') (Join-Path $Out 'docs/ui-all-channels-2026-10-17')
Copy-Item -LiteralPath (Join-Path $Root 'docs/ui-instrument-redesign-2026-10-16.md') -Destination (Join-Path $Out 'docs/ui-instrument-redesign-2026-10-16.md') -Force
Copy-Item -LiteralPath (Join-Path $Root 'docs/serial-realtime-resource-analysis-2026-10-16.md') -Destination (Join-Path $Out 'docs/serial-realtime-resource-analysis-2026-10-16.md') -Force
Copy-Item -LiteralPath (Join-Path $Root 'docs/system-audit-1.1.32-2026-10-16.md') -Destination (Join-Path $Out 'docs/system-audit-1.1.32-2026-10-16.md') -Force
Copy-Tree (Join-Path $Root 'docs/ui-system-audit-1.1.32-2026-10-16') (Join-Path $Out 'docs/ui-system-audit-1.1.32-2026-10-16')
Copy-Tree (Join-Path $Root 'docs/ui-instrument-redesign-2026-10-16') (Join-Path $Out 'docs/ui-instrument-redesign-2026-10-16')
Copy-Item -LiteralPath (Join-Path $Root 'scripts/start-ui-demo.ps1') -Destination (Join-Path $Out 'start-ui-demo.ps1') -Force
Copy-Tree (Join-Path $Root 'docs/examples') (Join-Path $Out 'docs/examples')
Copy-Item -LiteralPath (Join-Path $Root "$VerificationDir/verification-summary.json") -Destination (Join-Path $Out 'docs/verification/release-1.2.1.json') -Force
Copy-Item -LiteralPath (Join-Path $Root "$VerificationDir/source-freeze.json") -Destination (Join-Path $Out 'docs/verification/source-freeze.json') -Force
$SafeVerificationLogs=@('build-release.log','rust-tests.log','node-tests.log','mcp-tests.log','hub-runtime.log','hub-controls.log','hub-lock-retry-tests.log','matrix-provenance-tests.log','fpga-20-pass.log','fpga-20-fail.log','resource-pressure.log','automation-service-smoke.log','wireless-service-smoke.log','fpga-ui-com.log','fpga-ui-dongle.log','rule-ui-pointer.log','gui-smoke.log','build-profile.json')
New-Item -ItemType Directory -Force -Path (Join-Path $Out 'docs/verification/logs') | Out-Null
foreach($log in $SafeVerificationLogs){Copy-Item -LiteralPath (Join-Path $Root "$VerificationDir/$log") -Destination (Join-Path $Out "docs/verification/logs/$log") -Force}

Copy-Item -LiteralPath (Join-Path $Root 'evidence/serial-resource-1.1.32-2026-10-16/summary.json') -Destination (Join-Path $Out 'docs/verification/serial-resource-1.1.32.json') -Force
Copy-Item -LiteralPath (Join-Path $Root 'evidence/serial-resource-2026-10-16/summary.json') -Destination (Join-Path $Out 'docs/verification/serial-resource-1.1.31.json') -Force
Copy-Item -LiteralPath (Join-Path $Root "$VerificationDir/verification-summary.json") -Destination (Join-Path $Out 'docs/verification/wireless-plugin-verification.json') -Force
$DemoRelative = 'projects/default/tests/ScopeSerialDemo/runs/20261016_120000'
Copy-Tree (Join-Path $Root $DemoRelative) (Join-Path $Out $DemoRelative)
$DemoRunFile = Join-Path $Out "$DemoRelative/run.json"
$DemoRun = Get-Content -LiteralPath $DemoRunFile -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($artifact in $DemoRun.artifacts.items) {
    $artifact.path = "artifacts/$($artifact.output)/$([IO.Path]::GetFileName($artifact.path))"
}
$DemoRun.summary_md = 'artifacts/reports/ScopeSerialDemo_summary_20261016_120000.md'
Write-Utf8NoBom $DemoRunFile ($DemoRun | ConvertTo-Json -Depth 20)
if (-not $FpgaDemoRoot) {
    $FpgaDemoRoot=Join-Path $Root ('tmp/fpga-package-demo-'+[Guid]::NewGuid().ToString('N'))
    & node (Join-Path $Root 'scripts/verify-fpga-upgrade.mjs') $FpgaDemoRoot
    if ($LASTEXITCODE -ne 0) { throw 'Generate verified FPGA simulation demo failed' }
}
if (-not (Test-Path -LiteralPath (Join-Path $FpgaDemoRoot 'verification.json'))) { throw 'FPGA demo verification missing' }
$FpgaVerification=Get-Content -LiteralPath (Join-Path $FpgaDemoRoot 'verification.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$FpgaRelative = $FpgaVerification.run
if (-not $FpgaRelative -or [IO.Path]::IsPathRooted($FpgaRelative) -or $FpgaRelative -match '\.\.') { throw 'Invalid verified FPGA run path' }
$FpgaCount=@(Get-ChildItem -LiteralPath (Join-Path $FpgaDemoRoot 'recipes/fpga-firmware-test/cases') -File -Filter '*.json').Count
if ($FpgaCount -ne $FpgaVerification.item_count) { throw 'FPGA verified configuration count differs' }
Copy-Tree (Join-Path $Root 'recipes/fpga-firmware-test') (Join-Path $Out 'recipes/fpga-firmware-test')
# This is a verified historical run, not a runtime root: its logs are report evidence.
& robocopy (Join-Path $FpgaDemoRoot $FpgaRelative) (Join-Path $Out $FpgaRelative) /E /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
if ($LASTEXITCODE -ge 8) { throw 'Copy FPGA report evidence failed' }
Copy-Item -LiteralPath (Join-Path $FpgaDemoRoot 'projects/default/tests/FpgaFirmwareTest/matrix.json') -Destination (Join-Path $Out 'projects/default/tests/FpgaFirmwareTest/matrix.json') -Force
$FpgaRunFile=Join-Path $Out "$FpgaRelative/run.json"
$FpgaRun=Get-Content -LiteralPath $FpgaRunFile -Raw -Encoding UTF8 | ConvertFrom-Json
if (-not $FpgaRun.simulation_only) { throw 'FPGA demo must be explicit simulation' }
foreach ($artifact in $FpgaRun.artifacts.items) { $artifact.path="artifacts/$($artifact.output)/$([IO.Path]::GetFileName($artifact.path))" }
$FpgaRun | Add-Member -NotePropertyName summary_md -NotePropertyValue 'artifacts/reports/summary.md' -Force
Write-Utf8NoBom $FpgaRunFile ($FpgaRun | ConvertTo-Json -Depth 30)
$DemoProject = @{schema='wiparse.project/v1'; id='default'; name='default'; tests=@(@{id='ScopeSerialDemo';plugin_id='scope-serial-monitor';name='示波器串口监控（模拟效果检查）'},@{id='FpgaFirmwareTest';plugin_id='fpga-firmware-test';name="FPGA固件测试（$FpgaCount 项模拟效果检查）"})}
Write-Utf8NoBom (Join-Path $Out 'projects/default/project.json') ($DemoProject | ConvertTo-Json -Depth 6)
Copy-Tree (Join-Path $Root "services\testing-hub-marketplace") (Join-Path $Out "services\testing-hub-marketplace")
Write-Host "[package] seed marketplace data into package"
$PackageData = Join-Path $Out "services\testing-hub-marketplace\data"
$env:CLEAN = "1"
& node (Join-Path $Root "scripts\seed-marketplace-plugins.mjs") --data $PackageData
Remove-Item Env:CLEAN -ErrorAction SilentlyContinue
if ($LASTEXITCODE -ne 0) { throw "seed failed" }
Copy-Item -Force (Join-Path $Root "scripts\deploy-marketplace.ps1") (Join-Path $Out "scripts\deploy-marketplace.ps1")
Copy-Item -Force (Join-Path $Root "scripts\seed-marketplace-plugins.mjs") (Join-Path $Out "scripts\seed-marketplace-plugins.mjs")
Copy-Item -Force (Join-Path $Root "scripts\local-marketplace-sim.mjs") (Join-Path $Out "scripts\local-marketplace-sim.mjs")
if (Test-Path (Join-Path $Root "config.default.json")) {
    Copy-Item -Force (Join-Path $Root "config.default.json") (Join-Path $Out "config.default.json")
}
if (Test-Path (Join-Path $Root "Icon")) {
    Copy-Tree (Join-Path $Root "Icon") (Join-Path $Out "Icon")
}

$config = @{
    ui = @{ language = "zh"; theme = "dark" }
    apps = @{
        test_tool = @{
            plugins_dir = "test-tools/plugins"
            cli_path    = "bin/WiParse-CLI.exe"
            node_path   = "node"
            data_root   = "."
            marketplace = @{
                enabled     = $true
                base_url    = "http://127.0.0.1:$Port"
                install_dir = "marketplace"
                channel     = "stable"
            }
        }
    }
}
$configJson = $config | ConvertTo-Json -Depth 8
Write-Utf8NoBom (Join-Path $Out "config.json") $configJson

$readme = @"
# WiParse Testing Hub Marketplace Demo (Windows)

## 1. Start marketplace server

``````powershell
.\start-marketplace.ps1
``````

- URL: http://127.0.0.1:$Port
- Publish token: $Token

Seeded plugins:
- demo-marketplace-plugin@1.0.0
- market-echo@1.0.0
- market-counter@1.1.0
- market-preflight-lab@0.9.0

## 2. Launch WiParse

``````powershell
.\start-wiparse.ps1
``````

In **集成测试 / Testing Hub**:
1. Switch the left segment to **市场 / Market**
2. If needed, expand **服务器 / Server** and confirm URL ``http://127.0.0.1:$Port``
3. Refresh → Install a plugin → it opens in **插件** ready to run

Requires Node.js 18+ on PATH.

## CLI smoke

``````powershell
`$env:WIPARSE_MARKETPLACE_ALLOW_HTTP = "1"
node test-tools\marketplace.mjs catalog --url http://127.0.0.1:$Port --json
node test-tools\marketplace.mjs pull --plugin market-echo --version 1.0.0 ``
  --url http://127.0.0.1:$Port --install-dir .\marketplace --json
``````
"@
Write-Utf8NoBom (Join-Path $Out "README-MARKETPLACE-DEMO.md") $readme

$startMarket = @"
# Start local Testing Hub marketplace (seed + listen).
`$ErrorActionPreference = "Stop"
Set-Location `$PSScriptRoot
`$env:TOKEN = "$Token"
`$env:PORT = "$Port"
`$env:HOST = "127.0.0.1"
`$env:DATA_DIR = Join-Path `$PWD "services\testing-hub-marketplace\data"
& .\scripts\deploy-marketplace.ps1 @args
"@
Write-Utf8NoBom (Join-Path $Out "start-marketplace.ps1") $startMarket

$startGui = @"
# Launch WiParse against this demo folder.
`$ErrorActionPreference = "Stop"
`$Here = `$PSScriptRoot
Set-Location `$Here
`$env:WIPARSE_MARKETPLACE_ALLOW_HTTP = "1"
`$env:WIPARSE_PROJECT_ROOT = `$Here
`$env:WIPARSE_CONFIG = Join-Path `$Here "config.json"
`$env:WCM_CONFIG = `$env:WIPARSE_CONFIG
`$exe = Join-Path `$Here "bin\WiParse.exe"
Start-Process -FilePath `$exe -WorkingDirectory `$Here
"@
Write-Utf8NoBom (Join-Path $Out "start-wiparse.ps1") $startGui

$startService = @'
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$env:WIPARSE_SERVICE_ROOT = Join-Path $PSScriptRoot 'automation-data'
& (Join-Path $PSScriptRoot 'bin\WiParse-Service.exe') --data-root $env:WIPARSE_SERVICE_ROOT --port 7879
'@
Write-Utf8NoBom (Join-Path $Out "start-automation-service.ps1") $startService

$automationNotice = @"

## Automation service (development verification)

Run .\start-automation-service.ps1 before calling the new APIs.
Read docs/AUTOMATION-SERVICE.md and docs/specs/wireless-test/04-software-implementation-review.md.
industrial_approved=false. Complete SW-01..SW-12 and physical system acceptance are pending.
This bundle does not install a Windows Service or provide Dongle capture firmware.
wireless-charge-validation 0.1.1 provides offline configuration/rule replay only; live stimulus and industrial acceptance remain blocked.
Testing Hub includes $FpgaCount FPGA simulation configurations under recipes/fpga-firmware-test/cases. Historical report snapshots use shortened simulation timing; editable configurations retain original frozen timing. Select execution_mode=simulation explicitly before preflight/run. Hardware mode requires a separately accepted host adapter.
MCP 1.2.1 and its dependencies are bundled under mcp/wiparse; Node.js 18+ is still required.
Read docs/releases/1.2.1.md. The Test Report default project includes ScopeSerialDemo (synthetic data only).
"@
Add-Content -LiteralPath (Join-Path $Out 'README-MARKETPLACE-DEMO.md') -Value $automationNotice -Encoding utf8
$versionMatch = [regex]::Match([IO.File]::ReadAllText((Join-Path $Root 'Cargo.toml')), '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) { throw 'Cannot read workspace version' }
$releaseManifest = @{
    version = $versionMatch.Groups[1].Value
    mcp_version = (Get-Content -LiteralPath (Join-Path $Out 'mcp/wiparse/package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
    plugins = @(Get-ChildItem -LiteralPath (Join-Path $Out 'test-tools/plugins') -Directory | ForEach-Object { Get-Content -LiteralPath (Join-Path $_.FullName 'plugin.json') -Raw -Encoding UTF8 | ConvertFrom-Json | Select-Object id,version })
    demo = @{run=$DemoRelative; simulation_only=$true}
    fpga_demo = @{run=$FpgaRelative; simulation_only=$true; item_count=$FpgaCount; current_configs="original frozen business configurations"; historical_run_configs="isolated shortened simulation snapshots"}
    schema_version = 'wiparse.automation/1'
    industrial_approved = $false
    scope = 'SW automation foundations; complete software spec and joint industrial acceptance pending'
    built_at_utc = [DateTime]::UtcNow.ToString('o')
    source_freeze_sha256 = (Get-FileHash -LiteralPath (Join-Path $Out 'docs/verification/source-freeze.json') -Algorithm SHA256).Hash
    files = @(Get-ChildItem -LiteralPath (Join-Path $Out 'bin') -File | ForEach-Object { @{ path = "bin/$($_.Name)"; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash; bytes = $_.Length } })
    symbols = @(Get-ChildItem -LiteralPath (Join-Path $Out 'symbols') -File | ForEach-Object { @{ path = "symbols/$($_.Name)"; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } })
}
Write-Utf8NoBom (Join-Path $Out 'release-manifest.json') ($releaseManifest | ConvertTo-Json -Depth 8)

Write-Host "[package] writing archive $Archive"
& (Join-Path $PSScriptRoot 'write-portable-archive.ps1') -SourceDirectory $Out -Archive $Archive
if (-not (Test-Path $Archive)) {
    Compress-Archive -Path $Out -DestinationPath $Archive -Force
}

Write-Host "[package] done -> $Out"
Write-Utf8NoBom "$Archive.sha256" ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash + '  ' + (Split-Path $Archive -Leaf))
Write-Host "[package] archive -> $Archive"
Get-Item $Out, $Archive | Format-Table Name, Length, LastWriteTime
