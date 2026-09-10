# Scope & Serial Monitor (`scope-serial-monitor`)

Industrial capture loop: serial trigger (rising-edge) → delay → ScopeStop + serial off → shot/ISF/PDF → ScopeRun + serial on.

## Supported path (preferred)

**Testing Hub** → select plugin → PreFlight / Run / Stop.

Host runs:

```text
node test-tools/runner.mjs --plugin scope-serial-monitor --lifecycle run|preflight|stop --data-root <root> -- …
```

- Params from the form overlay `station.json` via `plugin-contract` template expansion.
- After merge, **`status_file` is always colocated** as `{isf_dir}/_loop_status.json` (so overriding ISF dir keeps the HUD readable).
- Status HUD is **in-panel** (no floating PowerShell window by default).
- stdout/stderr stream into Testing Hub **Output**.

## Preflight checks

- GUI API / version, browser for PDF
- ISF / report dir writability
- Single-instance lock (`run.lock`)
- Serial API reachability
- Oscilloscope presence + trigger compile

## Optional CLI helpers

| Script | Role |
|--------|------|
| `start.ps1` | Calls `runner.mjs` (hidden Node). Add `-Hud` only for floating overlay. |
| `stop.ps1` | `runner.mjs --lifecycle stop`, wait ≤30s, then force-kill. |
| `hud.ps1` | Legacy floating HUD; not required for Testing Hub. |

## Station notes

- `serial_triggers.rising_edge` (default `true`): fire on inactive→active only.
- `interlocks.hold_serial_off_during_capture` / `single_instance`: honored at runtime.
- Runtime files `run.lock` / `run.stop` are local and gitignored.
