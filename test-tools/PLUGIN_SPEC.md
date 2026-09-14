# WiParse Testing Hub 插件规范

**日期** 2026-09-15  
**宿主** WiParse GUI **1.1.13+**（Testing Hub / `test_tool`）  
**运行时** Node.js **>=18**，ESM `.mjs`  
**读者** 插件作者 / 工位 AI

对齐 schema：

- [`schemas/plugin.schema.json`](./schemas/plugin.schema.json)
- [`schemas/station.schema.json`](./schemas/station.schema.json)
- [`schemas/marketplace-package.schema.json`](./schemas/marketplace-package.schema.json)

共享库（插件走 HTTP / 契约，不要复制 GUI）：

- `lib/plugin-contract.mjs` — 参数合并、模板展开、`normalizeResult`、`pickInstrument`、overlay
- `lib/wiparse-sdk.mjs` — HTTP `/v1/health` + `/v1/invoke`
- `lib/line-triggers.mjs` — 通用行匹配（`regex` / `contains`）；**可选**，Hub 不内置
- `runner.mjs` — Hub 与 CLI 的唯一入口

示例：

- `plugins/example-smoke` — CLI + health
- `plugins/scope-serial-monitor` — capture_loop 示例（配方在插件里，不在 Hub）

---

## 1. Hub 必须保持仪器无关

Testing Hub **不知道** ASK、示波器、ISF、某条正则。它只做：

1. 发现 `plugins/<id>/plugin.json`
2. 按 `params[]` 画通用控件（含 `group` / `advanced` 折叠，不理解组名语义）
3. 把本轮表单写成 **overlay JSON 文件**，runner `--overlay <path>`
4. 读 `paths.status_file` 的 JSON：只用 `step` / `hint` / `cycle` / `elapsed_s`
5. 把插件回写的 `suggested_params` 填进未改过的表单字段

VISA、串口、波形格式全部在 **插件**（`plugin.json` / `station.json` / 插件 JS）。不要给 Hub 加 `plugin.id` 分支或 `SerialTriggerEditor`。

Hub 调用：

```text
node test-tools/runner.mjs
  --plugin <id>
  --lifecycle preflight|run|stop
  --cli <WiParse-CLI.exe>
  --data-root <root>
  --overlay <run-scoped.json>
  -- --<extra> <value> ...
```

overlay 是 **本轮运行** 的参数覆盖，**不会写回 `station.json`**。Windows 上避免把大段 JSON 塞进 argv。

环境变量：`WIPARSE_URL`、`WIPARSE_CLI`、`WIPARSE_DATA_ROOT`。

---

## 2. 目录（MUST）

```
test-tools/plugins/<id>/
  plugin.json      # 清单 + Hub 表单
  index.mjs        # 入口
  station.json     # 可选；有工位配置时再提供
```

- `<id>` 与 `plugin.json.id` 一致：`^[a-zA-Z0-9][a-zA-Z0-9._-]*$`
- 锁 / 停 / HUD 文件由 `paths.*` 声明。Hub **不** 硬编码 `_loop_status.json`。

---

## 3. Lifecycle（MUST）

| Lifecycle | Hub 按钮 | 含义 |
|-----------|----------|------|
| `preflight` | 预检 | 检查 GUI / 仪器 / 路径；**不要**开长循环；可返回 `suggested_params` |
| `run` | 运行 | 正式作业；capture_loop 直到 Stop |
| `stop` | 停止 | 写 `paths.stop_file` |

Hub **只** 用 `--lifecycle` 区分预检与运行。不要用表单参数名 `preflight_only` 驱动生命周期。CLI 仍可 `--preflight_only true` 作为兼容回退（`resolveLifecycle`）。

```js
export async function preflight(ctx) { return normalizeResult(raw, "preflight"); }
export async function stop(ctx) { /* requestStop(config) */ }
export default async function run(ctx) { /* ... */ }
```

### 结果信封（MUST）

```json
{
  "ok": true,
  "lifecycle": "preflight",
  "step": "preflight",
  "checks": [{ "id": "gui_api", "ok": true, "detail": "..." }],
  "suggested_params": {
    "prefer_device_id": "3",
    "scope_resource": "USB0::...::INSTR"
  },
  "suggested_params_policy": "untouched",
  "artifacts": {},
  "error": "only when ok=false"
}
```

`suggested_params` 的 key **必须是** `params[].name`。Hub 默认 `untouched`（不覆盖用户已改字段）。需要时可用 `"empty"` / `"always"`。

请 `return normalizeResult(raw, lifecycle)`。`ok === false` 时 runner exit 1。

### 运行中 HUD 信封

插件往 `paths.status_file` 写 JSON。Hub **只读**：

| 字段 | 含义 |
|------|------|
| `step` | 不透明状态字，原样显示（不要指望 Hub 翻译成「监控/已抓到」） |
| `hint` | 一行给人看的说明；有则优先于 `step` |
| `cycle` | 可选计数 |
| `elapsed_s` | 可选秒数；任意 step 都可显示 |

其余原始键可作为次要 kv。Hub **不解析** `trigger` / `filename` / `triggers[]`。

---

## 4. `plugin.json` 参数

| 字段 | 含义 |
|------|------|
| `name` | CLI / overlay 键 |
| `type` | 控件类型 |
| `default` | 缺省 |
| `path` | 写入 station 的点路径，如 `scope.resource` |
| `label` / `label_zh` | 表单标签 |
| `help` | hover |
| `hidden` | `true` 不画表单（仍可给 fills / CLI） |
| `group` / `group_zh` | 折叠分组标题；Hub 当字符串显示，不理解组名 |
| `advanced` | `true` 放进「高级参数」折叠 |
| `filter.kind` | `type=device` 时过滤 `instrument.list` |
| `filter_from` | 用另一个 param 的当前值当 kind |
| `fills` | 选设备后写入其它 param |
| `options` | `type=enum`：`{ value, label, label_zh }` |

### `type`

| type | Hub 控件 | 值 |
|------|----------|-----|
| `string` / `number` | 单行 | 文本 / 数字 |
| `boolean` | 开关 | `true` / `false` |
| `path` | 文本 + 浏览 | 路径 |
| `device` | 已连接仪器下拉 | `device_id` |
| `enum` | 下拉（`options`） | option.value |
| `serial_port` | 本机 COM 列表 | `COM7` |
| `json` | 多行 JSON | 对象或数组（overlay 里保持结构化） |
| `text` | 多行文本 | 字符串 |

本轮 **没有** `type:list`。复杂列表用 `json`。

`device` 的 `fills` 字段：`device_id`、`resource`、`kind`、`model`、`manufacturer`、`serial`（可用 `identity.model`）。

`kind` 与 GUI `instrument.list` 一致：`oscilloscope`、`dc_source`、`electronic_load`、`multimeter`、`debug_probe`、`usb_bridge`、`generic`。别名 `scope` / `psu` / `dmm` 仍可用。

需要 CLI 兼容时可保留隐藏的 `preflight_only`；Hub 预检按钮不读这个名字。

### 仪器选择示例

```json
{
  "name": "prefer_device_id",
  "type": "device",
  "path": "scope.prefer_device_id",
  "filter": { "kind": "oscilloscope" },
  "filter_from": "scope_kind",
  "fills": {
    "scope_resource": "resource",
    "scope_model": "model",
    "scope_kind": "kind"
  },
  "group": "instrument",
  "group_zh": "仪器",
  "label_zh": "仪器"
}
```

---

## 5. Overlay（MUST）

Hub 写入临时 JSON，例如：

```json
{
  "params": {
    "port": "COM7",
    "triggers": [{ "id": "T", "type": "contains", "pattern": "HIT" }]
  }
}
```

- runner：`--overlay <path>` → `loadOverlayFile` → 与 argv 合并（overlay 覆盖 argv）→ `applyParamPaths`
- **不** 把 overlay 写进 `station.json`
- 磁盘上的 `station.json` 仍是出厂/工位配方

---

## 6. `station.json` 路径

`paths` 对象 **必填**；其中的键都是可选字符串。

- `isf_dir` **不是** 契约必填。没有示波器的插件不要伪造它。
- HUD：声明 `paths.status_file`。需要跟目录走时用 `{isf_dir}/_loop_status.json`（契约二次展开 `{isf_dir}` / `{report_dir}` 等）。
- `colocateStatusWithIsf` 只在 **没有** `status_file` 且存在 `isf_dir` 时补默认文件名；不覆盖已声明路径。
- 模板：`{data_root}` `{product}` `{plugin_dir}` `{file_prefix}`，以及展开后的 `paths.*` 键。`{stamp}` 留给运行时。

---

## Marketplace（Phase 1 客户端）

```powershell
cd test-tools
node marketplace.mjs list --json
node marketplace.mjs verify --zip plugin.zip --meta meta.json
node marketplace.mjs install --zip plugin.zip --meta meta.json --data-root <root>
node marketplace.mjs catalog --url https://marketplace.example --channel stable
node marketplace.mjs pull --plugin <id> --version <ver> --url https://marketplace.example
node marketplace.mjs uninstall --plugin <id> --version <ver>
```

安装根目录默认 `{data_root}/marketplace`。Runner 合并 **active** 市场安装与捆绑 `plugins/`（同 id 时市场覆盖捆绑）。Hub 插件列表标题栏或右键可卸载市场安装；捆绑插件不能卸。

Schema：`schemas/marketplace-package.schema.json`。云端：`services/testing-hub-marketplace/`。

环境：`WIPARSE_CLI`、`WIPARSE_URL`、`WIPARSE_DATA_ROOT`、`WIPARSE_MARKETPLACE_URL`、`WIPARSE_MARKETPLACE_ALLOW_HTTP`。
