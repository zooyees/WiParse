# Run only after batch development is complete. Does not deploy or create ZIPs.
param(
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [string]$ScpSample,
    [switch]$BuildPreviewBinaries
)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
$prefix=[IO.Path]::GetFullPath((Join-Path $repo 'docs/verification')).TrimEnd('\')+'\'
if(-not $evidence.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Evidence must be inside workspace docs/verification'}
if(Test-Path -LiteralPath $evidence){throw 'Use a new evidence directory; prior results are preserved'}
New-Item -ItemType Directory -Path $evidence|Out-Null
. (Join-Path $PSScriptRoot 'candidate-sources.ps1')

$checks=[Collections.Generic.List[object]]::new()
function Run-Check([string]$Name,[string]$Program,[string[]]$Arguments){
    $started=[DateTime]::UtcNow
    $code=-1
    $priorPreference=$ErrorActionPreference
    try { $ErrorActionPreference='Continue'; & $Program @Arguments *> (Join-Path $evidence "$Name.log");$code=$LASTEXITCODE }
    catch { $_|Out-String|Add-Content -LiteralPath (Join-Path $evidence "$Name.log") }
    finally {$ErrorActionPreference=$priorPreference}
    $checks.Add(@{name=$Name;exit_code=$code;passed=($code -eq 0);duration_sec=([DateTime]::UtcNow-$started).TotalSeconds})
    Write-Output "$Name : exit $code"
}
$original=Get-Location
$previousConfig=[Environment]::GetEnvironmentVariable('WIPARSE_CONFIG','Process')
$previousWcmConfig=[Environment]::GetEnvironmentVariable('WCM_CONFIG','Process')
$previousNonce=[Environment]::GetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN','Process')
try {
    Set-Location -LiteralPath $repo
    $env:WIPARSE_CONFIG=Join-Path $evidence 'test-config.json'
    $env:WCM_CONFIG=$env:WIPARSE_CONFIG
    [Environment]::SetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN',$null,'Process')
    [IO.File]::WriteAllText($env:WIPARSE_CONFIG,'{"ai":{"enabled":false},"ui":{"language":"zh","theme":"light"}}',[Text.UTF8Encoding]::new($false))
    $sources=Get-WiParseCandidateSources -Repo $repo
    [IO.File]::WriteAllText((Join-Path $evidence 'source-before.json'),($sources|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
    $environment=@{utc=[DateTime]::UtcNow.ToString('o');os=[Environment]::OSVersion.ToString();node=(& node --version);cargo=(& cargo --version);rust=(& rustc --version)}
    [IO.File]::WriteAllText((Join-Path $evidence 'environment.json'),($environment|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    Run-Check 'runtime-update-isolation' 'powershell.exe' @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File','scripts/test-next-runtime-update.ps1','-EvidenceDirectory',$evidence)
    Run-Check 'frontend-tests' 'npm.cmd' @('--prefix','apps/wiparse-next','test')
    Run-Check 'frontend-build' 'npm.cmd' @('--prefix','apps/wiparse-next','run','build')
    Run-Check 'plugin-tests' 'npm.cmd' @('--prefix','test-tools','test')
    Run-Check 'mcp-build' 'npm.cmd' @('--prefix','mcp/wiparse','run','build')
    Run-Check 'mcp-tests' 'npm.cmd' @('--prefix','mcp/wiparse','test')
    Run-Check 'workspace-tests' 'cargo' @('test','--workspace','--locked','--','--test-threads=2')
    Run-Check 'tauri-tests' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml')
    $sampleStatus='not_provided'
    if($ScpSample){
        $sample=Get-Item -LiteralPath $ScpSample
        $sampleHash=(Get-FileHash -LiteralPath $sample.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText((Join-Path $evidence 'scp-sample.json'),(@{filename=$sample.Name;bytes=$sample.Length;sha256=$sampleHash}|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
        $priorSample=[Environment]::GetEnvironmentVariable('WIPARSE_WAVE_SAMPLE','Process')
        try {
            $env:WIPARSE_WAVE_SAMPLE=$sample.FullName
            Run-Check 'scp-source-view' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scope_record_loads_and_views_without_copying_source_arrays','--','--ignored','--nocapture')
            Run-Check 'scp-decode' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scp_record_uses_core_decode_and_field_explanations','--','--ignored','--nocapture')
            if((Get-FileHash -LiteralPath $sample.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sampleHash){throw 'The supplied sample changed during verification'}
            $sampleStatus='executed_see_check_results'
        } finally {[Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$priorSample,'Process')}
    }
    $automaticPassed=@($checks|Where-Object {-not $_.passed}).Count -eq 0
    $runtime=[ordered]@{}
    if($automaticPassed){foreach($file in @('mcp/wiparse/dist/index.js','mcp/wiparse/dist/http.js')){$runtime[$file]=(Get-FileHash -LiteralPath (Join-Path $repo $file) -Algorithm SHA256).Hash.ToLowerInvariant()}}
    $binaries=[ordered]@{}
    if($BuildPreviewBinaries -and $automaticPassed){
        Run-Check 'compatibility-release' 'cargo' @('build','--release','--locked','-p','wiparse-gui','-p','wiparse-cli','-p','wiparse-service')
        Run-Check 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')
        $automaticPassed=@($checks|Where-Object {-not $_.passed}).Count -eq 0
        if($automaticPassed){foreach($file in @('apps/wiparse-next/src-tauri/target/release/wiparse-next.exe','target/release/wiparse-gui.exe','target/release/wiparse.exe','target/release/wiparse-service.exe')){$binaries[$file]=(Get-FileHash -LiteralPath (Join-Path $repo $file) -Algorithm SHA256).Hash.ToLowerInvariant()}}
    }
    $after=Get-WiParseCandidateSources -Repo $repo
    $unchanged=($sources|ConvertTo-Json -Compress -Depth 5) -eq ($after|ConvertTo-Json -Compress -Depth 5)
    $manifest=@{kind='wiparse_next_candidate';created_at=[DateTime]::UtcNow.ToString('o');automatic_passed=($automaticPassed -and $unchanged);sources_unchanged=$unchanged;source_hashes=$sources;binary_hashes=$binaries;runtime_hashes=$runtime;checks=$checks.ToArray();scp_sample=$sampleStatus;desktop='not_executed';interactive_dpapi='not_executed';real_ai='not_executed';hardware='not_executed';formal_rounds_passed=0;industrial_approved=$false}
    [IO.File]::WriteAllText((Join-Path $evidence 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
    if(-not $manifest.automatic_passed){throw 'Consolidated checks failed or candidate sources changed; see candidate.json and logs'}
    Write-Output (Join-Path $evidence 'candidate.json')
} finally {
    [Environment]::SetEnvironmentVariable('WIPARSE_CONFIG',$previousConfig,'Process')
    [Environment]::SetEnvironmentVariable('WCM_CONFIG',$previousWcmConfig,'Process')
    [Environment]::SetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN',$previousNonce,'Process')
    Set-Location -LiteralPath $original
}
