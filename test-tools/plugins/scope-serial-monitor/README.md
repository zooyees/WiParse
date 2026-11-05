# Scope & Serial Monitor (`scope-serial-monitor`)

Industrial capture loop: serial trigger (rising-edge) → delay → ScopeStop + serial off → `instrument.capture` + `instrument.waveform_source` → PDF → ScopeRun + serial on.

GUI **1.1.16+**. Plugin **v0.7.1**. Visual editing requires the updated host; earlier hosts retain the JSON editor.

## 可视化触发配置

在集成测试中选择“示波器/串口监控”。常用页面只需填写串口、波特率、示波器和抓取条件；型号、地址、报告产品名等参数收在高级设置。

1. 选择串口和波特率，再选择已连接的示波器。点击“添加条件”，填入 `timeout`、`ERROR` 等关键词，收到匹配消息时自动保存截图和波形。默认忽略大小写。
2. 每条条件占一行，左侧开关控制是否启用。关键词直接在行内修改；点击“编辑”打开独立设置窗口，保存后生效，取消保留原配置。
3. 已有精确匹配在主页面只显示名称，匹配方式、大小写、忽略条件在编辑窗口中调整；名称、内部标识和标志收在窗口的高级选项。排序也在编辑窗口操作，同一消息只触发首条命中条件。
4. 高级设置中的“连续相同消息只抓取一次”默认开启：同一规则连续命中时只抓取一次，出现不匹配行后重新等待触发。
5. 点击“预检”检查正则语法和设备准备情况，通过后运行。空匹配内容、无启用规则、重复标识会在界面提示并阻止启动。

JSON 仍是存储和 CLI/API 格式，控件通过 `params[].widget: "line_triggers"` 声明；其他 JSON 参数不受影响。打开界面不会重写原值；编辑保留规则顺序、旧 `regex`/`text` 别名、`unless`/`except`/`exclude` 和未识别的扩展字段。读取失败时保留原文，只有“配置修复”中提供原文恢复入口。现有工厂 ASK/timeout 配方不做替换。

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
- **Line triggers** use the visual rule editor over the existing `json` data (`serial_triggers.items`). ASK 2 / timeout in `station.json` are only the factory recipe. Rule types: `regex` | `contains`; `unless` / `exclude` skip matches; `enabled: false` drops a rule. **连续命中只抓取一次** is `serial_triggers.rising_edge`.
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

- Report Markdown embeds each waveform screenshot directly under its capture. Waveform sources, logs and data remain portable relative links.

- `scope.resource` / `scope.model` default empty; Preflight fills them from the connected instrument.
- `serial_triggers.rising_edge` (default `true`). Exposed as Hub **连续命中只抓取一次**.
- `serial_triggers.items`: JSON array of `{ id, label?, enabled?, type, pattern, flags?, unless?, note? }`.
- `interlocks.hold_serial_off_during_capture` / `single_instance`: honored at runtime.
- Runtime files `run.lock` / `run.stop` / `status.json` live in `test_dir`. `product` is report-body only; folder tree is project / `file_prefix` / stamp.
