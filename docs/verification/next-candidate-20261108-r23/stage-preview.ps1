param([string]$Preview='D:\windlink\windlink\WiParse-R\dist\WiParse-Next-20261108-Preview-R23')
$ErrorActionPreference='Stop'
$repo='D:\windlink\windlink\WiParse-R'
$evidence=Join-Path $repo 'docs/verification/next-candidate-20261108-r23'
& (Join-Path $repo 'scripts/stage-next-preview.ps1') -VerificationManifest (Join-Path $evidence 'candidate.json') -OutputDirectory $Preview
New-Item -ItemType Directory -Path (Join-Path $Preview 'symbols') -Force|Out-Null
foreach($file in @('target/release/wiparse_gui.pdb','target/release/wiparse.pdb','target/release/wiparse_service.pdb','apps/wiparse-next/src-tauri/target/release/wiparse_next.pdb')){Copy-Item -LiteralPath (Join-Path $repo $file) -Destination (Join-Path $Preview 'symbols')}
foreach($file in @('gui-system-progress-review-2026-11-08-r23.md','waveform-viewport-index-review-2026-11-08.md')){Copy-Item -LiteralPath (Join-Path $repo "docs/reviews/$file") -Destination (Join-Path $Preview 'docs')}
# Keep evidence at the relative paths referenced by the original reports. R23 is a
# full verification; R21 is historical performance evidence, not a parent reuse.
foreach($number in @(21,22,23)){
 $source=Join-Path $repo "docs/verification/next-candidate-20261108-r$number"
 $target=Join-Path $Preview "docs/verification/next-candidate-20261108-r$number"
 New-Item -ItemType Directory -Path $target -Force|Out-Null
 Get-ChildItem -LiteralPath $source -File|Where-Object {$_.Extension -eq '.log' -or $_.Name -match '^(candidate|source-before|environment|scp-sample|release-benchmark)\.json$'}|ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination $target}
}
$readme=@'
WiParse Next R23 修正预览

运行 bin\WiParse.exe，保留完整运行目录，不要单独复制 EXE。
测试 R20 时请先正常关闭其窗口，再运行本版，避免共用 companion 端口。
Windows WebView2、Node.js 24、VISA/设备驱动仍是系统前提。
默认 AI 关闭，不包含用户 Key、个人配置和报告，不覆盖已有安装。

变更：精确视口索引；实时数据规则切换隔离；报告图片队列取消；
四个保存入口取消 Windows 硬链接依赖；损坏 WFM 帧头预分配保护。
详情见 docs/gui-system-progress-review-2026-11-08-r23.md。

自动验证及构建结果见 candidate-verification.json。
本批没有操作用户正在运行的 R20，未做实机/真实模型/完整 DPI/长时验收。
不代表正式工业验收通过。此目录仅 EXE 预览，未生成新 ZIP。
'@
[IO.File]::WriteAllText((Join-Path $Preview 'PREVIEW.txt'),$readme,[Text.UTF8Encoding]::new($false))
$status=Join-Path $Preview 'docs/MIGRATION-STATUS.md'
[IO.File]::WriteAllText($status,([IO.File]::ReadAllText($status).Replace('../../docs/reviews/','')),[Text.UTF8Encoding]::new($false))
# Reports are moved from docs/reviews into docs in the portable preview.
foreach($name in @('gui-system-progress-review-2026-11-08-r23.md','waveform-viewport-index-review-2026-11-08.md')){
 $path=Join-Path $Preview "docs/$name"
 [IO.File]::WriteAllText($path,([IO.File]::ReadAllText($path).Replace('../verification/','verification/')),[Text.UTF8Encoding]::new($false))
}
$manifestPath=Join-Path $Preview 'preview-manifest.json'
$manifest=Get-Content -Raw -LiteralPath $manifestPath|ConvertFrom-Json
$symbols=[ordered]@{}
Get-ChildItem -LiteralPath (Join-Path $Preview 'symbols') -File|ForEach-Object {$symbols[$_.Name]=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
$manifest|Add-Member -NotePropertyName symbol_sha256 -NotePropertyValue $symbols -Force
$manifest|Add-Member -NotePropertyName verification_evidence -NotePropertyValue 'docs/verification/next-candidate-20261108-r23/candidate.json' -Force
[IO.File]::WriteAllText($manifestPath,($manifest|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
$rows=@(Get-ChildItem -LiteralPath $Preview -Recurse -File|Sort-Object FullName|ForEach-Object {@{path=$_.FullName.Substring($Preview.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})
[IO.File]::WriteAllText((Join-Path $Preview 'files-sha256.json'),($rows|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output ('Files: '+$rows.Count)
