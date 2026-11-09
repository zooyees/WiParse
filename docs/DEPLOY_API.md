# WiParse C+E 部署说明（GUI 内嵌 API）

> 2026-11-08：下方早期单界面部署方式仍适用于独立 egui。Tauri 开发版本使用 `WiParse.exe` 新壳与 `WiParse-Host.exe` 兼容宿主，不能混用旧二进制；新 companion 要求全部调用认证。统一验收尚未完成。

架构：**C（GUI 单进程内嵌 localhost API）+ E（MCP/Agent 可直接 HTTP）**。

- **长驻进程**：`WiParse.exe` 独占串口 / 仪器，画面走进程内通道（不经网络）。
- **嵌入式 API**：默认 `http://127.0.0.1:7878`
- **CLI**：默认 attach `WIPARSE_URL` / `--url` / `http://127.0.0.1:7878`；GUI 未运行则报连接错误，不会悄悄打开本机串口。`--local` 才在本进程独占设备。
- **MCP**：`WIPARSE_URL` + `wiparse_brief` / `wiparse_select` / `wiparse_test` / `wiparse_send` / `wiparse_report_pack` / `wiparse_ui`（HTTP，紧凑 JSON）。

不单独部署 daemon。

---

## 1. 产物与目录

Release 打包后建议目录：

```
dist/
  WiParse.exe          # GUI（内嵌 API）
  WiParse-CLI.exe      # 薄 CLI（可 attach）
```

本机构建：

```powershell
cargo build --release -p wiparse-gui -p wiparse-cli
Copy-Item target\release\wiparse-gui.exe dist\WiParse.exe -Force
Copy-Item target\release\wiparse.exe     dist\WiParse-CLI.exe -Force
```

---

## 2. 日常启动

1. **先启动 GUI**（必须长驻）  
   双击 `WiParse.exe`，或：

   ```powershell
   .\dist\WiParse.exe
   ```

2. API 默认监听 `127.0.0.1:7878`。改绑定：

   ```powershell
   $env:WIPARSE_API_BIND = "127.0.0.1:7879"
   .\dist\WiParse.exe
   ```

3. 健康检查：

   ```powershell
   .\dist\WiParse-CLI.exe api health
   # 或
   curl http://127.0.0.1:7878/v1/health
   ```

---

## 3. 环境变量

| 变量 | 作用 | 默认 |
|------|------|------|
| `WIPARSE_API_BIND` | GUI 内嵌 API 监听地址（仅 GUI） | `127.0.0.1:7878` |
| `WIPARSE_URL` | CLI / MCP 连接的 API 根 URL | `http://127.0.0.1:7878` |

---

## 4. HTTP API

### `GET /v1/health`

进程存活与简要状态。

### `GET /v1/capabilities`

方法目录、参数示例、事件类型。

### `POST /v1/invoke`

```json
{ "method": "serial.ports", "params": {} }
```

响应信封：

```json
{
  "ok": true,
  "cmd": "serial.ports",
  "ts": "...",
  "data": { }
}
```

失败的 invoke 返回 **HTTP 400** 与同一信封（`ok: false` + `error.message`）。CLI 会展开该字段；只有 TCP 连不上才提示 GUI 未运行。

**有状态方法**（需 GUI 主线程，共享会话）示例：

- `serial.monitor.start` / `stop` / `status`（`serial.status` 为 status 别名）
- `serial.select`（只改口/波特率，不打开；监控已开时需先 stop）
- `serial.send` / `serial.read`（需先 `monitor.start`；停着时读缓冲用 `log.lines.get`）
- `log.lines.get`：`from_row` 为会话绝对序号；`total` 为下一序号（FIFO 后继续增长）。另有 `dropped`（已挤出窗口的行数）、`window`（当前缓冲长度）、`next_row`、`truncated`（游标已被挤掉）。实时页窗口仍是 5000 行。
- `instrument.*`（含 `instrument.overview` / `instrument.connect_all` / `instrument.waveform_source`）、`log.tabs.list`、`log.lines.get`、`log.brief`、`system.ui.state`
- `ui.show` / `ui.panels` / `ui.prefs` / `ui.serial.*` / `ui.wave.*`（`ui.wave.bus` 支持 `kind=ddsss`） / `ui.calc.*` / `ui.instrument.select`（`overview` / `kind` / `device_id`）
- `test.start` / `status` / `abort` / `pack`（闭环执行器 + 证据包）

无状态方法（parse / session / scope / wave 等）可在 API 线程直接执行。

### `GET /v1/events?since_seq=0`

NDJSON 事件流（Agent 订阅用），例如 `serial.line`、`instrument.measurements`。

CLI：

```powershell
.\dist\WiParse-CLI.exe api events --since-seq 0
```

---

## 5. CLI 用法（attach）

缺省 attach 默认 API（不先做 health 探测）。GUI 未开时会连接失败，而不是改走 `--local`。也可显式指定：

```powershell
$env:WIPARSE_URL = "http://127.0.0.1:7878"

.\dist\WiParse-CLI.exe api health
.\dist\WiParse-CLI.exe api capabilities
.\dist\WiParse-CLI.exe serial select --port COM4 --baud 200000
.\dist\WiParse-CLI.exe serial start --port COM3 --baud 2000000
.\dist\WiParse-CLI.exe serial send --port COM3 --hex AA55
.\dist\WiParse-CLI.exe serial read --port COM3 --max-logs 50
.\dist\WiParse-CLI.exe serial stop

.\dist\WiParse-CLI.exe ui show --tab serial
.\dist\WiParse-CLI.exe ui state
.\dist\WiParse-CLI.exe ui calc set --card lc --params "{\"inductance\":\"10\",\"capacitance\":\"100\"}"

.\dist\WiParse-CLI.exe api invoke serial.ports
.\dist\WiParse-CLI.exe api invoke --method serial.ports --params "{}"
```

强制本地（不 attach、自己开串口）：

```powershell
.\dist\WiParse-CLI.exe --local ports
```

典型 Agent 串口流程：

```powershell
.\dist\WiParse-CLI.exe serial start --port COM3 --baud 2000000
.\dist\WiParse-CLI.exe serial send --port COM3 --hex AA55
.\dist\WiParse-CLI.exe serial read --port COM3 --max-logs 50
.\dist\WiParse-CLI.exe serial stop
```

---

## 6. MCP（E：直连 HTTP）

`mcp/wiparse` 六个工具（紧凑 JSON，HTTP）：`wiparse_brief`、`wiparse_select`、`wiparse_test`、`wiparse_send`、`wiparse_report_pack`、`wiparse_ui`。不要把 `serial.txt` 或 ISF 点列读进模型。`wiparse_ui` 的 `op=instrument.overview` 返回孪生面板状态（无波形点列）；`op=instrument.waveform_source` 只返回路径/字节数。`op=project.*` / `report.open` 操作测试报告项目索引（路径须在当前项目 jail 内，**不**回传文件正文）。`op=test_tool.run` / `test_tool.stop` 驱动 Testing Hub 插件。

闭环计划与工位步骤见 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。

对机部署（含安装脚本）：见 [`DEPLOY_MCP.md`](DEPLOY_MCP.md)。不要把本仓库开发路径写进对机的 Cursor 配置。

Cursor / MCP 配置由 `mcp/wiparse/setup-mcp.ps1 -RegisterUser` 按对机真实路径生成；示例见 `mcp/wiparse/cursor.mcp.example.json`。

开发机重新编译 MCP：

```powershell
cd mcp\wiparse
npm install
npm run build
```

Agent 推荐：`wiparse_ui` 切到串口页 → `wiparse_select` 选口（不打开）→ `wiparse_test start` → 轮询 `wiparse_brief` → `wiparse_report_pack` 写报告。

---

## 7. 约束与注意

1. **必须先开 GUI**：有状态串口/仪器与画面共享同一进程。
2. **本机回环**：默认只绑 `127.0.0.1`，不要对公网暴露。
3. **串口互斥**：GUI 已 `monitor.start` 时，勿再用 `--local` 抢同一端口。
4. **仪器异步**：`instrument.measure` / `waveform` 等多为 `{ accepted: true }`，结果看 `/v1/events`。
5. 无 GUI 的 CI/脚本可继续 `--local` 或旧 headless 流程。

---

## 8. 快速验收清单

- [ ] 启动 `WiParse.exe`
- [ ] `WiParse-CLI.exe api health` → `ok: true`
- [ ] `api capabilities` 含 `ui.show` / `serial.select` / `instrument.*` / `ui.project.list` / `ui.report.open`
- [ ] `WiParse-CLI.exe ui state`（GUI 已开）
- [ ] MCP `wiparse_brief` / `wiparse_select` / `wiparse_ui` 有返回
- [ ] `dist` 中两个 exe 为本次 release 构建


## Tauri companion 桥协议 2

新壳验证产品 ID、桥协议、实例 UUID 和进程 ID；找不到匹配宿主时只启动同包兼容宿主，不把新壳自身作为后备宿主。设备、串口和插件运行状态继续由兼容宿主拥有。

健康检查公开身份元数据，控制调用和事件要求当前实例 Bearer 凭据。Next 启动时传递进程私有凭据；CLI 在当前用户上下文读取 DPAPI 保护的凭据引用；Node SDK/MCP 只向固定的 `127.0.0.1:7878` 提交实例凭据。自定义远程端点不接收该本机凭据。普通独立 egui 保留原接口策略。

客户端在网络超时、断开、无效响应或 HTTP 5xx 后报告结果未知，不得自动重放控制命令。只有明确 401 拒绝允许通过凭据感知 CLI 提交一次。任务需以返回的 `job_id` 和设备/资源上下文匹配完成结果。

当前新增适配包括：

- `ui.preferences.get_safe|set_safe`：受 revision 保护的外观、波特率默认值，不重启当前连接。
- `ui.serial.recording_settings_safe`：停止全部串口后修改后续记录的文件名、目录和启用状态，不重命名旧文件；`ui.serial.new_log_safe` 需要当前实时页身份、内容代际和明确同意。
- `ui.instrument.control_safe`：类型化手动命令及型号、通道、测试互锁检查；`ui.instrument.acquisition_safe` 控制已有采集调度；`ui.instrument.export_samples_safe` 返回异步 CSV 导出任务，已有目标不覆盖。
- `ui.test_tool.config.edit|project.edit`：完整 Schema 编辑，工程、插件、版本、哈希和 Schema 身份检查后保留原隐藏值；来源引用随数组重排保留，不按新索引交换隐藏字段。
- `ui.ai.serial.*_safe`：参见 [AI 接口](AI_INTERFACE.md)，复用原会话、告警和有限工具执行路径。

本节是源码契约说明，不是验收通过声明。集中验证与 EXE 预览由 `scripts/verify-next-candidate.ps1`、`scripts/stage-next-preview.ps1` 管理；不自动覆盖现场安装，也不创建 ZIP。

### Next 手动连接适配（2026-11-08）

`ui.instrument.connect_manual_safe` 接收 `resource` 和可选的 `kind`。地址去除外部空白后限 1024 字节、禁止控制字符；类型使用现有 `InstrumentKind` 枚举，null 表示自动识别。保留 VISA 别名、TCPIP、USB、probe/bridge/dongle URI 的原驱动语义，不通过 shell 执行地址。

与扫描资源连接共用原设备 worker；测试正在运行、扫描正在进行或资源已连接/等待连接时拒绝。成功回复只表示连接请求已入队，实际结果仍从 `instrument.list.resources` 的 `connecting` / `connected` / `connection_error` 查询。超时或未知结果不自动重发。新增宿主语义特性 `manual-connect-v1`，Next 要求对应宿主支持。
