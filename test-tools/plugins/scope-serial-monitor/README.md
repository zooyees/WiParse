# Scope & Serial Monitor (`scope-serial-monitor`)

Industrial capture loop: serial trigger (rising-edge) → delay → ScopeStop + serial off → `instrument.capture` + `instrument.waveform_source` → PDF → ScopeRun + serial on.

GUI **1.1.16+**. Plugin **v0.6.0**.

## Preferred: Testing Hub / CLI / MCP

GUI already running (`WiParse.exe`):

```powershell
wiparse ui project select --id default
wiparse ui test-tool run --id scope-serial-monitor --preflight true
wiparse ui test-tool run --id scope-serial-monitor
wiparse ui test-tool run --id scope-serial-monitor --params "{\"port\":\"COM7\",\"file_prefix\":\"mpp25w_test\"}"
wiparse ui test-tool stop
wiparse ui report open --path D:/data/projects/default/tests/mpp25w_test/runs/<stamp>/artifacts/reports/<file>.md
```

MCP `wiparse_ui`: `op=test_tool.run` with `{id, preflight?, params?}`；`op=test_tool.stop`。Hub 一次 run 会把测试用例登记为 `{ id: <file_prefix>, plugin_id: "scope-serial-monitor" }`，不必再用插件 id 当用例目录。

Direct runner (same layout as Hub):

```text
node test-tools/runner.mjs --plugin scope-serial-monitor --lifecycle run|preflight|stop --data-root <root> --project default -- …
```

- Params from the form overlay `station.json` via `plugin-contract` template expansion.
- **Line triggers** are a Hub `json` param (`serial_triggers.items`). ASK 2 / timeout in `station.json` are only the factory recipe. Rule types: `regex` | `contains`; `unless` / `exclude` skip matches; `enabled: false` drops a rule. **上升沿触发** is `serial_triggers.rising_edge`.
- Engine: `test-tools/lib/line-triggers.mjs`. Hub overlays pin the in-memory list so a running loop does not reload `station.json`.
- Triggers poll `log.lines.get` with an absolute `from_row`. After the live tab FIFO (5000 lines) wraps, keep using `next_row` / `total` from the reply — do not treat `total` as the window length.
- **Instrument** is a Hub param. Leave Auto and click Preflight to fill from `instrument.list`.
- Capture uses **`instrument.capture`** (PNG) and **`instrument.waveform_source`** (ISF). Do not use deprecated `scope.shot`.
- New runs write under `{data_root}/projects/{project}/tests/{test_id}/` where `test_id` is the sanitized **report name / `file_prefix`** (default `ScopeSerial`). Lock / stop / HUD live in that `test_dir`; artifacts under `runs/{stamp}/artifacts/{waves,reports,shots,docs}/`. Do not write into `plugin_dir`, `instrument_data/`, or `test report/`.
- `plugin.json` `outputs[]` declares those folders for the Test Report tree. One Hub run = one `stamp` under that test case. Session uses `ctx.stamp`.
- After merge, **`summary_md` is** `{report_dir}/{file_prefix}_summary_{stamp}.md`. Markdown images use a relative path into `shots/`.
- Status HUD is in-panel. stdout/stderr stream into Testing Hub Output.

## Preflight checks

- GUI API / version, browser for PDF
- `test_dir` writability (waveform dirs deferred until `run` has a stamp)
- Single-instance lock (`run.lock`)
- Serial API reachability
- Oscilloscope presence (`instrument.list`) + model/resource auto-fill; trigger compile

## Optional scripts

Prefer `wiparse ui test-tool`. These still call `runner.mjs` with `--project` and `--test-id` from `file_prefix`.

| Script | Role |
|--------|------|
| `start.ps1` | Hidden Node runner. `-FilePrefix` / `-TestId` / `WIPARSE_TEST_ID` select the case dir. `-Hud` uses `{test_dir}/status.json`. Logs go to `{test_dir}/helper_logs/`, not `plugin_dir`. |
| `stop.ps1` | `runner.mjs --lifecycle stop` against the same `test_id`. |
| `hud.ps1` | Legacy floating HUD; not required. |

## Station notes

- `scope.resource` / `scope.model` default empty; Preflight fills them from the connected instrument.
- `serial_triggers.rising_edge` (default `true`). Exposed as Hub **上升沿触发**.
- `serial_triggers.items`: JSON array of `{ id, label?, enabled?, type, pattern, flags?, unless?, note? }`.
- `interlocks.hold_serial_off_during_capture` / `single_instance`: honored at runtime.
- Runtime files `run.lock` / `run.stop` / `status.json` live in `test_dir`. `product` is report-body only; folder tree is project / `file_prefix` / stamp.
