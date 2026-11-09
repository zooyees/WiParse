$ErrorActionPreference='Stop'
$preview='D:\windlink\windlink\WiParse-R\dist\WiParse-Next-20261108-Preview-R20'
$verification=Get-Content -Raw -LiteralPath (Join-Path $preview 'candidate-verification.json')|ConvertFrom-Json
if(-not $verification.automatic_passed -or -not $verification.sources_unchanged){throw 'Candidate did not pass'}
$manifest=Get-Content -Raw -LiteralPath (Join-Path $preview 'preview-manifest.json')|ConvertFrom-Json
foreach($entry in $manifest.binary_sha256.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $preview ('bin/'+$entry.Name)) -Algorithm SHA256).Hash -ne $entry.Value){throw ('EXE mismatch: '+$entry.Name)}}
foreach($entry in $manifest.symbol_sha256.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $preview ('symbols/'+$entry.Name)) -Algorithm SHA256).Hash -ne $entry.Value){throw ('PDB mismatch: '+$entry.Name)}}
foreach($entry in $verification.runtime_hashes.PSObject.Properties){if((Get-FileHash -LiteralPath (Join-Path $preview $entry.Name) -Algorithm SHA256).Hash -ne $entry.Value){throw 'MCP mismatch'}}
foreach($number in @(17,18,19,20)){
 $child=Get-Content -Raw -LiteralPath (Join-Path $preview "docs/verification/next-candidate-20261108-r$number/candidate.json")|ConvertFrom-Json
 $parent=Join-Path $preview "docs/verification/next-candidate-20261108-r$($number-1)/candidate.json"
 if((Get-FileHash -LiteralPath $parent -Algorithm SHA256).Hash -ne $child.parent_verification_sha256){throw ('Verification chain mismatch: '+$number)}
}
Write-Output 'EXE/PDB/MCP hashes and verification chain passed'
$rows=@(Get-ChildItem -LiteralPath $preview -Recurse -File|Where-Object {$_.Name -ne 'files-sha256.json'}|Sort-Object FullName|ForEach-Object {@{path=$_.FullName.Substring($preview.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
[IO.File]::WriteAllText((Join-Path $preview 'files-sha256.json'),($rows|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
$bad=@($rows|Where-Object {(Get-Item -LiteralPath (Join-Path $preview $_.path)).Length -ne $_.bytes -or (Get-FileHash -LiteralPath (Join-Path $preview $_.path) -Algorithm SHA256).Hash -ne $_.sha256})
if($bad.Count){throw 'Package file verification failed'}
$summary=[ordered]@{files=$rows.Count;bytes=($rows|Measure-Object bytes -Sum).Sum;all_hashes_verified=$true;zip_created=$false;industrial_approved=$false}
$summary|ConvertTo-Json|Write-Output
[IO.File]::WriteAllText('D:\windlink\windlink\WiParse-R\docs\verification\next-candidate-20261108-r20\delivery.json',($summary|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
