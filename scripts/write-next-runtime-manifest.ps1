param([Parameter(Mandatory=$true)][string]$Directory,[Parameter(Mandatory=$true)][string]$Version,[long]$Build=([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()))
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath($Directory).TrimEnd('\')
if(!(Test-Path -LiteralPath (Join-Path $root 'bin/WiParse.exe'))){throw 'Not a Next runtime directory'}
$files=@()
foreach($tree in @('bin','mcp','test-tools','recipes','Icon')){
 $path=Join-Path $root $tree;if(!(Test-Path -LiteralPath $path)){continue}
 foreach($f in Get-ChildItem -LiteralPath $path -File -Recurse){
  $relative=$f.FullName.Substring($root.Length+1).Replace('\','/')
  if($relative -match '(^|/)(projects|runs|logs|log|cache|\.git|\.updates)/' -or $relative -match '(^|/)\.env$'){continue}
  $files+=@{path=$relative;bytes=$f.Length;sha256=(Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
 }
}
foreach($name in @('config.default.json','launch.cmd')){$f=Join-Path $root $name;if(Test-Path -LiteralPath $f){$files+=@{path=$name;bytes=(Get-Item -LiteralPath $f).Length;sha256=(Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant()}}}
$manifest=@{product='wiparse-next';schema=1;version=$Version;build=$Build;files=@($files|Sort-Object path)}
[IO.File]::WriteAllText((Join-Path $root 'runtime-manifest.json'),($manifest|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
Write-Output "Runtime manifest: $($files.Count) managed files"
