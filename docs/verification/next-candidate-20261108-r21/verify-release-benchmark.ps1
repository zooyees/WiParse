$ErrorActionPreference='Stop'
$repo='D:\windlink\windlink\WiParse-R'
Set-Location -LiteralPath $repo
. scripts/candidate-sources.ps1
$evidence=Join-Path $repo 'docs/verification/next-candidate-20261108-r21'
$manifestPath=Join-Path $evidence 'candidate.json'
$manifest=Get-Content -Raw -LiteralPath $manifestPath|ConvertFrom-Json
if(-not $manifest.automatic_passed){throw 'Candidate must pass first'}
$previous=[Environment]::GetEnvironmentVariable('WIPARSE_WAVE_SAMPLE','Process')
$checks=[Collections.Generic.List[object]]::new()
try{
 $env:WIPARSE_WAVE_SAMPLE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
 $sampleHash=(Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash
 foreach($pair in @(@('release-source-view','supplied_scope_record_loads_and_views_without_copying_source_arrays'),@('release-viewport-stress','supplied_scope_viewport_stress_uses_exact_index_and_bounded_output'))){
  $started=[DateTime]::UtcNow;$prior=$ErrorActionPreference
  try{$ErrorActionPreference='Continue';& cargo test --release --locked --features custom-protocol --manifest-path apps/wiparse-next/src-tauri/Cargo.toml $pair[1] -- --ignored --nocapture *> (Join-Path $evidence ($pair[0]+'.log'));$code=$LASTEXITCODE}finally{$ErrorActionPreference=$prior}
  $checks.Add(@{name=$pair[0];exit_code=$code;passed=($code -eq 0);duration_sec=([DateTime]::UtcNow-$started).TotalSeconds})
  Write-Output ($pair[0]+' : exit '+$code)
 }
 if((Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash -ne $sampleHash){throw 'Source waveform changed'}
 $current=Get-WiParseCandidateSources -Repo $repo
 if($current.Count -ne @($manifest.source_hashes.PSObject.Properties).Count){throw 'Source set changed'}
 foreach($entry in $current.GetEnumerator()){if($manifest.source_hashes.PSObject.Properties[$entry.Key].Value -ne $entry.Value){throw 'Source hash changed'}}
 foreach($entry in $manifest.binary_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash -ne $entry.Value){throw 'Verified artifact changed'}}
 $result=@{kind='release_viewport_index_benchmark';candidate_sha256=(Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant();source_and_binary_hashes_unchanged=$true;checks=$checks.ToArray();passed=(@($checks|Where-Object {-not $_.passed}).Count -eq 0);hardware_executed=$false;full_gui_latency_measured=$false;user_test_package='WiParse-Next-20261108-R20-Test.zip (unchanged)'}
 [IO.File]::WriteAllText((Join-Path $evidence 'release-benchmark.json'),($result|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
 if(-not $result.passed){throw 'Release benchmark failed'}
}finally{[Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$previous,'Process')}
