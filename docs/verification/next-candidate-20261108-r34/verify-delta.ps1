$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Set-Location -LiteralPath $repo
. (Join-Path $repo 'scripts/candidate-sources.ps1')
$basePath=Join-Path $repo 'docs/verification/next-candidate-20261108-r31/candidate.json'
$base=Get-Content -LiteralPath $basePath -Raw|ConvertFrom-Json
if($base.automatic_passed -ne $true){throw 'Verified R31 is required'}
$before=Get-WiParseCandidateSources -Repo $repo
$changed=@($before.Keys|Where-Object{$base.source_hashes.PSObject.Properties[$_].Value -ne $before[$_]})
$allowed=@('apps/wiparse-next/src/WaveWorkspace.tsx','apps/wiparse-next/src/style.css','apps/wiparse-next/src/App.tsx','apps/wiparse-next/src/ReportWorkspace.tsx','apps/wiparse-next/src/requestEpoch.ts','apps/wiparse-next/src/reportTree.ts','apps/wiparse-next/src/localization.en.json','apps/wiparse-next/tests/request-epoch.test.mjs','apps/wiparse-next/tests/report-tree.test.mjs')
if($before.Count -ne @($base.source_hashes.PSObject.Properties).Count+2 -or $changed.Count -ne $allowed.Count -or @($changed|Where-Object{$_ -notin $allowed}).Count){throw 'Unexpected source delta'}
$removed=@($base.source_hashes.PSObject.Properties.Name|Where-Object{!$before.Contains($_)})
if($removed.Count){throw 'Source files were removed'}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'source-before.json'),($before|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))

foreach($pair in $base.binary_hashes.PSObject.Properties){$baseBinary=if($pair.Name -eq 'apps/wiparse-next/src-tauri/target/release/wiparse-next.exe'){Join-Path $repo 'dist/WiParse-Next-20261108-Preview-R31/bin/WiParse.exe'}else{Join-Path $repo $pair.Name};if((Get-FileHash -LiteralPath $baseBinary -Algorithm SHA256).Hash -ne $pair.Value){throw 'Base binary identity changed'}}
foreach($pair in $base.runtime_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $repo $pair.Name) -Algorithm SHA256).Hash -ne $pair.Value){throw 'MCP runtime identity changed'}}
$checks=@()
foreach($check in $base.checks){if($check.name -notin @('frontend-tests','frontend-build','tauri-release')){$checks+=@{name=$check.name;passed=$check.passed;inherited_from='next-candidate-20261108-r31';reason='All source inputs and associated runtime binaries unchanged'}}}
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
$manifest=@{kind='wiparse_next_candidate';automatic_passed=$true;sources_unchanged=$true;created_at=[DateTime]::UtcNow.ToString('o');source_hashes=$after;binary_hashes=$binaries;runtime_hashes=$base.runtime_hashes;checks=$checks;verification_strategy='Report context isolation plus calculator page rendering and plot-first waveform layout on verified R31; native source/core/host/plugins/SCP unchanged';changed_sources=$changed;base_manifest_sha256=(Get-FileHash -LiteralPath $basePath -Algorithm SHA256).Hash.ToLowerInvariant();scp_sample=$base.scp_sample;desktop='see separate desktop acceptance';hardware='not_executed';real_ai='not_executed';formal_rounds_passed=0;industrial_approved=$false}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'candidate.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
Write-Output 'R34 frontend delta verified; backend evidence is inherited with exact source identity.'
