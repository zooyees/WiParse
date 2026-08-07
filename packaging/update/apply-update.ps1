param(
    [Parameter(Mandatory = $true)][string]$ZipPath,
    [Parameter(Mandatory = $true)][string]$InstallDir,
    [Parameter(Mandatory = $true)][string]$WaitPid
)

$ErrorActionPreference = "Stop"

Write-Host "WiParse update: waiting for PID $WaitPid ..."
while (Get-Process -Id $WaitPid -ErrorAction SilentlyContinue) {
    Start-Sleep -Milliseconds 400
}

$staging = Join-Path $env:TEMP "WiParse-update-staging"
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Path $staging | Out-Null

Write-Host "Extracting $ZipPath ..."
Expand-Archive -Path $ZipPath -DestinationPath $staging -Force

$exe = Join-Path $InstallDir "WiParse.exe"
$bak = "$exe.bak"
if (Test-Path $exe) {
    if (Test-Path $bak) { Remove-Item -Force $bak }
    Move-Item -Force $exe $bak
}

Get-ChildItem -Path $staging -Recurse -File | ForEach-Object {
    $rel = $_.FullName.Substring($staging.Length).TrimStart('\')
    $dest = Join-Path $InstallDir $rel
    $parent = Split-Path $dest -Parent
    if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent | Out-Null }
    Copy-Item -Force $_.FullName $dest
}

Write-Host "Restarting WiParse ..."
Start-Process -FilePath $exe
Remove-Item -Recurse -Force $staging
