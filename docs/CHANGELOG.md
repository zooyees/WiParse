# WiParse 版本记录

产品版本以工作区 `Cargo.toml` 的 `workspace.package.version` 为准（GUI / CLI / `wiparse-core` 共用）。MCP 包 `mcp/wiparse/package.json` 与之对齐。

发布流程：改版本号 → 更新本文 → `cargo build --release` 同步 `dist/` → 提交并推送。对机 MCP 安装见 [`DEPLOY_MCP.md`](DEPLOY_MCP.md)。工位闭环落地见 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。关于页仅展示简化要点；完整备份以本文为准。

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
