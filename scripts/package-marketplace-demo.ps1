# Build WiParse (Windows) + marketplace demo bundle for local testing.
#
#   .\scripts\package-marketplace-demo.ps1
# Output:
#   dist\wiparse-win-marketplace-demo\
#   dist\wiparse-win-marketplace-demo.zip
#   dist\WiParse.exe / dist\WiParse-CLI.exe (runtime copy)

param(
    [switch]$SkipBuild
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
    & cargo build --release -p wiparse-gui -p wiparse-cli
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$GuiExe = Join-Path $Root "target\release\wiparse-gui.exe"
$CliExe = Join-Path $Root "target\release\wiparse.exe"
if (-not (Test-Path $GuiExe)) { throw "missing $GuiExe" }
if (-not (Test-Path $CliExe)) { throw "missing $CliExe" }

if (Test-Path $Out) { Remove-Item $Out -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $Out "bin") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Out "scripts") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Out "marketplace") | Out-Null

Copy-Item -Force $GuiExe (Join-Path $Out "bin\WiParse.exe")
Copy-Item -Force $CliExe (Join-Path $Out "bin\WiParse-CLI.exe")

# Also refresh the daily dist/ runtime copy (skip if the running GUI has the file mapped).
New-Item -ItemType Directory -Force -Path (Join-Path $Root "dist") | Out-Null
foreach ($pair in @(
    @{ Src = $GuiExe; Dst = Join-Path $Root "dist\WiParse.exe" },
    @{ Src = $CliExe; Dst = Join-Path $Root "dist\WiParse-CLI.exe" }
)) {
    try {
        Copy-Item -Force $pair.Src $pair.Dst
    } catch {
        Write-Host "[package] skip $($pair.Dst) (in use): $($_.Exception.Message)"
    }
}
Copy-Tree (Join-Path $Root "test-tools") (Join-Path $Root "dist\test-tools")

Copy-Tree (Join-Path $Root "test-tools") (Join-Path $Out "test-tools")
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

Write-Host "[package] writing archive $Archive"
if (Test-Path $Archive) { Remove-Item $Archive -Force }
Push-Location (Split-Path $Out)
try {
    tar -a -c -f (Split-Path $Archive -Leaf) (Split-Path $Out -Leaf)
} finally {
    Pop-Location
}
if (-not (Test-Path $Archive)) {
    Compress-Archive -Path $Out -DestinationPath $Archive -Force
}

Write-Host "[package] done -> $Out"
Write-Host "[package] archive -> $Archive"
Get-Item $Out, $Archive | Format-Table Name, Length, LastWriteTime
