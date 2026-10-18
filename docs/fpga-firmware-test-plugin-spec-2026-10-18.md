# FPGA 固件测试插件设计规范

**文档状态**：Draft 0.6  
**日期**：2026-10-18  
**目标宿主**：WiParse 1.1.36+ / Testing Hub  
**插件名称**：FPGA固件测试  
**插件 ID**：`fpga-firmware-test`  
**宿主扩展**：[WiParse Testing Hub 扩展规格书](./testing-hub-extension-spec-2026-10-18.md)

## 1. 目标

本插件用于以配置文件驱动 FPGA 固件的批量自动化验证。一个插件实例可管理最多 500 个测试配置；每个配置映射为一个测试项，在 Testing Hub 的测试项表中显示名称、执行状态、固件、仪器绑定、耗时和结论。测试配置由插件内置的可视化编辑器创建和维护，JSON 仅作为内部持久化与交换格式，不要求也不支持用户日常直接编辑源码。

每个测试项应支持：

- 选择调试探针、示波器、日志通道及可选的电源等仪器角色；
- 指定 HEX/BIN 固件并记录下载选择；当前版本只预留下载接口，不执行下载；
- 从复位释放时刻计时，在指定时间截取波形画面，并可选读取源波形；
- 从同一时刻计时，抓取指定时长的普通 COM 或 WiParse Dongle UART 日志；
- 使用确定性规则完成结果判定；
- 可选 AI 辅助分析，但 AI 不得单独签发工业 PASS；
- 为每次执行保存配置快照、固件哈希、日志、波形、截图、规则结果和报告。

已确认的产品边界：`TBT` 表示 `To Be Tested/待测试`；正式状态集允许增加 `ERROR`、`SKIPPED`、`ABORTED`、`INCONCLUSIVE`；需要支持用例选择、标签分组、失败即停、循环/老化测试和 DUT 身份记录。当前工程版本允许编辑限值并立即用于下一次运行，但必须冻结并显示在最终报告中；正式审批作为后续生产能力。

## 2. 当前 WiParse 能力评估

| 需求 | 当前能力 | 结论 |
|---|---|---|
| 插件发现、预检、运行、停止 | 已有 `plugin.json`、`preflight/run/stop` 生命周期 | 可直接复用 |
| 通用配置参数 | 已有 string、number、boolean、path、device、enum、serial_port、json、text | 可用于选择配置目录和全局选项 |
| 测试项可视化配置 | 当前 `params[]` 只适合固定插件参数，不支持 500 个独立配置、可重复波形/规则和跨字段联动 | **需要新增 schema 驱动的配置工作区** |
| 固件烧录 | `instrument.command` 已支持 `ProbeFlash`；插件需求限定 HEX/BIN | 当前版本仅保留配置和适配器接口，不调用 `ProbeFlash` |
| 仪器发现和绑定 | `instrument.list/connect`，支持 debug_probe、oscilloscope、wiparse_dongle 等类型 | 可直接复用；配置中不应固化临时 `device_id` |
| 屏幕截图 | `instrument.capture` | 可直接复用 |
| 源波形保存 | `instrument.waveform_source` | 可直接复用，调用耗时可能达到数十秒 |
| 串口监控和日志读取 | `serial.monitor.start/stop/status`、`log.lines.get`，单页最多读取 2000 行 | 可直接复用，但要处理日志环形缓冲淘汰和独占关系 |
| 运行结果归档 | `projects/{project}/tests/{test_id}/runs/{stamp}`、`run.json`、outputs/artifacts | 可直接复用 |
| 执行状态与产品结论分离 | 结果契约已有 `execution_status`、`product_verdict`、`validation_verdict`、`industrial_approved` | 应直接采用 |
| 500 个配置生成实时测试项表 | 当前 Hub 只根据固定 `params[]` 画表单，HUD 只识别 step/hint/cycle/elapsed_s 和最多 6 个标量 | **需要扩展 Hub** |
| PASS/FAIL/TBT/TESTING 行状态 | 当前没有通用动态测试项协议 | **需要新增状态快照协议与表格 UI** |
| AI 辅助判定 | 当前插件契约没有模型调用、提示模板、数据脱敏或审计接口 | **需要独立的 AI 适配层；首版建议仅输出建议** |

### 核心结论

本方案不能被实现成“批量调用若干仪器命令”的单一插件脚本。完整产品由三层共同组成：Testing Hub 提供通用且虚拟化的 `test_matrix` 与配置工作区；FPGA 固件测试插件提供配置 schema、资源调度、证据采集和确定性判定；质量与追溯层负责 DUT 输入、实际使用限值、报告和未来审批。三层边界必须稳定，Hub 不写 FPGA 专用字段，插件不伪造审批状态，AI 不替代确定性判定规则。

基于当前 WiParse，插件执行器可复用既有仪器、日志、归档和结果契约；批量测试项表、选择/分组/循环执行、限值快照和 AI provider 接口需要补充。首版应先形成可审计的工程测试闭环，再逐步启用固件下载、全局 AI 和正式限值审批；不能因底层已有 `ProbeFlash` 就把尚未验证的下载链路承诺为生产能力。

## 3. 设计原则

1. **配置是测试输入，不是可执行脚本。** 配置只声明资源、时序、动作和规则，不允许包含任意 shell、PowerShell、JavaScript 或 SCPI 字符串，除非经过明确白名单。
2. **执行成功与产品通过分离。** `ok=true` 或 `execution_status=completed` 不等于产品 PASS。
3. **确定性规则拥有最终 PASS 权。** AI 可以解释、归因和建议复测，不能覆盖已确认 FAIL，也不能把缺失证据变成 PASS。
4. **设备按稳定身份绑定。** 配置使用 kind、serial、resource、model 等选择器；本轮预检解析成临时 `device_id`。
5. **每项单独建立计时基准。** 500 项顺序执行时，不能以点击“运行”的全局时刻作为每项波形和日志计时原点。
6. **原始证据不可被摘要替代。** 日志、波形、截图、固件哈希和配置快照必须保留。
7. **停止优先于继续。** Stop 后不得启动下一测试项；正在进行的不可中断 I/O 完成或超时后执行安全清理。
8. **可视化编辑是唯一受支持的配置入口。** UI 生成符合 schema 的配置；源码只读预览，不向普通用户提供 JSON 文本编辑器。

## 4. 总体架构

```mermaid
flowchart LR
    U[插件可视化配置工作区] --> A[配置仓库<br/>最多500个 JSON]
    A --> B[配置发现与Schema校验]
    B --> C[测试项快照<br/>matrix.json]
    C --> D[Testing Hub<br/>虚拟化测试项表]
    B --> E[预检<br/>资源解析/文件哈希/冲突检查]
    E --> F[顺序执行器]
    F --> G[固件接口校验<br/>当前版不下载]
    G --> H[复位释放并建立 capture_t0]
    H --> I[统一时序调度<br/>日志窗口/波形时点]
    I --> J[确定性规则判定]
    J --> K[可选AI辅助分析]
    K --> L[结果与证据归档]
    L --> C
```

采用**单 DUT、单工位、单套共享仪器顺序执行**。产品范围明确不支持多 DUT 或多工位并发，也不为并发调度、跨工位资源分配或多 DUT 状态聚合预留配置入口。一个插件 run 只绑定一个 DUT 身份和一个 `station_id`。

## 5. 插件目录

```text
test-tools/plugins/fpga-firmware-test/
  plugin.json
  station.json
  index.mjs
  README.md
  schemas/
    fpga-test-case.schema.json
    fpga-test-case.ui.json
    fpga-matrix-status.schema.json
  lib/
    config-loader.mjs
    resource-resolver.mjs
    scheduler.mjs
    firmware.mjs
    log-capture.mjs
    waveform-capture.mjs
    rules.mjs
    ai-adapter.mjs
    report.mjs
```

用户配置不应长期放在插件安装目录中，因为市场更新可能替换插件。建议配置根目录为：

```text
{data_root}/recipes/fpga-firmware-test/
  cases/*.json
  cases/_trash/*.json
  .editor/config-audit.jsonl
  firmware/*.{hex,bin}
  rules/*.json
  limits/*.json
```

Testing Hub 提供 `config_dir` 参数让用户选择其他受控目录。

## 6. `plugin.json` 建议

```json
{
  "id": "fpga-firmware-test",
  "name": "FPGA Firmware Test",
  "name_zh": "FPGA固件测试",
  "type": "custom",
  "version": "0.3.0",
  "entry": "index.mjs",
  "config": "station.json",
  "report_name": "FpgaFirmwareTest",
  "report_name_zh": "FPGA固件测试",
  "description": "Configuration-driven FPGA firmware validation suite.",
  "capabilities": ["configure", "preflight", "run", "stop"],
  "engines": { "node": ">=18", "wiparse": ">=1.1.36" },
  "ui": {
    "mode": "test_matrix",
    "matrix_status_file": "{test_dir}/matrix.json",
    "max_items": 500,
    "config_editor": {
      "schema": "schemas/fpga-test-case.schema.json",
      "ui_schema": "schemas/fpga-test-case.ui.json",
      "repository": "{config_dir}",
      "max_items": 500,
      "source_view": "read_only"
    }
  },
  "params": [
    {
      "name": "config_dir",
      "type": "path",
      "label": "Config directory",
      "label_zh": "配置目录",
      "group": "suite",
      "group_zh": "测试集"
    },
    {
      "name": "name_filter",
      "type": "string",
      "default": "",
      "label": "Name filter",
      "label_zh": "名称筛选",
      "group": "suite",
      "group_zh": "测试集"
    },
    {
      "name": "tag_filter",
      "type": "string",
      "default": "",
      "label": "Tag filter",
      "label_zh": "标签筛选",
      "group": "suite",
      "group_zh": "测试集"
    },
    {
      "name": "dut_serial",
      "type": "string",
      "label": "DUT serial number",
      "label_zh": "产品SN",
      "group": "identity",
      "group_zh": "DUT身份"
    },
    {
      "name": "board_revision",
      "type": "string",
      "label": "Board revision",
      "label_zh": "板卡版本",
      "group": "identity",
      "group_zh": "DUT身份"
    },
    {
      "name": "fpga_model",
      "type": "string",
      "label": "FPGA model",
      "label_zh": "FPGA型号",
      "group": "identity",
      "group_zh": "DUT身份"
    },
    {
      "name": "fixture_id",
      "type": "string",
      "label": "Fixture ID",
      "label_zh": "治具编号",
      "group": "identity",
      "group_zh": "DUT身份"
    },
    {
      "name": "station_id",
      "type": "string",
      "label": "Station ID",
      "label_zh": "工位编号",
      "group": "identity",
      "group_zh": "DUT身份"
    },
    {
      "name": "loop_count",
      "type": "number",
      "default": 1,
      "min": 1,
      "max": 100000,
      "label": "Loop count",
      "label_zh": "循环次数",
      "group": "execution",
      "group_zh": "执行"
    },
    {
      "name": "duration_limit_s",
      "type": "number",
      "default": 0,
      "min": 0,
      "label": "Aging duration limit",
      "label_zh": "老化时长上限",
      "unit": "s",
      "group": "execution",
      "group_zh": "执行"
    },
    {
      "name": "stop_on_fail",
      "type": "boolean",
      "default": false,
      "label": "Stop on FAIL",
      "label_zh": "失败即停止",
      "group": "execution",
      "group_zh": "执行"
    },
    {
      "name": "resume_mode",
      "type": "enum",
      "default": "fresh",
      "label": "Run mode",
      "label_zh": "运行方式",
      "options": [
        { "value": "fresh", "label": "Fresh", "label_zh": "全部重测" },
        { "value": "failed", "label": "Retry failed", "label_zh": "只重测失败项" },
        { "value": "unfinished", "label": "Resume unfinished", "label_zh": "继续未完成项" }
      ]
    },
    {
      "name": "ai_global_enable",
      "type": "boolean",
      "default": false,
      "label": "Allow AI assistance",
      "label_zh": "允许AI辅助",
      "advanced": true
    },
    {
      "name": "file_prefix",
      "type": "string",
      "default": "FpgaFirmwareTest",
      "label": "Report name",
      "label_zh": "报告名称"
    }
  ],
  "outputs": [
    { "id": "reports", "label_zh": "报告", "view": "report", "include": ["*.md", "*.json"] },
    { "id": "logs", "label_zh": "日志", "view": "log", "include": ["*.log", "*.txt", "*.jsonl"] },
    { "id": "waves", "label_zh": "波形", "view": "wave", "include": ["*.isf", "*.wfm", "*.csv"] },
    { "id": "shots", "label_zh": "截图", "view": "external", "include": ["*.png"] },
    { "id": "docs", "label_zh": "追溯数据", "view": "external", "include": ["*.json"] }
  ]
}
```

`ui.mode` 是本规范建议的新通用字段，当前 schema 和 GUI 尚未实现。未升级宿主时，插件仍可运行，但只能在日志和最终 Markdown 报告中展示测试项，不能显示实时表格。

## 7. 配置管理、编辑与发现

### 7.1 内部存储与发现规则

- v1 只接受 UTF-8 JSON，扩展名为 `.json`；暂不接受 YAML，避免额外解析依赖和隐式类型。
- JSON 文件由可视化编辑器管理；已有配置通过“导入”进入仓库，普通配置流程不要求用户打开或修改文件内容。
- 只扫描 `config_dir` 当前层，不递归。后续如需分组，再显式增加 `recursive`。
- 忽略以 `.`、`_` 开头的文件以及 `*.disabled.json`。
- 每个文件最大 1 MiB；总文件数最多 500；总配置体积建议不超过 32 MiB。
- `id` 在整个目录中必须唯一，匹配 `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`。
- `name` 必须存在，去除首尾空格后长度为 1–128 个字符；名称可以重复，但 UI 同时显示唯一 `id`。生产环境建议名称也唯一。
- 排序规则：`order` 升序，其次 `name` 自然排序，最后 `id`。
- 预检冻结文件绝对路径、SHA-256、schema 版本和解析结果。运行期间文件变化则中止后续项，不能静默使用新内容。

### 7.2 测试项表

建议列：

| 列 | 含义 |
|---|---|
| 选择 | 是否纳入本次运行 |
| 序号 | 冻结后的执行顺序 |
| 配置名称 | `name` |
| 配置 ID | `id`，可默认隐藏 |
| 状态 | TBT / TESTING / PASS / FAIL / ERROR / SKIPPED / ABORTED / INCONCLUSIVE |
| 固件 | 文件名与短 SHA-256 |
| 下载 | 是/否 |
| 仪器 | 已解析角色摘要 |
| 进度 | 当前阶段或计时 |
| 耗时 | 单项 elapsed |
| 说明 | 首个失败规则或错误代码 |

500 行必须使用虚拟化表格，只绘制可见行。支持名称、标签和状态过滤，全选/反选、仅运行选中项、进入可视化编辑和打开本项结果。默认不提供“打开配置文件并编辑源码”入口。用户选择集与筛选条件写入本次运行快照，确保报告能够解释哪些项目被执行或跳过。

### 7.3 执行选择、分组和循环

- 支持按名称、标签、状态筛选并勾选子集；未选项目显示 `SKIPPED`，原因记为 `not_selected`。
- 标签筛选默认采用“任一标签匹配”，高级选项可改为“全部标签匹配”。执行顺序仍按冻结后的 `order/name/id`，筛选不改变相对顺序。
- `stop_on_fail=true` 时，在当前 attempt 得出 FAIL 或执行 ERROR 后不再开始下一项；尚未开始的选中项标为 `SKIPPED`，原因分别记录 `stopped_after_fail` 或 `stopped_after_error`。
- `loop_count` 适用于当前选择集。每个 `item × loop_index` 都生成独立 attempt，不覆盖前一次结果。
- 老化测试允许以 `loop_count` 或可选 `duration_limit_s` 作为结束条件；二者同时存在时先到者生效。必须显示当前循环、累计循环、累计 PASS/FAIL/ERROR 和剩余条件。
- 套件最终结论按全部已执行 attempt 聚合：任一产品 FAIL 则套件 FAIL；没有 FAIL 但存在 ERROR/INCONCLUSIVE 则套件 INCONCLUSIVE；全部 required attempt PASS 才能 PASS。

### 7.4 插件内可视化配置工作区

插件页提供“配置测试项”入口。编辑器采用三栏结构，并在窄窗口下切换为列表页与编辑页：

| 区域 | 内容 |
|---|---|
| 左侧配置列表 | 名称、ID、启用状态、标签、校验状态、修改时间；支持搜索、筛选、排序和虚拟化滚动 |
| 中间配置表单 | 当前测试项的分组字段、可重复波形任务、日志配置和判定规则 |
| 右侧检查面板 | 实时错误/警告、资源绑定摘要、时间轴预览、预计耗时、只读 JSON 预览 |

中间表单按以下顺序分组：

1. **基本信息**：名称、ID、启用、执行顺序、标签、总超时；
2. **DUT 信息**：板卡型号、版本、FPGA 型号及运行时身份字段要求；
3. **仪器绑定**：调试器、示波器、DC 电源、日志端口等角色与稳定身份选择器；
4. **固件**：HEX/BIN 文件选择、格式、哈希、目标和下载开关；当前下载开关显示“接口预留/不可执行”；
5. **复位与计时**：DC 电源或 WiParse Dongle 二选一，并只显示所选方法相关字段；
6. **日志采集**：普通 COM 或 Dongle UART、波特率、开始时刻、时长和容量限制；
7. **波形任务**：可新增、复制、删除、拖动排序多个捕获卡片；每张卡配置时点、通道、截图、源波形和超时；
8. **判定规则与限值**：可新增、复制、删除、排序规则；按规则类型显示字段，实时展示单位和上下限；
9. **AI 辅助**：启用开关、provider profile、允许输入和脱敏策略；
10. **预览与说明**：生成的执行时间轴、所需仪器、预计证据量和最终只读配置摘要。

表单必须使用类型化控件：布尔开关、枚举下拉框、带单位数值框、文件选择器、仪器选择器、标签输入、时间轴和规则构建器。不得要求用户输入 JSON 片段、正则以外的代码、SCPI 或命令行。字段联动示例：选择 `dc_power` 后显示电源角色、通道、断电时间和电压阈值；选择 `wiparse_dongle` 后显示 Dongle 角色和硬件复位选项。

配置列表支持以下操作：新建、复制、重命名、启用/禁用、编辑、批量添加标签、调整顺序、归档、导入和导出。删除采用可恢复归档，先移入内部 `_trash`，不直接永久删除。导入只接受通过 schema 校验的配置包；导入后仍通过可视化表单维护。

### 7.5 编辑、校验与保存语义

- 新建或编辑时先操作内存草稿；未保存变化显示明显的“未保存”状态。
- 字段级校验即时执行；跨字段校验在变化后执行，例如 BIN base address、复位角色、日志角色、规则引用和波形时间窗口重叠。
- “保存”只有在阻断错误为零时可用；warning 允许保存，但在列表和预检中保持可见。
- 保存采用临时文件写入、flush、原子替换；同时更新 `editor_revision`、`updated_at` 和内容 SHA-256。
- UI 以配置 ID 管理文件名，用户不需要选择或编辑 JSON 文件名。ID 修改时执行受控重命名并检查引用。
- 打开编辑器时记录原文件哈希。若保存前检测到外部变化，禁止静默覆盖，要求重新加载或另存副本。
- 活动 run 使用冻结快照。运行期间允许查看配置，但禁止修改本次选择集中的配置；其他配置的修改也只对下一次预检生效。
- 保存成功后使旧预检结果失效，测试项回到 TBT，并提示重新预检。
- 高级诊断可以查看只读 JSON 和 schema 错误路径，但不提供源码编辑框。
- 每次保存、复制、归档、恢复和导入均写审计事件，至少包含操作者、时间、配置 ID、旧/新哈希和操作类型。

## 8. 单配置文件规范

### 8.1 示例

```json
{
  "schema_version": "wiparse.fpga-test-case/1",
  "editor_revision": 12,
  "updated_at": "2026-10-18T10:00:00Z",
  "id": "fpga_boot_001",
  "name": "基础启动与时钟验证",
  "enabled": true,
  "order": 10,
  "tags": ["smoke", "clock"],
  "timeout_s": 180,
  "dut": {
    "model": "FPGA-BOARD-A",
    "serial_required": true,
    "board_revision": "REV-A",
    "fpga_model": "XC7A100T",
    "fixture_id": "FIXTURE-01",
    "station_id": "STATION-01"
  },
  "instruments": {
    "programmer": {
      "kind": "debug_probe",
      "selector": { "serial": "WP-DONGLE-001" },
      "required": true
    },
    "scope": {
      "kind": "oscilloscope",
      "selector": { "model": "MDO3014" },
      "required": true
    },
    "power": {
      "kind": "dc_source",
      "selector": { "model": "DP832" },
      "required": true
    },
    "log": {
      "kind": "wiparse_dongle_uart",
      "selector": {
        "dongle_serial": "WP-DONGLE-001",
        "uart_channel": 0,
        "baud": 2000000
      },
      "required": true
    }
  },
  "firmware": {
    "download": false,
    "implementation": "reserved",
    "path": "../firmware/fpga_boot_v12.hex",
    "sha256": "<64-hex>",
    "target": "XC7A100T",
    "format": "hex",
    "base_address": null,
    "verify": true
  },
  "reset": {
    "method": "dc_power",
    "power_role": "power",
    "channel": 1,
    "off_time_ms": 500,
    "release_confirmation": {
      "type": "voltage_threshold",
      "minimum_v": 11.4,
      "stable_ms": 20,
      "timeout_ms": 2000
    }
  },
  "timing": {
    "anchor": "after_reset_release"
  },
  "log_capture": {
    "enabled": true,
    "start_at_s": 0,
    "duration_s": 30,
    "max_lines": 200000,
    "encoding": "utf-8"
  },
  "waveform_captures": [
    {
      "id": "boot_5s",
      "at_s": 5.0,
      "kind": "screenshot",
      "channels": ["CH1", "CH2"],
      "source_waveform": {
        "enabled": true,
        "format": "instrument_native"
      },
      "timeout_s": 90
    }
  ],
  "verdict": {
    "mode": "all",
    "rules": [
      {
        "id": "boot_ok",
        "type": "log_regex",
        "pattern": "\\bFPGA_BOOT_OK\\b",
        "minimum_matches": 1,
        "required": true
      },
      {
        "id": "no_fatal",
        "type": "log_not_regex",
        "pattern": "\\b(FATAL|ASSERT|PANIC)\\b",
        "required": true
      },
      {
        "id": "clock_frequency",
        "type": "waveform_metric_range",
        "capture": "boot_5s",
        "channel": "CH1",
        "metric": "frequency_hz",
        "minimum": 99900000,
        "maximum": 100100000,
        "unit": "Hz",
        "required": true
      }
    ],
    "ai_assist": {
      "enabled": false,
      "policy": "advisory_only",
      "provider_profile": "local-default",
      "inputs": ["log", "rule_results", "waveform_metrics"],
      "required": false
    }
  }
}
```

### 8.2 仪器选择

配置声明角色，不保存本次 GUI 分配的 `device_id`。预检通过 `instrument.list` 解析：

1. 先匹配物理 serial；
2. 再匹配 resource；
3. 再匹配 manufacturer/model；
4. 若仍有多个候选，预检失败并要求人工选择，禁止自动取第一台。

当 WiParse Dongle 同时暴露 DAP、UART、I2C 等通道时，应通过共同的 `dongle_serial` 绑定为同一物理设备，防止程序下载和日志采集落在不同 Dongle 上。

配置支持的角色建议为：`programmer`、`scope`、`log`、`power`、`dmm`、`load`、`bridge`。没有用到的角色省略。

日志角色只允许两类：

- `serial_port`：普通 COM，selector 至少包含 `port`、`baud`，可选数据位、停止位、校验和流控；
- `wiparse_dongle_uart`：WiParse Dongle UART，selector 至少包含稳定的 `dongle_serial`、`uart_channel` 和 `baud`。

两类日志源统一输出带单调时间戳的 UTF-8 JSONL 和原始字节文件。若解码失败，原始字节仍保留，判定规则不得基于丢失或替换后的字符给出 PASS。

进入测试模式后，WiParse 将串口所有权从串口终端移交给本插件，终端与集成测试不得分别打开同一物理端口。插件接管时冻结起始游标、清理本次逻辑窗口并记录交接事件；测试结束、停止或异常清理后把所有权交还串口终端。终端在测试期间可以显示全局日志服务提供的只读镜像，但不能更改波特率、清空缓冲或发送数据。若端口被 WiParse 之外的程序占用，预检仍应阻断。

### 8.3 固件下载

- 配置格式只接受 `hex` 和 `bin`；BIN 必须声明 `base_address`，HEX 不使用 `base_address`。
- 当前版本只校验路径、扩展名、格式、目标、SHA-256 和可选 base address，并把这些信息写入报告；不执行固件下载。
- 当前版本要求 `download=false`。若配置为 `download=true`，预检以 `FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED` 阻断，运行器不得调用 `ProbeFlash` 或外部烧录工具。
- 即使 `download=false`，仍要记录声明的固件版本/哈希；无法从 DUT 读取实际版本时，报告标为“未验证固件身份”。
- 预留 `FirmwareProgrammer` 版本化接口：`prepare → program → verify → reset → read_identity`。后续实现必须保持配置 schema 兼容，并针对 HEX/BIN 分别实现受信任适配器。
- 首版禁止配置任意外部烧录命令。后续若支持厂商工具，必须通过受信任适配器和参数白名单实现。

### 8.4 DUT 身份

每次套件运行前必须采集并冻结以下身份：产品 SN、板卡版本、FPGA 型号、治具编号、工位编号。本插件用于研发/实验室测试，不以流水线、MES 或 DUT 自动读回为前提；运行前表单输入值就是本次测试的权威身份数据。

- 产品 SN 在一次运行中必填且不得为空；同一 SN 的复测通过新的 `attempt_id` 关联。
- 操作员确认输入后生成 `identity_source=user_input`、输入时间和操作者标识；运行开始后冻结，修改身份必须新建 run。
- 配置可以声明板卡版本和 FPGA 型号的期望值；与输入值不匹配时在采集前阻断。
- 身份快照写入 `run.json`、每项 result 和最终报告；产品 SN 默认不发送给公网 AI。

### 8.5 波形捕获

`waveform_captures[]` 的基本动作是截取指定时刻的波形画面，即调用 `instrument.capture` 保存截图。`source_waveform.enabled=true` 时，在同一捕获任务中额外调用 `instrument.waveform_source` 读取仪器源波形。源波形读取失败时截图仍保留；若 required 规则引用源波形，则本项为 ERROR 或 INCONCLUSIVE，不能只凭截图 PASS。

同一示波器一次只允许一个源波形任务。预检必须根据 `at_s` 与各任务 `timeout_s` 判断时间窗口是否可能重叠；发现重叠时以 `WAVEFORM.SCHEDULE_OVERLAP` 阻断整个运行，并在对应配置行显示冲突的 capture ID 和时间窗口。不得自动排队、推迟执行或并行发送命令。

### 8.6 复位方式与释放时刻

每个配置通过 `reset.method` 二选一：

| method | 使用资源 | 动作 | `capture_t0` |
|---|---|---|---|
| `dc_power` | `dc_source` 电源角色 | 关闭指定通道 → 等待 `off_time_ms` → 打开通道 → 确认电压恢复 | 首次满足电压阈值并持续 `stable_ms` 的单调时钟时刻 |
| `wiparse_dongle` | WiParse Dongle 的 DAP/复位角色 | 执行硬件复位；适配器负责拉低并释放复位线 | Dongle 报告复位命令完成的单调时钟时刻；若后续提供独立 release 事件，则改用事件时刻 |

DC 电源复位使用 WiParse 现有 `SourceOutput {channel, enabled}` 能力。推荐 `release_confirmation.type=voltage_threshold`，通过电源读回值确认 DUT 供电真正恢复；若电源不支持足够快的读回，可配置 `command_ack`，此时 `capture_t0` 为输出开启命令成功返回时刻，并在报告中标记较大的计时不确定度。

WiParse Dongle 复位通过现有 `ProbeReset { hardware: true }` 能力执行，配置示例：

```json
{
  "reset": {
    "method": "wiparse_dongle",
    "dongle_role": "programmer",
    "hardware": true,
    "release_confirmation": {
      "type": "command_complete",
      "timeout_ms": 2000
    }
  },
  "timing": {
    "anchor": "after_reset_release"
  }
}
```

两种方法均必须记录方法、设备物理身份、命令时间、确认时间、`capture_t0`、确认方式和计时不确定度。复位失败或无法确认释放时，本项为 ERROR，不启动日志/波形计划。禁止在一次 attempt 中同时执行两种复位方式；如需回退，必须新建 attempt 并明确记录方法变化。

建议固定错误码：`RESET.CONFIG_INVALID`、`RESET.POWER_OFF_FAILED`、`RESET.POWER_ON_FAILED`、`RESET.RELEASE_TIMEOUT`、`RESET.DONGLE_FAILED`、`RESET.STATE_UNKNOWN`。其中 `STATE_UNKNOWN` 表示命令超时后无法确认 DUT 是否仍处于复位或供电状态，必须执行安全清理后才允许下一项。

## 9. 计时语义

规范同时保留三个时间：

- `suite_t0`：用户点击本次测试集“运行”的时间；只用于整套报告。
- `item_t0`：当前配置真正开始执行的时间；用于单项耗时。
- `capture_t0`：检测或发出 DUT 复位释放事件时建立的有效测试计时原点；波形和日志一律相对此时间。当前版不以固件下载作为前置条件。

配置中的 `waveform_captures[].at_s`、`log_capture.start_at_s` 和 `duration_s` 默认相对 `capture_t0`。

不建议将它们定义为相对 `suite_t0`。当第 300 个配置开始执行时，全局时间早已超过指定截取时间，测试无法正确执行。

调度器使用单调时钟，不使用墙上时间计算延时。墙上 UTC 时间只用于追溯。任务晚于计划时点超过 `late_tolerance_ms` 后，默认报告 ERROR，不允许悄悄延后并仍判 PASS。

## 10. 单项执行流程

```text
TBT
  → 解析并占用仪器
  → 校验固件声明（当前版不下载）
  → 配置示波器/日志通道
  → 按配置执行 DC 电源或 WiParse Dongle 复位
  → 确认复位释放，建立 capture_t0
  → TESTING：按时启动日志和波形任务
  → 等待最长采集窗口完成
  → 确定性规则判定
  → 可选AI辅助分析
  → 写入证据与单项报告
  → PASS / FAIL / ERROR / ABORTED
  → 安全清理并释放资源
```

日志抓取和波形定时必须由同一个调度器管理。若 `instrument.waveform_source` 在 `at_s` 触发后耗时 30–75 秒，`at_s` 表示发出捕获动作的目标时间，报告同时记录请求时间、仪器完成时间和时点误差。

## 11. 状态模型

主视图状态定义如下，其中 `TBT` 的正式含义为 `To Be Tested/待测试`：

| 显示状态 | 内部状态 | 含义 |
|---|---|---|
| TBT | `pending` | 已发现且尚未执行 |
| TESTING | `running` | 正在执行 |
| PASS | `completed` + `product_verdict=PASS` | 执行完整且全部必要确定性规则通过 |
| FAIL | `completed` + `product_verdict=FAIL` | 执行完整且至少一项产品规则失败 |

经确认，工业状态集同时增加：

| 状态 | 含义 |
|---|---|
| ERROR | 烧录、仪器、文件、超时、存储或程序异常，产品结论不能确定 |
| SKIPPED | 被过滤、禁用或因前置条件未满足而跳过 |
| ABORTED | 用户停止或进程取消 |
| INCONCLUSIVE | 执行完成但证据不足、质量不合格或判定边界不确定 |

扩展状态必须作为一等状态参与统计，不能折叠成 FAIL 或 TBT，否则会污染产品良率或掩盖执行失败。

状态转换必须单向且落盘：

```text
pending → running → PASS | FAIL | INCONCLUSIVE | ERROR | ABORTED
pending → SKIPPED
```

除显式“新建复测 attempt”外，不允许终态回到 TESTING。复测生成新的 attempt，不覆盖原结果。

## 12. 测试项状态文件协议

建议新增 `{test_dir}/matrix.json`，由插件原子写入，Hub 每 400 ms 检查 mtime/revision。示例：

```json
{
  "schema_version": "wiparse.test-matrix/1",
  "revision": 27,
  "run_id": "uuid",
  "attempt_id": "uuid",
  "updated_at": "2026-10-18T08:30:00.000Z",
  "active_item_id": "fpga_boot_001",
  "summary": {
    "total": 120,
    "pending": 117,
    "running": 1,
    "pass": 1,
    "fail": 0,
    "error": 1,
    "skipped": 0,
    "aborted": 0,
    "inconclusive": 0
  },
  "items": [
    {
      "id": "fpga_boot_001",
      "name": "基础启动与时钟验证",
      "order": 10,
      "status": "running",
      "phase": "capturing_waveform",
      "elapsed_s": 5.4,
      "message": "等待 CH1/CH2 源波形",
      "config_sha256": "...",
      "firmware_sha256": "...",
      "result_ref": null
    }
  ]
}
```

要求：

- 使用临时文件 + flush + 原子替换，避免 GUI 读到半个 JSON；
- `revision` 每次变化递增；
- 最多 500 项；单项 message 建议限制 256 字符；
- 表中不写完整日志、波形点列或 AI 长文本；
- 结束后完整快照复制到本次 `run_dir/artifacts/docs/matrix-final.json`；
- 当前 `status.json` 继续写 step/hint/cycle/elapsed_s，用于旧版 Hub 兼容。

## 13. 结果判定

### 13.1 支持的确定性规则

首版建议支持：

- `log_contains` / `log_regex` / `log_not_regex`；
- `log_match_count`；
- `waveform_metric_range`：频率、周期、占空比、峰峰值、最小/最大/均值、边沿时间等当前分析器可稳定输出的指标；
- `instrument_measure_range`；
- `file_exists` / `artifact_hash`；
- `all` / `any` / `not` 组合；
- `custom_adapter` 仅引用插件内置且受版本控制的适配器 ID，不允许配置任意代码路径。

每条规则必须包含 `id`、`type`、`required`、输入证据引用和失败原因。数值规则必须声明单位。缺失、截断、坏质量或单位不匹配不得判 PASS。

聚合规则：

- 任一 required 规则 FAIL → 产品 FAIL；
- 任一 required 规则缺证据/不确定 → INCONCLUSIVE；
- 所有 required 规则 PASS，且执行完整 → PASS；
- 执行错误 → execution ERROR，同时保留已确认的产品 FAIL，不能用错误覆盖它。

### 13.2 AI 辅助

建议字段：

```json
{
  "enabled": true,
  "policy": "advisory_only",
  "provider_profile": "local-default",
  "inputs": ["log_excerpt", "rule_results", "waveform_metrics"],
  "required": false,
  "timeout_s": 30,
  "redaction_profile": "fpga-default",
  "prompt_template_id": "fpga-fault-triage-v1"
}
```

插件当前提供版本化 AI 配置接口，支持两类 provider profile：

```json
{
  "profiles": {
    "local-default": {
      "kind": "local",
      "adapter": "openai_compatible",
      "endpoint": "http://127.0.0.1:11434/v1",
      "model": "configured-by-station"
    },
    "public-approved": {
      "kind": "public",
      "adapter": "openai_compatible",
      "endpoint": "https://approved-provider.example/v1",
      "model": "configured-by-station",
      "credential_ref": "secret-store:fpga-ai-public"
    }
  }
}
```

本地和公网模型都通过 adapter 调用，配置文件不得保存明文密钥。公网 profile 必须绑定数据外发权限与脱敏策略。后续 WiParse 提供全局 AI 接口时，新增 `wiparse_global` adapter，把插件请求映射到全局接口；测试用例只引用 `provider_profile`，无需批量修改 500 个配置。

AI 输出结构：

```json
{
  "recommendation": "PASS|FAIL|REVIEW",
  "confidence": 0.0,
  "reason_codes": ["..."],
  "summary": "...",
  "evidence_refs": ["..."],
  "model": "...",
  "prompt_template_id": "..."
}
```

约束：

- AI 默认 `advisory_only`；其结论单独显示，不进入正式产品 verdict 聚合。
- AI 不得覆盖确定性 FAIL，不得用推断补齐缺失证据，不得把 INCONCLUSIVE 提升为 PASS。
- AI 不可用、超时或输出格式错误时，只记录 AI ERROR，不使原本确定性 PASS 变为 FAIL；若业务明确要求 AI 必须完成，则总体为 INCONCLUSIVE，而不是产品 FAIL。
- 发送到外部模型前必须明确日志、波形摘要和产品信息的数据边界，支持脱敏、大小限制、审计和用户/企业批准。
- 固件二进制、完整原始波形、密钥、序列号默认禁止外发。
- 若未来允许 AI 参与放行，必须另建经验证、版本冻结、可复现和可审计的批准流程；不属于首版范围。

### 13.3 当前限值使用与报告

当前版本定位为研发/实验室测试，允许在配置中直接填写 `minimum/maximum/unit`，也允许通过 `limit_ref` 引用限值文件。限值修改保存后可立即用于**下一次预检和运行**，不要求审批或电子签名。已经开始的 run 必须继续使用启动时冻结的限值快照，禁止运行中修改后影响正在执行的判定。

限值文件示例：

```json
{
  "schema_version": "wiparse.test-limits/1",
  "limits_version": "FPGA-A-2026.10",
  "mode": "engineering_editable",
  "product": "FPGA-BOARD-A",
  "entries": {
    "fpga-clock-100m.frequency": {
      "minimum": 99900000,
      "maximum": 100100000,
      "unit": "Hz"
    }
  }
}
```

- 预检解析内联限值和全部 `limit_ref`，检查范围、单位和量纲；找不到引用、范围反转或单位不匹配均为阻断错误。
- 每次 run 冻结实际使用的限值内容、来源文件、配置项、SHA-256 和冻结时间。后续编辑不能改变历史结果。
- 最终报告必须在摘要和每条规则明细中显示：规则名称、测量值、下限、上限、单位、PASS/FAIL、限值来源及快照哈希。
- 报告显著标记“工程可编辑限值/未执行正式审批”，当前版本固定 `industrial_approved=false`。
- 后续进入生产使用时，再在同一限值 schema 上增加审批状态、审批人、电子签名、变更单号和只读版本；不影响当前配置格式。

## 14. 预检要求

点击“预检”后应发现配置并生成 TBT 表，但不得烧录、复位 DUT 或启动长采集。预检至少检查：

1. 配置目录存在且在允许范围内；
2. 配置工作区没有未保存草稿；文件数量、大小、JSON/schema、`editor_revision`、内容哈希和 ID 唯一性合法；
3. 所有固件存在、路径未越界、SHA-256 匹配，且格式为 HEX/BIN、BIN base address 合法；`download=true` 以未实现错误阻断；
4. WiParse API 与版本；
5. 每项所有 required 仪器均能唯一解析；
6. `reset.method` 只能是 `dc_power` 或 `wiparse_dongle`，对应电源/Dongle 角色可唯一解析；DC 通道、断电时长、阈值和超时合法；
7. 同一物理 Dongle 的 DAP、复位和 UART 通道绑定一致；
8. 测试模式能够把普通 COM 或 WiParse Dongle UART 从 WiParse 串口终端移交给插件；若被外部程序占用则阻断；
9. `timeout_s` 大于复位、最长日志/波形计划加预留时间；
10. 规则引用的 capture、channel、metric 均存在，且同一示波器的源波形时间窗口不重叠；
11. AI 开启时，本地/公网 provider profile、适配器、权限、凭据引用、脱敏策略和模型配置存在；
12. 用户输入的产品 SN、板卡版本、FPGA 型号、治具编号和工位编号完整且与配置兼容；
13. 内联限值及所有 `limit_ref` 可解析，范围、单位和量纲合法；
14. 选择集、标签筛选、停止策略、循环次数和老化结束条件合法；
15. 数据根目录可写且空间达到工位设定阈值；
16. 估算整套最短/最长运行时间和证据空间。

同一 `station_id` 已有活动 run 时，新 run 预检返回 `RUN.STATION_BUSY`。执行器不得同时运行两个测试项；日志窗口与波形定时可以在同一测试项内按统一调度器协同执行，这不构成多 DUT 并发。

任何阻断问题必须列到具体配置行。非阻断问题显示 warning；用户可查看详情，但运行时仍冻结全部 warning。

## 15. 证据与目录

建议一次测试集 run 对应一个 WiParse run：

```text
projects/{project}/tests/FpgaFirmwareTest/runs/{stamp}/
  run.json
  artifacts/
    reports/
      summary.md
      suite-result.json
    docs/
      configuration-manifest.json
      dut-identity.json
      limits.snapshot.json
      reset-events.jsonl
      matrix-final.json
      errors.jsonl
    items/{item_id}/
      config.snapshot.json
      result.json
      firmware.json
      logs/serial.log
      waves/*.isf
      shots/*.png
      ai/analysis.json
```

由于现有 `outputs[]` 以固定目录分组，首版实现时可把 items 放在 `docs` 或为插件增加固定 `items` external output。报告中必须提供到每个测试项证据的相对链接。

每项 `result.json` 至少包含：

- schema、run_id、attempt_id、item_id；
- 配置、固件和本次实际限值快照 SHA-256；
- 产品 SN、板卡版本、FPGA 型号、治具编号和工位编号；
- resolved instruments 与物理身份；
- suite/item/capture 单调时间和 UTC 时间；
- execution_status、product_verdict、validation_verdict；
- 每条规则输入、输出和 evidence refs；
- AI 建议及模型审计字段；
- 首个错误 ID、所有错误、清理结果；
- 每条规则实际使用的上下限、单位、来源和运行模式；
- 当前版本固定 `industrial_approved=false`，报告标明限值可编辑且未经正式审批。

当前版本默认不自动删除证据：报告、配置、身份、限值快照、日志、截图和源波形随项目一直保留，直到用户显式归档或删除整个 run。不得只删除某个失败项的原始证据而保留其 PASS/FAIL 结论。删除/归档操作记录操作者、时间、run_id 和原因；预检按预计数据量检查剩余空间，空间不足时阻断。后续如需容量策略，可增加无损压缩和项目级保留期限，但不得改变既有报告引用和哈希。

## 16. 停止、恢复和复测

- Stop 创建 stop file，执行器在每个调度边界检查；不再开始新项。
- 当前版本没有烧录步骤；未来启用下载后，不得强杀正在烧录的探针进程，应等待 API 返回或超时再进入恢复流程。
- 正在源波形读取时等待调用完成或超时，随后安全清理。
- 停止后当前项为 ABORTED，未开始项保持 TBT；整套 `execution_status=cancelled`。
- `resume_mode=unfinished` 新建 attempt，复用同一冻结配置 manifest；若配置或固件哈希已变化，必须重新预检并新建 run。
- `retry failed` 不能覆盖原 FAIL；生成新的 item attempt 并在报告中关联 previous_attempt_id。

## 17. Testing Hub 通用扩展

需要修改的宿主能力：

1. `PluginInfo` 解析可选 `ui.mode=test_matrix`、`matrix_status_file`、`max_items` 和 `ui.config_editor`；
2. `preflight` 完成后重新读取 matrix 文件，即使当前没有 run job；
3. 增加虚拟化测试项表、过滤、选择和状态统计；
4. 运行时除现有 `status.json` 外，按 mtime/revision 读取 matrix 文件；
5. 对 matrix 文件设置大小与项目 jail 限制；
6. 表格只展示协议字段，不理解 FPGA、固件或规则语义；
7. API `ui.test_tool.state` 可返回摘要和分页后的 items，避免一次返回全部长文本；
8. 增加 schema 驱动的配置工作区，支持 object、array、`oneOf`、条件字段、单位、文件和仪器选择器；
9. UI schema 只描述分组、顺序、帮助文本、时间轴和规则构建器等表现，不承载执行逻辑；
10. 增加受项目 jail 约束的配置 API：list/get/save/duplicate/archive/restore/import/export，并使用 revision/hash 做并发冲突检测；
11. 活动 run 时向编辑器提供只读锁状态；保存后自动使旧 preflight revision 失效；
12. 保持旧插件 UI 和生命周期完全兼容。

建议新增分页接口结构：

```json
{
  "plugin_id": "fpga-firmware-test",
  "matrix": {
    "revision": 27,
    "total": 500,
    "offset": 0,
    "limit": 100,
    "items": []
  }
}
```

不建议把 500 个配置注册为 500 个独立插件，也不建议把 500 个配置塞进 `params[].options`。前者污染插件市场和项目树，后者无法表达逐项实时状态。

配置工作区是宿主的通用能力，宿主只解释 schema/UI schema 和文件操作协议，不硬编码“固件”“复位”“波形”等 FPGA 字段。具体字段、条件和帮助文本由插件提供。

## 18. 验收标准

### 配置与 UI

- 用户可以完全通过插件 UI 新建、复制、编辑、禁用、排序、归档、恢复、导入和导出测试项，不需要打开 JSON 源文件；
- DC 电源/WiParse Dongle 复位、COM/Dongle UART、波形任务和规则类型切换后，只显示相关字段；
- 字段级与跨字段错误能定位到具体控件；存在阻断错误时不能保存或运行；
- 保存使用原子替换；外部文件变化或 revision 冲突时不得覆盖用户或其他会话的修改；
- 活动 run 的配置只读；保存其他配置后旧预检失效，下一次运行使用新 revision；
- 归档可恢复，非法导入不会部分写入配置仓库；只读 JSON 预览不能修改内容；
- 能发现 1、100、500 个合法配置，顺序稳定；501 个时预检明确失败；
- 重复 ID、坏 JSON、路径越界、固件哈希错误能定位到具体行；
- 500 行滚动与状态刷新不明显卡顿；
- 名称/标签/状态过滤、选择和重新预检不会丢失用户选择，除非配置集合变化；
- 可按选择集执行、失败即停、循环计数或老化时长运行，并正确显示每个 attempt；
- 一个 run 只能绑定一个 DUT 和一个工位；同一工位已有活动 run 时明确阻断；
- 每行状态与磁盘 matrix 最终状态一致。

### 执行

- `download=false` 不调用 ProbeFlash；当前版 `download=true` 必须在预检以 `FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED` 阻断；
- DC 电源复位能执行关断/等待/开启/电压确认；WiParse Dongle 能执行硬件复位；每个配置只能选择一种；
- 两种复位方式都建立唯一 `capture_t0`，日志和波形计划均引用该时间基准，失败时不得开始采集；
- 波形动作相对 `capture_t0` 的触发误差被记录，并符合工位容差；
- 每个波形任务至少保存截图；启用源波形读取时同时保存原始源波形及读取状态；
- 两个源波形时间窗口可能重叠时，预检以 `WAVEFORM.SCHEDULE_OVERLAP` 阻断，不得排队执行；
- 日志窗口严格覆盖声明的起止范围，发生缓冲淘汰时不能 PASS；
- 进入测试模式后插件从 WiParse 串口终端接管端口，结束后交还；普通 COM 与 WiParse Dongle UART 均能采集、时间戳化并保留原始字节；
- Stop 或失败即停后不开始下一项；循环/老化的每个 attempt 独立保存，复测不覆盖原结果；
- 用户输入的产品 SN、板卡版本、FPGA 型号、治具编号和工位编号在运行前冻结并写入每项结果；
- 修改后的限值可以用于下一次运行；运行中的限值不变，最终报告完整显示实际测量值、上下限、单位、来源和快照哈希；
- 设备断线、磁盘满、API 超时归类为 ERROR/INCONCLUSIVE，不记为产品 FAIL；
- 产品规则 FAIL 后发生执行异常，原 FAIL 仍被保留。

### AI

- AI 关闭时全部功能可运行；
- AI 超时、格式错误或不可用时不改变确定性 verdict；
- AI 不能把 FAIL/INCONCLUSIVE 提升为 PASS；
- AI 输入、脱敏结果、模型、模板和输出均可审计；
- 本地和公网 provider profile 均可配置；插件配置不保存明文凭据；
- 未授权时不向外部服务发送日志或产品信息。

## 19. 已确认决策与剩余不明确项

### 19.1 已确认并纳入设计

1. `TBT` 表示 `To Be Tested/待测试`，内部状态为 `pending`。
2. 波形截取的基本动作是截图，并提供源波形读取选项。
3. 波形和日志的统一计时原点是复位释放后的 `capture_t0`；每个配置可选择 DC 电源复位或 WiParse Dongle 复位。
4. 固件只考虑 HEX/BIN；当前版保留配置和程序接口，不实现下载。
5. 日志来源为普通 COM 和 WiParse Dongle UART。
6. 接受 ERROR、SKIPPED、ABORTED、INCONCLUSIVE 扩展状态。
7. AI 支持本地模型和公网模型；当前由插件提供配置接口，后续迁移到 WiParse 全局 AI 接口。
8. 需要用例子集选择、标签分组、失败即停、循环次数和老化测试。
9. 需要记录产品 SN、板卡版本、FPGA 型号、治具编号和工位编号。
10. 多波形任务可能重叠时由预检阻断，不排队、不并行。
11. 测试模式启动后插件接管 WiParse 串口，结束后交还串口终端。
12. DUT 身份以运行前用户输入为权威来源。
13. 公网 AI 只发送脱敏后的允许字段，保持 advisory only。
14. 当前版本限值可编辑并立即用于下一次运行；最终报告显示本次实际使用的全部限值，当前不要求电子签名。
15. 异常恢复和证据保留按第 19.3 节建议方案实施。
16. 不支持多 DUT 或多工位并发；一个 run 只测试一个 DUT，并在一个工位内顺序执行选中项。
17. 测试项通过插件内可视化编辑器创建和维护，JSON 只作为内部持久化/交换格式，普通用户不直接编辑源码。

### 19.2 实现前仍需确定

1. **判定规则清单**：首版必须支持的波形指标、协议解码、寄存器读取和日志规则仍需产品测试用例确认。

### 19.3 六个关键问题的已确认实施策略

#### A. 多波形任务重叠策略

一个配置可以在多个时点抓图，例如 5 秒、10 秒和 20 秒。截图通常很快，但读取源波形可能占用示波器数秒到数十秒。如果 5 秒开始的源波形到 10 秒仍未读完，10 秒任务就与它重叠。若直接排队，10 秒波形可能实际在 35 秒才读取，却仍被误认为“10 秒波形”；若并行访问同一台示波器，命令可能互相干扰。

实施规则为：同一示波器同一时间只允许一个源波形读取；预检根据每个任务的 `timeout_s` 检查可能重叠，发现重叠就阻断并指出配置项；不静默排队。截图与源波形作为同一捕获任务，先在目标时点截图，再读取源波形。每次捕获记录计划时刻、实际开始、完成时刻和偏差。未来只有在具体仪器驱动证明支持独立会话或仪器内部快照后，才开放并行或“先锁存、后传输”。

#### B. 日志端口所有权

COM 端口通常不能同时被两个程序打开。WiParse Dongle UART 即使能够由后台服务共享，也会出现谁负责清空缓冲、从哪一行开始、谁停止监控的问题。如果插件运行前串口终端已经占用端口，强行关闭它可能丢失用户日志；直接复用又可能把测试前的旧日志混入本次判定。

实施规则为：进入测试模式后，WiParse 内部把端口从串口终端移交给插件，双方不产生占用冲突。插件记录起始游标和交接时间，独占修改配置、清空逻辑窗口和发送数据的权限；终端只显示只读日志镜像。结束、停止或异常清理后，插件释放端口并交还终端。若端口被 WiParse 之外的程序占用，预检阻断且不强制关闭外部程序。

#### C. DUT 身份权威数据源

“权威数据源”指报告最终采用哪一组身份。本插件用于非流水线场景，产品 SN、板卡版本、FPGA 型号、治具编号和工位编号均以运行前用户输入为准。用户确认后记录 `identity_source=user_input`、操作者和输入时间，并冻结到本次 run；修改身份必须新建 run。配置声明的期望板卡/FPGA 与输入不匹配时仍在采集前阻断。

#### D. AI 外发策略

本地模型的数据不离开工位；公网模型需要把请求发到外部服务。“外发策略”决定可发送的字段、脱敏方式、大小限制、允许的服务商和审批条件。日志可能包含产品 SN、网络地址、密钥、客户数据或尚未公开的故障信息，原始波形和固件也可能属于设计资产。

实施规则为：公网 AI 默认只接收规则结果、已脱敏的短日志片段和数值波形摘要；禁止发送固件、完整原始波形、完整日志、产品 SN、密钥及凭据。每个公网 provider 使用白名单 profile、密钥库引用和明确的字段 allowlist。报告保存模型、模板版本、发送字段清单、请求内容哈希和响应，但不回存明文凭据。公网 AI 始终为 `advisory_only`；未获得外发许可时自动跳过，不能影响确定性 verdict。

#### E. 当前限值变更与报告

限值直接决定 PASS/FAIL，例如时钟允许 99.9–100.1 MHz。当前版本允许用户修改限值，保存后立即用于下一次预检和运行，不要求审批角色或电子签名。为防止结果无法复现，run 启动时必须冻结限值；运行途中修改文件不能改变本次判定。

最终报告必须显示每条规则的测量值、下限、上限、单位、结果、来源和限值快照哈希，并标明“工程可编辑限值”。历史报告继续保留原快照，不随配置文件更新。当前版本固定 `industrial_approved=false`；未来需要生产放行时再增加角色分离和电子签名。

#### F. 异常恢复及证据保留规则

异常恢复解决测试到一半进程崩溃、电脑掉电、设备断线后怎么继续。若直接从中间步骤续跑，DUT 状态和仪器状态可能已经未知，结果不可相信。证据保留解决 500 个用例和老化循环产生大量日志/波形后，哪些必须长期保存、何时压缩归档、谁能删除。

实施规则为：每个 attempt 使用原子状态文件和追加式事件日志。异常时当前 attempt 标为 ERROR/ABORTED，已完成项保持不变；恢复后从“重新占用资源 → 重新复位 DUT → 新建 attempt”开始，不从半个波形或半段日志继续。DC 电源在异常清理时进入工位定义的安全状态。当前版本默认不自动删除证据，全部内容随项目保留；用户只能显式归档或删除整个 run，并留下审计记录。磁盘空间不足应在预检阻断，运行中接近阈值则安全停止，不能通过删除当前证据继续测试。

## 20. 主要反对意见

1. **反对把 AI 作为 PASS 的必要或充分条件。** AI 输出不可天然复现，也可能受模型版本、提示词和上下文影响。它适合问题定位和建议复测，不适合首版工业放行。
2. **反对只使用 PASS/FAIL/TBT/TESTING 四态。** 该模型无法区分产品失败与工位、设备、存储、配置错误，会导致良率和责任分析失真。
3. **反对把 500 个配置直接画成普通 egui 行。** 必须虚拟化，否则轮询和重绘会拖慢 Testing Hub。
4. **反对以整套开始时间作为每项采集原点。** 顺序执行时后续项目无法满足相对时间要求。
5. **反对在配置中保存临时 device_id。** 重启或重新连接后 ID 会变化，应使用稳定物理身份解析。
6. **反对允许配置文件执行任意命令。** 这会把测试配置变成代码执行入口，破坏插件市场信任边界。
7. **反对只记录配置文件名和最终状态。** 必须冻结配置、固件和规则哈希，否则结果不可复现。
8. **反对把预留接口描述成可用的固件下载能力。** 当前版本必须明确阻断 `download=true`；只有 HEX/BIN 适配器、校验、失败恢复和工位验收完成后才能启用。
9. **反对只显示 PASS/FAIL 而不显示实际限值。** 当前允许工程人员直接修改限值，因此报告必须冻结并显示测量值、上下限、单位、来源和哈希，否则结果无法复现。
10. **反对让公网 AI 默认接收原始数据。** 公网调用必须由 provider profile、凭据引用、脱敏策略和外发许可共同门禁。
11. **反对把 JSON 源码编辑作为正常配置流程。** 500 个配置包含条件字段、数组、引用和单位，直接编辑容易产生结构错误，也无法提供及时校验、资源选择和时间轴冲突提示。

## 21. 推荐实施顺序

1. 固化配置 schema、UI schema、DUT 身份 schema、限值 schema、错误码和离线样例。
2. 实现宿主通用配置工作区、配置 CRUD/归档/导入导出 API、原子保存和 revision 冲突检测；先用模拟配置完成 UI 验收。
3. 实现通用 `wiparse.test-matrix/1` 协议及 Hub 虚拟化表格，包括选择、标签、扩展状态和循环进度。
4. 实现发现器、预检和离线模拟执行，再接入复位、普通 COM、WiParse Dongle UART、示波器截图和可选源波形。
5. 实现确定性规则、可编辑限值、运行时限值快照、证据目录和可追溯报告。
6. 实现插件级本地/公网 AI provider 接口，保持 advisory 且默认关闭；WiParse 全局 AI 上线后通过 adapter 迁移。
7. 固件下载继续保持接口预留和 `download=true` 阻断；另立开发与验收项目后再启用 HEX/BIN 下载。
8. 当前版本保持 `industrial_approved=false`；未来进入生产场景时，再实现限值审批、权限、电子签名和工位验收。

