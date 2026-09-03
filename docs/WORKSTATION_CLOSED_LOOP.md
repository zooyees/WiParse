# 测试工位落地：通用闭环（wait + 示波器停采 / 截图 / 波形源）

适用版本 **WiParse 1.1.5**。工位机通常只有安装目录（exe + config + 计划 JSON），**没有** `.rs` 源码。落地 = 换本 zip 里的 GUI/CLI/MCP，改 `config.json`，用计划 JSON 跑测试。

本文说明 **改了什么**、**怎么用**、**ASK 71 只是示例**。引擎不写死 `0x71`。

---

## 1. 改了什么（相对 1.1.4）

### 1.1 通用等待（上升沿）

| 步骤 | 含义 | 工位怎么写 |
|------|------|------------|
| `wait.packet` | 解码后的 Qi 包名（0x71 的名字是 **`ID`**，不是把 0x71 写进引擎） | `"packet": "ID"`，建议 `"rising": true` |
| `wait.header` | 任意 header 字节 | `"header": "0x71"` 或 `113` |
| `wait.phase` | 当前阶段（与以前相同，**不是**上升沿） | `"phase": "pt"` |
| `wait_line` | 对 **新出现** 的 live 行做正则 | `"regex": "ASK\\s+71"`，可选 `exclude` |

`rising: true`：进入该步骤时先拍快照，**之后新出现的包/头**才算命中。用来避免屏幕上已有旧 ASK 71 导致立刻 Pass。

`wait_line` 始终是上升沿：步骤开始时已在环形缓冲里的行不算。命中行会写入证据包摘要的 `last_hit_line`。

默认超时：`wait` 仍 8s；`wait_line` / 仪表步骤默认 **600s**（ISF 读取经常要 30–75s）。

### 1.2 仪表步骤（与触发解耦，执行器会阻塞）

| 步骤 | 行为 | 阻塞？ |
|------|------|--------|
| `instrument.command` | 与 GUI 控件同一套 `ControlCommand`。可用 `"ScopeStop"` 或 `{ "ScopeStop": null }` | 等到命令完成或 `timeout_s` |
| `capture_scope` + `"save": true` | 截 PNG **写到** `evidence/<run>/scope/<tag>.png` | 等到文件写出 |
| `capture_scope`（默认 `save: false`） | 与 1.1.3/`qi_pt_smoke` 相同：只排队截图，立刻进入下一步 | 不阻塞 |
| `instrument.waveform_source` | 等同 GUI 按钮 **「读取波形源文件」**（显示通道的源文件/ISF，不是 CURVe） | 等到落盘或超时 |

**不会做的事：**

- 不改 `instrument.waveform`（CURVe 采样）
- 不调用 `ui.wave.browser`，也不改波形分析页的 `waveform_browser_dir`
- 不把 40MB ISF 或点列塞进 MCP；只记 **路径 + 字节数**
- 不把 UIA 点按钮当产品路径

### 1.3 配置（与分析浏览器目录分开）

`config.json` → `apps.instruments`：

```json
"waveform_browser_dir": "D:\\waves\\analysis",
"waveform_source_dir": "D:\\waves\\isf_out"
```

- `waveform_source_dir`：闭环 `instrument.waveform_source` 的默认目录（计划里可用占位符）。
- 空字符串 **不会** 回退去改分析页浏览器目录。未配 `dir` 时弹出「另存为」（不适合无人值守）。

计划 JSON 占位符：`{instruments.waveform_source_dir}`（`test.start` 时替换）。

### 1.4 API / CLI / MCP

- HTTP：`instrument.waveform_source`（立即 `{ accepted, job_id, device_id }`；完成看事件 `instrument.job_done`，含 `path` / `bytes`）
- CLI：`wiparse ui instrument waveform-source --id 1 --dir D:\isf --filename ask71.isf`
- MCP：`wiparse_ui` 增加 `op=instrument.waveform_source`（仍是 6 个工具，不是新工具名）
- `instrument.command` 接受 `"ScopeStop"` 字符串

闭环仍用 `wiparse_test start` + 计划 JSON；不要让模型去点 GUI，也不要 Read `serial.txt`。

---

## 2. 工位怎么换版本

1. **完全退出** 正在跑的 `WiParse.exe`（不要覆盖正在运行的 exe）。
2. 解压新的 `WiParse-Deploy.zip` 覆盖安装目录（推荐 `D:\software\WiParse` 或工位现用路径如 `D:\SOFTWARE\WiParse-Rust`）。
3. 保留工位自己的 `config.json`（口、波特率、VISA 资源）。**补上** `apps.instruments.waveform_source_dir`（指向要存 ISF 的目录，需可写）。
4. 拷贝示例计划 `docs\examples\ask71_waveform_source.json`（或按下面改自己的计划）。
5. 覆盖 `mcp\wiparse` 后跑 `setup-mcp.cmd`，然后 **完全退出 Cursor 再打开**（半重启会仍是旧 MCP）。
6. 先双击新 `WiParse.exe`，仪表页确认示波器已连接，串口页选好口。

GUI 与 MCP 必须同为 **1.1.5+**。

---

## 3. 示例：ASK 71 出现后停示波器并保存 ISF

引擎只认识「包名 ID / header / 行正则」，**ASK 71 不是写死逻辑**。下面这份计划是给工位抄的例子。

文件：[`docs/examples/ask71_waveform_source.json`](examples/ask71_waveform_source.json)

```json
{
  "id": "ask71_waveform_source",
  "abort": { "timeout_s": 900, "capture_scope": false },
  "steps": [
    { "wait": { "packet": "ID", "rising": true, "timeout_s": 600 } },
    { "instrument.command": { "command": "ScopeStop", "timeout_s": 15 } },
    { "capture_scope": { "tag": "ask71", "save": true, "timeout_s": 60 } },
    {
      "instrument.waveform_source": {
        "dir": "{instruments.waveform_source_dir}",
        "filename": "ask71.isf",
        "overwrite": false,
        "timeout_s": 600
      }
    }
  ]
}
```

若解码名不稳定、更想盯串口原文，把第一步换成：

```json
{ "wait_line": { "regex": "ASK\\s+71", "timeout_s": 600 } }
```

或 `"header": "0x71"`。三者都是通用原语，选一种即可。

`overwrite: false` 时若 `ask71.isf` 已存在，会写成 `ask71-2.isf`、`ask71-3.isf`…

跑法（GUI 已开、示波器已连、串口已选）：

```powershell
cd D:\software\WiParse
.\WiParse-CLI.exe test run --plan docs\examples\ask71_waveform_source.json --port COM3 --baud 2000000
.\WiParse-CLI.exe test status
.\WiParse-CLI.exe test pack
```

MCP：`wiparse_test` `action=start`，`plan` 贴上述 JSON。轮询 `wiparse_brief` / `wiparse_test status`，结束用 `wiparse_report_pack`。

### 证据在哪

- 运行目录：`evidence\<时间戳>_ask71_waveform_source\`
- PNG：`scope\ask71.png`（仅 `save: true`）
- ISF：在 `waveform_source_dir`（或步骤里写死的 `dir`），**路径**记在 `correlate.json` / pack 摘要，默认不复制 40MB 进证据包
- `last_hit_line`：若用了 `wait_line`

旧的 `qi_pt_smoke.json` 不用改：`capture_scope` 默认仍是 fire-and-forget。

---

## 4. 单独调示波器（不跑整份计划）

示波器必须已在 GUI 仪表页连接。

```powershell
.\WiParse-CLI.exe ui instrument list
.\WiParse-CLI.exe ui instrument command --id 1 --json "\"ScopeStop\""
.\WiParse-CLI.exe ui instrument waveform-source --id 1 --dir D:\isf --filename ask71.isf
```

MCP：`wiparse_ui` `op=instrument.command` / `instrument.waveform_source`，参数放在 `params`。

`dir` 省略会弹「另存为」；无人值守必须带 `dir` 或配好 `waveform_source_dir`。

---

## 5. 不要用的做法

- 不要用 UIA / 坐标去点「读取波形源文件」
- 不要把 ISF 路径塞进 `instrument.waveform`（那是 CURVe）
- 不要把 `capture_scope` 当成读波形源
- 不要让 Cursor 去 Read 串口 txt 或 40MB ISF
- Cursor 若显示 0 个 MCP 工具：那是宿主 IPC 问题，**完全退出 Cursor**，不是本功能的替代实现

---

## 6. 工位验收清单

1. 关于页 / `wiparse version` 为 **1.1.5**
2. `config.json` 已设 `waveform_source_dir`，且与 `waveform_browser_dir` 不是同一个需求混用
3. 示波器已连接；点一次 GUI「读取波形源文件」仍会弹另存为（按钮行为未改）
4. 跑 `ask71_waveform_source.json`：新出现 ID/ASK71 后示波器停止，PNG 在 evidence，ISF 在配置目录
5. 旧 `qi_pt_smoke.json` 仍能 Pass
6. Cursor 里 `wiparse_ui` 的 `op` 能选到 `instrument.waveform_source`；改 MCP 后必须全退 Cursor
