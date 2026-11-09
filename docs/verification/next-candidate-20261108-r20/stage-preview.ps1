param([string]$Preview='D:\windlink\windlink\WiParse-R\dist\WiParse-Next-20261108-Preview-R20')
$ErrorActionPreference='Stop'
$repo='D:\windlink\windlink\WiParse-R'
$evidence=Join-Path $repo 'docs/verification/next-candidate-20261108-r20'
& (Join-Path $repo 'scripts/stage-next-preview.ps1') -VerificationManifest (Join-Path $evidence 'candidate.json') -OutputDirectory $Preview
New-Item -ItemType Directory -Path (Join-Path $Preview 'symbols') -Force|Out-Null
foreach($file in @('target/release/wiparse_gui.pdb','target/release/wiparse.pdb','target/release/wiparse_service.pdb','apps/wiparse-next/src-tauri/target/release/wiparse_next.pdb')){Copy-Item -LiteralPath (Join-Path $repo $file) -Destination (Join-Path $Preview 'symbols')}
Copy-Item -LiteralPath (Join-Path $repo 'dist/WiParse-Next-20261108-Preview-R12/examples') -Destination $Preview -Recurse
foreach($file in @('gui-migration-candidate-review-2026-11-08.md')){Copy-Item -LiteralPath (Join-Path $repo "docs/reviews/$file") -Destination (Join-Path $Preview 'docs')}
Copy-Item -LiteralPath (Join-Path $repo 'dist/WiParse-Next-20261108-Preview-R12/PREVIEW.txt') -Destination $Preview
$manifestPath=Join-Path $Preview 'preview-manifest.json'
$manifest=Get-Content -Raw -LiteralPath $manifestPath|ConvertFrom-Json
$symbols=[ordered]@{}
Get-ChildItem -LiteralPath (Join-Path $Preview 'symbols') -File|ForEach-Object {$symbols[$_.Name]=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest|Add-Member -NotePropertyName symbol_sha256 -NotePropertyValue $symbols -Force
[IO.File]::WriteAllText($manifestPath,($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
# Preserve the verification chain at the same relative paths as the original evidence.
foreach($number in @(16,17,18,19,20)){
 $source=Join-Path $repo "docs/verification/next-candidate-20261108-r$number"
 $target=Join-Path $Preview "docs/verification/next-candidate-20261108-r$number"
 New-Item -ItemType Directory -Path $target -Force|Out-Null
 Get-ChildItem -LiteralPath $source -File|Where-Object {$_.Name -match '^(candidate.*|source-before|environment|scp-sample|desktop-partial)\.json$' -or $_.Extension -eq '.log'}|ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination $target}
}
$dpapi=Join-Path $Preview 'docs/verification/next-candidate-20261108-r4'
New-Item -ItemType Directory -Path $dpapi -Force|Out-Null
foreach($name in @('normal-user-dpapi.json','source-before.json','ai_provider_authorization_uses_dpapi_credentials.log','protected_key_validation_accepts_current_user_ciphertext_and_rejects_corruption.log','ai_key_editor_encrypts_new_key_in_interactive_user_profile.log','protected_ai_token_file_is_user_bound_and_removed_by_its_owner.log')){Copy-Item -LiteralPath (Join-Path $repo "docs/verification/next-candidate-20261108-r4/$name") -Destination $dpapi}
Copy-Item -LiteralPath (Join-Path $repo 'dist/WiParse-Next-20261108-R16-DesktopValidation/examples/gui-preview/scope-comparison.csv') -Destination (Join-Path $Preview 'examples/gui-preview')
Copy-Item -LiteralPath (Join-Path $evidence 'desktop-partial.json') -Destination (Join-Path $Preview 'docs/desktop-partial.json')
$manifest=Get-Content -Raw -LiteralPath $manifestPath|ConvertFrom-Json
$manifest|Add-Member -Force -NotePropertyName desktop_evidence -NotePropertyValue 'docs/desktop-partial.json'
$manifest|Add-Member -Force -NotePropertyName verification_evidence -NotePropertyValue 'docs/verification/next-candidate-20261108-r20/candidate.json'
[IO.File]::WriteAllText($manifestPath,($manifest|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
$portableStatus=Join-Path $Preview 'docs/MIGRATION-STATUS.md'
$statusText=[IO.File]::ReadAllText($portableStatus).Replace('../../docs/reviews/','')
[IO.File]::WriteAllText($portableStatus,$statusText,[Text.UTF8Encoding]::new($false))
$rows=@(Get-ChildItem -LiteralPath $Preview -Recurse -File|Where-Object {$_.Name -ne 'files-sha256.json'}|Sort-Object FullName|ForEach-Object {@{path=$_.FullName.Substring($Preview.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
[IO.File]::WriteAllText((Join-Path $Preview 'files-sha256.json'),($rows|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output ('Files: '+$rows.Count)
