# Stage a verified Tauri candidate without altering installed WiParse or creating a ZIP.
param(
    [Parameter(Mandatory=$true)][string]$VerificationManifest,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$output=[IO.Path]::GetFullPath($OutputDirectory)
$allowed=[IO.Path]::GetFullPath((Join-Path $repo 'dist')).TrimEnd('\')+'\'
if(-not $output.StartsWith($allowed,[StringComparison]::OrdinalIgnoreCase)){throw 'Preview output must be inside workspace dist'}
if(Test-Path -LiteralPath $output){throw 'Use a new preview directory; existing versions are preserved'}
$verification=Get-Content -LiteralPath $VerificationManifest -Raw|ConvertFrom-Json
if($verification.kind -ne 'wiparse_next_candidate' -or $verification.automatic_passed -ne $true){throw 'A successful consolidated candidate verification is required'}
foreach($entry in $verification.source_hashes.PSObject.Properties){
    $source=[IO.Path]::GetFullPath((Join-Path $repo $entry.Name))
    if(-not $source.StartsWith($repo+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Invalid candidate source path'}
    if((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $entry.Value){throw "Source changed after candidate verification: $($entry.Name)"}
}
# Reject newly added runtime source files as well as changes to recorded files.
. (Join-Path $PSScriptRoot 'candidate-sources.ps1')
$current=Get-WiParseCandidateSources -Repo $repo
if($current.Count -ne @($verification.source_hashes.PSObject.Properties).Count){throw 'Candidate source set changed after verification'}
foreach($entry in $current.GetEnumerator()){if($verification.source_hashes.PSObject.Properties[$entry.Key].Value -ne $entry.Value){throw "Candidate source set changed: $($entry.Key)"}}
if(-not (Test-Path -LiteralPath (Join-Path $repo 'test-tools/node_modules/ajv/package.json'))){throw 'Plugin runtime dependencies are missing; restore from the lockfile before verification'}
$binaries=@(
    @('apps/wiparse-next/src-tauri/target/release/wiparse-next.exe','WiParse.exe'),
    @('target/release/wiparse-gui.exe','WiParse-Host.exe'),
    @('target/release/wiparse.exe','WiParse-CLI.exe'),
    @('target/release/wiparse-service.exe','WiParse-Service.exe')
)
foreach($pair in $binaries){
    $expected=$verification.binary_hashes.PSObject.Properties[$pair[0]].Value
    if(-not $expected -or (Get-FileHash -LiteralPath (Join-Path $repo $pair[0]) -Algorithm SHA256).Hash -ne $expected){throw "Candidate binary missing or changed: $($pair[0])"}
}
foreach($file in @('mcp/wiparse/dist/index.js','mcp/wiparse/dist/http.js')){
    $expected=$verification.runtime_hashes.PSObject.Properties[$file].Value
    if(-not $expected -or (Get-FileHash -LiteralPath (Join-Path $repo $file) -Algorithm SHA256).Hash -ne $expected){throw "Verified MCP artifact missing or changed: $file"}
}
New-Item -ItemType Directory -Path (Join-Path $output 'bin') -Force|Out-Null
foreach($pair in $binaries){Copy-Item -LiteralPath (Join-Path $repo $pair[0]) -Destination (Join-Path $output "bin/$($pair[1])")}
function Copy-RuntimeTree([string]$relative){
    $source=Join-Path $repo $relative
    if(-not (Test-Path -LiteralPath $source -PathType Container)){throw "Runtime directory missing: $relative"}
    $destination=Join-Path $output $relative
    New-Item -ItemType Directory -Path $destination -Force|Out-Null
    $excluded=@('logs','reports','runs','tmp','data')|ForEach-Object {Join-Path $source $_}
    & robocopy $source $destination /E /NFL /NDL /NJH /NJS /nc /ns /np /XJ /XD .git .cache @excluded /XF .env '.env.*' '*.pem' '*.key' run.stop run.lock _loop_status.json _loop_hint.txt |Out-Null
    if($LASTEXITCODE -ge 8){throw "Runtime copy failed: $relative"}
}
foreach($relative in @('test-tools','mcp/wiparse','recipes/fpga-firmware-test','recipes/wireless-whitebox')){Copy-RuntimeTree $relative}
if(Test-Path -LiteralPath (Join-Path $repo 'Icon')){Copy-RuntimeTree 'Icon'}
# Generate clean portable defaults; never copy a user's config or saved credentials.
$config=@{ai=@{enabled=$false};ui=@{language='zh';theme='light'};apps=@{test_tool=@{plugins_dir='test-tools/plugins';cli_path='bin/WiParse-CLI.exe';node_path='node';data_root='.';marketplace=@{enabled=$false;install_dir='marketplace'}}}}
$json=$config|ConvertTo-Json -Depth 12
foreach($name in @('config.json','config.default.json')){[IO.File]::WriteAllText((Join-Path $output $name),$json,[Text.UTF8Encoding]::new($false))}
New-Item -ItemType Directory -Path (Join-Path $output 'docs') -Force|Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'apps/wiparse-next/MIGRATION-STATUS.md') -Destination (Join-Path $output 'docs/MIGRATION-STATUS.md')
Copy-Item -LiteralPath $VerificationManifest -Destination (Join-Path $output 'candidate-verification.json')
$readme="WiParse Next EXE preview`r`n`r`nRun bin\WiParse.exe and keep the adjacent runtime files. Windows WebView2 and Node.js 24 are required. VISA and device drivers remain system prerequisites.`r`nAI is disabled and no user configuration, credentials or reports are included. Existing installations are untouched. No ZIP is created.`r`nAutomatic checks passed for the recorded candidate. Desktop, hardware and formal acceptance status are listed in candidate-verification.json; automatic checks do not imply industrial approval.`r`n"
[IO.File]::WriteAllText((Join-Path $output 'PREVIEW.txt'),$readme,[Text.UTF8Encoding]::new($false))
$hashes=@{}
foreach($pair in $binaries){$hashes[$pair[1]]=(Get-FileHash -LiteralPath (Join-Path $output "bin/$($pair[1])") -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest=@{kind='tauri_exe_preview';created_at=[DateTime]::UtcNow.ToString('o');zip_created=$false;industrial_approved=$false;binary_sha256=$hashes}
[IO.File]::WriteAllText((Join-Path $output 'preview-manifest.json'),($manifest|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
& (Join-Path $PSScriptRoot 'write-next-runtime-manifest.ps1') -Directory $output -Version (Split-Path $output -Leaf)
$global:LASTEXITCODE=0
Write-Output (Join-Path $output 'bin/WiParse.exe')
