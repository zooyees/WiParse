# WiParse CLI 参考

JSON CLI（`wiparse` / `WiParse-CLI.exe`）。**默认 attach** 到已启动的 `WiParse.exe`（`http://127.0.0.1:7878`）；`--local` 才在本进程开串口/示波器。

```powershell
wiparse --help
wiparse serial --help
```

## 全局选项

| 选项 | 说明 |
|------|------|
| `--pretty` | 美化 JSON |
| `--quiet` / `-q` | 只输出 `data` 或 `error` |
| `--config <path>` | 配置文件（`WCM_CONFIG`） |
| `--url <url>` | GUI API 地址 |
| `--local` | 不 attach，本进程独占设备 |

输出始终是 JSON 信封（stdout 成功 / stderr 失败 exit 1）。`serial stream` 与 `api events` 为 NDJSON，无信封。GUI 业务失败是 HTTP 400 + 同一信封；CLI 展开 `error.message`，只有真正连不上才提示 `Is WiParse.exe running?`。

```json
{ "ok": true, "cmd": "version", "ts": "...", "data": { } }
```

## 命令树

```
wiparse
├── version
├── ports
├── api                 # 需 GUI
│   ├── health | capabilities | invoke | events
├── serial
│   ├── start | stop | status | select
│   ├── read | send
│   └── stream                  # 仅 `--local` 长连接
├── log     (tabs | brief)      # 需 GUI；brief 为压缩摘要，不含原文
├── test    (run | status | abort | pack)  # 需 GUI；本地闭环 + 证据包
├── parse   (line | metrics | file | stdin)
├── session (list | show)
├── wave    (live | session | export)
├── scope   (list | shot | wave)  # 本地 VISA 示波器
└── ui      # 需 GUI：切页 / 面板 / 参数
    ├── state | show | panels | prefs
    ├── serial (open | close | clear | filter | tab | name | browser)
    ├── wave   (open | close | select | browser | bus | cursor | fit)
    ├── calc   (get | set)
    └── instrument (select | scan | list | connect | disconnect | measure | capture | waveform | waveform-source | command)
```

仪表页也可用 `wiparse ui instrument ...`；复杂 SCPI/控件仍可用 `api invoke instrument.command`。

---

## 日常（GUI 已开）

```powershell
wiparse api health
wiparse ports
wiparse serial select --port COM4 --baud 200000
wiparse serial start --port COM3 --baud 2000000
wiparse serial send --port COM3 --hex AA55
wiparse serial read --port COM3 --max-logs 50
wiparse serial stop
# `serial read` / `serial send` 需监控已开；停着时读缓冲用 `log tabs` 或 `api invoke log.lines.get`。

wiparse log brief --since 0
wiparse test run --plan docs/examples/qi_pt_smoke.json --port COM3 --baud 2000000
wiparse test status
wiparse test pack
wiparse test abort --reason user

wiparse api invoke serial.ports
wiparse api invoke serial.status
wiparse api invoke instrument.list
wiparse api invoke --method parse.line --params "{\"text\":\"ASK 02 00 F \"}"
```

PowerShell 里 `--params` 的引号容易写错；串口监控优先用 `serial start/send/read/stop`。

## 无 GUI / CI

```powershell
wiparse --local ports
wiparse --local serial read --port COM3 --duration 5 --max-logs 100
wiparse --local serial send --port COM3 --hex AA55
wiparse --local parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
wiparse --local parse metrics --text "AA55:9000:1500:8500:1400:4000:3000:45:80:EDED"
wiparse --local parse file --path capture.log --limit 500
```

无 GUI 时必须加 `--local`（包括 `parse` / `session` / `wave` / `scope`）。缺省会 attach `127.0.0.1:7878`，不会悄悄打开本机串口。

本地 `serial read` 必须带 `--duration`、`--max-metrics`、`--max-logs` 或 `--demo`，否则会立刻结束。

---

## 其它命令

```powershell
wiparse session list --limit 20
wiparse session show --id 1
wiparse wave live --port COM3 --duration 5 --demo
wiparse wave session --session-id 1 --format json
wiparse wave export --session-id 1 --format csv --out wave.csv
wiparse scope list
wiparse scope shot --index 0 --out shot.png
wiparse scope wave --index 0 --channel CH1 --points 10000
wiparse api events --since-seq 0
```

`wave live` 始终在本进程开串口；GUI 已占用该口时加 `--local` 会失败，应改用 `serial start` + 事件流。

## GUI 页面控制（需 WiParse.exe）

切主标签、显隐工具、改语言/主题、以及各页参数。画面必须开着。

```powershell
wiparse ui state
wiparse ui show --tab serial
wiparse ui show --tab calculator
wiparse ui show --tab instruments
wiparse ui show --tab waveform
wiparse ui panels --serial true --waveform true
wiparse ui prefs --language zh --theme dark --debug false

wiparse ui serial open --path capture.txt
wiparse ui serial filter --query ASK --tab-id 0
wiparse ui serial tab --tab-id 0
wiparse ui serial clear
wiparse ui serial name --name "Live Packet Log"
wiparse ui serial browser --dir D:\logs

wiparse ui wave open --path scope.csv
wiparse ui wave select --index 0
wiparse ui wave bus --kind i2c --scl 0 --sda 1
wiparse ui wave bus --kind ddsss --signal 0 --sequence seqa

wiparse ui wave cursor --x1 0 --x2 0.001
wiparse ui wave fit
wiparse ui wave close

wiparse ui calc get
wiparse ui calc set --card lc --params "{\"inductance\":\"10\",\"capacitance\":\"100\"}"

wiparse ui instrument scan
wiparse ui instrument list
wiparse ui instrument connect --resource "TCPIP0::192.168.1.1::INSTR" --kind oscilloscope
wiparse ui instrument select --id 1
wiparse ui instrument measure --id 1
wiparse ui instrument capture --id 1
wiparse ui instrument waveform --id 1 --channel 1 --points 10000
wiparse ui instrument command --id 1 --query "*IDN?"
```

串口监控仍用 `serial start/stop/select/send`。仪表读写也可用 `api invoke instrument.*`。

波形页 **DDSSS**：示波器采 VCTX 或 ILTX（线圈电压/电流亦可），加载 ISF 后协议分析选 `DDSSS`，通道指到该曲线。默认 SEQA、无 extension；`--sequence auto` 会试 A–D。可选 `--fop`（Hz，**85 kHz–1.78 MHz**）。波形顶部分四行标注（贴在解码通道上沿，随 Y 轴拖动跟随）：**包名**（如 `CE`）→ **字节** hex → **chip→bit**（`St` / `b0`–`b7` / `P` / `Sp`）→ **chip** 0/1（与扩频序列不一致的 chip 标橙色 `x`；Table 4 门限内 bit 仍解出）。点选数据包可看字段解码。合成检验文件：`docs/examples/ddsss_vctx.isf`（SS / CE / RP8 / CHS / ID）；带误码与更多消息：`docs/examples/ddsss_vctx_errors.isf`（chip `x`、`CHS!` 校验错，`CE P!` 奇偶错）。

## 闭环测试（GUI）

执行器在 `WiParse.exe` 内跑：本地判定 / 宏白名单发送（默认可选 `allow_raw_hex`）/ 证据包落盘。AI 只看 `log brief` 和 `test pack` 摘要。

```powershell
wiparse test run --plan docs/examples/qi_pt_smoke.json --port COM3
wiparse log brief --since 0
wiparse test status
wiparse test pack
```

证据目录：`evidence/<时间戳>_<plan_id>/`（`manifest.json`、`serial.txt`、`metrics.csv`、`events.jsonl`、`correlate.json`、`brief_final.json`、`report.skeleton.md`）。

计划 JSON 字段：`id`、`macros`、`allow_raw_hex`、`abort`、`steps`。

步骤：`wait`（`phase` / `packet` / `header` / `rising`）、`wait_line`（`regex` / `exclude`）、`action`、`sleep`、`expect`、`capture_scope`（`save` 默认 false）、`instrument.command`、`instrument.waveform_source`。

`wait.packet` 用解码名（0x71 = `ID`）。`rising: true` 只计本步开始之后的新包。`capture_scope.save: true` 与 `instrument.waveform_source` 会阻塞到文件写出（ISF 可能 30–75s）。占位符 `{instruments.waveform_source_dir}`。工位说明：[`WORKSTATION_CLOSED_LOOP.md`](WORKSTATION_CLOSED_LOOP.md)。示例：[`examples/ask71_waveform_source.json`](examples/ask71_waveform_source.json)、[`examples/qi_pt_smoke.json`](examples/qi_pt_smoke.json)。

```powershell
wiparse ui instrument waveform-source --id 1 --dir D:\isf --filename wave.isf
wiparse test run --plan docs/examples/ask71_waveform_source.json --port COM3
```

`abort.timeout_s` 到期一律 Failed（空计划也不再算 Pass）。

## 环境变量

| 变量 | 说明 |
|------|------|
| `WIPARSE_URL` | GUI API 根 URL |
| `WCM_CONFIG` | 配置文件（同 `--config`） |

MCP：[`mcp/wiparse/README.md`](../mcp/wiparse/README.md)。部署：[`DEPLOY_API.md`](DEPLOY_API.md)。
