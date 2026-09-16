# UART Bridge (`wiparse-uart-bridge`)

Generic **transparent UART** host for Testing Hub (WIPARSE-IF CH-UART). Bytes are not framed. Same plugin works for any TCP UART bridge or WinUSB CDC that matches the ICD, not only the WIPARSE dongle.

## Transports

| `transport` | Path |
|-------------|------|
| `tcp` | Raw TCP `:1234`. First payload `1200`–`2000000` (2–7 ASCII digits) sets baud and is **not** forwarded |
| `usb` | WinUSB `MI_01` / GUID `{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0101}` / `WIPARSE UART`, bulk `0x02`/`0x83`. SET_LINE_CODING + DTR. `auto` falls back to TCP if USB is busy or unplugged |
| `serial` | GUI `serial.monitor.*` when a COM / `ttyACM` exists (optional INF / Linux CDC ACM) |
| `auto` | WinUSB then TCP |

Default 115200 8N1, baud range 1200–2e6, 5–8 data, none/odd/even, stop 1/1.5/2.

## Operations

- `capture` — record until `duration_s` or Hub **Stop** (`run.stop`)
- `send` — hex and/or text then a short capture
- `recipe` — `[{ "send_text": "AT\\r\\n", "expect_contains": "OK", "wait_ms": 300 }]`
- `ping` / `discover`

Logs: `{artifacts_dir}/logs/uart_{stamp}.{jsonl,bin,hex}` under `tests/{report_name}/runs/{stamp}/`.

```powershell
wiparse ui test-tool run --id wiparse-uart-bridge --params "{\"transport\":\"tcp\",\"baud\":115200,\"duration_s\":5}"
wiparse ui test-tool stop
```
