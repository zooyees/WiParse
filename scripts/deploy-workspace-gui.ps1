param([Parameter(Mandatory=$true)][string]$Package)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'WiParse.exe'
$source = Join-Path ([IO.Path]::GetFullPath($Package)) 'bin/WiParse.exe'
if (-not (Test-Path -LiteralPath $target -PathType Leaf) -or -not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'GUI source or target is missing.' }
$running = Get-Process WiParse -ErrorAction SilentlyContinue | Where-Object { $_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq [IO.Path]::GetFullPath($target) }
if ($running) { throw 'Close the workspace WiParse before updating it.' }
$backup = Join-Path $root ('backup-spacing-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
if (Test-Path -LiteralPath $backup) { throw 'Backup already exists.' }
New-Item -ItemType Directory -Path $backup | Out-Null
$original = Join-Path $backup 'WiParse.exe'
Copy-Item -LiteralPath $target -Destination $original
$symbolSource = Join-Path ([IO.Path]::GetFullPath($Package)) 'symbols/wiparse_gui.pdb'
$symbolTarget = Join-Path $root 'WiParse.pdb'
$symbolBackup = Join-Path $backup 'WiParse.pdb'
$symbolExisted = Test-Path -LiteralPath $symbolTarget -PathType Leaf
if (-not (Test-Path -LiteralPath $symbolSource -PathType Leaf)) { throw 'Matching GUI symbols are missing.' }
if ($symbolExisted) { Copy-Item -LiteralPath $symbolTarget -Destination $symbolBackup }
try {
    Copy-Item -LiteralPath $source -Destination $target -Force
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash) { throw 'GUI checksum mismatch.' }
    Copy-Item -LiteralPath $symbolSource -Destination $symbolTarget -Force
    if ((Get-FileHash -LiteralPath $symbolSource -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $symbolTarget -Algorithm SHA256).Hash) { throw 'GUI symbols checksum mismatch.' }
} catch {
    Copy-Item -LiteralPath $original -Destination $target -Force
    if ($symbolExisted) { Copy-Item -LiteralPath $symbolBackup -Destination $symbolTarget -Force }
    elseif (Test-Path -LiteralPath $symbolTarget) { Remove-Item -LiteralPath $symbolTarget -Force }
    throw
}
Write-Output "Updated workspace GUI; backup: $backup"
