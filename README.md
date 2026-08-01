# WiParse-Rust

Rust rewrite of [WiParse](../WiParse) — Qi wireless charging test utility (GUI + headless CLI).

## Layout

```
WiParse-Rust/
├── crates/
│   ├── wiparse-core/   # config, paths, metrics, Qi protocol, serial helpers
│   ├── wiparse-cli/    # `wiparse` JSON CLI (compatible envelope with Python WiParseCLI)
│   └── wiparse-gui/    # egui desktop shell (serial / scope panels to be ported)
├── Icon/               # WiParse.ico
└── config.default.json
```

## Build

Requires [Rust](https://rustup.rs/) (stable) and a C linker (MSVC Build Tools on Windows).

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cd D:\windlink\windlink\WiParse-Rust
cargo build --release
```

Binaries:

| Binary | Path |
|--------|------|
| CLI | `target/release/wiparse.exe` |
| GUI | `target/release/wiparse-gui.exe` |

## CLI (JSON envelope)

Compatible with Python `WiParseCLI` shape:

```json
{ "ok": true, "cmd": "version", "ts": "...", "data": { ... } }
```

Full command reference: [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md).

C+E API deploy (GUI embeds localhost API; CLI/MCP attach): [`docs/DEPLOY_API.md`](docs/DEPLOY_API.md).

Packaged binaries: `dist/WiParse.exe`, `dist/WiParse-CLI.exe`.

MCP server for agents: [`mcp/wiparse/README.md`](mcp/wiparse/README.md).

```powershell
cargo run -p wiparse-cli -- version
cargo run -p wiparse-cli -- ports
cargo run -p wiparse-cli -- parse line --text "TX0:[12:00:00.000] ASK 02 00 F "
cargo run -p wiparse-cli -- parse metrics --text "AA55:9000:1500:8500:1400:4000:3000:45:80:EDED"
```

## GUI layout (Python parity)

```
MonitorWindow
└── main_tabs          ← peer tabs (one visible at a time)
    ├── 串口工具       ← left sidebar + log_file_tabs + LogTabPage
    ├── 计算器         ← LC / 带通 / Q / RC / CRC / 转换与科学计算
    └── Tektronix示波器 ← placeholder
```

Serial tool mirrors Python `log_panel`:
- Left: port / baud / Start·Stop / New Live Log / Clear / name / dir / Open
- Right: closable file tabs; live tab always at index 0
- Per tab: Split View / Panes / Side-by-Side / filters (`|` OR) / Auto Parse / live append

| Area | Status |
|------|--------|
| Config deep-merge + `WCM_CONFIG` | ✅ |
| Metrics frame `AA55…EDED` | ✅ |
| ASK/FSK tables + field decode (EPT/CE/CFG/SRQ/MSR/FOD/ID/CAP…) | ✅ |
| Serial read / stream / send | ✅ |
| SQLite sessions + metrics/logs | ✅ |
| CLI `wave live/session/export`, `session list/show` | ✅ |
| CLI `scope list` (shot/wave stub → use Python) | ✅ |
| Charge-state estimator (CC/CV/trickle/idle) | ✅ |
| egui live serial + V/I waveform plots | ✅ |
| Tektronix SCPI HARDCopy / CURVe | ⏳ (Python) |
