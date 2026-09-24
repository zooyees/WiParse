# Test-tool plugins

Supported plugins in this tree:

| Id | Type | `report_name` | Role |
|----|------|---------------|------|
| `example-smoke` | smoke | ExampleSmoke | Contract / CI smoke |
| `scope-serial-monitor` | capture_loop | ScopeSerial | Serial rising-edge → scope capture |
| `wiparse-dap` | instrument | WiparseDap | CMSIS-DAP v2 (USB / USB-IP / GUI probe-rs / pyOCD) |
| `wiparse-uart-bridge` | serial | WiparseUart | UART byte bridge (WinUSB CDC / TCP :1234 / COM) |
| `wiparse-i2c-rpc` | instrument | WiparseI2c | I2C master text RPC (WinUSB / TCP :1236) |
| `wiparse-netcfg` | instrument | WiparseNetcfg | Dongle STA NVS（USB MI_05 EP0；DHCP 默认；TCP :1238 仅 GOT_IP 后） |

Host ICD: DAP-Link `docs/WIPARSE-IF-SPEC.md` + `ACCESS_INTERFACES.md` (VID `0xC251` PID `0xF01A`). Shared protocol code: `test-tools/lib/wiparse-if*.mjs`.

Do **not** keep renamed or copy folders here (e.g. legacy `tianshu-*`, spaced names).
They appear in Testing Hub and confuse operators.

Canonical host path: **Testing Hub** → `node test-tools/runner.mjs …`
