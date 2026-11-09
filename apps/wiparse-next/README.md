# WiParse Next

基于 React 18、TypeScript 和 Tauri 2 的 Windows 工作台，复用 WiParse 核心算法及兼容宿主的设备/串口/插件执行路径。

当前处于集中迁移开发阶段，未完成正式验收。按用户要求，先完成代码、界面、文档和构建配置，再冻结候选版本统一验证，不再逐模块重复构建。具体能力、限制和证据见 [迁移状态](MIGRATION-STATUS.md)。

## 开发

需要 Node.js 24、Rust/MSVC、Windows WebView2。前端使用 `npm ci --legacy-peer-deps` 安装锁定依赖，`npm run dev` 启动 Vite；原生入口为 `npm run tauri dev`。浏览器中的 Vite 页面没有桌面文件和设备接口。

Next 会检查本机受保护兼容宿主。发布布局要求 Next、`WiParse-Host.exe`（兼容构建名也可为 `wiparse-gui.exe`）及 CLI 与必要运行文件按预览脚本配置；不能把普通 `WiParse.exe` 自身当作宿主回退启动，避免递归。未受保护的旧独立界面需正常关闭后让 Next 启动匹配宿主。

UI 不能任意指定 HTTP 目的地址或任意调用桥方法；读取使用白名单，写入使用类型化入口和宿主上下文/任务检查。companion 身份凭据只交给固定本机桥；真实 AI API 密钥由当前用户 DPAPI 管理。

## 统一验证与发布

集中开发收尾后执行 `scripts/verify-next-candidate.ps1 -EvidenceDirectory <新的证据目录>`；需要单次生成候选发布二进制时加 `-BuildPreviewBinaries`，正确 SCP 样本可通过 `-ScpSample` 指定。`scripts/stage-next-preview.ps1` 仅接受成功且源码、EXE、MCP 哈希一致的候选清单，创建新的预览目录，不覆盖旧预览或现场安装。前端、核心、宿主、CLI、插件和 MCP 的验证日志集中保存。

前端检查包括 `npm test`、`npm run build`，原生检查包括独立 `src-tauri` 工作区测试及兼容宿主/核心/CLI 和插件回归。全部候选源码冻结后再运行，历史测试记录不能证明新改动通过。

正式门禁见 [五轮验收](ACCEPTANCE-PLAN.md)。开发预览不覆盖现场目录，不包含用户密钥。按用户偏好先提供 EXE 效果预览；用户确认后才制作完整 ZIP。不得以模拟或未执行检查代替真实验收。
