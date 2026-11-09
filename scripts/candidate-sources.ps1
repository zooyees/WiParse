# Shared candidate source identity; importing this file performs no checks or writes.
function Get-WiParseCandidateSources {
    param([Parameter(Mandatory=$true)][string]$Repo)
    $repo=[IO.Path]::GetFullPath($Repo)
    $roots=@('third_party','Icon','crates','apps/wiparse-next/src','apps/wiparse-next/src-tauri/src','apps/wiparse-next/src-tauri/capabilities','apps/wiparse-next/src-tauri/icons','apps/wiparse-next/public','apps/wiparse-next/tests','apps/wiparse-next/tools','test-tools','mcp/wiparse','recipes','scripts')
    $searchRoots=@($roots|ForEach-Object {Join-Path $repo $_}|Where-Object {Test-Path -LiteralPath $_ -PathType Container})
    $files=@(& rg --files @searchRoots -g '!**/node_modules/**' -g '!**/target/**' -g '!**/dist/**' -g '!**/runs/**' -g '!**/logs/**' -g '!**/reports/**' -g '!**/tmp/**' -g '!**/.git/**' -g '*.rs' -g '*.ts' -g '*.tsx' -g '*.css' -g '*.json' -g '*.mjs' -g '*.js' -g '*.toml' -g '*.lock' -g '*.ps1' -g '*.py' -g '*.svg' -g '*.png' -g '*.ico')
    if($LASTEXITCODE -ne 0){throw 'Cannot enumerate candidate sources'}
    $files=@($files|ForEach-Object {([IO.Path]::GetFullPath($_)).Substring($repo.TrimEnd('\').Length+1)})
    $files+=@('Cargo.toml','Cargo.lock','apps/wiparse-next/package.json','apps/wiparse-next/package-lock.json','apps/wiparse-next/index.html','apps/wiparse-next/tsconfig.json','apps/wiparse-next/vite.config.ts','apps/wiparse-next/src-tauri/Cargo.toml','apps/wiparse-next/src-tauri/Cargo.lock','apps/wiparse-next/src-tauri/build.rs','apps/wiparse-next/src-tauri/tauri.conf.json')
    $hashes=[ordered]@{}
    foreach($file in ($files|Sort-Object -Unique)){
        $path=Join-Path $repo $file
        if(Test-Path -LiteralPath $path -PathType Leaf){$hashes[$file.Replace('\','/')]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()}
    }
    return $hashes
}
