$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$output=Join-Path $repo 'dist/WiParse-Next-20261109-Preview-R41'
if(!(Test-Path -LiteralPath (Join-Path $output 'runtime-manifest.json'))){throw 'Staging has not completed'}
$symbols=@(
 @('apps/wiparse-next/src-tauri/target/release/wiparse_next.pdb','wiparse_next.pdb'),
 @('target/release/wiparse_gui.pdb','wiparse_gui.pdb'),
 @('target/release/wiparse.pdb','wiparse.pdb'),
 @('target/release/wiparse_service.pdb','wiparse_service.pdb')
)
foreach($pair in $symbols){$source=Join-Path $repo $pair[0];if(!(Test-Path -LiteralPath $source)){throw "Missing symbols: $source"};Copy-Item -LiteralPath $source -Destination (Join-Path $output ('bin/'+$pair[1]))}
New-Item -ItemType Directory -Path (Join-Path $output 'docs/reviews') -Force|Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'docs/reviews/gui-migration-completion-2026-11-08.md') -Destination (Join-Path $output 'docs/reviews')
Copy-Item -LiteralPath (Join-Path $repo 'docs/NEXT_RUNTIME_UPDATE.md') -Destination (Join-Path $output 'docs')
foreach($version in @('r26','r27','r29','r30','r31','r34','r36','r37','r38','r39','r40','r41')){
 $folder=if($version -in @('r36','r37','r38','r39','r40','r41')){'next-candidate-20261109-'+$version}else{'next-candidate-20261108-'+$version}
 $source=Join-Path $repo ('docs/verification/'+$folder)
 $destination=Join-Path $output ('docs/verification/'+$folder)
 New-Item -ItemType Directory -Path $destination -Force|Out-Null
 foreach($file in Get-ChildItem -LiteralPath $source -File){if($file.Extension -in @('.log','.json','.ps1','.jpg','.csv') -and $file.Name -ne 'test-config.json' -and $file.Name -ne 'delivery.json' -and $file.Name -ne 'deliver.ps1'){Copy-Item -LiteralPath $file.FullName -Destination $destination}}
}
Copy-Item -LiteralPath (Join-Path $repo 'docs/reviews/waveform-regression-restoration-2026-11-09.md') -Destination (Join-Path $output 'docs/reviews')
Copy-Item -LiteralPath (Join-Path $repo 'apps/wiparse-next/MIGRATION-STATUS.md') -Destination (Join-Path $output 'docs/MIGRATION-STATUS.md')
$notes=@'
WiParse Next R41 · 2026-11-09

运行 bin/WiParse.exe，保留相邻运行文件。包含 GUI、Host、CLI、Service、MCP、插件及匹配调试符号。
本轮重点恢复波形分析：多通道默认显示、文件追加、联合适应、Y 缩放/二维平移、原始测量和精确光标、协议时间轴/完整字节搜索/异常筛选/事务定位。
修正单路/双路叠加请求超过原生绘图桶数上限，以及文件关闭后协议信号下拉框误显示已选择的问题。详见 docs/reviews/waveform-regression-restoration-2026-11-09.md。
前端 96 项通过；原生 64 项通过及 SCP 实测来自 R38，R39/R40 原生源码身份不变。桌面验收与继承边界见 docs/verification。
AI 默认关闭，不含用户密钥、工程、日志。首次使用需创建/选择工程。
本次按要求只生成 EXE 预览目录，不创建完整 ZIP，不覆盖现有安装。
Windows 需要 WebView2；插件/MCP 需要 Node.js 24；真实设备仍需相应驱动与 VISA。
真实仪表、真实 AI、多屏 DPI 和长时工业验收仍需现场测试。
'@
[IO.File]::WriteAllText((Join-Path $output 'RELEASE-NOTES.txt'),$notes,[Text.UTF8Encoding]::new($false))
& (Join-Path $repo 'scripts/write-next-runtime-manifest.ps1') -Directory $output -Version 'WiParse-Next-20261109-R41'
$config=Get-Content -LiteralPath (Join-Path $output 'config.json') -Raw|ConvertFrom-Json
if($config.ai.enabled -ne $false){throw 'Preview AI must be disabled'}
foreach($value in @($config.apps.test_tool.plugins_dir,$config.apps.test_tool.cli_path,$config.apps.test_tool.data_root,$config.apps.test_tool.marketplace.install_dir)){if([IO.Path]::IsPathRooted($value)){throw 'Portable defaults must be relative'}}
$hashes=[ordered]@{}
foreach($file in Get-ChildItem -LiteralPath $output -File -Recurse){
 $relative=$file.FullName.Substring($output.Length+1).Replace('\','/')
 if($relative -eq 'files-sha256.json'){continue}
 $hashes[$relative]=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
if($hashes.Count -lt 4000){throw 'Incomplete runtime inventory'}
[IO.File]::WriteAllText((Join-Path $output 'files-sha256.json'),($hashes|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
foreach($entry in $hashes.GetEnumerator()){if((Get-FileHash -LiteralPath (Join-Path $output $entry.Key) -Algorithm SHA256).Hash -ne $entry.Value){throw "Delivery changed: $($entry.Key)"}}
if(@(Get-ChildItem -LiteralPath $output -File -Recurse).Count -ne $hashes.Count+1){throw 'Delivery inventory differs'}
$proof=@{passed=$true;directory=$output;file_count=$hashes.Count+1;inventory_excludes_only_its_own_file=$true;inventory_sha256=(Get-FileHash -LiteralPath (Join-Path $output 'files-sha256.json') -Algorithm SHA256).Hash.ToLowerInvariant();candidate_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'candidate.json') -Algorithm SHA256).Hash.ToLowerInvariant();zip_created=$false;user_installation_modified=$false;symbol_files=4;portable_defaults=$true}
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'delivery.json'),($proof|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
Write-Output ('Delivered '+(Join-Path $output 'bin/WiParse.exe'))
