# Stage a runnable EXE preview. Deliberately never creates a ZIP or changes an old preview.
param([string]$OutputDirectory)
$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$version=[regex]::Match([IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml')),'(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if(-not $OutputDirectory){$OutputDirectory=Join-Path $repo "dist/wiparse-$version-preview"}
$output=[IO.Path]::GetFullPath($OutputDirectory)
$distPrefix=[IO.Path]::GetFullPath((Join-Path $repo 'dist')).TrimEnd('\')+'\'
if(-not $output.StartsWith($distPrefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Preview must be inside workspace dist'}
if(Test-Path -LiteralPath $output){throw 'Choose a new preview directory; existing previews are preserved'}
foreach($name in @('wiparse-gui.exe','wiparse.exe','wiparse-service.exe')){
 $item=Get-Item -LiteralPath (Join-Path $repo "target/release/$name")
 if($item.VersionInfo.ProductVersion -and $item.VersionInfo.ProductVersion -ne $version){throw "Build version mismatch: $name"}
}
New-Item -ItemType Directory -Path (Join-Path $output 'bin') -Force|Out-Null
function Copy-PreviewTree($source,$destination,$exclude=@()){
 New-Item -ItemType Directory -Path $destination -Force|Out-Null
 & robocopy $source $destination /E /NFL /NDL /NJH /NJS /nc /ns /np /XJ /XD .git logs @exclude /XF run.stop run.lock _loop_status.json _loop_hint.txt '*.generated.json' | Out-Null
 if($LASTEXITCODE -ge 8){throw "Preview copy failed: $source"}
}
foreach($pair in @(@('wiparse-gui.exe','WiParse.exe'),@('wiparse.exe','WiParse-CLI.exe'),@('wiparse-service.exe','WiParse-Service.exe'),@('wiparse_gui.pdb','WiParse.pdb'))){Copy-Item -LiteralPath (Join-Path $repo "target/release/$($pair[0])") -Destination (Join-Path $output "bin/$($pair[1])")}
Copy-PreviewTree (Join-Path $repo 'test-tools') (Join-Path $output 'test-tools')
New-Item -ItemType Directory -Path (Join-Path $output 'docs/reviews') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'docs/reviews/engineering-agent-candidate-2026-11-11.md') -Destination (Join-Path $output 'docs/reviews')
foreach ($name in @("engineering-agent-$version.md",'ai-wiparse-engineering-fit-2026-11-11.md','ai-agent-intelligence-review-2026-11-10.md')) {
 $path=Join-Path $repo "docs/reviews/$name"
 if(Test-Path -LiteralPath $path){Copy-Item -LiteralPath $path -Destination (Join-Path $output 'docs/reviews')}
}
Copy-PreviewTree (Join-Path $repo 'mcp/wiparse') (Join-Path $output 'mcp/wiparse')
Copy-PreviewTree (Join-Path $repo 'recipes/fpga-firmware-test') (Join-Path $output 'recipes/fpga-firmware-test')
if(Test-Path -LiteralPath (Join-Path $repo 'docs/specs/wireless-test')){Copy-PreviewTree (Join-Path $repo 'docs/specs/wireless-test') (Join-Path $output 'docs/specs/wireless-test')}
Copy-Item -LiteralPath (Join-Path $repo 'config.default.json') -Destination (Join-Path $output 'config.default.json')
$config=@{ui=@{language='zh';theme='dark'};apps=@{test_tool=@{plugins_dir='test-tools/plugins';cli_path='bin/WiParse-CLI.exe';node_path='node';data_root='.';marketplace=@{enabled=$false;install_dir='marketplace'}}}}
[IO.File]::WriteAllText((Join-Path $output 'config.json'),($config|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
New-Item -ItemType Directory -Path (Join-Path $output 'docs') -Force|Out-Null
if(Test-Path -LiteralPath (Join-Path $repo 'docs/licenses')){Copy-PreviewTree (Join-Path $repo 'docs/licenses') (Join-Path $output 'docs/licenses')}
foreach($doc in @('HUB_PROJECT_CONFIG_CONTRACT.md','HUB_UI_FIELDS_CONTRACT.md','AI_INTERFACE.md','CHANGELOG.md')){Copy-Item -LiteralPath (Join-Path $repo "docs/$doc") -Destination (Join-Path $output "docs/$doc")}
if(Test-Path -LiteralPath (Join-Path $repo "docs/ui-testing-hub-$version")){Copy-PreviewTree (Join-Path $repo "docs/ui-testing-hub-$version") (Join-Path $output "docs/ui-testing-hub-$version")}
$releaseNote=Join-Path $repo "docs/releases/$version.md"
if(Test-Path -LiteralPath $releaseNote){
 New-Item -ItemType Directory -Path (Join-Path $output 'docs/releases') -Force | Out-Null
 Copy-Item -LiteralPath $releaseNote -Destination (Join-Path $output 'docs/releases')
 $notes=[IO.File]::ReadAllText($releaseNote).Replace('](../reviews/','](reviews/')
 [IO.File]::WriteAllText((Join-Path $output 'docs/release-notes.md'),$notes,[Text.UTF8Encoding]::new($false))
}
# The shipped demo recipes are legacy migration sources. Keep them untouched so
# users can inspect the plan and explicitly commit within the preview UI.
$readme="WiParse $version EXE preview`r`n`r`nRun bin\WiParse.exe in this directory; keep adjacent dependencies. Node.js 18+ is required for test plugins.`r`nThe 20 shipped FPGA examples retain their original parameters. Open Project settings, preview the legacy migration, inspect mappings, then explicitly commit. Existing user data is not migrated automatically.`r`nNo ZIP has been generated. Full packaging follows user confirmation.`r`n"
[IO.File]::WriteAllText((Join-Path $output 'PREVIEW.txt'),$readme,[Text.UTF8Encoding]::new($false))
$hashes=@{}
foreach($name in @('WiParse.exe','WiParse-CLI.exe','WiParse-Service.exe','WiParse.pdb')){
 $sha=[Security.Cryptography.SHA256]::Create()
 $stream=[IO.File]::OpenRead((Join-Path $output "bin/$name"))
 try{$hashes[$name]=[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','').ToLowerInvariant()}
 finally{$stream.Dispose();$sha.Dispose()}
}
$manifest=@{kind='exe_preview';version=$version;created_at=[DateTime]::UtcNow.ToString('o');zip_created=$false;industrial_approved=$false;binary_sha256=$hashes}
[IO.File]::WriteAllText((Join-Path $output 'preview-manifest.json'),($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
$global:LASTEXITCODE=0
Write-Output (Join-Path $output 'bin/WiParse.exe')
