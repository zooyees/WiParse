# WiParse 版本记录

产品版本以工作区 `Cargo.toml` 的 `workspace.package.version` 为准（GUI / CLI / `wiparse-core` 共用）。MCP 包 `mcp/wiparse/package.json` 与之对齐。

发布流程：改版本号 → 更新本文 → `cargo build --release` 同步 `dist/` → 提交并推送。对机 MCP 安装见 [`DEPLOY_MCP.md`](DEPLOY_MCP.md)。工位闭环落地见 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。关于页仅展示简化要点；完整备份以本文为准。

---

## Unreleased

- 捆绑插件对齐 **1.1.26** 契约：清单均有 `report_name` / `report_name_zh`；表单可选 `file_prefix` 覆盖用例目录；`engines.wiparse >=1.1.16`。`example-smoke` **v0.2.1** 声明 `outputs[]` 并写入 `tests/{report_name}/runs/{stamp}/`。`wiparse-dap` / `wiparse-uart-bridge` / `wiparse-i2c-rpc` **v1.0.1**。
- `wiparse-netcfg` **v1.1.0**：对齐 DAP-Link ICD **v1.4** / 固件 `bcdDevice 0x0208`。USB `MI_05` EP0 优先，TCP `:1238` 仅 GOT_IP 后可用。PSK 不落日志。

---

## 1.1.26 — 2026-09-16

### Test Report / Plugin

- 每个插件清单必须有 **`report_name`**（可选 `report_name_zh`）。测试报告左侧用例名只用报告名，不用插件营销名、目录 id 或 run 时间戳。时间戳只出现在结果层。
- 用例标题单一规则：自定义前缀 → `report_name` → 目录名。Hub / runner 默认 `tests/{test_id}/` 跟随 `report_name`；表单 `file_prefix` 仍可覆盖。

### Testing Hub

- `wiparse-netcfg` **v1.0.0**：CH-NETCFG 配网插件。USB `MI_05` EP0（`0x20`/`0x21`）优先，TCP `:1238` 作已入网后备。`APPLY` 后轮询 `STATUS wifi=`，日志脱敏 PSK。需要固件 `bcdDevice 0x0206`。

---

## 1.1.25 — 2026-09-16

### Test Report

- 左侧树按统一网格重排：用例 → 结果时间戳 → 产物分组 → 文件，行高 24px、缩进 16px，用例之间留 6px 空隙。
- 标题单行省略；数量 / 通过失败 / 文件类型放在行尾，不再和名称挤在一起。
- 插件中文名、结果 `MM-DD  HH:MM`；仅报告组默认展开，单组且文件很少时扁平化。

---

## 1.1.24 — 2026-09-16

### Instrument Control

- **WIPARSE I2C** 与 UART 同一套 WinUSB 打开：GUID `{7B3E9C41-…0103}` → 关联接口 bulk `0x03`/`0x84`，失败再 TCP `:1236`。SCAN/PING 不再依赖 dongle 在局域网里。
- **DAP**：连接时只占用 USB CMSIS-DAP，不再对空芯片名做 Auto attach。无目标 MCU 时探针仍可用；填芯片名后点 Attach 再 Halt/烧录。

---

## 1.1.23 — 2026-09-15

### Serial

- WIPARSE UART USB 按 ICD 1.1 打开：WinUSB GUID `{7B3E9C41-…0101}` → `WinUsb_Initialize` → 关联接口 bulk `0x02`/`0x83`；旧固件仍回退 PDO `GLOBALROOT`，再回退 WiFi TCP。

---

## 1.1.22 — 2026-09-15

### Serial

- 设备管理器里的 **WIPARSE UART** 属于 WinUSB（USBDevice），默认不进 Ports。串口工具按通用 UART 会话列出 `winusb://wiparse/uart`：Windows 用 SetupAPI Hardware ID + `WinUsb_Initialize`（与 Hub `winusb-host.cs` 相同，不依赖 CMSIS GUID）；打开失败则自动回退 WiFi TCP `:1234`。真 COMx 仅在可选 `usbser` INF 绑定后出现。设备控制 dongle UART 同一顺序：WinUSB → TCP。

---

## 1.1.21 — 2026-09-15

### Serial

- WIPARSE UART USB 只认 ICD **MI_01**（及 CDC 数据面 MI_02），不再把 I2C **MI_03** / DAP **MI_00** 的幽灵 `PortName` 标成 UART。未真正绑定 `usbser` 的 WinUSB 口不再注入列表（打开会 `ERROR_FILE_NOT_FOUND`）。拔掉 dongle 后刷新会丢掉这些口。

---

## 1.1.20 — 2026-09-15

### Serial

- 串口工具左侧栏宽度锁定：COM 下拉关闭态只用短标签（`COMx` / `COMx · USB` / `WiFi · host`），长产品名截断，不再把主日志区挤出空隙。下拉菜单仍显示完整 `WIPARSE UART · USB/WiFi` 名称。
- WIPARSE UART 枚举：Windows 从 `USB\VID_C251&PID_F01A` 与同 ContainerID 的 `PortName` 补 COM（不只依赖 Ports 类 / 产品名字串）；WiFi TCP 后台探测，失败不长时间缓存。侧栏增加可见「刷新端口」。

---

## 1.1.19 — 2026-09-15

### Serial

- 串口工具自动枚举 **WIPARSE UART**：USB CDC（VID `0xC251` / 产品名）与 WiFi TCP（`192.168.137.123:1234` / `dap.local:1234`）都会列出。两者同时在线时各占一条，名称标注 **USB** 或 **WiFi**。WiFi 项以 `tcp://host:1234` 打开（首包波特率与 dongle 一致）。

---

## 1.1.18 — 2026-09-15

### Instrument Control

- 新增 **WiParse-dongle** 设备种类（`wiparse_dongle`）。USB `VID=0xC251 PID=0xF01A` 识别为一块复合接口：DAP（probe-rs USB CMSIS-DAP）、UART（TCP `:1234` 或 CDC COM）、I2C 文本 RPC（TCP `:1236`）。工作区内分 DAP / UART / I2C 三个通道，覆盖 Halt/Run/烧录/RTT、UART 收发、SCAN/PING/READ/WRITE/WRITEREAD。地址形如 `dongle://wiparse/serial=…`。CLI：`wiparse dongle …`。

### Test Report

- 左侧树按层级缩进（用例 → 结果时间戳 → 产物分组），固定行高避免文字重叠。
- 用例标题跟随 Hub **报告名称 / `file_prefix`**，不再用插件显示名或结果时间戳。Hub 开跑时写入 `project.json`；报告页「添加测试」同样按报告名称登记。

---

## 1.1.17 — 2026-09-15

### Test Report

- 左侧树收紧：新建项目默认折叠；文件名去掉前缀与时间戳噪音；HTML/PDF 组默认收起；无结果的用例默认折叠。
- Markdown 预览中 **Ctrl+单击** 图片或表格里的波形/截图/PDF 路径，按类型在本页打开（波形嵌入、图片预览；HTML/PDF 仍走系统程序）。

### Serial / 插件触发

- 实时页 5000 行 FIFO 之后，`log.lines.get` 的 `from_row` / `total` 改为会话绝对序号（另给 `dropped` / `window` / `next_row`）。插件监控不再在窗口写满后空转。数据分析 Serial Live 游标同样按绝对序号跟随。

---

## 1.1.16 — 2026-09-15

### Testing Hub

- 左侧可新建项目；与测试报告共用 `apps.project.active_id`。
- GUI **不再**向 runner 传 `--test-id`（旧 runner 会直接失败）。用例目录由 overlay 里的报告名称 / `file_prefix` 决定，并设置 `WIPARSE_TEST_ID`。
- 参数表单：`group` 为等宽分区标题；JSON/文本高度封顶并内部滚动；宿主路径在左侧文件夹区。

### Test Report

- 索引三层：**项目 → 测试用例 → 测试结果**（`runs/{stamp}`）。波形 / 报告 / 截图挂在该次结果下，不再跨 stamp 摊平。
- 右侧就地预览 Markdown、波形（嵌入分析）、图片、文本；PDF/HTML 仍走系统程序。单击 ISF 不再跳转波形页。
- Markdown 预览允许同一次结果 `artifacts/` 下的兄弟目录（如 `reports/` 里的 `../shots/*.png`）；越出 `artifacts/` 仍拦截。
- 下拉列出 `projects/` 下每个合法目录；兼容旧 `project.json`（`tests` 为字符串数组或缺少 JSON），新建项目后仍能看到旧项目。

### 插件契约

- 规范见 [`test-tools/PLUGIN_SPEC.md`](../test-tools/PLUGIN_SPEC.md)（GUI **1.1.16+**）。
- `tests/{test_id}/` 用报告名；`plugin_id` 另存。`outputs[]` 的 `view` 决定报告页打开方式。`preflight` / `stop` 不分 stamp。禁止新 run 写入 `plugin_dir`。截图用 `instrument.capture`，不要 `scope.shot`。
- `scope-serial-monitor` **v0.6.0**：产物写入 `tests/{file_prefix}/`（默认 `ScopeSerial`），`plugin_id` 另存为 `scope-serial-monitor`。需要 GUI **1.1.16+**。

---

## 1.1.15 — 2026-09-15

Testing Hub can create a project on the left. A plugin `run` uses the form Report name / `file_prefix` as `tests/{test_id}/`. Artifacts stay under `runs/{stamp}/artifacts/`. Test Report lists those tests after switching project.

The right pane previews by type in place: Markdown, waveform sources (embedded analysis plot), images, text. PDF/HTML still open in the system app. Clicking an ISF no longer jumps to the Waveform tab.

---

## 1.1.14 — 2026-09-15

测试报告页改为项目回放索引（项目 → Hub 测试 → `outputs[]` 实际文件）。单击 ISF/log 跳转现有波形/串口页；Markdown 仍在报告页预览。不新增主标签，不同页托管查看器（P2）。

- 索引按脏标记 / Hub 作业 epoch 刷新（运行中约 800ms，空闲约 2.5s），不再每帧扫盘。
- 集成测试与测试报告共用 `apps.project.active_id`；Hub 左侧可选项目。
- 单击波形替换当前文档，Ctrl/Cmd 叠加；PNG/PDF/HTML 留在报告页并用系统程序打开；`.txt` 跟随所在 `outputs[]` 分组。
- 文件右键：系统打开 / 在文件夹中显示；测试标题右键可从 `project.json` 移除（不删磁盘产物）。
- Hub 参数表单改为统一网格：`group` 是等宽分区标题（不再用缩进折叠打乱对齐）；宿主路径（数据根 / Node / CLI）放到左侧文件夹区。
- CLI：`ui project` / `ui report open` / `ui test-tool run|stop`（`run` 可带 `--params` JSON）。MCP `wiparse_ui` 同步这些 `op`。
- 示波器/串口监控 **v0.5.1**：截图改走 `instrument.capture`（不再用已废弃的 `scope.shot`）；辅助脚本使用 `--project`，锁/HUD/日志离开 `plugin_dir`。
- Testing Hub 捆绑三通道插件（ICD **WIPARSE-IF-001**）：`wiparse-dap`、`wiparse-uart-bridge`、`wiparse-i2c-rpc`。协议库 `test-tools/lib/wiparse-if*.mjs`；Windows WinUSB 宿主 `lib/winusb-host.cs`。GUI USB 扫描把 `VID=0xC251 PID=0xF01A` 识别为 CMSIS-DAP。

---

## 1.1.13 — 2026-09-15

Testing Hub 把「行触发条件」做成通用 `json` 参数；示波器/串口监控插件不再把 ASK 写进平台。市场安装的插件可在插件列表卸载。

### Testing Hub / 插件契约

- Hub 表单经 `--overlay` 文件下发（不写回 `station.json`），避免大 JSON 塞进 argv。
- HUD 只读 `step` / `hint` / `cycle` / `elapsed_s`，不再把插件 step 译成「监控/已抓到」。
- `paths.isf_dir` 改为可选；Hub 只展开 `status_file` / `stop_file`。
- 预检只走 `--lifecycle preflight`；`params[].group` / `advanced` 折叠。
- **卸载**：市场安装的插件可在插件列表标题栏或右键菜单卸载；捆绑插件不可卸。市场详情页原有卸载仍然有效。

- 新参数类型：`json`（多行 JSON）、`text`（多行文本）。Hub 仍不识别 ASK / 示波器语义。
- `station.json` 里的对象/数组会 pretty-print 进表单；`--name` 覆盖经 `applyParamPaths` 解析后写回 station。
- 通用行匹配库：`test-tools/lib/line-triggers.mjs`（`regex` / `contains`、`unless`、上升沿）。

- 项目落盘：`{data_root}/projects/{project}/tests/{plugin.id}/`；`run` 使用 `runs/{stamp}/artifacts/`。runner 增加 `--project` / `--stamp`，结束时写 `run.json`。
- `plugin.json` 可声明 `outputs[]`（`view`: wave / log / report / external）。`artifacts` 规范成 `items[]`，路径必须在本次 `run_dir` 内。
- `colocateStatusWithIsf` 不再覆盖已声明 `status_file`；缺省 HUD 在 `test_dir/status.json`。
- `preflight` / `stop` 不分配 stamp、不创建 `runs/`。禁止新 run 写入 `plugin_dir`、`instrument_data/`、`test report/`。

### 示波器/串口监控

- `scope-serial-monitor` **v0.5.0**：表单可改 `serial_triggers.items` 与 **上升沿触发**。Hub 覆盖后循环不再用磁盘 `station.json` 冲掉表单。
- `station.json` 里的 ASK 2 / timeout 仅为出厂配方，改表单或改 JSON 即可换规则。
- 写盘改到 `projects/default/tests/scope-serial-monitor/runs/<stamp>/artifacts/{waves,reports,shots,docs}`；锁/停/HUD 在 `test_dir`。会话 stamp 使用 `ctx.stamp`。

### 兼容性

- 配置键仍为 `test_tool`。未声明 `json` 参数的旧插件行为不变。
- 旧 GUI 把 `json` 当成单行字符串仍可跑（挤）；多行编辑需本版界面。
- 旧 `instrument_data/` 与 `test report/` 文件不搬家；报告树（P1）只扫 `projects/`。
- 本机 `--project` 默认 `default`（可用 `WIPARSE_PROJECT`）。测试报告页项目 Combo 仍属 P1。

---

## 1.1.12 — 2026-09-13

仪表控制增加调试探针 / FT4222 与 **设备总览** 数字孪生（前面板 LCD 跟实测走）。CLI / MCP 可读取同一套状态。界面标签不变。

### 仪表控制

- 扫描合并 USB VID/PID：J-Link / ST-Link / CMSIS-DAP（`debug_probe`）与 FT4222H（`usb_bridge`），地址形如 `probe://jlink/serial=…`、`bridge://ft4222/serial=…`。
- 探针（probe-rs USB，**不捆绑** `JLinkARM.dll`）：Halt / Run / 复位、寄存器、擦除、速度、内存、HEX/BIN/ELF 烧录、RTT。ST-Link 被 Cube 独占时提示 WinUSB。
- FT4222：无 DLL 仍可识别；SPI/I2C/GPIO 需 `LibFT4222.dll`（exe 旁或 `vendor/ftdi`，`scripts/sync-ftdi-dlls.ps1`）。
- 左侧 **设备总览**：数字孪生工位。直流电源按真实通道画 LCD（设定或实测 V/A/W + 端子灯）；示波器 / 负载 / 万用表 / 探针 / 桥同步前面板。发现列表 **连接全部**。
- `instrument.command` 含 `Probe*` / `Bridge*`；Hub `filter.kind = debug_probe | usb_bridge`。调试模式提供 DEMO 工作区。

### CLI / HTTP / MCP

- CLI：`wiparse probe` / `wiparse bridge` / `ui instrument connect-all` / `ui instrument overview`；`ui instrument list --kind`；`ui instrument select --overview`。
- HTTP：`instrument.overview`（孪生 LCD JSON）；`instrument.list` 含 `status` / `readings`；`ui.instrument.select` 支持 `overview` / `kind`。
- MCP `wiparse_ui`：`op=instrument.overview`；`instrument.command` 可发 Probe\* / Bridge\*。GUI 1.1.12+。

### 兼容性

- 配置键仍为 `test_tool`。GUI 主标签与 1.1.11 相同。
- 未连接探针 / 桥时行为与 1.1.11 相同。FTDI DLL 仍不进 git。

---

## 1.1.11 — 2026-09-12

Testing Hub 插件市场可在应用内浏览/安装；Hub 布局简化并消除控件重叠。界面仍为原标签（串口 / 仪表 / 波形 / 数据 / 集成测试 / 报告 / 计算器）。

### Testing Hub / 市场

- 左侧 **插件 | 市场** 分段，去掉启用开关与顶栏往返；安装成功后切回插件并选中即可运行。
- 市场契约与 schema：`test-tools/schemas/marketplace-package.schema.json`；`plugin.json` 可选 `publisher` / `sandbox` / `marketplace.channel`。
- Node 客户端：`marketplace.mjs`（verify / install / list / catalog / pull）；runner 合并捆绑插件与 `{data_root}/marketplace` 已激活版本；SHA-256，可选 Ed25519。
- Rust：`wiparse_core::marketplace` + `apps.test_tool.marketplace`；HTTPS 拉目录。本机 `127.0.0.1` / `localhost` / `::1` 明文 HTTP 可直接访问；其它 HTTP 仍需 `WIPARSE_MARKETPLACE_ALLOW_HTTP`。
- 云端骨架 `services/testing-hub-marketplace`：catalog / 详情 / 下载 / Bearer 发布与删除。
- Windows：`scripts/deploy-marketplace.ps1` 播种并监听 `http://127.0.0.1:8787`；`scripts/package-marketplace-demo.ps1` 产出演示 zip，并尽力同步 `dist/WiParse.exe`（GUI 占用则跳过拷贝）。

### 布局 / 文档

- 搜索+刷新、插件/市场分段改为精确分格，圆角控件不再互相覆盖。
- 根目录 README 补充功能、使用范围、编译与 dist 工作流。
- 修正 `.gitignore` 的 `log/` 误忽略 `crates/wiparse-core/src/log/`。

### 兼容性

- 配置键仍为 `test_tool`。GUI 主标签与 1.1.10 相同。
- 旧插件未走市场安装时行为不变。
- 关于页展示本版简化要点；完整备份以本文为准。

---

## 1.1.10 — 2026-09-11

Testing Hub 把仪器当成通用参数，不再把示波器写进平台。插件预检可读取当前已连接设备并回填表单。

### Testing Hub（平台）

- 新参数类型：`device`（已连接仪器下拉）、`enum`、`serial_port`、`boolean` 复选框。Hub **不**识别示波器/电源，只按 `filter.kind` / `filter_from` 过滤。
- `type: device` 可用 `fills` 把 `resource` / `model` / `kind` 写入其它 param。
- 预检 JSON 的 `suggested_params` 按参数名回填；默认策略 `untouched`（不覆盖用户刚改过的字段）。

### 插件 / 契约

- `pickInstrument` / `suggestedParamsFromInstrument` 进入 `plugin-contract.mjs`。
- `scope-serial-monitor` **v0.3.0**：仪器、型号、VISA、类型均在 Hub 表单；预检填充当前示波器；不再写死 MDO3014 / 某条 USB VISA。
- 规范：[`test-tools/PLUGIN_SPEC.md`](../test-tools/PLUGIN_SPEC.md)。

### 兼容性

- 配置键仍为 `test_tool`。旧插件未声明 `device` 参数时行为与 1.1.9 相同。
- 换仪器种类：改插件 `filter.kind` / `scope_kind`，不必改 GUI。

---

## 1.1.9 — 2026-09-11

Testing Hub 布局与报告命名对齐产线操作；插件契约写成可给其他 AI / 开发者直接执行的规范。

### Testing Hub

- 顶栏 **Clear / Preflight / Run**（运行中 Clear / Stop）同一行按文字宽度排列，不再与 Output 的 Clear 叠在右下角。
- 参数行改为行矩形坐标放置；插件列表 painter 整行点击（文字不可选中、不重叠）。
- Output 按行整理：预检 OK/X、`[wait #n] hint`；成功 HTTP invoke 不再刷屏。

### 插件 / 契约

- `file_prefix`（表单「报告名称/前缀」）同时决定 ISF/PDF/PNG **与** `{report_dir}/{file_prefix}_summary_{stamp}.md`。
- `summary_md` 与 PNG 共置 `report_dir`，MD 用相对路径引用波形图。
- 规范全文：[`test-tools/PLUGIN_SPEC.md`](../test-tools/PLUGIN_SPEC.md)（2026-09-11）。
- `scope-serial-monitor` **v0.2.3**。

### 兼容性

- 配置键仍为 `test_tool`；生命周期仍仅 `preflight` / `run` / `stop`。
- 插件 `engines.wiparse` 仍建议 `>=1.1.8`。

---

## 1.1.8 — 2026-09-10

Testing Hub 与示波器/串口监控插件的工业级加固：布局与交互、停止语义、路径共置、触发上升沿、遗留清理。关于页展示本版简化要点。

### 新增 / UI

- Testing Hub **左右分栏**：左侧插件参数 / Advanced 已移除后的配置区，右侧 Output 占满高度；顶栏跨列（标题、状态胶囊、PreFlight/Run/Stop）。
- 插件列表 **整行可点**（修复点在标题文字无法选中）。
- 参数表单 **单列对齐**（固定标签宽 + 输入框 + 路径 `…`）。
- 主工具栏顺序：**集成测试 → 测试报告**（设置菜单同序）。
- 面板内状态胶囊替代巨幅浅色 Idle 条；运行中显示紧凑 HUD。

### 行为 / 加固

- **Stop 语义**：写 `stop_file` 后保留 `job` 最多约 30s（仍占用 Run、继续抽日志），到期再 force-kill；避免 800ms 硬杀导致锁/串口/示波器半截状态，也避免停完立刻再开第二实例。
- PreFlight 成功不再误标为 **captured**（空/`armed` → `idle`）。
- 切换插件：先填 `plugin.json` 默认值，下一帧合并 `station.json`（mtime 缓存），减轻点击卡顿。
- Windows 子进程 `CREATE_NO_WINDOW`，控制台输出进 Output。
- **`status_file` 与 `isf_dir` 共置**：`{isf_dir}/_loop_status.json`（`plugin-contract.colocateStatusWithIsf` + GUI `resolve_runtime_paths`）。
- 插件 `scope-serial-monitor` **v0.2.1**：
  - 真正实现 `rising_edge`（inactive→active）
  - HTTP invoke/`health` 失败闭环；走 `wiparse-sdk` HTTP 客户端
  - `ctx.log` 贯通；主循环 `try/finally` 释锁并尽量恢复示波器/串口
  - 预检：单实例锁、串口 API、状态路径
  - 跨 chunk 触发上下文（滚动约 80 行）
  - `start.ps1`/`stop.ps1` 改走 `runner.mjs`；去掉 tianshu/绝对路径遗留；浮动 HUD 仅 `-Hud`
- 移除遗留插件目录 `tianshu-xinwei-ask02`；`plugins/README` 约定仅保留 `example-smoke` + `scope-serial-monitor`。
- gitignore：`run.stop` / `run.lock` / 插件 logs / `_loop_status.json` 等运行时文件。

### 文档 / 部署

- README / `UPDATE.md` / MCP / 插件 engines 与 station `min_version` 对齐 **1.1.8**。
- 插件 README 明确 Testing Hub 为正式路径。

### 兼容性

- 配置键仍为 `test_tool`；MCP `wiparse_ui` 切页别名不变。
- 旧闭环计划 / DDSSS / 仪表 API 行为不变。
- 仍不支持进程内多 Session 并行（一 Run 一 Node 子进程）。

---

## 1.1.7 — 2026-09-10

产线「配方式」测试插件宿主 + 报告 / 数据分析面板，并完成契约硬化与资源/停止路径加固。关于页展示本版简化要点。

### 新增

- **集成测试（Testing Hub）**面板（原「测试工具」显示名）：发现并运行 `test-tools/plugins/*`（Node）。标准生命周期仅 `preflight` / `run` / `stop`；参数表单覆盖 station 字段（前缀、路径、串口、API 等）；轮询 `_loop_status.json` 提示。UI 为左列表 + 右三区（顶栏操作 / 参数与高级 / 日志）。配置键仍为 `test_tool`；切页别名含 `testing_hub` / `hub`。
- **插件契约**：`PLUGIN_SPEC.md`、`schemas/plugin.schema.json`、`schemas/station.schema.json`；`plugin-contract.mjs` 校验 manifest/station、`engines`、`normalizeResult`、`requestStop`、路径模板 `{data_root}` / `{product}` / `{plugin_dir}`。
- **Runner**：`node runner.mjs --lifecycle preflight|run|stop`；能力门控；engines 探测（Node 硬失败，WiParse 版本未知时 warning）。
- **插件**：
  - `example-smoke`：CLI version + 可选 API health。
  - `scope-serial-monitor`（显示名 **示波器/串口监控** / **Scope & Serial Monitor**；原 `tianshu-xinwei-ask02`）：串口上升沿 → ScopeStop + 串口停 → 截图/ISF/PDF/MD 总报告 → 恢复；参数可覆盖；保留 `hud.ps1` / `start.ps1` / `stop.ps1` 可选辅助。
- **测试报告**面板：目录浏览、轻量 Markdown IR 预览（标题/列表/表格/围栏代码/`![alt](path)` 图片）；不依赖 `egui_commonmark`。
- **数据分析**面板：串口 Live / 文件抽取多通道曲线；WIDA 磁盘缓存 + 视口 min/max LOD。
- 配置 / i18n / CLI `--panels` / MCP `wiparse_ui`：`test_tool`、`test_report`、`data_analysis` 等面板开关与切页。
- 关于页「本版更新」简要列表（中英）。

### 行为 / 加固

- Stop：主机优先写 `stop_file`（路径与 Node `loadStationConfig` 一致，相对路径相对 plugin 目录）→ 后台短暂等待 → kill；UI 线程不再 `sleep`。
- 结果契约：`{ ok, lifecycle, step?, checks?, artifacts?, error? }`；runner 打印 `[runner] result …`。
- `log.lines.get` 增加 `total`，单页 `limit` 上限 2000；ASK02 触发等待改为增量 `from_row`，避免长日志只看到前 5000 行。
- CLI / HTTP SDK：进程超时、stdout/stderr 截断；ASK02 PDF / health 超时；`resolveScope` 有限重试；ISF 等待不再每轮 `serialStop`；串口保持间隔放宽。
- 报告缺图一次失败即哨兵，避免每帧重解码；相对图片路径限制在报告目录下；Windows 外部打开路径作独立 argv。
- Live 抽取：`live_tail` 与点数有上限；WIDA 头长度与批量点读取防护。
- 运行中状态文件按 mtime 节流读取；日志 trim 按 UTF-8 边界。

### 文档 / 部署

- `test-tools/README.md`、`PLUGIN_SPEC.md`；README / `UPDATE.md` 版本对齐 **1.1.7**。
- MCP `wiparse` 包版本 **1.1.7**；`wiparse_ui` 说明含集成测试 / 报告 / 数据分析。
- 插件 `engines.wiparse` / station `min_version` 建议 **>=1.1.7**（仍兼容声明 `>=1.1.6` 的旧站配置，但新面板需本版 GUI）。

### 兼容性

- 旧闭环计划 / DDSSS / `instrument.waveform_source` 行为不变。
- 不做插件商店、签名校验或进程内动态加载；插件仍为外挂 Node 子进程。

---

## 1.1.6 — 2026-09-04

波形页离线 **DDSSS** 协议分析：对 VCTX / ILTX（或线圈电压电流）做差分解调 + 相关，再复用现有 Qi 包解码。

### 新增

- 协议下拉 `DDSSS`：通道、序列 Auto/SEQA–D、扩展码、可选手动 FOP。
- `ui.wave.bus --kind ddsss --signal 0`（可选 `--sequence` / `--extension` / `--fop`）。MCP `wiparse_ui` 的 `wave.bus` 透传同样参数。
- 合成检验波形 [`docs/examples/ddsss_vctx.isf`](examples/ddsss_vctx.isf)：SEQA，5 个 Qi 包（SS / CE / RP8 / CHS / ID）。
- 带误码扩展源 [`docs/examples/ddsss_vctx_errors.isf`](examples/ddsss_vctx_errors.isf)：15 个 ASK 包，24 个 chip 翻转，CE 奇偶错误（`CE P!`），CHS 校验错误（`CHS!`）。

### 行为

- 默认合同 SEQA、无 extension；Auto 在解出校验正确的包后停止穷举。
- FOP 范围 **85 kHz–1.78 MHz**；过零对齐分窗；浅调制自适应死区，搜不到包时再零死区重试。
- 波形标注分四行，贴在解码通道波形上沿（随 Y 轴拖动跟随，不挡模拟迹线）：包名、字节 hex、**chip→bit**（`St` / `b0`–`b7` / `P` / `Sp`）、chip 0/1。同一 bit 的 chip 与 bit 标签同色。chip 时间窗对齐两周 FOP。
- 点选数据包后侧栏显示字段解码（如 `control_error`、校验和）。误码分层标注：扩频 chip 与 Table 4 相关不一致标橙色 `x`（bit 仍可过门限解出）；校验失败 `NAME!`；奇偶失败 `NAME P!`；帧错误 `NAME F!`。不因此丢锁。侧栏统计「误码 N」，`info` 含 `cs_err` / `P_err` / `F_err` / `chip_err`。 bit 行可见时即画 chip 竖线（SEQA 每 bit 31 chip，不必再放大到单 chip 才显示）。
- X1/X2 测量光标不再作为 DDSSS 时间窗：打开光标测量或拖动光标不会清掉已有解码叠加层。 UART/I2C/SPI/I2S 仍可按光标窗口重解码。
- `info` 含 FOP、fchip、kbps、Nseq、ext、phase、invert、depth、DQM。
- 已设手动 FOP 时不再扫整条波形估频；包络对均匀采样走索引快路径。
- 一期不做串口 ASK 并行检出，也不跑 Ping/SRQ/DQM 状态机。

### 文档 / 部署

- README、`UPDATE.md`、`CLI_REFERENCE`、`WORKSTATION_CLOSED_LOOP`、`DEPLOY_MCP` 与 1.1.6 对齐（含 DDSSS 四行标注）。

---

## 1.1.5 — 2026-09-03

通用闭环：等待任意包/头/行（上升沿），阻塞式示波器停采 / 截图落盘 / 读取波形源文件。ASK 71 只是示例计划，引擎不写死 0x71。

### 新增

- 计划步骤：`wait.packet` + `rising`、`wait.header`、`wait_line`（正则 + 可选 `exclude`）。
- `instrument.command` 步骤（接受 `"ScopeStop"` 与 `{ "ScopeStop": null }`）；执行器等到完成或超时。
- `capture_scope.save: true`：阻塞直到 PNG 写入 `evidence/.../scope/`；默认 `false` 保持 `qi_pt_smoke` 的排队即过。
- `instrument.waveform_source`：与 GUI「读取波形源文件」同一路径。`dir` 有值则跳过另存为；`overwrite: false` 时加 `-2`、`-3`…。不改波形分析浏览器目录，不改 `instrument.waveform`（CURVe）。
- 配置 `apps.instruments.waveform_source_dir`（与 `save_dir` / `waveform_browser_dir` 独立）。计划占位符 `{instruments.waveform_source_dir}`。
- HTTP / CLI / MCP：`instrument.waveform_source`；完成事件 `instrument.job_done`（路径 + 字节数，无点列）。
- 示例 [`docs/examples/ask71_waveform_source.json`](examples/ask71_waveform_source.json)；工位说明 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。

### 行为

- 闭环仪表步骤在 ISF/截图完成前保持 Running，不再 `advance("queued")` 假 Pass。
- 空的 `waveform_source_dir` 不会回退到分析页目录。

### 文档 / 部署

- `CLI_REFERENCE`、`DEPLOY_API`、`DEPLOY_MCP`、MCP README 与 1.1.5 对齐。对机 zip 含示例计划与工位 MD。

---

## 1.1.4 — 2026-09-03

CLI / MCP 可驱动 GUI 全部主页面（切页、显隐工具、参数输入）。

### 新增

- CLI：`wiparse ui …`（需已启动的 `WiParse.exe`，不要加 `--local`）。
  - `state` / `show` / `panels` / `prefs`
  - `serial`：open / close / clear / filter / tab / name / browser
  - `wave`：open / close / select / browser / bus / cursor / fit
  - `calc`：get / set（`lc` `bandpass` `q` `rc` `crc` `convert`）
  - `instrument`：select / scan / list / connect / disconnect / measure / capture / waveform / command
- HTTP 有状态方法：`ui.state`、`ui.show`、`ui.panels`、`ui.prefs`、`ui.serial.*`、`ui.wave.*`、`ui.calc.*`、`ui.instrument.select`。
- MCP 第六个工具：`wiparse_ui`（`op` + 可选 `tab` / `params`）。

### 文档 / 部署

- `docs/CLI_REFERENCE.md`、`docs/DEPLOY_API.md`、`docs/DEPLOY_MCP.md`、`mcp/wiparse/README.md` 与 1.1.4 对齐。
- 对机 zip 含 `DEPLOY_MCP.md`、安装脚本、已编译 MCP；GUI 与 MCP 需同为 **1.1.4+**。

### 兼容性

- 旧 MCP（5 工具）仍可连新 GUI，但没有页面控制；对机请用本版本 zip 里的 `mcp/wiparse` 并跑 `setup-mcp.cmd`，然后**完全退出 Cursor** 再开。
- 切页不会打开串口；监控仍用 `serial start/stop/select/send`。

---

## 1.1.3 — 2026-09-03

闭环测试（C+E）、CLI/MCP 对齐，以及评审中的逻辑修正。总线解码与波形标注继续增强。

### 新增

- **闭环测试执行器**（GUI 进程内）：`test.start` / `status` / `abort` / `pack`。计划 JSON 支持 `wait` / `action` / `sleep` / `expect` / `capture_scope`，以及 `macros`、`abort`（`on_ept` / `csum_gt` / `timeout_s` / `vin_gt` / `capture_scope`）。
- **LiveBrief**：压缩会话事实（阶段、包计数、告警、关键事件），API `log.brief`，可用 `since_row` 做增量。
- **证据包**：`evidence/<时间戳>_<plan_id>/`（`manifest.json`、`serial.txt`、`metrics.csv`、`events.jsonl`、`correlate.json`、`brief_final.json`、`report.skeleton.md`）。
- **`serial.select`**：只改 GUI 口/波特率，**不打开**串口；监控已开时必须先 stop。
- MCP 五个工具：`wiparse_brief`、`wiparse_select`、`wiparse_test`、`wiparse_send`、`wiparse_report_pack`。
- CLI：`serial select`、`log brief`、`test run|status|abort|pack`。
- 对机部署脚本：`mcp/wiparse/setup-mcp.ps1` / `setup-mcp.cmd`，文档 `DEPLOY_MCP.md`。
- 示例计划：`docs/examples/qi_pt_smoke.json`。
- 大文件串口 TXT 编辑虚拟化（不全量排版）；总线解码波形标签加大，便于阅读。

### 行为变化（调用方需要知道）

- CLI **默认始终 attach** `http://127.0.0.1:7878`（或 `--url` / `WIPARSE_URL`）。GUI 未运行时报连接错误，**不再悄悄打开本机串口**。本进程独占设备必须加 `--local`（包括 `parse` / `session` / `wave` / `scope`）。
- `test.start`：若已在监控且请求的 `port`/`baud` 与当前不同，先停再按新参数打开；不再 silently 沿用旧口。
- 计划里 `action` 默认只能是 `macros` 名字或 `NOP`；裸 hex 需 `"allow_raw_hex": true`。
- `wait.packet` 查 `packet_counts`（如 CE/RP），不再只扫 notables 环。
- `abort.timeout_s` 到期一律 **Failed**（空步骤计划也不再算 Pass）。
- 示波器自动 capture 只打到示波器设备：优先当前选中的示波器，否则第一台示波器；不会打到电源/负载。
- 业务失败的 invoke 统一 **HTTP 400 + JSON 信封**；JSON 无效或缺 `method` 同样立即 400。
- `serial.monitor.status` 的 `status` 字段为 `open COMx @ baud` 或 `stopped`，不再复用文件页 UI 字符串。
- MCP 改为纯 HTTP，删除通过 CLI 包装的 `cli.ts`；环境变量只需要 `WIPARSE_URL`。

### 修复

- 测试 Send 失败或 `write_tx` 缺失时 `fail`，不再第二次 tick 当成已发送通过。
- 示波器 correlate 时间用 `live_brief.elapsed_s()`，不再固定 `t=0`。
- stateful invoke 超时注释与实现统一为 15s。
- 去掉无意义的 `sync_serial_status` 别名。

### 总线解码 / 波形（同批）

- I2C / UART / 数字阈值对振铃与毫伏级抖动更稳健；I2C 补 10-bit、General Call、repeated start 读等。
- 波形分析标签绘制优先级调整，协议标注更可读。

### 文档

- `docs/CLI_REFERENCE.md`、`docs/DEPLOY_API.md`、`mcp/wiparse/README.md` 与 1.1.3 行为对齐。

### 兼容性

- 旧 MCP（四工具、走 CLI）与 1.1.3 GUI 不匹配；对机请用本版本 zip 中的 `mcp/wiparse` 并跑 `setup-mcp.cmd`。
- GUI 与 MCP 必须同机；默认 API 只绑 `127.0.0.1`。

---

## 1.1.2 — 2026-08-30

总线解码叠到波形、协议分析修正。

- 波形上标注 START / STOP / 数据。
- 改进 I2C / SPI / UART / I2S 解码。
- 同步打包 `dist/WiParse.exe`、`dist/WiParse-CLI.exe`。

---

## 1.1.1 — 2026-08-08

波形分析、稳定性、在线更新。

- 每通道 Y 轴控制；按时间的包络 LOD。
- 仪器 worker 错误处理。
- 在线更新模块（`latest.json`、SHA256、About 检查更新）与 `docs/UPDATE.md`、`packaging/update/`。
- 同步打包 dist 二进制。

---

## 1.1.0 及更早（摘要）

来自提交历史，未单独打过 `CHANGELOG` 条目：

| 版本/提交 | 内容 |
|-----------|------|
| 分层加载与搜索 | 波形分层加载；搜索跳转 / Y 轴缩放修复；总线解码初版 |
| 示波器包络 | 专业包络显示；全密度波形采集 |
| 电流探头 | 波形电流单位；实时日志改名更安全 |
| 多厂商采集 | 示波器源采集与另存为转换 |
| Rigol WFM | 离线 Rigol WFM 解析；Tek 风格通道色 |
| 波形分析面板 | VISA 屏幕源采集；GUI 布局 |
| 串口编辑 | 串口工具编辑、鼠标命中 |
| 科学计算器 | 计算器功能优化 |
| 调试模式 | 增加调试模式 |
| 仪表重构 | 示波器 / 万用表 / 电子负载 / DC 电源 |
