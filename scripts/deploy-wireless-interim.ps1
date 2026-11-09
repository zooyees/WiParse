param([Parameter(Mandatory=$true)][string]$Package)
$ErrorActionPreference = 'Stop'
$source = [IO.Path]::GetFullPath($Package)
$target = [IO.Path]::GetFullPath('D:\software\WiParse')
if (-not (Test-Path -LiteralPath (Join-Path $source 'release-manifest.json'))) { throw 'Package manifest missing' }
if (-not (Test-Path -LiteralPath (Join-Path $target 'WiParse.exe'))) { throw 'Existing installation not found' }
$running = Get-Process WiParse,WiParse-CLI,WiParse-Service -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($target + '\',[StringComparison]::OrdinalIgnoreCase) }
if ($running) { throw 'Close WiParse processes running from the target installation before updating.' }
$backup = Join-Path $target ('backup-wireless-ft4222-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
if (Test-Path -LiteralPath $backup) { throw 'Backup already exists' }
New-Item -ItemType Directory -Path $backup | Out-Null
$items = @(
  @{Src='bin/WiParse.exe';Dst='WiParse.exe'},
  @{Src='bin/WiParse-CLI.exe';Dst='WiParse-CLI.exe'},
  @{Src='bin/WiParse-Service.exe';Dst='WiParse-Service.exe'},
  @{Src='test-tools/lib/host/fpga-host-adapter.mjs';Dst='test-tools/lib/host/fpga-host-adapter.mjs'},
  @{Src='test-tools/lib/hub/run-binding.mjs';Dst='test-tools/lib/hub/run-binding.mjs'}
)
$plugin = Join-Path $source 'test-tools/plugins/wireless-whitebox'
foreach ($file in Get-ChildItem -LiteralPath $plugin -Recurse -File) {
  $rel = [IO.Path]::GetRelativePath($source,$file.FullName)
  $items += @{Src=$rel;Dst=$rel}
}
$completed = [System.Collections.Generic.List[object]]::new()
try {
  foreach ($item in $items) {
    $src=Join-Path $source $item.Src; $dst=Join-Path $target $item.Dst; $bak=Join-Path $backup $item.Dst
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) { throw "Missing package file $src" }
    $existed=Test-Path -LiteralPath $dst -PathType Leaf
    if ($existed) { New-Item -ItemType Directory -Force -Path (Split-Path -Parent $bak) | Out-Null; Copy-Item -LiteralPath $dst -Destination $bak }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dst) | Out-Null
    $completed.Add(@{Dst=$dst;Bak=$bak;Existed=$existed})
    Copy-Item -LiteralPath $src -Destination $dst -Force
    if ((Get-FileHash -LiteralPath $src -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $dst -Algorithm SHA256).Hash) { throw "Hash mismatch $dst" }
  }
} catch {
  for ($i=$completed.Count-1; $i -ge 0; $i--) {
    $item=$completed[$i]
    if ($item.Existed) { Copy-Item -LiteralPath $item.Bak -Destination $item.Dst -Force }
    elseif (Test-Path -LiteralPath $item.Dst) { Remove-Item -LiteralPath $item.Dst -Force }
  }
  throw
}
@{package=$source;backup=$backup;files=$items.Count;verified=$true} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $backup 'deployment.json') -Encoding utf8
Write-Output "Updated $($items.Count) files; backup: $backup"
