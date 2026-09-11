# WiParse Test Plugin Spec

Standard contract for Node.js plugins under `test-tools/plugins/<id>/`.

Machine-readable schemas:

- [`schemas/plugin.schema.json`](./schemas/plugin.schema.json)
- [`schemas/station.schema.json`](./schemas/station.schema.json)
- [`schemas/marketplace-package.schema.json`](./schemas/marketplace-package.schema.json)

## Layout

```
plugins/<id>/
  plugin.json      # required manifest (validated on discover/run)
  index.mjs        # entry: run / preflight / stop
  station.json     # optional station config (capture_loop etc.)
```

## Lifecycle

Host only uses three lifecycle ops:

| Lifecycle | Meaning |
|-----------|---------|
| `preflight` | Validate host/config/instruments; no long-running loop |
| `run` | Start the plugin |
| `stop` | Request graceful stop (`paths.stop_file` or `stop()`) |

```powershell
node runner.mjs --plugin <id> --lifecycle preflight
node runner.mjs --plugin <id> --lifecycle run
node runner.mjs --plugin <id> --lifecycle stop
```

`--preflight_only true` is treated as `--lifecycle preflight`.

Entry exports (recommended):

```js
export async function preflight(ctx) { ... }
export async function stop(ctx) { ... }
export default async function run(ctx) { ... }  // lifecycle === "run"
```

## Result contract

Every lifecycle must resolve to:

```json
{
  "ok": true,
  "lifecycle": "run",
  "step": "optional",
  "checks": [{ "id": "gui_api", "ok": true, "detail": "..." }],
  "artifacts": { "summary_md": "...", "stop_file": "..." },
  "error": "only when ok=false"
}
```

Use `normalizeResult(raw, lifecycle)` from `lib/plugin-contract.mjs`.

## `plugin.json`

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Unique id (`[a-zA-Z0-9][a-zA-Z0-9._-]*`) |
| `name` | yes | Display name (EN) |
| `name_zh` | no | Display name (ZH) |
| `type` | yes | `smoke` / `serial` / `instrument` / `capture_loop` / `custom` |
| `version` | no | Semver |
| `entry` | no | Entry file (default `index.mjs`) |
| `config` | no | Station config relative to plugin dir |
| `description` | no | Short summary |
| `capabilities` | no | Subset of `preflight` / `run` / `stop` |
| `engines` | no | `{ "node": ">=18", "wiparse": ">=1.1.8" }` |
| `params` | no | Overridable parameters |

### `params[]`

| Field | Description |
|-------|-------------|
| `name` | CLI/GUI key (`--name value`) |
| `type` | `string` / `number` / `boolean` / `path` |
| `default` | Default when not overridden |
| `path` | Dot path into station config, e.g. `paths.file_prefix` |
| `label` / `label_zh` | UI labels |
| `help` | Hint text |

### Recommended capture-loop params

| name | path |
|------|------|
| `file_prefix` | `paths.file_prefix` |
| `isf_dir` | `paths.isf_dir` |
| `report_dir` | `paths.report_dir` |
| `product` | `station.product` |
| `port` | `serial.port` |
| `baud` | `serial.baud` |
| `api` | `gui.api` |
| `preflight_only` | *(maps to lifecycle preflight)* |

## Path templates

- `{data_root}` â€?workspace / configured data root  
- `{product}` â€?`station.product`  
- `{plugin_dir}` â€?absolute plugin directory  
- `{stamp}` â€?filled at runtime by the plugin  

## Station config

Required: `gui.api`, `paths.file_prefix`, `paths.isf_dir`, `paths.report_dir`.  
Validated by `validateStationConfig` when `loadStationConfig` runs.

## Runner

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin example-smoke --lifecycle preflight
node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --file_prefix MyTest
node runner.mjs --plugin scope-serial-monitor --lifecycle stop
```

## Marketplace (Phase 1 client)

Local install / verify / pull from the cloud catalog:

```powershell
cd test-tools
node marketplace.mjs list --json
node marketplace.mjs verify --zip plugin.zip --meta meta.json
node marketplace.mjs install --zip plugin.zip --meta meta.json --data-root <root>
node marketplace.mjs catalog --url https://marketplace.example --channel stable
node marketplace.mjs pull --plugin <id> --version <ver> --url https://marketplace.example
```

Install root defaults to `{data_root}/marketplace`. Runner merges **active** marketplace installs with bundled `plugins/` (marketplace wins on id conflict).

Schemas: `schemas/marketplace-package.schema.json`. Cloud server: `services/testing-hub-marketplace/`.

Env: `WIPARSE_CLI`, `WIPARSE_URL`, `WIPARSE_DATA_ROOT`, `WIPARSE_MARKETPLACE_URL`, `WIPARSE_MARKETPLACE_ALLOW_HTTP`.
