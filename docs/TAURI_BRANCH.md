# Tauri 新框架开发分支

分支：`codex/tauri-gui-migration`。从 `cursor/four-job-shell-and-marketplace` 的 `fa62efe` 创建，收录当前工作区截至 2026-11-09 R46 的迁移源码、核心/兼容宿主配套调整、CLI/MCP/插件、文档、测试与验证证据。保留原分支提交历史。

## 入口和运行关系

新 GUI：`apps/wiparse-next`，采用 Tauri 2、React 18、TypeScript 和 Vite。

`crates/wiparse-gui` 继续提供兼容设备宿主，发布时名为 `WiParse-Host.exe`；`crates/wiparse-core` 共享算法，CLI/Service、MCP 和插件按既有契约配套。R46 串口日志使用认证的本机事件流唤醒界面，GUI 与 Host 需成套更新。仓库历史 `dist/*.exe` 属于旧交付位置，不能据此判断新框架构建；请按验证/暂存脚本生成完整运行目录。

## 构建与验证

Windows 需要 Rust/MSVC、Node.js 24、WebView2；真实仪表另需驱动/VISA。首先在 `apps/wiparse-next` 执行 `npm ci --legacy-peer-deps`。MCP 与插件依赖分别按各自锁文件安装。

回到仓库根目录执行统一验证并生成发布二进制：

```powershell
./scripts/verify-next-candidate.ps1 -EvidenceDirectory docs/verification/my-new-candidate -BuildPreviewBinaries
```

可用 `-ScpSample` 指定自己的 SCP 波形。历史实测样本不随 Git 提交。验证完成后，以新目录暂存运行包：

```powershell
./scripts/stage-next-preview.ps1 -VerificationManifest docs/verification/my-new-candidate/candidate.json -OutputDirectory dist/WiParse-Next-MyPreview
```

运行生成目录的 `bin/WiParse.exe`，保留全部相邻运行文件。开发模式及接口约束见 [Next README](../apps/wiparse-next/README.md)。

## 当前验证基线

R46：前端 110 项、Tauri 原生 67 项、工作区 578 项通过；提供的 SCP 实测重新运行，30,000,000 点加载/绘图通过。桌面模拟串口验证实时跟随、超过 5000 行 FIFO 后更新、向上翻阅暂停和底部恢复，停止时观察到 14690 行；波形打开替换与追加保留验证通过。

完整边界与证据见 [R46 修正报告](reviews/serial-live-wave-budget-fix-2026-11-09-r46.md)、[候选清单](verification/next-candidate-20261109-r46/candidate.json)、[桌面验证](verification/next-candidate-20261109-r46/desktop.json) 和 [迁移状态](../apps/wiparse-next/MIGRATION-STATUS.md)。不把模拟检查或历史继承证据当作工业现场验收。

## 仓库与本机文件边界

本分支保存源码、文档、合成测试样本和验证证据。编译输出、展开预览包、备份、缓存、测试运行配置及本机串口记忆不纳入提交，也不会因此删除本机文件。新建完整分发包遵循运行清单和配套校验流程；用户工程、密钥、采集记录及配置保持独立。
