# WiParse CLI 参考

Headless JSON CLI，与 Python `WiParseCLI` 信封格式兼容。二进制：`wiparse` / `WiParse-CLI.exe`。

## 全局选项

| 选项 | 说明 |
|------|------|
| `--json` | 输出 JSON 信封（默认开启） |
| `--pretty` | 美化 JSON |
| `--quiet` | 仅输出 `data` 或 `error` 体 |
| `--config <path>` | 指定配置文件（设置 `WCM_CONFIG`） |

## 响应信封

成功（stdout）：

```json
{
  "ok": true,
  "cmd": "version",
  "ts": "2026-07-18T08:00:00.000+08:00",
  "data": { }
}
```

失败（stderr，exit 1）：

```json
{
  "ok": false,
  "cmd": "serial.read",
  "ts": "...",
  "error": { "code": "ERROR", "message": "..." }
}
```

`serial stream` 为 NDJSON 流式输出，不使用上述信封（每行一条 `{type,data}`）。

---

## 命令树

```
wiparse
├── version
├── ports
├── serial
│   ├── read
│   ├── stream          # 长连接 NDJSON，MCP 不直接暴露
│   └── send
├── parse
│   ├── line
│   ├── metrics
│   ├── file
│   └── stdin
├── session
│   ├── list
│   └── show
├── wave
│   ├── live
│   ├── session
│   └── export
└── scope
    ├── list
    ├── shot
    └── wave
```

> **说明**：GUI「仪表控制」工作台（示波器/电源/负载/万用表 VISA）目前仅 GUI 实现，CLI 侧 `scope` 为 Tektronix/VISA 示波器快捷命令（`wiparse-core::scope`）。

---

## version

```powershell
wiparse version
```

返回 `version`、`name`、`edition`。

---

## ports

枚举本机串口：

```powershell
wiparse ports
```

---

## serial read

从串口采集 metrics / log，可选写入 SQLite。

```powershell
wiparse serial read --port COM3 --baud 2000000 --duration 5 --max-metrics 100 --save-db
wiparse serial read --port COM3 --demo
```

| 参数 | 说明 |
|------|------|
| `--port` | 串口名 |
| `--baud` | 波特率，默认 2000000 |
| `--duration` | 秒，超时停止 |
| `--max-metrics` | 最多 metrics 条数 |
| `--max-logs` | 最多 log 条数 |
| `--demo` | 演示帧，无需硬件 |
| `--save-db` | 写入 SQLite 会话 |

`data`：`metrics[]`、`logs[]`、可选 `session`。

---

## serial send

发送十六进制字节：

```powershell
wiparse serial send --port COM3 --baud 2000000 --hex "AA55..."
```

---

## serial stream

持续输出 NDJSON（`metrics` / `log` 类型）。Agent 单次调用请用 `serial read --duration`。

---

## parse line

解析 Qi ASK/FSK 报文行：

```powershell
wiparse parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
```

---

## parse metrics

解析 AA55 metrics 帧：

```powershell
wiparse parse metrics --text "AA55:9000:1500:8500:1400:4000:3000:45:80:EDED"
```

---

## parse file / stdin

批量解析文件或标准输入中的 Qi / metrics：

```powershell
wiparse parse file --path capture.log --limit 500
Get-Content capture.log | wiparse parse stdin --limit 100
```

---

## session list / show

SQLite 会话管理（需 `--save-db` 或 GUI 采集）：

```powershell
wiparse session list --limit 20
wiparse session show --id 1
```

---

## wave live

实时采集 metrics 并转为波形 JSON：

```powershell
wiparse wave live --port COM3 --duration 5 --channels "v_in,i_in,v_out,i_out,p" --demo
```

通道默认：`v_in,i_in,v_out,i_out,p`。

---

## wave session / export

从已保存会话导出波形：

```powershell
wiparse wave session --session-id 1 --from 0 --to 60 --format json
wiparse wave export --session-id 1 --format csv --out wave.csv
```

---

## scope list / shot / wave

VISA 示波器（Tektronix 等）：

```powershell
wiparse scope list
wiparse scope shot --index 0 --out shot.png
wiparse scope wave --index 0 --channel CH1 --points 10000
```

---

## 环境变量

| 变量 | 说明 |
|------|------|
| `WCM_CONFIG` | 配置文件路径（`--config` 等效） |

配置项见 `config.default.json`：`system.db_name`、串口/示波器/仪表默认值等。

---

## MCP 集成

见 [`mcp/wiparse/README.md`](../mcp/wiparse/README.md)。MCP 通过 stdio 调用本 CLI，工具名前缀 `wiparse_`。
