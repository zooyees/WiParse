# Test-tool plugins

Supported plugins in this tree:

| Id | Type | Role |
|----|------|------|
| `example-smoke` | smoke | Contract / CI smoke |
| `scope-serial-monitor` | capture_loop | Serial rising-edge → scope capture |
| `wiparse-dap` | instrument | CMSIS-DAP v2 (USB / USB-IP / GUI probe-rs / pyOCD) |
| `wiparse-uart-bridge` | serial | UART byte bridge (WinUSB CDC / TCP :1234 / COM) |
| `wiparse-i2c-rpc` | instrument | I2C master text RPC (WinUSB / TCP :1236) |

Host ICD: DAP-Link `docs/WIPARSE-IF-SPEC.md` + `ACCESS_INTERFACES.md` (VID `0xC251` PID `0xF01A`). Shared protocol code: `test-tools/lib/wiparse-if*.mjs`.

Do **not** keep renamed or copy folders here (e.g. legacy `tianshu-*`, spaced names).
They appear in Testing Hub and confuse operators.

Canonical host path: **Testing Hub** → `node test-tools/runner.mjs …`
