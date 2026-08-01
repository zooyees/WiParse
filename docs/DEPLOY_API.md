# WiParse C+E 部署说明（GUI 内嵌 API）

架构：**C（GUI 单进程内嵌 localhost API）+ E（MCP/Agent 可直接 HTTP）**。

- **长驻进程**：`WiParse.exe` 独占串口 / 仪器，画面走进程内通道（不经网络）。
- **嵌入式 API**：默认 `http://127.0.0.1:7878`
- **CLI**：自动探测或通过 `--url` / `WIPARSE_URL` attach；`--local` 强制独立模式。
- **MCP**：优先 `WIPARSE_URL` + `wiparse_invoke`；也可走 CLI attach。

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
| `WIPARSE_LOCAL` | MCP 侧设为任意值时强制 CLI 本地模式 | （未设） |
| `WIPARSE_CLI_PATH` | MCP 解析 CLI 路径 | 自动探测 `dist/WiParse-CLI.exe` |

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

**有状态方法**（需 GUI 主线程，共享会话）示例：

- `serial.monitor.start` / `stop` / `status`
- `serial.send`（需先 `monitor.start`）
- `instrument.*`、`log.tabs.list`、`log.lines.get`、`system.ui.state`

无状态方法（parse / session / scope / wave 等）可在 API 线程直接执行。

### `GET /v1/events?since_seq=0`

NDJSON 事件流（Agent 订阅用），例如 `serial.line`、`instrument.measurements`。

CLI：

```powershell
.\dist\WiParse-CLI.exe api events --since-seq 0
```

---

## 5. CLI 用法（attach）

GUI 已启动时，CLI 会自动探测默认 API；也可显式指定：

```powershell
$env:WIPARSE_URL = "http://127.0.0.1:7878"

.\dist\WiParse-CLI.exe api health
.\dist\WiParse-CLI.exe api capabilities
.\dist\WiParse-CLI.exe api invoke --method serial.ports --params "{}"

.\dist\WiParse-CLI.exe ports
.\dist\WiParse-CLI.exe serial send --port COM3 --hex AA55
.\dist\WiParse-CLI.exe --url http://127.0.0.1:7878 parse line --text "..."
```

强制本地（不 attach、自己开串口）：

```powershell
.\dist\WiParse-CLI.exe --local ports
```

典型 Agent 串口流程：

```powershell
.\dist\WiParse-CLI.exe api invoke --method serial.monitor.start --params "{\"port\":\"COM3\",\"baud\":2000000}"
.\dist\WiParse-CLI.exe api invoke --method serial.send --params "{\"hex\":\"AA55\"}"
.\dist\WiParse-CLI.exe api invoke --method serial.read --params "{\"max_logs\":50}"
.\dist\WiParse-CLI.exe api invoke --method serial.monitor.stop --params "{}"
```

---

## 6. MCP（E：直连 HTTP）

`mcp/wiparse` 新增工具：

- `wiparse_health` / `wiparse_capabilities` / `wiparse_invoke`（直连 GUI API）
- 原有 `wiparse_ports` 等仍走 CLI；若设置了 `WIPARSE_URL`，CLI 会带 `--url` attach

Cursor / MCP 配置示例：

```json
{
  "mcpServers": {
    "wiparse": {
      "command": "node",
      "args": ["D:/windlink/windlink/WiParse-R/mcp/wiparse/dist/index.js"],
      "env": {
        "WIPARSE_URL": "http://127.0.0.1:7878",
        "WIPARSE_CLI_PATH": "D:/windlink/windlink/WiParse-R/dist/WiParse-CLI.exe"
      }
    }
  }
}
```

构建 MCP：

```powershell
cd mcp\wiparse
npm install
npm run build
```

Agent 推荐：先 `wiparse_health`，再 `wiparse_capabilities`，之后一律 `wiparse_invoke`。

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
- [ ] `api capabilities` 含 `serial.*` / `instrument.*`
- [ ] `api invoke --method serial.ports`
- [ ] MCP `wiparse_invoke` 调用 `system.ui.state` 有返回
- [ ] `dist` 中两个 exe 为本次 release 构建
