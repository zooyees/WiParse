$ErrorActionPreference='Stop'
$repo='D:\windlink\windlink\WiParse-R'
Set-Location -LiteralPath $repo
. scripts/candidate-sources.ps1
$evidence=Join-Path $repo 'docs/verification/next-candidate-20261108-r19'
$manifestPath=Join-Path $evidence 'candidate.json'
$manifest=Get-Content -Raw -LiteralPath $manifestPath|ConvertFrom-Json
if(-not $manifest.automatic_passed){throw 'Candidate must pass first'}
$previous=[Environment]::GetEnvironmentVariable('WIPARSE_WAVE_SAMPLE','Process')
try{
 $env:WIPARSE_WAVE_SAMPLE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
 $sourceHash=(Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash
 $checks=[Collections.Generic.List[object]]::new()
 foreach($pair in @(@('scp-source-view','supplied_scope_record_loads_and_views_without_copying_source_arrays'),@('scp-decode','supplied_scp_record_uses_core_decode_and_field_explanations'))){
   $started=[DateTime]::UtcNow;$prior=$ErrorActionPreference
   try{$ErrorActionPreference='Continue';& cargo test --locked --manifest-path apps/wiparse-next/src-tauri/Cargo.toml $pair[1] -- --ignored --nocapture *> (Join-Path $evidence ($pair[0]+'.log'));$code=$LASTEXITCODE}finally{$ErrorActionPreference=$prior}
   $checks.Add(@{name=$pair[0];passed=($code -eq 0);exit_code=$code;duration_sec=([DateTime]::UtcNow-$started).TotalSeconds})
   Write-Output ($pair[0]+' : exit '+$code)
 }
 if((Get-FileHash -LiteralPath $env:WIPARSE_WAVE_SAMPLE -Algorithm SHA256).Hash -ne $sourceHash){throw 'Source waveform changed'}
 $current=Get-WiParseCandidateSources -Repo $repo
 if($current.Count -ne @($manifest.source_hashes.PSObject.Properties).Count){throw 'Source set changed'}
 foreach($entry in $current.GetEnumerator()){if($manifest.source_hashes.PSObject.Properties[$entry.Key].Value -ne $entry.Value){throw 'Source hash changed'}}
 Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $evidence 'candidate-before-scp-supplement.json')
 $manifest.checks=@($manifest.checks|Where-Object {$_.name -notin @('scp-source-view','scp-decode')})+$checks.ToArray()
 $manifest.scp_sample='executed_on_final_candidate_see_check_results'
 $manifest.automatic_passed=(@($manifest.checks|Where-Object {-not $_.passed}).Count -eq 0)
 $sample=Get-Item -LiteralPath $env:WIPARSE_WAVE_SAMPLE
 [IO.File]::WriteAllText((Join-Path $evidence 'scp-sample.json'),(@{filename=$sample.Name;bytes=$sample.Length;sha256=$sourceHash.ToLowerInvariant()}|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
 [IO.File]::WriteAllText($manifestPath,($manifest|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
 if(-not $manifest.automatic_passed){throw 'Final SCP regression failed'}
}finally{[Environment]::SetEnvironmentVariable('WIPARSE_WAVE_SAMPLE',$previous,'Process')}
