# Scope & Serial Monitor (`scope-serial-monitor`)

Industrial capture loop: serial trigger (rising-edge) → delay → ScopeStop + serial off → shot/ISF/PDF → ScopeRun + serial on.

## Supported path (preferred)

**Testing Hub** → select plugin → PreFlight / Run / Stop.

Host runs:

```text
node test-tools/runner.mjs --plugin scope-serial-monitor --lifecycle run|preflight|stop --data-root <root> --project default -- …
```

- Params from the form overlay `station.json` via `plugin-contract` template expansion.
- **Line triggers** are a Hub `json` param (`serial_triggers.items`). Edit the array in Testing Hub; ASK 2 / timeout in `station.json` are only the factory recipe, not platform code. Rule types: `regex` | `contains`; `unless` / `exclude` skip matches; `enabled: false` drops a rule. **上升沿触发** is a checkbox (`serial_triggers.rising_edge`).
- Engine: `test-tools/lib/line-triggers.mjs` (any plugin can reuse). Hub overlays pin the in-memory list so a running loop does not reload `station.json` over the form.
- **Instrument** (kind / model / VISA / device id) is a Hub param. Leave **自动** and click **预检** to fill from the currently connected oscilloscope (`instrument.list`). Changing model or kind does not require a GUI rebuild.
- New runs write under `{data_root}/projects/{project}/tests/scope-serial-monitor/` (`run.lock` / `run.stop` / `status.json`) and `runs/{stamp}/artifacts/{waves,reports,shots,docs}/`. Do not write into `plugin_dir`, `instrument_data/`, or `test report/`.
- `plugin.json` `outputs[]` declares those folders for the Test Report tree. One Hub run = one `stamp`; each trigger adds files, not a new `runs/` folder. Use `ctx.stamp` for the session.
- After merge, **`summary_md` is always** `{report_dir}/{file_prefix}_summary_{stamp}.md`. Changing **报告名称/前缀** in Testing Hub renames the summary MD and all ISF/PDF/PNG records. Markdown images use a relative path into `shots/`.
- Status HUD is **in-panel** (no floating PowerShell window by default).
- stdout/stderr stream into Testing Hub **Output**.

## Preflight checks

- GUI API / version, browser for PDF
- ISF / report dir writability
- Single-instance lock (`run.lock`)
- Serial API reachability
- Oscilloscope presence (`instrument.list`) + model/resource auto-fill; trigger compile

## Optional CLI helpers

| Script | Role |
|--------|------|
| `start.ps1` | Calls `runner.mjs` (hidden Node). Add `-Hud` only for floating overlay. |
| `stop.ps1` | `runner.mjs --lifecycle stop`, wait ≤30s, then force-kill. |
| `hud.ps1` | Legacy floating HUD; not required for Testing Hub. |

## Station notes

- `scope.resource` / `scope.model` default empty; Testing Hub Preflight fills them from the connected instrument.
- `serial_triggers.rising_edge` (default `true`): fire on inactive→active only. Exposed as Hub **上升沿触发**.
- `serial_triggers.items`: JSON array of `{ id, label?, enabled?, type, pattern, flags?, unless?, note? }`. Optional `serial_triggers.file` loads items from a sidecar JSON when `items` is empty.
- `interlocks.hold_serial_off_during_capture` / `single_instance`: honored at runtime.
- Runtime files `run.lock` / `run.stop` / `status.json` live in `test_dir` (gitignored locally). `product` is report-body only.
