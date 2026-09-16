# CMSIS-DAP Probe (`wiparse-dap`)

Generic **CMSIS-DAP v2** host for Testing Hub. Speaks the same channel as DAP-Link `docs/WIPARSE-IF-SPEC.md` (CH-DAP), but VID/PID/host/port/chip are all parameters — any CMSIS-DAP (or J-Link / ST-Link already in Instrument Control) works.

## Transports

| `transport` | Path |
|-------------|------|
| `usb` | WinUSB `MI_00`, GUID `{CDB3B5AD-293B-4663-AA36-1AAE46463776}`, bulk `0x01`/`0x81`, logical packet 512 |
| `tcp` | USB/IP 1.1.1 `OP_REQ_DEVLIST` on `:3240`, fallback elaphureLink magic `0x8A656C70` |
| `gui` | `instrument.list` kind=`debug_probe` + `Probe*` commands (probe-rs) |
| `tool` | Shell: `pyocd`, `openocd`, `probe-rs`, … |
| `auto` | USB PnP → USB/IP → GUI probe |

Dongle defaults: `VID=0xC251` `PID=0xF01A`, host `dap.local` / `192.168.137.123`, SWJ 1 MHz, no SWO.

## Operations

`session` (default) discovers every path and, if a GUI probe is connected, runs `ProbeStatus`. Other ops: `dap-info`, `usbip-list`, `halt` / `run` / `reset` / `regs` / `mem-read` / `mem-write` / `flash` / `erase` / `speed` / `rtt-*` / `recipe`.

Recipe example:

```json
[
  { "op": "connect" },
  { "op": "halt" },
  { "op": "mem-read", "address": "08000000", "length": 64 }
]
```

Artifacts: `projects/{project}/tests/{report_name}/runs/{stamp}/artifacts/{logs,reports,dumps}/` (`file_prefix` may override the case dir).

```powershell
wiparse ui test-tool run --id wiparse-dap --preflight true
wiparse ui test-tool run --id wiparse-dap --params "{\"op\":\"session\",\"chip\":\"stm32f103c8\"}"
```
