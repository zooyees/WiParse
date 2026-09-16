# I2C RPC Master (`wiparse-i2c-rpc`)

Generic **I2C master text RPC** client (WIPARSE-IF CH-I2C). USB and TCP share the same UTF-8 line protocol. Not tied to a single EEPROM map — any 7-bit device on the bus is addressed from the form or a recipe.

## Transports

| `transport` | Path |
|-------------|------|
| `tcp` | `:1236`, `TCP_NODELAY`, one client. Lines end in `\\n` or `\\r` |
| `usb` | WinUSB `MI_03` / GUID `{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0103}` / `WIPARSE I2C`, bulk `0x03`/`0x84`. Line coding and DTR ignored |
| `serial` | Second CDC ACM / optional COM via GUI monitor |
| `auto` | WinUSB then TCP |

## Commands (ICD §8.4)

`HELP` `PING` `SCAN` `SPEED` `TIMEOUT` `RECOVER`/`BUSRESET` `WRITE` `READ` `WRITEREAD`

Addresses are hex bytes (`50`, `0A`, `0x50`). `READ`/`WRITEREAD` length `n` is decimal 1–256. Replies: `OK …` or `ERR <reason>`.

Recipe:

```json
[
  { "cmd": "SCAN" },
  { "cmd": "PING", "addr": "50" },
  { "cmd": "WRITE", "addr": "50", "data": "00 12 34" },
  { "cmd": "WRITEREAD", "addr": "50", "data": "00", "len": 2 }
]
```

Or a `#` comment script, one ICD line per row.

Default run: `SCAN` after optional `SPEED`/`TIMEOUT`. Bus lock wait is 2 s on the dongle (`ERR busy`). Artifacts: `tests/{report_name}/runs/{stamp}/artifacts/{logs,reports}/`.

```powershell
wiparse ui test-tool run --id wiparse-i2c-rpc --params "{\"op\":\"scan\"}"
wiparse ui test-tool run --id wiparse-i2c-rpc --params "{\"op\":\"read\",\"addr\":\"50\",\"length\":2}"
```
