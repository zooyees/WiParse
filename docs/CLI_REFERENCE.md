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
└── scope   (list | shot | wave)  # 本地 VISA 示波器
```

仪表控制（电源/负载/万用表）没有单独子命令，用 `api invoke instrument.*`。

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

## 闭环测试（GUI）

执行器在 `WiParse.exe` 内跑：本地判定 / 宏白名单发送（默认可选 `allow_raw_hex`）/ 证据包落盘。AI 只看 `log brief` 和 `test pack` 摘要。

```powershell
wiparse test run --plan docs/examples/qi_pt_smoke.json --port COM3
wiparse log brief --since 0
wiparse test status
wiparse test pack
```

证据目录：`evidence/<时间戳>_<plan_id>/`（`manifest.json`、`serial.txt`、`metrics.csv`、`events.jsonl`、`correlate.json`、`brief_final.json`、`report.skeleton.md`）。

计划 JSON 字段：`id`、`macros`（名字→hex）、`allow_raw_hex`（默认 false；为 true 才允许 `action` 写裸 hex）、`abort`（`on_ept` / `csum_gt` / `timeout_s` / `vin_gt` / `capture_scope`）、`steps`（`wait` / `action` / `sleep` / `expect` / `capture_scope`）。`abort.timeout_s` 到期一律 Failed（空计划也不再算 Pass）。

## 环境变量

| 变量 | 说明 |
|------|------|
| `WIPARSE_URL` | GUI API 根 URL |
| `WCM_CONFIG` | 配置文件（同 `--config`） |

MCP：[`mcp/wiparse/README.md`](../mcp/wiparse/README.md)。部署：[`DEPLOY_API.md`](DEPLOY_API.md)。
