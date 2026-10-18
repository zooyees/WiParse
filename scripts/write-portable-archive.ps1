param([Parameter(Mandatory=$true)][string]$SourceDirectory,[Parameter(Mandatory=$true)][string]$Archive)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
$distPrefix=[IO.Path]::GetFullPath((Join-Path $repoRoot 'dist')).TrimEnd('\')+'\'
$source=(Resolve-Path -LiteralPath $SourceDirectory).Path
$target=[IO.Path]::GetFullPath($Archive)
if(-not $source.StartsWith($distPrefix,[StringComparison]::OrdinalIgnoreCase) -or -not $target.StartsWith($distPrefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Archive inputs and output must stay below workspace dist'}
if($target.StartsWith($source.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Archive cannot be placed inside its source'}
if(Get-ChildItem -LiteralPath $source -Recurse -Force | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint } | Select-Object -First 1){throw 'Archive source contains a reparse point'}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$temporary=$target+'.tmp-'+[Guid]::NewGuid().ToString('N')
try {
    # Explicit UTF-8 is essential: Windows bsdtar can omit the ZIP language flag.
    [IO.Compression.ZipFile]::CreateFromDirectory($source,$temporary,[IO.Compression.CompressionLevel]::Optimal,$true,[Text.UTF8Encoding]::new($false))
    if(Test-Path -LiteralPath $target){[IO.File]::Replace($temporary,$target,$null)}else{[IO.File]::Move($temporary,$target)}
}finally{if(Test-Path -LiteralPath $temporary){Remove-Item -LiteralPath $temporary -Force}}
Get-Item -LiteralPath $target | Select-Object Name,Length,LastWriteTime
