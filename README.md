# WiParse

**[中文](#中文)** · **[English](#english)**

无线充电（Qi）测试与工位工具。当前版本 **1.1.12**（以工作区 `Cargo.toml` 的 `workspace.package.version` 为准）。

Wireless charging (Qi) lab and station utility. Current version **1.1.12**.

许可：**Proprietary**。本仓库不是通用示波器软件，也不是面向公网的服务。

---

## 中文

- [1. 产品定位](#1-产品定位)
- [2. 使用范围与边界](#2-使用范围与边界)
- [3. 系统组成](#3-系统组成)
- [4. 功能说明](#4-功能说明)
- [5. 日常使用](#5-日常使用)
- [6. 编译、打包与 dist](#6-编译打包与-dist)
- [7. 集成测试（Testing Hub）](#7-集成测试testing-hub)
- [8. 工位闭环与自动化](#8-工位闭环与自动化)
- [9. 配置与环境变量](#9-配置与环境变量)
- [10. 仓库结构](#10-仓库结构)
- [11. 文档索引](#11-文档索引)

### 1. 产品定位

WiParse 面向 **Qi 无线充电研发台、产线工位和闭环测试**：把串口报文、示波器/电源等仪表、离线波形、数据抽取、测试报告和可插拔产线脚本放在同一个桌面程序里，并提供 JSON CLI、本机 HTTP API 和 MCP，供脚本与 Agent 驱动，而不是代替工程师去“点按钮”。

它解决的典型问题：

- 看清 ASK / FSK 报文，并按 Qi 包名做协议解析（例如 ID / CE / RP，而不是把某个 header 写死进引擎）。
- 在报文出现的上升沿停示波器、截图、读取波形源文件（ISF），形成可复查的证据包。
- 在仪表页 **设备总览** 里看数字孪生工位：电源按真实通道画 LCD（实测 V/A/W），示波器 / 负载 / 万用表 / 探针 / FT4222 同步前面板状态。
- 离线打开 Tek / Rigol 波形，做 I2C / SPI / UART / I2S / **DDSSS** 总线解码。
- 用 Node 插件把“预检 / 运行 / 停止”做成产线配方，而不是把示波器型号写进 GUI。

长驻进程是 **`WiParse.exe`（GUI）**：独占串口和 VISA 仪器，画面走进程内通道。CLI / MCP **默认只 attach** 到 GUI 的 `http://127.0.0.1:7878`，不会在 GUI 未开时悄悄打开本机串口。

### 2. 使用范围与边界

#### 适用

| 场景 | 说明 |
|------|------|
| 研发台 | 串口实时监控、Qi 协议解析、LiveBrief、仪表联调 |
| 波形实验室 | 打开 ISF / WFM / CSV，总线解码（含 Qi DDSSS Draft 5） |
| 产线工位 | 计划 JSON + `wait` 上升沿 + 示波器停采 / 截图 / 波形源；目录式部署，无 MSI |
| 自动化 / Agent | CLI attach、HTTP `/v1/invoke`、Cursor MCP（6 个工具） |
| 插件化测试 | Testing Hub：`preflight` / `run` / `stop`；本机或远程插件市场 |

#### 运行环境

- **主平台：Windows x64**（MSVC 链接器；日常产物为 `dist\WiParse.exe` / `dist\WiParse-CLI.exe`）。
- 仪表：本机需可用的 **VISA**（NI-VISA / TekVISA 等）；支持示波器、直流电源、电子负载、万用表、通用 SCPI，以及 USB 调试探针（J-Link / ST-Link / CMSIS-DAP）与 FT4222 桥。
- Testing Hub / MCP：需要 **Node.js 18+**。
- 语言：界面中/英；设置菜单可切换，配置键 `ui.language`。
- 主题：深色 / 浅色，`ui.theme`。

#### 明确不做

- 不是公网服务器：嵌入式 API **只绑 localhost**（默认 `127.0.0.1:7878`）。
- 不是通用 DAQ / LabVIEW 替代品；不以任意传感器总线为中心。
- 测试报告页 **不内嵌 HTML/PDF 引擎**，只做 Markdown 浏览与轻量预览（图片 / 表格 / 代码块）。
- MCP **不能单独工作**：必须先启动 GUI。不要让模型去点 UI，也不要 `Read` 原始 `serial.txt` 或订阅波形点列。
- 插件市场对非回环的明文 HTTP 默认拒绝；只有 `127.0.0.1` / `localhost` / `::1` 可直接 HTTP，其它 HTTP 需 `WIPARSE_MARKETPLACE_ALLOW_HTTP=1`，生产环境应使用 HTTPS。

### 3. 系统组成

```
工程师 / 工位脚本 / Cursor Agent
        │
        ├── WiParse.exe          桌面 GUI（独占串口、VISA；内嵌 HTTP API）
        │         └── http://127.0.0.1:7878
        │
        ├── WiParse-CLI.exe      JSON CLI（默认 attach GUI；`--local` 才本进程占设备）
        ├── mcp/wiparse          MCP（HTTP，6 个工具）
        └── test-tools           Node 插件 runner + 市场客户端
                    └── 可选：本机市场服务 :8787
```

| 产物 | 路径 | 作用 |
|------|------|------|
| 桌面 GUI | `dist\WiParse.exe`（源码 crate：`wiparse-gui`） | 日常操作界面 + 内嵌 API |
| JSON CLI | `dist\WiParse-CLI.exe`（源码 crate：`wiparse-cli`，编译名为 `wiparse.exe`） | 自动化、健康检查、解析、工位计划 |
| 核心库 | `crates/wiparse-core` | 配置、路径、Qi 协议、仪表驱动、更新、市场客户端 |
| 插件 | `test-tools/plugins/` | Testing Hub 配方 |
| 市场服务 | `services/testing-hub-marketplace` | 目录 / 下载 / Bearer 发布 |
| MCP | `mcp/wiparse` | 给 Cursor 等 Agent 用 |

配置文件默认在可写根目录下的 `config.json`（exe 旁，或 `WIPARSE_CONFIG` / `WCM_CONFIG` 指定）。首次可从 `config.default.json` 复制。数据、日志、示波器截图、证据包也都落在该根目录（或 `WIPARSE_PROJECT_ROOT` 指向的目录）。

### 4. 功能说明

GUI 顶栏标签（设置菜单可显隐各页）：

**串口工具** → **仪表控制** → **波形分析** → **数据分析** → **集成测试** → **测试报告** → **计算器**

#### 4.1 串口工具

实时打开 COM 口（常用波特率含 115200 / 1M / 2M），多标签日志，过滤，自动重连。

- 日志行可按 ASK / FSK 做 **点选解析**（Auto Parse）：非 ASK/FSK 行跳过，关闭解析可省 CPU。
- **LiveBrief**：压缩会话事实（阶段、计数、告警），给 CLI `log brief` / MCP `wiparse_brief` 用，不回传原文。
- 支持打开已有 log 文件做离线浏览；大文件虚拟化，不全量排版。
- 演示模式：`serial.demo_mode`（无硬件时走演示流）。

#### 4.2 仪表控制

经 VISA/SCPI 管理仪器，按 **种类** 出卡片，而不是把某一型号写死进平台：

| 种类 | 能力（依仪器 profile） |
|------|------------------------|
| 示波器 | 连接、通道、停止采集、截图 PNG、CURVe 波形、**读取波形源文件（ISF）** |
| 直流电源 | 多通道输出 / 保护；总览 LCD 按识别通道数显示设定或实测 V/A/W |
| 电子负载 | 模式与测量 |
| 万用表 | 功能 / 量程 / NPLC |
| 调试探针 | J-Link / ST-Link / CMSIS-DAP（probe-rs USB，**无需** JLinkARM.dll）：Halt/Run/复位、寄存器、擦除、速度、烧录、内存、RTT |
| USB 桥 | FT4222 SPI/I2C/GPIO。扫描不需 DLL；事务从 exe 旁 / `vendor/ftdi` 加载 `LibFT4222.dll` |
| 通用 SCPI | 原始命令（查询以 `?` 结尾） |

左侧 **设备总览** 为默认落地页：数字孪生工位（WIPARSE 核心 + 各仪器前面板）。发现列表可 **连接全部**。点击孪生机箱进入该种类控制台。实物图可放 `{save_dir}/device_photos/{序列号或型号}.png|.jpg`。

截图默认目录 `apps.instruments.save_dir` / Tek 兼容键 `apps.tektronix_scope.save_dir`。  
闭环保存 ISF 用 **`waveform_source_dir`**，与波形分析页的浏览器目录 **`waveform_browser_dir` 分开**，空字符串不会回退去改分析页路径。

调试模式（`ui.debug_mode`）可显示全部仪表卡片（含演示连接），便于无硬件时看布局。FTDI DLL 搜索：exe 旁 → `vendor/ftdi` → `WIPARSE_FTDI_DIR`；**不要**捆绑 `JLinkARM.dll`。

#### 4.3 波形分析

离线波形浏览器：平移 / 光标、通道开关与 Y 轴、包络 LOD。

**打开格式：**

- Tektronix ISF、WFM#001/#002/#003、示波器导出 CSV
- Rigol 系列 WFM
- 可另存为 ISF / WFM

**总线解码**（叠在波形上，后台计算）：I2C、SPI、UART、I2S、**DDSSS**（Qi PRx→PTx DSSS ASK，规范为 Qi *DDSSS Communications* Draft 5）。UART 波特率与 DDSSS FOP 可手动或自动。示例文件：`docs/examples/ddsss_vctx.isf`。

闭环路径 **不会** 调用本页浏览器去改目录；工位保存的是路径 + 字节数，不把几十 MB 点列塞进 MCP。

#### 4.4 数据分析

从串口 Live 行或文本/CSV/LOG 文件按 **帧头 + 过滤词** 抽最多 8 条曲线（mV / mA / °C），共用帧序号 X 轴。解析结果可走 **WIDA 缓存** 与按像素 LOD，避免大文件卡死界面。

#### 4.5 集成测试（Testing Hub）

Node 插件宿主。左侧 **插件 | 市场** 分段：

- **插件**：搜索、刷新、折叠的本地目录；列表显示名称 / 版本 / 本地或已装。
- **市场**：同一套搜索；折叠的服务器 URL / 渠道；目录状态为可安装 / 已装 / 可更新。安装成功后切回插件并选中，即可预检 / 运行。

生命周期只有三个：`preflight`（预检，可回填 `suggested_params`）→ `run` → `stop`。平台 **不识别** “这是示波器还是电源”，只按插件声明的 `device` / `enum` / `serial_port` / `boolean` 以及 `filter.kind` 过滤已连接设备。示例插件 **示波器/串口监控**（`scope-serial-monitor`）把仪器、VISA、报告前缀放在表单里，预检填充当前已连接示波器。

契约与 schema：[`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md)。

#### 4.6 测试报告

按文件夹浏览 Markdown；右侧轻量预览（标题、表格、代码块、图片）。可用系统关联程序打开原文件。不在进程内渲染完整 HTML/PDF。

插件生成的 `summary_md` 与 PNG 共置报告目录，MD 用相对路径引用波形图；`file_prefix` 同时决定 ISF / PDF / PNG 与摘要文件名。

#### 4.7 计算器

工程计算卡片：

- LC 谐振（含 ESR）
- 带通滤波器（参数、结果、电路示意）
- Q 值（峰值衰减：精确 Q / 近似 Q）
- RC 时间常数
- CRC（CRC-8、MAXIM、CRC-16 IBM/MODBUS/CCITT/XMODEM、CRC-32 等）
- 换算器：ASCII ↔ 字节、进制、科学计算（角度制可选）

### 5. 日常使用

#### 5.1 打开 GUI（工位 / 本机已打包）

1. 完全退出旧的 `WiParse.exe`（不要覆盖正在运行的文件）。
2. 双击安装目录或仓库里的 `dist\WiParse.exe`。
3. 设置：语言、主题、要显示的面板。
4. **串口工具**：选端口与波特率 → 开始监控。
5. **仪表控制**：扫描，需要时点 **连接全部**；在 **设备总览** 查看孪生面板，或点类型卡片进入控制台。
6. 需要插件时打开 **集成测试**；需要本机市场时先起市场服务（见第 7 节）。

健康检查（GUI 必须已开）：

```powershell
.\dist\WiParse-CLI.exe api health
# 或
curl http://127.0.0.1:7878/v1/health
```

改 API 绑定：

```powershell
$env:WIPARSE_API_BIND = "127.0.0.1:7879"
.\dist\WiParse.exe
```

CLI 侧用 `WIPARSE_URL` 或 `--url` 指向同一地址。

#### 5.2 开发时跑调试版

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo run -p wiparse-gui
# 或先编译再开：
cargo build -p wiparse-gui
.\target\debug\wiparse-gui.exe
```

调试二进制在 `target\debug\`，体积大、未优化，**不会**自动覆盖 `dist\`。验证界面请关掉旧窗口再开新 exe。

#### 5.3 CLI 快速试用

默认 attach 已启动的 GUI。无 GUI / CI 必须加 `--local`。

```powershell
.\dist\WiParse-CLI.exe version
.\dist\WiParse-CLI.exe ports
.\dist\WiParse-CLI.exe parse line --text "TX0:[12:00:00.000] ASK 02 00 F "

# GUI 已开：串口监控
.\dist\WiParse-CLI.exe serial select --port COM4 --baud 200000
.\dist\WiParse-CLI.exe serial start --port COM3 --baud 2000000
.\dist\WiParse-CLI.exe serial read --port COM3 --max-logs 50
.\dist\WiParse-CLI.exe serial stop

# GUI 已开：仪表总览 / 探针
.\dist\WiParse-CLI.exe ui instrument overview
.\dist\WiParse-CLI.exe ui instrument connect-all
.\dist\WiParse-CLI.exe probe list
.\dist\WiParse-CLI.exe bridge list

# 无 GUI
.\dist\WiParse-CLI.exe --local ports
.\dist\WiParse-CLI.exe --local parse file --path capture.log --limit 500
```

完整命令树见 [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md)。输出为 JSON 信封；业务失败为 HTTP 400 + 同一信封。

### 6. 编译、打包与 dist

仓库里有三层产物，不要混用。

| 你要做的事 | 命令 | 写到哪里 |
|------------|------|----------|
| **编译（调试）** | `cargo build -p wiparse-gui -p wiparse-cli` | 仅 `target\debug\`，**不写 dist** |
| **Release 编译** | `cargo build --release -p wiparse-gui -p wiparse-cli` | 仅 `target\release\`，默认也 **不写 dist** |
| **同步日常副本** | 见下方 `Copy-Item` | `dist\WiParse.exe`、`dist\WiParse-CLI.exe` |
| **打包市场演示** | `.\scripts\package-marketplace-demo.ps1` | `dist\wiparse-win-marketplace-demo\` 与 `.zip`，并尽量覆盖上述两个 exe |

Release 配置：`lto = "fat"`、`codegen-units = 1`、`strip = "symbols"`，完整编一次大约十几分钟。

**把日常可运行副本放到 `dist/`（发布 / 工位常用路径）：**

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cd D:\windlink\windlink\WiParse-R
cargo build --release -p wiparse-gui -p wiparse-cli
Copy-Item -Force target\release\wiparse-gui.exe dist\WiParse.exe
Copy-Item -Force target\release\wiparse.exe     dist\WiParse-CLI.exe
```

注意：

- 源码名是 `wiparse-gui.exe` / `wiparse.exe`，**dist 对外名**才是 `WiParse.exe` / `WiParse-CLI.exe`。
- GUI 正在运行时 Windows 会锁住 `dist\WiParse.exe`。打包脚本对此是 **尽力拷贝**：占用则跳过并打印 `skip ... (in use)`，演示 zip 仍会生成。要更新 `dist` 请先退出 GUI。
- 只说“编译”时，工程只产出 `target\`。未跑 `Copy-Item` 或打包脚本，`dist` 会停留在上一轮 release。

市场演示包（含 GUI/CLI、播种插件、本机市场脚本）：

```powershell
.\scripts\package-marketplace-demo.ps1
# → dist\wiparse-win-marketplace-demo\
# → dist\wiparse-win-marketplace-demo.zip
# 可选：-SkipBuild 若 target\release 已经是刚编好的
```

包内启动：进入演示目录后 `.\start-marketplace.ps1`，再 `.\start-wiparse.ps1`。  
Linux 对应脚本：`scripts/package-marketplace-demo.sh`。

工位整包部署、MCP 安装见 [`docs/DEPLOY_API.md`](docs/DEPLOY_API.md)、[`docs/DEPLOY_MCP.md`](docs/DEPLOY_MCP.md)。在线更新架构见 [`docs/UPDATE.md`](docs/UPDATE.md)（HTTPS 清单 + SHA256；不覆盖 `config.json` 与数据）。

发布流程（版本记录约定）：改 `workspace.package.version` → 更新 [`docs/CHANGELOG.md`](docs/CHANGELOG.md) → release 编译并同步 `dist/` → 提交。

### 7. 集成测试（Testing Hub）

#### 7.1 捆绑插件（命令行）

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin example-smoke --lifecycle preflight -- --check_api false
node runner.mjs --plugin scope-serial-monitor --lifecycle preflight
node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --file_prefix MyTest
node runner.mjs --plugin scope-serial-monitor --lifecycle stop
```

GUI：**集成测试** → 选插件 → 改参数 → 预检 / 运行 / 停止。

环境变量：`WIPARSE_CLI`、`WIPARSE_URL`、`WIPARSE_DATA_ROOT`、`WIPARSE_MARKETPLACE_URL`。

#### 7.2 本机插件市场

```powershell
.\scripts\deploy-marketplace.ps1
```

默认 `http://127.0.0.1:8787`，发布 token `dev-token`。播种示例：`demo-marketplace-plugin`、`market-echo`、`market-counter`、`market-preflight-lab`。

然后：GUI → **集成测试** → **市场** → 展开 **服务器** 确认 URL → 刷新 → 安装。安装目录默认在数据根下的 `marketplace`。

市场 HTTP API 与发布格式见 [`services/testing-hub-marketplace/README.md`](services/testing-hub-marketplace/README.md)。

### 8. 工位闭环与自动化

工位机通常只有安装目录（exe + `config.json` + 计划 JSON），没有源码。引擎 **不写死 ASK 71**：计划里写包名 `ID`、header `0x71` 或行正则。`rising: true` 表示进入步骤后再出现的新包才算命中，避免屏幕上已有旧报文立刻 Pass。

示例计划：

- [`docs/examples/qi_pt_smoke.json`](docs/examples/qi_pt_smoke.json)
- [`docs/examples/ask71_waveform_source.json`](docs/examples/ask71_waveform_source.json)（ID 上升沿 → ScopeStop → 截图落盘 → 读 ISF）

CLI（GUI 已开）：

```powershell
.\dist\WiParse-CLI.exe test run --plan docs\examples\qi_pt_smoke.json --port COM3 --baud 2000000
.\dist\WiParse-CLI.exe test status
.\dist\WiParse-CLI.exe test pack
```

MCP 工具：`wiparse_brief`、`wiparse_select`、`wiparse_test`、`wiparse_send`、`wiparse_report_pack`、`wiparse_ui`。对机安装：解压部署 zip → 先开 GUI → 跑 `mcp\wiparse\setup-mcp.cmd` → **完全退出并重开 Cursor**。

落地细节：[`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md)。

### 9. 配置与环境变量

`config.default.json` 主要段落：

| 节 | 用途 |
|----|------|
| `ui` | 语言、主题、渲染间隔、面板显隐、调试模式 |
| `serial` | 默认波特率、演示模式、自动重连 |
| `log_monitor` | 实时日志保存目录 |
| `apps.instruments` | VISA 超时、截图目录、`waveform_browser_dir` / `waveform_source_dir` |
| `apps.test_tool` | 插件目录、CLI/Node 路径、`marketplace.base_url` / `channel` |
| `update` | 在线更新清单 URL（空则禁用） |
| `alerts` | 温度 / OVP / OCP 等阈值（会话告警） |

| 环境变量 | 作用 |
|----------|------|
| `WIPARSE_API_BIND` | GUI 内嵌 API 监听（默认 `127.0.0.1:7878`） |
| `WIPARSE_URL` | CLI / MCP / 插件 attach 的 API 根 |
| `WIPARSE_CONFIG` / `WCM_CONFIG` | 配置文件路径 |
| `WIPARSE_PROJECT_ROOT` / `WIPARSE_APP_ROOT` | 可写工程根（演示包、便携目录） |
| `WIPARSE_CLI` | 插件 runner 使用的 CLI 路径 |
| `WIPARSE_DATA_ROOT` | 插件数据根 |
| `WIPARSE_MARKETPLACE_URL` | 覆盖市场目录地址 |
| `WIPARSE_MARKETPLACE_ALLOW_HTTP` | 允许非回环明文 HTTP（仅调试） |
| `WIPARSE_FTDI_DIR` | 可选，FT4222 `LibFT4222.dll` / `ftd2xx.dll` 目录 |
| `PYVISA_LIBRARY` | 可选，VISA 库路径（与仪器栈相关） |

### 10. 仓库结构

```
WiParse-R/
├── crates/
│   ├── wiparse-core/     # 协议、仪表、路径、更新、市场
│   ├── wiparse-cli/      # JSON CLI
│   └── wiparse-gui/      # egui 桌面
├── test-tools/           # runner、schema、捆绑插件、市场客户端
├── services/
│   └── testing-hub-marketplace/
├── mcp/wiparse/          # MCP（需 GUI HTTP）
├── vendor/ftdi/          # FTDI 运行库说明（DLL 不进 git）
├── scripts/              # 播种/部署市场、打包演示、sync-ftdi-dlls.ps1
├── packaging/update/     # 在线更新安装与发布脚本
├── docs/                 # 变更、CLI、部署、工位、示例计划
├── third_party/winit/    # Win11 DPI 拖动补丁（crates-io patch）
└── dist/                 # 对外二进制与演示包（需显式同步，见第 6 节）
```

依赖：Rust stable + Windows MSVC；可选 Node 18+、VISA。Workspace `Cargo.toml` 将 `winit` patch 到 `third_party/winit`（Win11 24H2 混 DPI 拖动）。

### 11. 文档索引

| 文档 | 内容 |
|------|------|
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | 版本详细记录（关于页只展示简化要点） |
| [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) | CLI 命令树与 JSON 信封 |
| [`docs/DEPLOY_API.md`](docs/DEPLOY_API.md) | GUI 内嵌 API、启动与健康检查 |
| [`docs/DEPLOY_MCP.md`](docs/DEPLOY_MCP.md) | 对机 MCP 安装 |
| [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md) | 工位 wait / 示波器 / ISF |
| [`docs/UPDATE.md`](docs/UPDATE.md) | HTTPS 在线更新 |
| [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md) | 插件契约（给开发者 / 其它 AI） |
| [`test-tools/README.md`](test-tools/README.md) | runner 与市场客户端 |
| [`mcp/wiparse/README.md`](mcp/wiparse/README.md) | MCP 工具约定 |
| [`docs/MDO3014_SCPI命令手册.md`](docs/MDO3014_SCPI命令手册.md) | MDO3014 SCPI 参考（仪器手册摘录） |

---

## English

- [1. What this is](#1-what-this-is)
- [2. Scope](#2-scope)
- [3. Architecture](#3-architecture)
- [4. Features](#4-features)
- [5. How to use](#5-how-to-use)
- [6. Build, package, and dist](#6-build-package-and-dist)
- [7. Testing Hub](#7-testing-hub)
- [8. Station closed loop](#8-station-closed-loop)
- [9. Config and environment](#9-config-and-environment)
- [10. Layout](#10-layout)
- [11. Docs](#11-docs)

### 1. What this is

WiParse is a **Qi wireless-charging lab and production-station** tool: serial decode, VISA instruments, offline waveforms, data extract, Markdown reports, and plugin recipes in one desktop app, plus JSON CLI, localhost HTTP, and MCP for scripts and agents.

The long-lived process is **`WiParse.exe`**. It owns the serial port and VISA devices. CLI and MCP **attach** to `http://127.0.0.1:7878` by default and will **not** silently open hardware if the GUI is down. Use `--local` only for CI / no-GUI.

### 2. Scope

**In scope:** Qi ASK/FSK decode and LiveBrief; scope stop / screenshot / ISF on a rising-edge wait; digital-twin **instrument overview** (per-device LCD); Tek/Rigol waveforms with I2C, SPI, UART, I2S, and **DDSSS** (Qi Draft 5); Node Testing Hub plugins; directory-style station deploy (no MSI); Cursor MCP.

**Environment:** Windows x64 primary; VISA for bench instruments plus USB debug probes (J-Link / ST-Link / CMSIS-DAP via probe-rs) and FT4222; Node.js 18+ for Hub and MCP; UI zh/en, dark/light.

**Out of scope:** public internet services (API is localhost-only); general-purpose DAQ; in-app HTML/PDF engines; MCP without a running GUI; plaintext HTTP marketplaces except loopback (or `WIPARSE_MARKETPLACE_ALLOW_HTTP`).

License: **Proprietary**.

### 3. Architecture

```
Operator / station script / Cursor
   ├── WiParse.exe      GUI + embedded API :7878
   ├── WiParse-CLI.exe  JSON CLI (attach GUI; --local for in-process I/O)
   ├── mcp/wiparse      six MCP tools over HTTP
   └── test-tools       plugin runner + marketplace client
```

| Artifact | Path |
|----------|------|
| GUI | `dist\WiParse.exe` (`wiparse-gui`) |
| CLI | `dist\WiParse-CLI.exe` (crate binary `wiparse.exe`) |
| Core | `crates/wiparse-core` |

Writable root is the exe directory, or `WIPARSE_PROJECT_ROOT` / `WIPARSE_CONFIG`.

### 4. Features

Tabs: **Serial Tool** · **Instrument Control** · **Waveform Analysis** · **Data Analysis** · **Testing Hub** · **Test Report** · **Calculator**.

| Panel | What it does |
|-------|----------------|
| Serial | COM monitor, filters, ASK/FSK click-to-parse, LiveBrief, large-file virtualization |
| Instruments | Cards by kind (scope / DC source / load / DMM / debug probe / FT4222 / generic SCPI). Default **device overview** twin with live LCD (PSU channels show measured V/A/W). Connect-all on the discovery list. Scope: stop, PNG, CURVe, **waveform source (ISF)**. `waveform_source_dir` is separate from the analysis browser dir |
| Waveform | Offline ISF / Tek WFM / CSV / Rigol WFM; pan/cursors; bus decode including DDSSS |
| Data analysis | Frame header + up to 8 filter series from Live serial or files; WIDA cache + LOD |
| Testing Hub | Plugins \| Market; `preflight` / `run` / `stop`; generic `device` params (no scope type hardcoded in the GUI) |
| Test report | Markdown browser + light preview (not HTML/PDF) |
| Calculator | LC, band-pass, Q from peak decay, RC, CRC, ASCII/radix/scientific converters |

### 5. How to use

Start `dist\WiParse.exe`, pick language/theme, open serial and connect instruments. Health:

```powershell
.\dist\WiParse-CLI.exe api health
```

Debug from source (does **not** update `dist`):

```powershell
cargo build -p wiparse-gui
.\target\debug\wiparse-gui.exe
```

CLI attach vs local:

```powershell
.\dist\WiParse-CLI.exe parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
.\dist\WiParse-CLI.exe --local ports
```

Full CLI: [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md).

### 6. Build, package, and dist

| Intent | Command | Output |
|--------|---------|--------|
| Debug compile | `cargo build -p wiparse-gui -p wiparse-cli` | `target\debug\` only |
| Release compile | `cargo build --release -p wiparse-gui -p wiparse-cli` | `target\release\` only |
| Daily runtime copy | `Copy-Item` below | `dist\WiParse.exe`, `dist\WiParse-CLI.exe` |
| Marketplace demo | `.\scripts\package-marketplace-demo.ps1` | `dist\wiparse-win-marketplace-demo\` + zip, and a best-effort copy of the two exes |

```powershell
cargo build --release -p wiparse-gui -p wiparse-cli
Copy-Item -Force target\release\wiparse-gui.exe dist\WiParse.exe
Copy-Item -Force target\release\wiparse.exe     dist\WiParse-CLI.exe
```

A running GUI locks `dist\WiParse.exe`; the demo packager **skips** that copy instead of failing the zip. Close the GUI first if you need `dist` refreshed. Saying “compile” in this repo does not imply a `dist` update.

Release uses fat LTO (often 10+ minutes). Version bump process: `Cargo.toml` → `docs/CHANGELOG.md` → release build → sync `dist/`.

### 7. Testing Hub

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin scope-serial-monitor --lifecycle preflight
```

Local catalog:

```powershell
.\scripts\deploy-marketplace.ps1
```

Then Testing Hub → **Market** → Server URL `http://127.0.0.1:8787` → Refresh → Install. Contract: [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md).

### 8. Station closed loop

The engine matches packet **names**, headers, or line regex — ASK 71 is an example, not hard-coded. Use `rising: true` so old packets on screen do not pass immediately.

Examples: [`docs/examples/qi_pt_smoke.json`](docs/examples/qi_pt_smoke.json), [`docs/examples/ask71_waveform_source.json`](docs/examples/ask71_waveform_source.json).

MCP tools: `wiparse_brief`, `wiparse_select`, `wiparse_test`, `wiparse_send`, `wiparse_report_pack`, `wiparse_ui`. Station write-up: [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md).

### 9. Config and environment

See `config.default.json` for `ui`, `serial`, `apps.instruments`, `apps.test_tool.marketplace`, and `update`. Important variables: `WIPARSE_API_BIND`, `WIPARSE_URL`, `WIPARSE_CONFIG` / `WCM_CONFIG`, `WIPARSE_PROJECT_ROOT`, `WIPARSE_CLI`, `WIPARSE_DATA_ROOT`, `WIPARSE_MARKETPLACE_URL`, `WIPARSE_MARKETPLACE_ALLOW_HTTP`, `WIPARSE_FTDI_DIR`.

### 10. Layout

```
WiParse-R/
├── crates/          core, cli, gui
├── test-tools/      plugins + marketplace client
├── services/        marketplace server
├── mcp/wiparse/     MCP
├── vendor/ftdi/     FTDI DLL notes (binaries not in git)
├── scripts/         seed / deploy / package demo / sync-ftdi-dlls.ps1
├── docs/            changelog, CLI, deploy, examples
└── dist/            shipped binaries (explicit sync)
```

### 11. Docs

| Doc | Content |
|-----|---------|
| [`docs/CHANGELOG.md`](docs/CHANGELOG.md) | Release notes |
| [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) | CLI |
| [`docs/DEPLOY_API.md`](docs/DEPLOY_API.md) | Embedded API |
| [`docs/DEPLOY_MCP.md`](docs/DEPLOY_MCP.md) | MCP on another PC |
| [`docs/WORKSTATION_CLOSED_LOOP.md`](docs/WORKSTATION_CLOSED_LOOP.md) | Station wait / ISF |
| [`docs/UPDATE.md`](docs/UPDATE.md) | Online update |
| [`test-tools/PLUGIN_SPEC.md`](test-tools/PLUGIN_SPEC.md) | Plugin contract |
| [`mcp/wiparse/README.md`](mcp/wiparse/README.md) | MCP tools |
