# Publish WiParse release artifacts and manifest to the update server.
# Usage:
#   .\publish-release.ps1 -Version 1.0.2 -DeployRoot "\\server\share\wiparse" -Notes "Bug fixes"

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$DeployRoot,
    [string]$Channel = "stable",
    [string]$Notes = "",
    [string]$ZipPath = "..\..\WiParse-Deploy.zip"
)

$ErrorActionPreference = "Stop"
$releaseDir = Join-Path $DeployRoot "releases\$Version"
New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null

$zipName = "WiParse-$Version-win64.zip"
$destZip = Join-Path $releaseDir $zipName
Copy-Item -Force $ZipPath $destZip

$hash = (Get-FileHash -Path $destZip -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $destZip).Length

$manifest = @{
    product = "wiparse"
    channel = $Channel
    version = $Version
    min_version = "1.0.0"
    published_at = (Get-Date).ToUniversalTime().ToString("o")
    notes = $Notes
    packages = @(
        @{
            target = "windows-x64"
            url = "https://YOUR-UPDATE-HOST/wiparse/releases/$Version/$zipName"
            size = $size
            sha256 = $hash
            filename = $zipName
        }
    )
} | ConvertTo-Json -Depth 5

$manifestPath = Join-Path $DeployRoot "$Channel\latest.json"
New-Item -ItemType Directory -Force -Path (Split-Path $manifestPath) | Out-Null
Set-Content -Path $manifestPath -Value $manifest -Encoding UTF8

Write-Host "Published $destZip"
Write-Host "Manifest: $manifestPath"
Write-Host "SHA256: $hash"
