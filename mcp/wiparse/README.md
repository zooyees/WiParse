# WiParse MCP Server 1.1.30

通过 [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) 把 WiParse 暴露给 Agent。

架构 **C+E**：直连运行中的 `WiParse.exe` API（`WIPARSE_URL`）。闭环测试在 GUI 进程内执行；MCP 只拉 **Brief / 证据摘要**，不传原始串口或波形点列。

## 前置条件

- Node.js 18+
- 已启动的 `WiParse.exe`；建议与 MCP 同为 1.1.30，旧 GUI 不包含本次报告修复。

对机部署（解压 zip、注册 Cursor）见 [`docs/DEPLOY_MCP.md`](../../docs/DEPLOY_MCP.md)。双击包内 `mcp/wiparse/setup-mcp.cmd` 即可。

## 开发构建

```powershell
cd mcp/wiparse
npm install
npm run build
npm test
```

## 环境变量

| 变量 | 说明 |
|------|------|
| `WIPARSE_URL` | GUI API 根地址（默认 `http://127.0.0.1:7878`） |

## Cursor 配置

不要手写本仓库路径。在对机运行 `setup-mcp.ps1 -RegisterUser`，或把生成的 `cursor.mcp.generated.json` 拷进 `%USERPROFILE%\.cursor\mcp.json`。

示例（路径按对机安装目录改）：[`cursor.mcp.example.json`](cursor.mcp.example.json)。

## MCP 工具（6 个）

| 工具 | 说明 |
|------|------|
| `wiparse_brief` | 压缩会话事实（阶段、计数、告警、关键事件）。用 `since_row` 做 cursor。 |
| `wiparse_select` | 只改 GUI 口/波特率，**不打开**串口。监控已开时需先 stop。 |
| `wiparse_test` | `start` / `status` / `abort` / `pack`。`start` 需要计划 JSON。 |
| `wiparse_send` | 向 GUI 监控口排队 hex（优先用计划里的 macros）。 |
| `wiparse_report_pack` | 证据包摘要（路径 + brief + correlate）。据此写报告，不要读 `serial.txt`。 |
| `wiparse_ui` | 切页 / 面板 / 语言主题 / 各页参数。`op`：`show`、`instrument.overview`、`project.list|select|create|add_test|remove_test`、`report.open`（不回传正文）、`test_tool.run|stop`、`wave.bus`、`instrument.command` Probe*/Bridge*。可选 `tab` / `params`。 |

示例计划：[`docs/examples/qi_pt_smoke.json`](../../docs/examples/qi_pt_smoke.json)、[`docs/examples/ask71_waveform_source.json`](../../docs/examples/ask71_waveform_source.json)、[`docs/examples/ddsss_vctx.isf`](../../docs/examples/ddsss_vctx.isf)。工位说明：[`docs/WORKSTATION_CLOSED_LOOP.md`](../../docs/WORKSTATION_CLOSED_LOOP.md)（含离线 DDSSS 四行标注）。

Agent 约定：不要 `Read` 日志文件；不要订阅 `/v1/events`。

CLI 说明见 [`docs/CLI_REFERENCE.md`](../../docs/CLI_REFERENCE.md)。

## 1.1.30 测试报告使用

服务启动声明的版本直接读取 package.json，避免已编译代码和包版本不一致。六个工具名称和既有接口保持兼容。

使用 `wiparse_ui` 的 `project.select` 选择 default，再通过 `report.open` 传入活动项目内的完整文件路径；省略 view 时按扩展名识别。MD 显示报告，JSON/JSONL 在报告工具内显示数据，ISF/WFM/CSV 打开波形，PNG 显示图片。报告内数据链接可普通单击，并用“返回报告”回到 MD。MCP 只返回操作状态，不传报告正文或波形点列。

示例报告位于发布包 `projects/default/tests/ScopeSerialDemo/runs/20261013_231510/artifacts/reports/`。以实际解压路径替换路径前缀；跨项目路径仍被拒绝。

独立 WiParse-Service 的自动化开发接口及现场刺激限制见 `docs/AUTOMATION-SERVICE.md`；本 MCP 六工具连接 GUI，不会自动代替该服务或批准实物运行。
