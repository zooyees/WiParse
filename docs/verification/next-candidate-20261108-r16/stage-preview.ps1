param([string]$Preview='D:\windlink\windlink\WiParse-R\dist\WiParse-Next-20261108-Preview-R16')
$ErrorActionPreference='Stop'
$repo='D:\windlink\windlink\WiParse-R'
$evidence=Join-Path $repo 'docs/verification/next-candidate-20261108-r16'
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
$rows=@(Get-ChildItem -LiteralPath $Preview -Recurse -File|Where-Object {$_.Name -ne 'files-sha256.json'}|Sort-Object FullName|ForEach-Object {@{path=$_.FullName.Substring($Preview.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
[IO.File]::WriteAllText((Join-Path $Preview 'files-sha256.json'),($rows|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output ('Files: '+$rows.Count)
