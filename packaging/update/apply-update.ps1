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

$operationId = [Guid]::NewGuid().ToString("N")
$staging = Join-Path $env:TEMP "WiParse-update-$operationId\staging"
$backupRoot = Join-Path $env:TEMP "WiParse-update-$operationId\backup"
New-Item -ItemType Directory -Force -Path $staging, $backupRoot | Out-Null

Write-Host "Extracting $ZipPath ..."
Expand-Archive -Path $ZipPath -DestinationPath $staging -Force

$installFull = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\') + '\'
$files = @(Get-ChildItem -LiteralPath $staging -Recurse -File)
$stagedExe = Join-Path $staging "WiParse.exe"
if (-not (Test-Path -LiteralPath $stagedExe -PathType Leaf)) {
    throw "update archive does not contain WiParse.exe at its root"
}
$created = [Collections.Generic.List[string]]::new()
$replaced = [Collections.Generic.List[object]]::new()

try {
    foreach ($file in $files) {
        $rel = $file.FullName.Substring($staging.Length).TrimStart('\')
        $dest = [IO.Path]::GetFullPath((Join-Path $InstallDir $rel))
        if (-not $dest.StartsWith($installFull, [StringComparison]::OrdinalIgnoreCase)) {
            throw "archive path escapes install directory: $rel"
        }
        $parent = Split-Path $dest -Parent
        if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
        if (Test-Path -LiteralPath $dest) {
            $backup = Join-Path $backupRoot $rel
            $backupParent = Split-Path $backup -Parent
            New-Item -ItemType Directory -Force -Path $backupParent | Out-Null
            Copy-Item -LiteralPath $dest -Destination $backup -Force
            $replaced.Add([pscustomobject]@{ Destination = $dest; Backup = $backup })
        } else {
            $created.Add($dest)
        }
        Copy-Item -LiteralPath $file.FullName -Destination $dest -Force
    }

    $exe = Join-Path $InstallDir "WiParse.exe"
    Write-Host "Restarting WiParse ..."
    $process = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 2
    if ($process.HasExited) {
        throw "new WiParse exited immediately with code $($process.ExitCode)"
    }
} catch {
    Write-Host "Update failed; restoring previous installation: $($_.Exception.Message)"
    foreach ($path in $created) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    foreach ($item in $replaced) {
        Copy-Item -LiteralPath $item.Backup -Destination $item.Destination -Force
    }
    throw
} finally {
    $operationRoot = Split-Path $staging -Parent
    if (Test-Path -LiteralPath $operationRoot) { Remove-Item -LiteralPath $operationRoot -Recurse -Force }
}
