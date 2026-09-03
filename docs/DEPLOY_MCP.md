# 在另一台电脑部署 WiParse MCP

MCP **不能单独工作**：它只通过 HTTP 连本机已启动的 `WiParse.exe`（默认 `http://127.0.0.1:7878`）。对机需要 **GUI 1.1.5+ + Node.js 18+ + Cursor**。闭环工位步骤见 [`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。

推荐安装目录（可改）：`D:\software\WiParse`

本版本 MCP 有 **6** 个工具：`wiparse_brief`、`wiparse_select`、`wiparse_test`、`wiparse_send`、`wiparse_report_pack`、`wiparse_ui`。

---

## 1. 对机要拷什么

用仓库打好的 **`WiParse-Deploy.zip`**（不要只拷 `dist\index.js`，也不要拷源码仓库 / `target\`）。解压后应有：

```
WiParse.exe
WiParse-CLI.exe
config.default.json
DEPLOY_MCP.md          ← 本文
DEPLOY_API.md
  docs\
  CLI_REFERENCE.md
  WORKSTATION_CLOSED_LOOP.md
  examples\qi_pt_smoke.json
  examples\ask71_waveform_source.json
mcp\wiparse\
  dist\index.js        ← 已编译，不必再 tsc
  package.json
  package-lock.json
  setup-mcp.ps1
  setup-mcp.cmd
  README.md
  cursor.mcp.example.json
  node_modules\        ← 若 zip 里已带，可不上网；没有则 setup 会 npm install
```

---

## 2. 对机一次性准备

1. 安装 **Node.js 18+ LTS**（勾选 Add to PATH）  
   https://nodejs.org/
2. 安装 **Cursor**
3. 解压 zip 到例如 `D:\software\WiParse`
4. **先双击 `WiParse.exe`**，确认能打开
5. 安装 MCP（任选一种）：

**方式 A（推荐）** 双击：

```
D:\software\WiParse\mcp\wiparse\setup-mcp.cmd
```

**方式 B** PowerShell：

```powershell
cd D:\software\WiParse\mcp\wiparse
powershell -ExecutionPolicy Bypass -File .\setup-mcp.ps1 -RegisterUser
```

脚本会：

- 检查 Node ≥ 18
- 若缺依赖则 `npm install --omit=dev`
- 用 **node.exe 绝对路径** 写入 `%USERPROFILE%\.cursor\mcp.json`（避免 Cursor 找不到 PATH 里的 node）
- 探测 GUI `/v1/health`（没开 GUI 只警告，不失败）

只给某个工程用、不写用户配置：

```powershell
.\setup-mcp.ps1 -ProjectDir "D:\your\project"
```

6. **完全退出并重新打开 Cursor**（MCP 只在启动时读配置；半重启经常仍是旧工具列表）

---

## 3. 手动配置（不用脚本时）

编辑 `%USERPROFILE%\.cursor\mcp.json`（没有就新建）。路径必须是对机真实路径，建议正斜杠，`command` 用 `node.exe` 全路径：

```json
{
  "mcpServers": {
    "wiparse": {
      "command": "C:/Program Files/nodejs/node.exe",
      "args": ["D:/software/WiParse/mcp/wiparse/dist/index.js"],
      "env": {
        "WIPARSE_URL": "http://127.0.0.1:7878"
      }
    }
  }
}
```

`node.exe` 不在默认位置时，在对机运行 `where.exe node` 把输出填进 `command`。

---

## 4. 日常使用

1. 先开 `WiParse.exe`（必须长驻）
2. 再开 Cursor
3. 应看到 **6** 个工具：

| 工具 | 作用 |
|------|------|
| `wiparse_brief` | 压缩会话事实（阶段、包计数、告警）。用 `since_row` 做增量。 |
| `wiparse_select` | 只改 GUI 口/波特率，**不打开**串口。监控已开时需先 stop。 |
| `wiparse_test` | `start` / `status` / `abort` / `pack`。`start` 需要计划 JSON。 |
| `wiparse_send` | 向 GUI 监控口排队 hex（优先用计划 macros）。 |
| `wiparse_report_pack` | 证据包摘要。据此写报告，不要读 `serial.txt`。 |
| `wiparse_ui` | 切页 / 显隐工具 / 语言主题 / 各页参数。`op` + 可选 `tab` / `params`。 |

Agent 习惯：`wiparse_ui`（`op=show` `tab=serial`）→ `wiparse_select`（只选口）→ `wiparse_test start` → 轮询 `wiparse_brief` → `wiparse_report_pack` 写报告。不要让模型去 Read `serial.txt`。

`wiparse_ui` 示例：

| 目标 | `op` | 其它参数 |
|------|------|----------|
| 看当前页 | `state` | — |
| 切到串口页 | `show` | `tab=serial` |
| 切到计算器 | `show` | `tab=calculator` |
| 切到仪表 | `show` | `tab=instruments` |
| 切到波形 | `show` | `tab=waveform` |
| 计算器填参 | `calc.set` | `params={ "card":"lc", "fields":{ "inductance":"10", "capacitance":"100" } }` |
| 打开波形文件 | `wave.open` | `params={ "path":"D:/waves/scope.csv" }` |

示例计划在包内 `docs\examples\qi_pt_smoke.json`。

CLI 验收（可选）：

```powershell
cd D:\software\WiParse
.\WiParse-CLI.exe api health
.\WiParse-CLI.exe api capabilities
.\WiParse-CLI.exe ui state
.\WiParse-CLI.exe ui show --tab serial
```

---

## 5. 对机已有 WiParse、只补 MCP

把 zip 里的 `mcp\wiparse` **整个覆盖**到现有安装目录（与 `WiParse.exe` 相对路径为 `mcp\wiparse`），然后执行第 2 步的 `setup-mcp.cmd`，并**完全退出 Cursor**。

GUI 必须是 **1.1.5+**（含 `instrument.waveform_source` 与阻塞式闭环仪表步骤）。旧 GUI 配新 MCP 会缺方法；只换 MCP 不换 GUI 时，`wiparse_ui` 的 `instrument.waveform_source` 会报 unknown method。

若 Cursor 仍只显示 4～5 个工具：配置已写入但进程没重载，请杀掉所有 Cursor 进程再开。

---

## 6. 故障

| 现象 | 处理 |
|------|------|
| Cursor 里没有 wiparse | 确认 mcp.json 已写、路径存在；完全退出 Cursor |
| 只有旧工具、没有 `wiparse_ui` | 覆盖了旧 `mcp\wiparse` 后必须完全重启 Cursor；确认 `dist\index.js` 来自 1.1.5 zip |
| `GUI API down` / connect failed | 先开 `WiParse.exe`；或检查 `WIPARSE_URL` 是否与 `WIPARSE_API_BIND` 一致 |
| `unknown method: ui.show` / `instrument.waveform_source` | GUI 太旧，换成 zip 里的 `WiParse.exe`（1.1.5+） |
| `Cannot find module @modelcontextprotocol/sdk` | 在 `mcp\wiparse` 跑 `npm install --omit=dev`，或改用带 `node_modules` 的 zip |
| `node` 不是内部命令 / Cursor 起不来 MCP | `command` 改成 `C:/Program Files/nodejs/node.exe`（或 `where.exe node` 的路径） |
| 选口失败、监控已开 | 先在 GUI 停监控，或让 Agent 调 stop 再 `wiparse_select` |
| 7878 连不上 | 看 GUI 是否改了 `WIPARSE_API_BIND`；本机防火墙；不要用另一台机器的 IP（默认只绑 127.0.0.1） |
| Cursor `Failed to acquire MessagePort` / 0 tools | Cursor 宿主 IPC 问题，与 WiParse 无关：完全退出 Cursor 再开 |

MCP **不要**对公网暴露。两台电脑之间远程控串口不在本架构内：MCP 与 GUI 必须在同一台机器。
