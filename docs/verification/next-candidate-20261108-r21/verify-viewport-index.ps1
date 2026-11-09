param([Parameter(Mandatory=$true)][string]$ParentManifest)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$evidence=$PSScriptRoot
Set-Location -LiteralPath $repo
. (Join-Path $repo 'scripts/candidate-sources.ps1')
$parent=Get-Content -LiteralPath $ParentManifest -Raw|ConvertFrom-Json
if(-not $parent.automatic_passed -or -not $parent.sources_unchanged){throw 'Requires a successful frozen parent candidate'}
if(Test-Path -LiteralPath (Join-Path $evidence 'candidate.json')){throw 'Candidate already exists'}
$before=Get-WiParseCandidateSources -Repo $repo
if($before.Count -ne (@($parent.source_hashes.PSObject.Properties).Count+1)){throw 'Source set changed'}
$changed=@($before.GetEnumerator()|Where-Object {$parent.source_hashes.PSObject.Properties[$_.Key].Value -ne $_.Value}|ForEach-Object {$_.Key})
$allowed=@('apps/wiparse-next/src-tauri/src/main.rs','apps/wiparse-next/src-tauri/src/wave_extrema.rs','apps/wiparse-next/src-tauri/src/wave_view.rs')
if($changed.Count -ne 3 -or @($changed|Where-Object {$_ -notin $allowed}).Count){throw "Only exact viewport index sources may differ: $changed"}
foreach($set in @($parent.binary_hashes,$parent.runtime_hashes)){foreach($entry in $set.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash -ne $entry.Value){throw "Parent artifact changed: $($entry.Name)"}}}
[IO.File]::WriteAllText((Join-Path $evidence 'source-before.json'),($before|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
$checks=[Collections.Generic.List[object]]::new()
function Run-Check([string]$Name,[string]$Program,[string[]]$Arguments){
 $started=[DateTime]::UtcNow;$code=-1;$prior=$ErrorActionPreference
 try{$ErrorActionPreference='Continue';& $Program @Arguments *> (Join-Path $evidence "$Name.log");$code=$LASTEXITCODE}
 catch{$_|Out-String|Add-Content -LiteralPath (Join-Path $evidence "$Name.log")}
 finally{$ErrorActionPreference=$prior}
 $checks.Add(@{name=$Name;exit_code=$code;passed=($code -eq 0);duration_sec=([DateTime]::UtcNow-$started).TotalSeconds})
 Write-Output "$Name : exit $code"
}
$priorConfig=$env:WIPARSE_CONFIG;$priorWcm=$env:WCM_CONFIG;$priorNonce=$env:WIPARSE_LOCAL_API_TOKEN
try{
 $env:WIPARSE_CONFIG=Join-Path $evidence 'test-config.json';$env:WCM_CONFIG=$env:WIPARSE_CONFIG
 [Environment]::SetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN',$null,'Process')
 [IO.File]::WriteAllText($env:WIPARSE_CONFIG,'{"ai":{"enabled":false},"ui":{"language":"zh","theme":"light"}}',[Text.UTF8Encoding]::new($false))
 Run-Check 'frontend-tests' 'npm.cmd' @('--prefix','apps/wiparse-next','test')
 Run-Check 'frontend-build' 'npm.cmd' @('--prefix','apps/wiparse-next','run','build')
 Run-Check 'tauri-tests' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml')
 $priorSample=[Environment]::GetEnvironmentVariable('WIPARSE_WAVE_SAMPLE','Process')
 try{
  $env:WIPARSE_WAVE_SAMPLE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
  $sample=Get-Item -LiteralPath $env:WIPARSE_WAVE_SAMPLE
  $sampleHash=(Get-FileHash -LiteralPath $sample.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText((Join-Path $evidence 'scp-sample.json'),(@{filename=$sample.Name;bytes=$sample.Length;sha256=$sampleHash}|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
  if(@($checks|Where-Object {-not $_.passed}).Count -eq 0){
   Run-Check 'scp-source-view' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scope_record_loads_and_views_without_copying_source_arrays','--','--ignored','--nocapture')
   Run-Check 'scp-decode' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scp_record_uses_core_decode_and_field_explanations','--','--ignored','--nocapture')
   Run-Check 'viewport-stress' 'cargo' @('test','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','supplied_scope_viewport_stress_uses_exact_index_and_bounded_output','--','--ignored','--nocapture')
  }
  if((Get-FileHash -LiteralPath $sample.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sampleHash){throw 'Source waveform changed'}
 }finally{[Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$priorSample,'Process')}
 if(@($checks|Where-Object {-not $_.passed}).Count -eq 0){Run-Check 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')}
 $unchanged=($before|ConvertTo-Json -Compress -Depth 5) -eq ((Get-WiParseCandidateSources -Repo $repo)|ConvertTo-Json -Compress -Depth 5)
 $binary=[ordered]@{};$runtime=[ordered]@{}
 foreach($entry in $parent.binary_hashes.PSObject.Properties){$hash=(Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant();if($entry.Name -notlike '*wiparse-next.exe' -and $hash -ne $entry.Value){throw "Unchanged backend artifact drifted: $($entry.Name)"};$binary[$entry.Name]=$hash}
 foreach($entry in $parent.runtime_hashes.PSObject.Properties){$hash=(Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant();if($hash -ne $entry.Value){throw "MCP artifact drifted: $($entry.Name)"};$runtime[$entry.Name]=$hash}
 $reused=@($parent.checks|Where-Object {$_.name -notin @('frontend-tests','frontend-build','tauri-tests','tauri-release','scp-source-view','scp-decode','viewport-stress')})
 foreach($check in $reused){$check|Add-Member -Force -NotePropertyName reused_from -NotePropertyValue '../next-candidate-20261108-r20/candidate.json'}
 $all=@($reused)+$checks.ToArray()
 $manifest=@{kind='wiparse_next_candidate';created_at=[DateTime]::UtcNow.ToString('o');automatic_passed=($unchanged -and @($all|Where-Object {-not $_.passed}).Count -eq 0);sources_unchanged=$unchanged;source_hashes=$before;binary_hashes=$binary;runtime_hashes=$runtime;checks=$all;parent_verification_sha256=(Get-FileHash -LiteralPath $ParentManifest -Algorithm SHA256).Hash.ToLowerInvariant();changed_sources=$changed;reuse_basis='Only native waveform viewport indexing and its exactness/resource regression coverage changed. Frontend, core algorithms, host, drivers, plugins, MCP and backend artifacts are hash-identical to frozen r20. Frontend build/tests, all Tauri tests, real SCP load/decode, 200-viewport stress and native release build rerun. The user test ZIP remains an immutable r20 snapshot.';normal_user_dpapi=@{passed=$true;checks=5;evidence='../next-candidate-20261108-r4/normal-user-dpapi.json'};scp_sample='executed_on_index_candidate_see_check_results';desktop='not_executed';interactive_dpapi='not_executed';real_ai='not_executed';hardware='not_executed';formal_rounds_passed=0;industrial_approved=$false}
 [IO.File]::WriteAllText((Join-Path $evidence 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
 if(-not $manifest.automatic_passed){throw 'CSS candidate verification failed'}
 Write-Output (Join-Path $evidence 'candidate.json')
}finally{
 [Environment]::SetEnvironmentVariable('WIPARSE_CONFIG',$priorConfig,'Process')
 [Environment]::SetEnvironmentVariable('WCM_CONFIG',$priorWcm,'Process')
 [Environment]::SetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN',$priorNonce,'Process')
}
