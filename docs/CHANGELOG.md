# WiParse 版本记录

产品版本以工作区 `Cargo.toml` 的 `workspace.package.version` 为准（GUI / CLI / `wiparse-core` 共用）。MCP 包 `mcp/wiparse/package.json` 与之对齐。

发布流程：改版本号 → 更新本文 → `cargo build --release` 同步 `dist/` → 提交并推送。对机 MCP 安装见 [`DEPLOY_MCP.md`](DEPLOY_MCP.md)。工位闭环落地见 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。

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
