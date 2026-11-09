param([string]$EvidenceDirectory='docs/verification/migration-implementation-2026-11-08')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$repo=Split-Path -Parent $PSScriptRoot
$base=Join-Path $repo ('target/update-test-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $base -Force|Out-Null
$root=Join-Path $base 'installed';$incoming=Join-Path $base 'incoming'
$names=@('bin/WiParse.exe','bin/WiParse-Host.exe','bin/WiParse-CLI.exe','bin/WiParse-Service.exe','mcp/wiparse/package.json','test-tools/package.json','test-tools/plugins/demo/config.json')
function Fixture($dir,$version){foreach($name in $names){$path=Join-Path $dir $name;[IO.Directory]::CreateDirectory((Split-Path $path))|Out-Null;[IO.File]::WriteAllText($path,"fake-$version-$name")}; & (Join-Path $repo 'scripts/write-next-runtime-manifest.ps1') -Directory $dir -Version $version|Out-Null}
function Run-Helper($mode,$session,$archive){$request=Join-Path $base ([Guid]::NewGuid().ToString('N')+'.json');[IO.File]::WriteAllText($request,(@{root=$root;session=$session;mode=$mode;archive=$archive;parent_pid=0;host_pid=0;restart=$false}|ConvertTo-Json),[Text.UTF8Encoding]::new($false));$output=& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $repo 'apps/wiparse-next/src-tauri/src/runtime_update.ps1') -Request $request 2>&1;$exit=$LASTEXITCODE;return @{exit=$exit;text=($output -join "`n")}}
function Assert($condition,$reason){if(!$condition){throw $reason}}
Fixture $root 'old';Fixture $incoming 'new'
[IO.File]::WriteAllText((Join-Path $root 'config.json'),'user-secret-config')
$user=Join-Path $root 'test-tools/projects/my-work/data.txt';[IO.Directory]::CreateDirectory((Split-Path $user))|Out-Null;[IO.File]::WriteAllText($user,'user-project')
$zip=Join-Path $base 'next.zip';[IO.Compression.ZipFile]::CreateFromDirectory($incoming,$zip)
$session=Join-Path $root ('.updates/'+[Guid]::NewGuid().ToString())
$stage=Run-Helper 'stage' $session $zip;Assert ($stage.exit -eq 0) "stage failed: $($stage.text)"
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-old-*') 'Staging changed installed binary'
$apply=Run-Helper 'apply' $session '';Assert ($apply.exit -eq 0) "apply failed: $($apply.text)"
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-new-*') 'Apply did not replace complete runtime'
Assert ((Get-Content -LiteralPath (Join-Path $root 'config.json') -Raw) -eq 'user-secret-config') 'Personal configuration changed'
Assert ((Get-Content -LiteralPath $user -Raw) -eq 'user-project') 'User project changed'
$rollback=Run-Helper 'rollback' $session '';Assert ($rollback.exit -eq 0) "rollback failed: $($rollback.text)"
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-old-*') 'Rollback failed'
# A change after staging must fail before touching any installed managed file.
$session2=Join-Path $root ('.updates/'+[Guid]::NewGuid().ToString());$stage2=Run-Helper 'stage' $session2 $zip;Assert ($stage2.exit -eq 0) $stage2.text
[IO.File]::AppendAllText((Join-Path $root 'bin/WiParse-CLI.exe'),'local-edit')
$conflict=Run-Helper 'apply' $session2 '';Assert ($conflict.exit -ne 0) 'Concurrent modification was not rejected'
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-old-*') 'Conflict modified a different file'
# Recovery after interrupted application restores files from the pre-change backup.
$journal=Get-Content -LiteralPath (Join-Path $session 'journal.json') -Raw|ConvertFrom-Json;$journal.phase='applying';$journal|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $session 'journal.json') -Encoding UTF8
[IO.File]::WriteAllText((Join-Path $root 'bin/WiParse.exe'),'interrupted-write')
$recover=Run-Helper 'rollback' $session '';Assert ($recover.exit -eq 0) $recover.text
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-old-*') 'Interrupted update recovery failed'
# Consecutive updates can only roll back the version currently installed.
$sessionB=Join-Path $root ('.updates/'+[Guid]::NewGuid().ToString());$b=Run-Helper 'stage' $sessionB $zip;Assert ($b.exit -eq 0) $b.text;$b=Run-Helper 'apply' $sessionB '';Assert ($b.exit -eq 0) $b.text
Fixture $incoming 'newer';$newerZip=Join-Path $base 'newer.zip';[IO.Compression.ZipFile]::CreateFromDirectory($incoming,$newerZip)
$sessionC=Join-Path $root ('.updates/'+[Guid]::NewGuid().ToString());$c=Run-Helper 'stage' $sessionC $newerZip;Assert ($c.exit -eq 0) $c.text;$c=Run-Helper 'apply' $sessionC '';Assert ($c.exit -eq 0) $c.text
$stale=Run-Helper 'rollback' $sessionB '';Assert ($stale.exit -ne 0) 'Stale rollback mixed runtime versions'
Assert ((Get-Content -LiteralPath (Join-Path $root 'bin/WiParse.exe') -Raw) -like 'fake-newer-*') 'Rejected rollback changed runtime'
$latest=Run-Helper 'rollback' $sessionC '';Assert ($latest.exit -eq 0) $latest.text
$replay=Run-Helper 'rollback' $sessionC '';Assert ($replay.exit -ne 0) 'Rollback replay accepted'
Assert ((Get-Content -LiteralPath (Join-Path $root 'test-tools/plugins/demo/config.json') -Raw) -like 'fake-new-*') 'Plugin default config did not track its runtime'
# Traversal entries never leave staging or get silently extracted.
$bad=Join-Path $base 'traversal.zip';[IO.File]::Copy($zip,$bad);$archive=[IO.Compression.ZipFile]::Open($bad,[IO.Compression.ZipArchiveMode]::Update);$entry=$archive.CreateEntry('../escape.exe');$writer=[IO.StreamWriter]::new($entry.Open());$writer.Write('bad');$writer.Dispose();$archive.Dispose()
$malicious=Run-Helper 'stage' (Join-Path $root ('.updates/'+[Guid]::NewGuid().ToString())) $bad;Assert ($malicious.exit -ne 0) 'Traversal archive accepted'
Assert (!(Test-Path -LiteralPath (Join-Path $root 'escape.exe'))) 'Traversal wrote outside staging'
$result=@{passed=$true;fixture=$base;checks=@('stage-is-nondestructive','complete-apply','preserve-config-and-project','rollback','concurrent-change-rejected','interrupted-update-recovery','zip-traversal-rejected','consecutive-update-rollback-binding','rollback-replay-rejected','plugin-default-config-managed');real_executable_started=$false}
New-Item -ItemType Directory -Path $EvidenceDirectory -Force|Out-Null
[IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'runtime-update.json'),($result|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
Write-Output 'Runtime update isolation checks passed'
$global:LASTEXITCODE=0
