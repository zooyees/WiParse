# WiParse Testing Hub 插件规范

**日期** 2026-09-16  
**宿主** WiParse GUI **1.1.16+**（Testing Hub / `test_tool`；清单字段对齐 **1.1.26**）  
**运行时** Node.js **>=18**，ESM `.mjs`  
**读者** 插件作者 / 工位 AI

对齐 schema：

- [`schemas/plugin.schema.json`](./schemas/plugin.schema.json)
- [`schemas/station.schema.json`](./schemas/station.schema.json)
- [`schemas/marketplace-package.schema.json`](./schemas/marketplace-package.schema.json)

共享库（插件走 HTTP / 契约，不要复制 GUI）：

- `lib/plugin-contract.mjs` — 参数合并、模板展开、`normalizeResult`、`pickInstrument`、overlay、项目布局
- `lib/wiparse-sdk.mjs` — HTTP `/v1/health` + `/v1/invoke`
- `lib/line-triggers.mjs` — 通用行匹配（`regex` / `contains`）；**可选**，Hub 不内置
- `runner.mjs` — Hub 与 CLI 的唯一入口

示例：

- `plugins/example-smoke` — CLI + health，声明 `outputs[]` 与 `report_name`
- `plugins/scope-serial-monitor` — capture_loop 示例（配方在插件里，不在 Hub）
- `plugins/wiparse-dap` — CMSIS-DAP v2（USB / USB-IP / GUI probe-rs）
- `plugins/wiparse-uart-bridge` — UART 透传（WinUSB / TCP :1234）
- `plugins/wiparse-i2c-rpc` — I2C 文本 RPC（WinUSB / TCP :1236）
- `plugins/wiparse-netcfg` — CH-NETCFG 配网（USB MI_05 EP0 优先，DHCP 默认；TCP :1238 仅 GOT_IP 后）

---

## 1. Hub 必须保持仪器无关

Testing Hub **不知道** ASK、示波器、ISF、某条正则。它只做：

1. 发现 `plugins/<id>/plugin.json`
2. 按 `params[]` 画通用控件（`group` 是等宽分区标题，不理解组名语义；`advanced` 折叠）
3. 把本轮表单写成 **overlay JSON 文件**，runner `--overlay <path>`
4. 读 `paths.status_file` 的 JSON：只用 `step` / `hint` / `cycle` / `elapsed_s`
5. 把插件回写的 `suggested_params` 填进未改过的表单字段

VISA、串口、波形格式全部在 **插件**（`plugin.json` / `station.json` / 插件 JS）。不要给 Hub 加 `plugin.id` 分支或 `SerialTriggerEditor`。

宿主路径（数据根 / Node / CLI）在 Hub **左侧文件夹区**，不进插件 `params[]`。

Hub 调用：

```text
node test-tools/runner.mjs
  --plugin <id>
  --lifecycle preflight|run|stop
  --cli <WiParse-CLI.exe>
  --data-root <root>
  --project <id>
  --overlay <run-scoped.json>
  --plugins-root <dir>
  --marketplace-dir <dir>
  -- --<extra> <value> ...
```

**不要**让 GUI 传 `--test-id`（旧 runner 会当成 unknown arg 直接退出）。用例目录由 overlay 里的 **`report_name`**（或用户覆盖的 `paths.file_prefix`）决定；GUI 另设环境变量 `WIPARSE_TEST_ID` / `WIPARSE_PROJECT` 作后备。CLI 仍可显式 `--test-id` / `--stamp`。

overlay 是 **本轮运行** 的参数覆盖，**不会写回 `station.json`**。Windows 上避免把大段 JSON 塞进 argv。

环境变量：`WIPARSE_URL`、`WIPARSE_CLI`、`WIPARSE_DATA_ROOT`、`WIPARSE_PROJECT`、`WIPARSE_TEST_ID`、`WIPARSE_STAMP`。

GUI 已开时，Agent / 脚本经 CLI 驱动 Hub，不要自己 spawn runner：

```powershell
wiparse ui test-tool run --id <plugin.id>
wiparse ui test-tool run --id <plugin.id> --preflight true
wiparse ui test-tool run --id <plugin.id> --params "{\"port\":\"COM7\"}"
wiparse ui test-tool stop
wiparse ui project list
wiparse ui project select --id <project.id>
wiparse ui project create --id <project.id>
wiparse ui report open --path <path-under-project>
```

MCP `wiparse_ui` 对应 `op`：`test_tool.run|stop`、`project.*`、`report.open`（**不**回传文件正文）。

---

## 2. 目录（MUST）

```
test-tools/plugins/<id>/
  plugin.json      # 清单 + Hub 表单 + outputs[]
  index.mjs        # 入口
  station.json     # 可选；有工位配置时再提供
```

- `<id>` 与 `plugin.json.id` 一致：`^[a-zA-Z0-9][a-zA-Z0-9._-]*$`
- **`report_name`（MUST）**：测试报告左侧 **测试用例** 标题，也是默认 `tests/{test_id}/` 与 `file_prefix`。不能用 run 时间戳，不要用插件营销名（`name` / `name_zh`）冒充。可选 `report_name_zh`。
- 锁 / 停 / HUD 文件由 `paths.*` 声明。Hub **不** 硬编码 `_loop_status.json`。
- **禁止** 新 `run` 写入 `plugin_dir`、`instrument_data/`、`test report/`、旧 `evidence/`

---

## 3. Lifecycle（MUST）

| Lifecycle | Hub 按钮 | 含义 |
|-----------|----------|------|
| `preflight` | 预检 | 检查 GUI / 仪器 / 路径；**不要**开长循环；可返回 `suggested_params`；**不分** stamp、**不建** `runs/` |
| `run` | 运行 | 正式作业；capture_loop 直到 Stop；分配 stamp，写 `runs/{stamp}/` |
| `stop` | 停止 | 写 `paths.stop_file`（缺省 `{test_dir}/run.stop`）；不建 `runs/` |

Hub **只** 用 `--lifecycle` 区分预检与运行。不要用表单参数名 `preflight_only` 驱动生命周期。CLI 仍可 `--preflight_only true` 作为兼容回退（`resolveLifecycle`）。

```js
export async function preflight(ctx) { return normalizeResult(raw, "preflight"); }
export async function stop(ctx) { /* requestStop(config) */ }
export default async function run(ctx) { /* use ctx.stamp / ctx.layout */ }
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
  "session": "<ctx.stamp>",
  "artifacts": { "items": [{ "output": "waves", "path": "...", "view": "wave" }] },
  "error": "only when ok=false"
}
```

`suggested_params` 的 key **必须是** `params[].name`。Hub 默认 `untouched`（不覆盖用户已改字段）。需要时可用 `"empty"` / `"always"`。

请 `return normalizeResult(raw, lifecycle)`。`ok === false` 时 runner exit 1。

`artifacts` 用 `{ items:[{ output, path, view }] }`。路径必须在本次 `run_dir` 内。`stop` 不要写产物。

`run` 必须使用 `ctx.stamp` / `ctx.layout.run_dir`，一次 capture_loop **共用一个 stamp**。

### 运行中 HUD 信封

插件往 `paths.status_file`（缺省 `{test_dir}/status.json`）写 JSON。Hub **只读**：

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

顶层 **MUST**：`id`、`name`、`type`、**`report_name`**。`report_name` 是测试报告用例名，不是一次 run 的 stamp。

`params` 里的 `file_prefix`（「报告名称/前缀」）是 **可选覆盖**：空则用 `plugin.json.report_name`。它同时决定产物文件名 **与** 测试用例目录 `tests/{test_id}/`。

### `type`

| type | Hub 控件 | 值 |
|------|----------|-----|
| `string` / `number` | 单行 | 文本 / 数字 |
| `boolean` | 开关 | `true` / `false` |
| `path` | 文本 + 浏览 | 路径 |
| `device` | 已连接仪器下拉 | `device_id` |
| `enum` | 下拉（`options`） | option.value |
| `serial_port` | 本机 COM 列表 | `COM7` |
| `json` | 高度封顶的多行 JSON | 对象或数组（overlay 里保持结构化） |
| `text` | 高度封顶的多行文本 | 字符串 |

本轮 **没有** `type:list`。复杂列表用 `json`。

`device` 的 `fills` 字段：`device_id`、`resource`、`kind`、`model`、`manufacturer`、`serial`（可用 `identity.model`）。

`kind` 与 GUI `instrument.list` 一致：`oscilloscope`、`dc_source`、`electronic_load`、`multimeter`、`debug_probe`、`usb_bridge`、`generic`。别名 `scope` / `psu` / `dmm` 仍可用。

需要 CLI 兼容时可保留隐藏的 `preflight_only`；Hub 预检按钮不读这个名字。


### 仪器 HTTP（不要点 GUI）

截图 / 波形源走 invoke，**不要**调用已废弃的 `scope.shot`：

- `instrument.capture` `{ device_id, path, wait: true }` — HARDCopy PNG
- `instrument.waveform_source` `{ device_id, dir, filename }`
- `instrument.command` `{ device_id, command: "ScopeStop"|"ScopeRun" }`
- `instrument.list` / `instrument.connect` / `ui.instrument.select`

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
    "file_prefix": "mpp25w_test",
    "triggers": [{ "id": "T", "type": "contains", "pattern": "HIT" }]
  }
}
```

- runner：`--overlay <path>` → `loadOverlayFile` → 与 argv 合并（overlay 覆盖 argv）→ `applyParamPaths`
- **不** 把 overlay 写进 `station.json`
- 磁盘上的 `station.json` 仍是出厂/工位配方
- `file_prefix` 经 `sanitizeTestId` 后成为 `test_id`（目录名）；非法或空则回退 `plugin.json.report_name`，再回退 `plugin.id`

---

## 6. 项目落盘与测试报告（MUST）

索引与磁盘都是三层：**项目 → 测试用例 → 测试结果**。

```
{data_root}/projects/{project}/
  project.json
  tests/{test_id}/                 # 用例：plugin.json report_name（或覆盖后的 file_prefix）
    run.lock
    run.stop
    status.json
    runs/{stamp}/                  # 一条结果 = 一次 Hub run
      run.json
      artifacts/{output.id}/       # waves / reports / shots / docs
```

| 层 | 是什么 | 谁决定 |
|----|--------|--------|
| 项目 | `projects/{id}/` | Hub / 测试报告顶栏；共用 `apps.project.active_id` |
| 测试用例 | `tests/{test_id}/` | `report_name`（插件规则）；表单 `file_prefix` 可覆盖。`plugin_id` 另存 |
| 测试结果 | `runs/{stamp}/` | runner 分配的 stamp；其下 `outputs[]` 才是波形 / 报告 / 截图 |

`project.json` 的 `tests[]`：

```json
{ "id": "mpp25w_test", "plugin_id": "scope-serial-monitor", "name": "mpp25w_test" }
```

- `id`：用例目录名（sanitize 后的报告名）
- `plugin_id`：可缺省；缺省时 `id` 视为旧式插件 id
- `name`：显示名；缺省用插件 `report_name`，**不用** run 时间戳
- 兼容旧文件：`tests` 可以是字符串数组 `["scope-serial-monitor"]`
- 第一次 `run` 由 runner `ensureTestInProject` 写入。磁盘上已有、JSON 未登记的 `tests/{id}/` 仍会出现在测试报告树里；若目录名是 stamp 或插件 id，树标题仍显示 `report_name`

默认 `paths`：`test_dir` / `run_dir` / `artifacts_dir` / `lock_file` / `stop_file` / `status_file`。`product` 与 `file_prefix` 用于文件名与报告正文。`plugin_dir` 只读。

模板还展开 `{data_root}` `{project}` `{test_id}` `{plugin_id}` `{plugin_dir}` `{stamp}`，以及展开后的 `paths.*`。`isf_dir` **不是** 契约必填。

`plugin.json` `outputs[]`：`id` / `label` / `label_zh` / `view`（`wave` | `log` | `report` | `external`）/ `include` / `always`。

| `view` | 测试报告行为 |
|--------|----------------|
| `wave` | 本页嵌入波形分析（ISF / WFM / CSV）；Ctrl/Cmd 叠加 |
| `report` | 本页预览 Markdown |
| `log` | 本页预览文本 |
| `external` | png 本页预览；pdf / html 用系统程序打开 |

测试报告树：项目下拉 → 用例 → **每次结果（stamp）** → 分组标题 + 文件。不要把多次 run 的波形/报告摊平到同一个用例下。双击用例或结果打开该层最新报告。右键移除只改 `project.json`，不删磁盘。

`colocateStatusWithIsf` **不得覆盖**已声明 `status_file`；缺省 `{test_dir}/status.json`。

旧 `evidence/`、`instrument_data/`、`test report/` **不搬家**，报告树只扫 `projects/`。

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

安装根目录默认 `{data_root}/marketplace`。Runner 合并 **active** 市场安装与捆绑 `plugins/`（同 id 时市场覆盖捆绑）。Hub 插件列表标题栏或右键可卸载市场安装；**捆绑插件不能卸**。

Schema：`schemas/marketplace-package.schema.json`。云端：`services/testing-hub-marketplace/`。

环境：`WIPARSE_CLI`、`WIPARSE_URL`、`WIPARSE_DATA_ROOT`、`WIPARSE_MARKETPLACE_URL`、`WIPARSE_MARKETPLACE_ALLOW_HTTP`。
