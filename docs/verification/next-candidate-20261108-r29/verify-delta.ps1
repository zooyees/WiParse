$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Set-Location -LiteralPath $repo
. (Join-Path $repo 'scripts/candidate-sources.ps1')
$basePath=Join-Path $repo 'docs/verification/next-candidate-20261108-r27/candidate.json'
$base=Get-Content -LiteralPath $basePath -Raw|ConvertFrom-Json
if($base.automatic_passed -ne $true){throw 'Verified R27 and its full R26 baseline are required'}
$before=Get-WiParseCandidateSources -Repo $repo
$changed=@($before.Keys|Where-Object{$base.source_hashes.PSObject.Properties[$_].Value -ne $before[$_]})
$allowed=@('apps/wiparse-next/src-tauri/src/resource_policy.rs','apps/wiparse-next/src-tauri/src/wave_view.rs','apps/wiparse-next/src/waveMapping.ts','apps/wiparse-next/src/WaveWorkspace.tsx','apps/wiparse-next/src/WavePlot.tsx','apps/wiparse-next/tests/wave-mapping.test.mjs')
if($before.Count -ne @($base.source_hashes.PSObject.Properties).Count -or $changed.Count -ne $allowed.Count -or @($changed|Where-Object{$_ -notin $allowed}).Count -gt 0){throw 'Unexpected delta; unchanged groups cannot be reused'}
foreach($pair in $base.binary_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'Base binary identity changed'}}
foreach($pair in $base.runtime_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'MCP runtime identity changed'}}
$checks=@()
foreach($check in $base.checks){if($check.name -notin @('frontend-tests','frontend-build','tauri-tests','scp-source-view','scp-decode','tauri-release')){$checks+=@{name=$check.name;passed=$check.passed;inherited_from='next-candidate-20261108-r27';reason='All source inputs and associated runtime binaries unchanged'}}}
function Run-Delta($Name,$Program,$Arguments){
 $start=[DateTime]::UtcNow;$prior=$ErrorActionPreference
 try{$ErrorActionPreference='Continue';& $Program @Arguments *> (Join-Path $PSScriptRoot ($Name+'.log'));$exit=$LASTEXITCODE}finally{$ErrorActionPreference=$prior}
 $script:checks+=@{name=$Name;passed=($exit -eq 0);exit_code=$exit;elapsed_seconds=([DateTime]::UtcNow-$start).TotalSeconds}
 if($exit -ne 0){throw "Delta verification failed: $Name"}
}
Run-Delta 'frontend-tests' 'npm.cmd' @('--prefix','apps/wiparse-next','test')
Run-Delta 'frontend-build' 'npm.cmd' @('--prefix','apps/wiparse-next','run','build')
$priorSample=[Environment]::GetEnvironmentVariable('WIPARSE_WAVE_SAMPLE','Process')
$priorConfig=[Environment]::GetEnvironmentVariable('WIPARSE_CONFIG','Process')
$priorWcm=[Environment]::GetEnvironmentVariable('WCM_CONFIG','Process')
try{
 $env:WIPARSE_CONFIG=Join-Path $PSScriptRoot 'test-config.json';$env:WCM_CONFIG=$env:WIPARSE_CONFIG
 [IO.File]::WriteAllText($env:WIPARSE_CONFIG,'{"ai":{"enabled":false}}',[Text.UTF8Encoding]::new($false))
 Run-Delta 'tauri-tests' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml')
 $env:WIPARSE_WAVE_SAMPLE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
 $sampleHash=(Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash
 Run-Delta 'scp-source-view' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scope_record_loads_and_views_without_copying_source_arrays','--','--ignored','--nocapture')
 Run-Delta 'scp-decode' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scp_record_uses_core_decode_and_field_explanations','--','--ignored','--nocapture')
 if((Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash -ne $sampleHash){throw 'SCP sample changed during verification'}
}finally{
 [Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$priorSample,'Process')
 [Environment]::SetEnvironmentVariable('WIPARSE_CONFIG',$priorConfig,'Process')
 [Environment]::SetEnvironmentVariable('WCM_CONFIG',$priorWcm,'Process')
}
Run-Delta 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')
$after=Get-WiParseCandidateSources -Repo $repo
if(($before|ConvertTo-Json -Compress -Depth 5) -ne ($after|ConvertTo-Json -Compress -Depth 5)){throw 'Sources changed during delta verification'}
$binaries=[ordered]@{}
foreach($key in $base.binary_hashes.PSObject.Properties.Name){$binaries[$key]=(Get-FileHash -LiteralPath (Join-Path $repo $key) -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest=@{kind='wiparse_next_candidate';automatic_passed=$true;sources_unchanged=$true;created_at=[DateTime]::UtcNow.ToString('o');source_hashes=$after;binary_hashes=$binaries;runtime_hashes=$base.runtime_hashes;checks=$checks;verification_strategy='Waveform capacity/display delta on verified R27 and full R26 validation';changed_sources=$changed;base_manifest_sha256=(Get-FileHash -LiteralPath $basePath -Algorithm SHA256).Hash.ToLowerInvariant();scp_sample=$base.scp_sample;desktop='see separate desktop acceptance';hardware='not_executed';real_ai='not_executed';formal_rounds_passed=0;industrial_approved=$false}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
Write-Output 'Waveform delta verified; unchanged core/host/service/plugin/MCP/update checks inherit exact baseline evidence; frontend/native/SCP and EXE rebuilt.'
