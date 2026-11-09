$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Set-Location -LiteralPath $repo
. (Join-Path $repo 'scripts/candidate-sources.ps1')
$basePath=Join-Path $repo 'docs/verification/next-candidate-20261108-r26/candidate.json'
$base=Get-Content -LiteralPath $basePath -Raw|ConvertFrom-Json
if($base.automatic_passed -ne $true){throw 'Full R26 validation is required'}
$before=Get-WiParseCandidateSources -Repo $repo
$changed=@($before.Keys|Where-Object{$base.source_hashes.PSObject.Properties[$_].Value -ne $before[$_]})
if($before.Count -ne @($base.source_hashes.PSObject.Properties).Count -or $changed.Count -ne 1 -or $changed[0] -ne 'apps/wiparse-next/src/style.css'){throw 'Only the reviewed CSS delta can reuse R26 backend verification'}
foreach($pair in $base.binary_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'Base binary identity changed'}}
foreach($pair in $base.runtime_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'MCP runtime identity changed'}}
$checks=@()
foreach($check in $base.checks){if($check.name -notin @('frontend-tests','frontend-build','tauri-release')){$checks+=@{name=$check.name;passed=$check.passed;inherited_from='next-candidate-20261108-r26';reason='All source inputs and associated runtime binaries unchanged'}}}
function Run-Delta($Name,$Program,$Arguments){
 $start=[DateTime]::UtcNow;$prior=$ErrorActionPreference
 try{$ErrorActionPreference='Continue';& $Program @Arguments *> (Join-Path $PSScriptRoot ($Name+'.log'));$exit=$LASTEXITCODE}finally{$ErrorActionPreference=$prior}
 $script:checks+=@{name=$Name;passed=($exit -eq 0);exit_code=$exit;elapsed_seconds=([DateTime]::UtcNow-$start).TotalSeconds}
 if($exit -ne 0){throw "Delta verification failed: $Name"}
}
Run-Delta 'frontend-tests' 'npm.cmd' @('--prefix','apps/wiparse-next','test')
Run-Delta 'frontend-build' 'npm.cmd' @('--prefix','apps/wiparse-next','run','build')
Run-Delta 'tauri-release' 'cargo' @('build','--release','--locked','--manifest-path','apps/wiparse-next/src-tauri/Cargo.toml','--features','custom-protocol')
$after=Get-WiParseCandidateSources -Repo $repo
if(($before|ConvertTo-Json -Compress -Depth 5) -ne ($after|ConvertTo-Json -Compress -Depth 5)){throw 'Sources changed during delta verification'}
$binaries=[ordered]@{}
foreach($key in $base.binary_hashes.PSObject.Properties.Name){$binaries[$key]=(Get-FileHash -LiteralPath (Join-Path $repo $key) -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest=@{kind='wiparse_next_candidate';automatic_passed=$true;sources_unchanged=$true;created_at=[DateTime]::UtcNow.ToString('o');source_hashes=$after;binary_hashes=$binaries;runtime_hashes=$base.runtime_hashes;checks=$checks;verification_strategy='CSS-only delta on complete R26 validation';changed_sources=$changed;base_manifest_sha256=(Get-FileHash -LiteralPath $basePath -Algorithm SHA256).Hash.ToLowerInvariant();scp_sample=$base.scp_sample;desktop='see separate desktop acceptance';hardware='not_executed';real_ai='not_executed';formal_rounds_passed=0;industrial_approved=$false}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
Write-Output 'CSS delta verified; unchanged backend, plugin, MCP and SCP checks inherit their exact R26 evidence.'
