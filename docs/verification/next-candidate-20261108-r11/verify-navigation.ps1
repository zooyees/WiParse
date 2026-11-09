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
if($before.Count -ne @($parent.source_hashes.PSObject.Properties).Count){throw 'Source set changed'}
$changed=@($before.GetEnumerator()|Where-Object {$parent.source_hashes.PSObject.Properties[$_.Key].Value -ne $_.Value}|ForEach-Object {$_.Key})
$allowed=@('apps/wiparse-next/src/WaveWorkspace.tsx','apps/wiparse-next/src/waveCursor.ts','apps/wiparse-next/tests/wave-cursor.test.mjs','apps/wiparse-next/src/localization.en.json')
if($changed.Count -ne 4 -or @($changed|Where-Object {$_ -notin $allowed}).Count){throw "Only recorded panel presentation corrections may differ: $changed"}
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
 if(@($checks|Where-Object {-not $_.passed}).Count -eq 0){Run-Check 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')}
 $unchanged=($before|ConvertTo-Json -Compress -Depth 5) -eq ((Get-WiParseCandidateSources -Repo $repo)|ConvertTo-Json -Compress -Depth 5)
 $binary=[ordered]@{};$runtime=[ordered]@{}
 foreach($entry in $parent.binary_hashes.PSObject.Properties){$hash=(Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant();if($entry.Name -notlike '*wiparse-next.exe' -and $hash -ne $entry.Value){throw "Unchanged backend artifact drifted: $($entry.Name)"};$binary[$entry.Name]=$hash}
 foreach($entry in $parent.runtime_hashes.PSObject.Properties){$hash=(Get-FileHash -LiteralPath (Join-Path $repo $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant();if($hash -ne $entry.Value){throw "MCP artifact drifted: $($entry.Name)"};$runtime[$entry.Name]=$hash}
 $reused=@($parent.checks|Where-Object {$_.name -notin @('frontend-tests','frontend-build','tauri-tests','tauri-release')})
 foreach($check in $reused){$check|Add-Member -Force -NotePropertyName reused_from -NotePropertyValue '../next-candidate-20261108-r10/candidate.json'}
 $all=@($reused)+$checks.ToArray()
 $manifest=@{kind='wiparse_next_candidate';created_at=[DateTime]::UtcNow.ToString('o');automatic_passed=($unchanged -and @($all|Where-Object {-not $_.passed}).Count -eq 0);sources_unchanged=$unchanged;source_hashes=$before;binary_hashes=$binary;runtime_hashes=$runtime;checks=$all;parent_verification_sha256=(Get-FileHash -LiteralPath $ParentManifest -Algorithm SHA256).Hash.ToLowerInvariant();changed_sources=$changed;reuse_basis='Only frontend waveform navigation, its pure geometry tests and tooltip translation changed. Frontend and Tauri checks/build rerun; backend/plugin/MCP source and artifact hashes unchanged. Parent evidence chain retained by hash.';normal_user_dpapi=@{passed=$true;checks=5;evidence='../next-candidate-20261108-r4/normal-user-dpapi.json'};scp_sample='passed_in_unchanged_parent';desktop='not_executed';interactive_dpapi='not_executed';real_ai='not_executed';hardware='not_executed';formal_rounds_passed=0;industrial_approved=$false}
 [IO.File]::WriteAllText((Join-Path $evidence 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
 if(-not $manifest.automatic_passed){throw 'CSS candidate verification failed'}
 Write-Output (Join-Path $evidence 'candidate.json')
}finally{
 [Environment]::SetEnvironmentVariable('WIPARSE_CONFIG',$priorConfig,'Process')
 [Environment]::SetEnvironmentVariable('WCM_CONFIG',$priorWcm,'Process')
 [Environment]::SetEnvironmentVariable('WIPARSE_LOCAL_API_TOKEN',$priorNonce,'Process')
}
