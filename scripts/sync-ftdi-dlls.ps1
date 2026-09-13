# Copy FTDI FT4222 redistributable DLLs next to WiParse (optional).
# Does not download. Requires a local FTDI install or a source folder.
param(
    [string]$Source = "",
    [string]$Dest = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
if (-not $Dest) {
    $Dest = Join-Path $root "vendor\ftdi"
}
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

$names = @(
    "LibFT4222.dll", "LibFT4222-64.dll", "libft4222.dll",
    "ftd2xx.dll", "FTD2XX.dll", "FTD2XX64.dll"
)

$roots = @()
if ($Source) { $roots += $Source }
$pf = ${env:ProgramFiles}
$pf86 = ${env:ProgramFiles(x86)}
if ($pf) { $roots += (Join-Path $pf "FTDI") }
if ($pf86) { $roots += (Join-Path $pf86 "FTDI") }
if ($env:SystemRoot) { $roots += (Join-Path $env:SystemRoot "System32") }

$copied = @()
foreach ($dir in $roots) {
    if (-not (Test-Path $dir)) { continue }
    Get-ChildItem -Path $dir -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $names -contains $_.Name } |
        ForEach-Object {
            $target = Join-Path $Dest $_.Name
            Copy-Item $_.FullName $target -Force
            $copied += $target
        }
}

$distVendor = Join-Path $root "dist\vendor\ftdi"
if (Test-Path (Join-Path $root "dist")) {
    New-Item -ItemType Directory -Force -Path $distVendor | Out-Null
    Get-ChildItem $Dest -File -Filter *.dll -ErrorAction SilentlyContinue |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $distVendor $_.Name) -Force }
    if (Test-Path (Join-Path $root "dist\WiParse.exe")) {
        Get-ChildItem $Dest -File -Filter *.dll -ErrorAction SilentlyContinue |
            ForEach-Object { Copy-Item $_.FullName (Join-Path $root "dist\$($_.Name)") -Force }
    }
}

if ($copied.Count -eq 0) {
    Write-Host "No FTDI DLLs found. Install LibFT4222 / D2XX, or pass -Source <folder>."
    exit 1
}
Write-Host "Copied:"
$copied | Sort-Object -Unique | ForEach-Object { Write-Host "  $_" }
