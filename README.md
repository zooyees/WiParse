# WiParse

**[中文](#中文)** · **[English](#english)**

无线充电测试工具（Rust）：桌面 GUI + JSON CLI + HTTP API + MCP。当前版本 **1.1.8**。

Wireless charging test utility (Rust): desktop GUI + JSON CLI + HTTP API + MCP. Current version **1.1.8**.

---

## 中文

### 简介

WiParse 面向 Qi / 工位闭环场景：串口监控与协议解析、仪表控制、波形与总线分析、数据分析、测试报告预览，以及 **集成测试（Testing Hub）** 插件式产线脚本。

详细变更见 [`docs/CHANGELOG.md`](docs/CHANGELOG.md)。工位闭环见 [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md)。插件契约见 [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md)。

### 主要能力

| 面板 / 模块 | 说明 |
|-------------|------|
| 串口工具 | 实时监控、过滤、协议解析（ASK/FSK 等）、LiveBrief |
| 仪表控制 | 示波器等仪表连接、命令、截图、波形源文件（ISF） |
| 波形分析 | 离线波形浏览；总线解码含 **DDSSS** |
| 数据分析 | 串口 Live / 文件抽取多通道曲线；WIDA 缓存与 LOD |
| 测试报告 | Markdown 报告浏览与轻量预览（图片/表格/代码块） |
| **集成测试** | Node 插件宿主：`preflight` / `run` / `stop`；示例含 **示波器/串口监控** |
| 计算器 | LC / 带通 / Q / RC / CRC / 单位换算等 |
| CLI / HTTP / MCP | 工位自动化与 Agent 驱动（见部署文档） |

### 仓库结构

```
WiParse-R/
├── crates/
│   ├── wiparse-core/   # 配置、路径、协议、仪表驱动、更新
│   ├── wiparse-cli/    # JSON CLI（`dist/WiParse-CLI.exe`）
│   └── wiparse-gui/    # egui 桌面（`dist/WiParse.exe`）
├── test-tools/         # 集成测试插件（runner + marketplace 客户端）
├── services/
│   └── testing-hub-marketplace/  # 插件市场云端目录/制品服务
├── mcp/wiparse/        # MCP 服务（需已启动的 GUI HTTP）
├── docs/               # 变更、CLI、部署、工位闭环
└── dist/               # 打包产物
```

### 构建

需要 [Rust](https://rustup.rs/)（stable）与 Windows 上的 MSVC 链接器。

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cd D:\windlink\windlink\WiParse-R
cargo build --release -p wiparse-gui -p wiparse-cli
Copy-Item -Force target\release\wiparse-gui.exe dist\WiParse.exe
Copy-Item -Force target\release\wiparse.exe dist\WiParse-CLI.exe
```

| 产物 | 路径 |
|------|------|
| GUI | `dist/WiParse.exe` |
| CLI | `dist/WiParse-CLI.exe` |

### 快速试用 CLI

```powershell
.\dist\WiParse-CLI.exe version
.\dist\WiParse-CLI.exe ports
.\dist\WiParse-CLI.exe parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
```

完整命令：[`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md)。  
本机 API / MCP：[`docs/DEPLOY_API.md`](docs/DEPLOY_API.md)、[`docs/DEPLOY_MCP.md`](docs/DEPLOY_MCP.md)、[`mcp/wiparse/README.md`](mcp/wiparse/README.md)。

### 集成测试插件

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin scope-serial-monitor --lifecycle preflight
node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --port COM7
node runner.mjs --plugin scope-serial-monitor --lifecycle stop
```

GUI：**集成测试** → 选择 **示波器/串口监控** → 预检 / 运行 / 停止。  
环境变量：`WIPARSE_CLI`、`WIPARSE_URL`、`WIPARSE_DATA_ROOT`。

### 文档索引

| 文档 | 内容 |
|------|------|
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | 版本详细记录 |
| [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) | CLI 参考 |
| [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md) | 工位闭环落地 |
| [`docs/UPDATE.md`](docs/UPDATE.md) | 在线更新 |
| [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md) | 插件契约 |

---

## English

### Overview

WiParse is a Qi / station-oriented test utility: serial monitoring and protocol decode, instrument control, waveform and bus analysis, data analysis, test-report preview, and a plugin-based **Testing Hub** for production recipes.

See [`docs/CHANGELOG.md`](docs/CHANGELOG.md) for release notes, [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md) for closed-loop stations, and [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md) for the plugin contract.

### Features

| Panel / module | Description |
|----------------|-------------|
| Serial Tool | Live monitor, filters, ASK/FSK decode, LiveBrief |
| Instruments | Scope connect / commands / capture / waveform source (ISF) |
| Waveform Analysis | Offline browser; bus decode including **DDSSS** |
| Data Analysis | Serial Live / file extract; WIDA cache + LOD |
| Test Report | Markdown browser with lightweight preview |
| **Testing Hub** | Node plugins: `preflight` / `run` / `stop`; includes **Scope & Serial Monitor** |
| Calculator | LC / bandpass / Q / RC / CRC / converters |
| CLI / HTTP / MCP | Automation and agent control |

### Layout

```
WiParse-R/
├── crates/
│   ├── wiparse-core/   # config, paths, protocol, instruments, update
│   ├── wiparse-cli/    # JSON CLI (`dist/WiParse-CLI.exe`)
│   └── wiparse-gui/    # egui desktop (`dist/WiParse.exe`)
├── test-tools/         # Testing Hub plugins (runner + marketplace client)
├── services/
│   └── testing-hub-marketplace/  # Plugin marketplace catalog/artifact server
├── mcp/wiparse/        # MCP server (requires running GUI HTTP API)
├── docs/               # changelog, CLI, deploy, workstation
└── dist/               # packaged binaries
```

### Build

Requires [Rust](https://rustup.rs/) (stable) and a C linker (MSVC Build Tools on Windows).

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cd D:\windlink\windlink\WiParse-R
cargo build --release -p wiparse-gui -p wiparse-cli
Copy-Item -Force target\release\wiparse-gui.exe dist\WiParse.exe
Copy-Item -Force target\release\wiparse.exe dist\WiParse-CLI.exe
```

| Binary | Path |
|--------|------|
| GUI | `dist/WiParse.exe` |
| CLI | `dist/WiParse-CLI.exe` |

### CLI quick start

```powershell
.\dist\WiParse-CLI.exe version
.\dist\WiParse-CLI.exe ports
.\dist\WiParse-CLI.exe parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
```

Full reference: [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md).  
API / MCP: [`docs/DEPLOY_API.md`](docs/DEPLOY_API.md), [`docs/DEPLOY_MCP.md`](docs/DEPLOY_MCP.md), [`mcp/wiparse/README.md`](mcp/wiparse/README.md).

### Testing Hub plugins

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin scope-serial-monitor --lifecycle preflight
node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --port COM7
node runner.mjs --plugin scope-serial-monitor --lifecycle stop
```

GUI: **Testing Hub** → **Scope & Serial Monitor** → Preflight / Run / Stop.  
Env: `WIPARSE_CLI`, `WIPARSE_URL`, `WIPARSE_DATA_ROOT`.

### Docs

| Doc | Content |
|-----|---------|
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | Detailed release notes |
| [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) | CLI reference |
| [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md) | Station closed-loop |
| [`docs/UPDATE.md`](docs/UPDATE.md) | Online update |
| [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md) | Plugin contract |
