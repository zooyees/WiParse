$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Set-Location -LiteralPath $repo
. (Join-Path $repo 'scripts/candidate-sources.ps1')
$basePath=Join-Path $repo 'docs/verification/next-candidate-20261109-r37/candidate.json'
$base=Get-Content -LiteralPath $basePath -Raw|ConvertFrom-Json
if($base.automatic_passed -ne $true){throw 'Verified R37 is required'}
$before=Get-WiParseCandidateSources -Repo $repo
$changed=@($before.Keys|Where-Object{$base.source_hashes.PSObject.Properties[$_].Value -ne $before[$_]})
$allowed=@('apps/wiparse-next/src/WaveWorkspace.tsx','apps/wiparse-next/src/WavePlot.tsx','apps/wiparse-next/src/style.css','apps/wiparse-next/src/localization.en.json','apps/wiparse-next/src/waveAnalysis.ts','apps/wiparse-next/src/ProtocolFrameList.tsx','apps/wiparse-next/src/ProtocolTimeline.tsx','apps/wiparse-next/src/WaveMeasurements.tsx','apps/wiparse-next/src-tauri/src/main.rs','apps/wiparse-next/src-tauri/src/wave_view.rs','apps/wiparse-next/src-tauri/src/protocol_overlay.rs','apps/wiparse-next/tests/wave-analysis.test.mjs')
if($before.Count -ne @($base.source_hashes.PSObject.Properties).Count+6 -or $changed.Count -ne $allowed.Count -or @($changed|Where-Object{$_ -notin $allowed}).Count){throw 'Unexpected source delta'}
$removed=@($base.source_hashes.PSObject.Properties.Name|Where-Object{!$before.Contains($_)})
if($removed.Count){throw 'Source files were removed'}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'source-before.json'),($before|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))

foreach($pair in $base.binary_hashes.PSObject.Properties){$baseBinary=if($pair.Name -eq 'apps/wiparse-next/src-tauri/target/release/wiparse-next.exe'){Join-Path $repo 'dist/WiParse-Next-20261109-Preview-R37/bin/WiParse.exe'}else{Join-Path $repo $pair.Name};if((Get-FileHash -LiteralPath $baseBinary -Algorithm SHA256).Hash -ne $pair.Value){throw 'Base binary identity changed'}}
foreach($pair in $base.runtime_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'MCP runtime identity changed'}}
$checks=@()
foreach($check in $base.checks){if($check.name -notin @('frontend-tests','frontend-build','tauri-tests','scp-source-view','scp-decode','tauri-release')){$checks+=@{name=$check.name;passed=$check.passed;inherited_from='next-candidate-20261109-r37';reason='All source inputs and associated runtime binaries unchanged'}}}
function Run-Delta($Name,$Program,$Arguments){
 $start=[DateTime]::UtcNow;$prior=$ErrorActionPreference
 try{$ErrorActionPreference='Continue';& $Program @Arguments *> (Join-Path $PSScriptRoot ($Name+'.log'));$exit=$LASTEXITCODE}finally{$ErrorActionPreference=$prior}
 $script:checks+=@{name=$Name;passed=($exit -eq 0);exit_code=$exit;elapsed_seconds=([DateTime]::UtcNow-$start).TotalSeconds}
 if($exit -ne 0){throw "Delta verification failed: $Name"}
}
Run-Delta 'frontend-tests' 'npm.cmd' @('--prefix','apps/wiparse-next','test')
Run-Delta 'frontend-build' 'npm.cmd' @('--prefix','apps/wiparse-next','run','build')
$priorConfig=$env:WIPARSE_CONFIG;$priorWcm=$env:WCM_CONFIG;$priorSample=$env:WIPARSE_WAVE_SAMPLE
try {
 $env:WIPARSE_CONFIG=Join-Path $PSScriptRoot 'test-config.json';$env:WCM_CONFIG=$env:WIPARSE_CONFIG
 [IO.File]::WriteAllText($env:WIPARSE_CONFIG,'{"ai":{"enabled":false}}',[Text.UTF8Encoding]::new($false))
 Run-Delta 'tauri-tests' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml')
 $env:WIPARSE_WAVE_SAMPLE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
 $sampleHash=(Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash
 Run-Delta 'scp-source-view' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scope_record_loads_and_views_without_copying_source_arrays','--','--ignored','--nocapture')
 Run-Delta 'scp-decode' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scp_record_uses_core_decode_and_field_explanations','--','--ignored','--nocapture')
 if((Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash -ne $sampleHash){throw 'SCP sample changed'}
} finally {
 [Environment]::SetEnvironmentVariable('WIPARSE_CONFIG',$priorConfig,'Process');[Environment]::SetEnvironmentVariable('WCM_CONFIG',$priorWcm,'Process');[Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$priorSample,'Process')
}
Run-Delta 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')
$after=Get-WiParseCandidateSources -Repo $repo
if(($before|ConvertTo-Json -Compress -Depth 5) -ne ($after|ConvertTo-Json -Compress -Depth 5)){throw 'Sources changed during delta verification'}
$binaries=[ordered]@{}
foreach($key in $base.binary_hashes.PSObject.Properties.Name){$binaries[$key]=(Get-FileHash -LiteralPath (Join-Path $repo $key) -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest=@{kind='wiparse_next_candidate';automatic_passed=$true;sources_unchanged=$true;created_at=[DateTime]::UtcNow.ToString('o');source_hashes=$after;binary_hashes=$binaries;runtime_hashes=$base.runtime_hashes;checks=$checks;verification_strategy='Waveform functional restoration on verified R37; frontend, native, supplied SCP and release rerun; core/host/service/plugins/MCP unchanged';changed_sources=$changed;base_manifest_sha256=(Get-FileHash -LiteralPath $basePath -Algorithm SHA256).Hash.ToLowerInvariant();scp_sample=$base.scp_sample;desktop='see separate desktop acceptance';hardware='not_executed';real_ai='not_executed';formal_rounds_passed=0;industrial_approved=$false}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
Write-Output 'R38 waveform delta verified; unchanged core/host/service/plugin/MCP checks inherit exact source identity; native and supplied SCP rerun.'
