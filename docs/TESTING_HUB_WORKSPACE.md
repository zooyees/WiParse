# Testing Hub 配置工作区：规格评审、实现及验证

日期：2026-10-18。对应 `testing-hub-extension-spec-2026-10-18.md` Draft 0.1。

本次实现通用宿主能力，不把 FPGA、固件下载、示波器采集或 AI 判定逻辑放入 Hub。本次扩展纳入 1.2.0 发行包，保留旧插件参数页面兼容。

## 1. 容量和显示规则

**500 是最大支持数量，不是默认数量或必须数量。** 空仓库显示空状态；5 个配置显示 5 个，5 个测试项显示 5 行；超过 500 拒绝导入或保存。筛选后的数量和仓库实际数量分别显示，不填充空行。

提供了独立的 `hub-workspace-demo` 通用模拟插件和 `test-tools/examples/hub-workspace-demo-five.config-bundle.json`，示例只有 5 项，不连接仪器，不用于产品合格判定。

## 2. 规格评审与落实决定

| 评审点 | 处理决定 |
|---|---|
| `configure` 同时是能力声明和校验入口 | 两者均校验；缺少导出、超时、异常输出均阻断保存和预检，不退回可编辑 JSON |
| Draft 2020-12 不宜自行实现残缺 validator | 使用 Ajv 2020 + ajv-formats；限定关键字和本地引用，禁止远程 schema；打包携带离线依赖 |
| 参数、配置和业务预检清单可能有不同哈希 | 宿主计算配置清单哈希，包含插件/入口/schema/UI、声明的本轮参数、配置 revision/hash 和所选 ID；插件通过 `ctx.testMatrix` 确认宿主清单。业务清单可自行另存，但不能代替宿主绑定 |
| 预检 revision 示例未规定独立序号算法 | 当前宿主使用清单 SHA256 作为预检 revision，插件返回相同值；选择集或参数变化使绑定失效 |
| 外部配置目录的兄弟 `.editor` 可能越过授权根目录 | 宿主元数据放到 `{data_root}/projects/{project}/.editor/{plugin_id}/{repository_hash}/`。每个仓库独立事务和预检记录，外部目录不必授权整个父目录 |
| Windows 大小写不敏感 | ID 按大小写折叠检测冲突；拒绝仅大小写不同的重命名，避免同一文件先覆盖再删除。导入替换保留已有文件 ID 的大小写 |
| schema 或 UI 更新影响活动运行 | schema fingerprint 包含清单、入口、schema 资源与 UI；变化后配置写操作整体拒绝，矩阵响应标记工作区只读 |
| UI Schema 存在两种已出现的声明方式 | 同时接受 `layout.groups` / 顶层 `groups`，widget 对象 `type` / `widget` / 字符串；未知 widget 使用 schema 基础控件 |
| 时间轴与资源摘要缺少字段映射标准 | 仅使用声明的 widget 路径；时间轴可声明 `time_field`、`label_field`、`unit`。未声明时间字段时显示顺序，不猜测 FPGA 或波形字段含义 |
| 500 项批量操作可能超过旧 15 秒 API 预算 | 配置工作在独立后台进程中执行，4 个并发 worker 上限，worker 60 秒/配置 API 65 秒预算；旧 API 预算不变，Stop 不等待配置 worker |
| 运行中途进程消失但矩阵仍显示 running | 保留原始快照并标记 `MATRIX.INTERRUPTED_RUN`；不伪造 PASS，也不由宿主改写插件矩阵 |

## 3. 实现结构和责任边界

| 模块 | 实现 |
|---|---|
| GUI 工作区 | `crates/wiparse-gui/src/test_tool/workspace.rs`：列表/表单/校验三栏，窄窗口两级导航，保存/放弃/取消、关闭保护、虚拟矩阵、结果导航 |
| 宿主后台 IPC | `test-tools/hub-workspace.mjs`：限定操作、输入/输出容量、脱敏和统一错误码 |
| 存储 | `test-tools/lib/hub/config-store.mjs`：草稿、保存、重命名、批量启用/标签、归档恢复、bundle 导入导出、revision/hash 冲突、审计及事务恢复 |
| 清单和路径 | `lib/hub/manifest.mjs`、`common.mjs`：路径变量、批准根目录、junction/symlink 检查、文件大小/数量上限、同目录原子替换 |
| Schema | `lib/hub/schema.mjs`：受控 Draft 2020-12、本地引用、默认值、UI 安全检查、writeOnly 脱敏 |
| 插件轻量校验 | `lib/hub/configure-worker.mjs`：独立进程调用 `configure(ctx)`，只提供候选配置，不提供设备连接入口；10 秒超时和有界输出 |
| 矩阵 | `lib/hub/matrix.mjs`：状态、revision/attempt、摘要和结果路径校验，坏快照保留最近有效值 |
| 生命周期绑定 | `lib/hub/run-binding.mjs`、`runner.mjs`：预检确认、运行快照、活动配置锁、同 data_root 单工位运行锁、run.json 记录 |
| API/CLI/MCP | GUI backend 后台派发；CLI `ui test-tool config …/matrix/preflight-state`；MCP 在现有 `wiparse_ui` 中增加相同操作 |
| 安装和离线交付 | Marketplace 安装检查声明的 schema/UI 资源；打包脚本复制 `test-tools/node_modules`，运行时不下载资源 |

配置和插件仍是当前用户权限下的本地文件。校验进程隔离提供超时、错误隔离与受限上下文，不等于操作系统级恶意代码沙箱。签名、安装信任和仪器资源租约沿用既有机制。

## 4. 操作流程

1. 启动开发构建，进入集成测试，选择“配置工作区示例”。
2. 进入配置页，展开“导入、导出与归档”，导入五项示例 bundle；默认冲突策略为跳过，也支持重命名和带 revision/hash 的替换。
3. 点击配置编辑。字段由 schema 生成；数组可以添加、删除、上下移动及拖动排序；仍被引用的 ID 阻止删除。修改后自动进行结构校验，保存前调用插件 configure 进行跨字段校验。
4. 用勾选框选择本轮配置。运行期间选择集固定，活动配置只读；其他配置可保存，影响下一次预检，不改变本轮快照。
5. 点击预检。成功后才能运行；修改配置、选择集、插件/schema 或本轮参数后重新预检。
6. 测试矩阵显示实际项目数、状态、阶段、进度、耗时、attempt、错误信息和结果链接。手动滚动取消跟随，勾选“跟随运行项”恢复。
7. 结果链接在现有测试报告/波形入口打开，经过项目路径 jail，不另列整个证据目录。

本轮参数中可“添加授权目录”。授权目录在本机配置 `apps.test_tool.approved_config_roots` 中保存。API 请求不能通过任意传入的 context 自行扩大根目录；GUI 从受信任的本机配置生成 worker context。独立 runner 可显式使用 `--approved-config-root <目录>`。

## 5. 插件接入要求

清单必须声明 `configure`，提供有效 `ui.mode=test_matrix`、`max_items`、`matrix_status_file` 和 `config_editor.schema/repository`。schema 和 UI 资源必须位于插件目录内。

```js
export async function configure(ctx) {
  return {ok:true, lifecycle:'configure', errors:[], warnings:[],
    normalized_value:ctx.overlay.config};
}

export async function preflight(ctx) {
  // 在此完成插件自己的资源/业务预检。
  return {ok:true, test_matrix:{
    manifest_sha256:ctx.testMatrix.manifest_sha256,
    revision:ctx.testMatrix.manifest_sha256,
    item_count:ctx.testMatrix.selected_item_ids.length
  }};
}
```

运行读取 `ctx.testMatrix.values` 或 `ctx.configSnapshotDir` 的冻结配置，按 `ctx.testMatrix.selected_item_ids` 执行；`ctx.matrixFile` 是受控矩阵文件位置。不能再次从可变仓库读取输入并视作本轮配置。运行锁覆盖所选配置；仓库其他项变化不改写本轮快照。

矩阵使用 `wiparse.test-matrix/1`：带 plugin_id/run_id/preflight_revision/updated_at/revision、正确汇总、实际 items；每行提供 id/name/tags/selected/status/phase/progress/elapsed_s/attempt/message/result_ref。同 run revision 不回退，同 attempt 的终态不回到 running；复测递增 attempt，新尝试不能指向旧尝试结果。

没有 configure 的旧插件仍走既有参数表单、HUD 和 preflight/run/stop。新插件的 station.json 可以是其自身业务 schema，不强套旧版 GUI station 结构。

### FPGA 插件接入验证

插件开发工位交付的 0.3.0 存在 configure 导出缺失、预检绑定和矩阵协议不匹配、缺少实际 runner 接入等问题。本次在插件职责范围内修正为 0.3.1，保持 Hub 不包含 FPGA 业务分支。通过真实 runner 的显式模拟模式验证五项 PASS，以及只选两项时两项 FAIL、三项 SKIPPED；报告链接包含 JSON、日志、截图、波形 CSV。硬件模式缺少合格 adapter 时明确阻断，不能据模拟结果宣布真实硬件已验收。详见 `FPGA_PLUGIN_INTEGRATION_REVIEW.md`。

### 路径与便携性

设置保留用户输入的相对路径，默认插件路径为 `test-tools/plugins`。CLI、Node 路径和数据根目录只在文件访问或子进程边界按应用根目录解析；系统 PATH 中的 `node` 等裸命令仍按 PATH 查找。配置文件的固件路径使用 `x-path-base=config_dir`；样例使用相对路径，报告使用相对链接。外部仪器、网络资源和用户主动指定的外部绝对路径保持原意，不强制改写。预检绑定属于当前位置和内容的运行状态，不随样例打包，移动目录后必须重新预检。

## 6. CLI 与 MCP

```text
WiParse-CLI ui test-tool config list --plugin hub-workspace-demo
WiParse-CLI ui test-tool config get --plugin hub-workspace-demo --config case_001
WiParse-CLI ui test-tool config create --plugin hub-workspace-demo --config new_case
WiParse-CLI ui test-tool config save --plugin hub-workspace-demo --config case_001 --file candidate.json --expected-revision 1 --expected-sha256 <sha256>
WiParse-CLI ui test-tool config import --plugin hub-workspace-demo --file test-tools/examples/hub-workspace-demo-five.config-bundle.json
WiParse-CLI ui test-tool matrix --plugin hub-workspace-demo --offset 0 --limit 100
WiParse-CLI ui test-tool preflight-state --plugin hub-workspace-demo
```

其他操作可使用 `--params '{...}'`，包括过滤/分页、批量 expected map、导出 ids、restore archive_id 等。保存时不得省略 revision/hash；冲突需要重新读取后合并，不提供强制覆盖。

MCP 使用 `wiparse_ui`，如 `op=test_tool.config.list`、`op=test_tool.matrix.get`。保存参数同 API；运行在 `test_matrix` 传入配置清单哈希、预检 revision 和所选 ID。响应有界，bundle 使用文件路径，不返回整个 bundle、日志或波形正文。

## 7. 数据一致性与恢复

保存事务顺序：写并同步 intent（旧/新内容、原审计长度、审计事件）→同目录临时文件写入并 fsync→原子替换→审计追加并 fsync→提交标记和 intent 清理。不会先删除目标配置再改名。审计失败时回滚内容与审计长度。

启动读写先恢复事务。审计与新文件均符合 journal 才确认提交，否则回滚旧内容。写锁包含 PID；失效锁由串行恢复门处理，避免两个进程同时接管。恢复中断或不确定锁状态保持拒绝写入，不猜测删除 journal。

若出现 `CONFIG.RECOVERY_REQUIRED`：先停止该数据目录的全部 runner，备份配置仓库及对应 `.editor`；检查日志/磁盘权限/审计路径。确认无活动恢复进程后，由维护人员处理遗留 recovery.lock，再启动恢复。不要手工删除 transactions 下的 intent 来绕过错误。

归档保留历史；恢复会重新校验并产生新 revision。导入先 staging 整批校验和限额检查，任何阻断错误都不提交部分配置。导出失败会撤销目标文件修改。已导出的 bundle 不代表配置发生变更，不使预检无故失效。

## 8. 验证范围

- Node：配置 CRUD/revision/hash、严格 schema、500 项导入分页和批量修改、0/5 项实际数量、故障回滚、恢复 journal、junction 越界、矩阵终态/rollback/坏文件保留、本地引用和条件校验、runner 无副作用 configure、预检/运行/快照/过期阻断、旧插件与会话回归。
- Rust：工作区全量回归及新工作区 0/5/500 项、中英文、明暗主题、640/1100/1920 宽布局、dirty 导航保护、虚拟列表、字段默认值和只读预览脱敏。
- GUI 验证采用实际 egui 绘制输出生成图片，不是独立设计稿。首帧字体准备与并行测试竞争会抬高首帧耗时；缓存帧性能单独记录，不把首帧数据混作持续刷新指标。
- FPGA 业务的模拟验收与发行运行检查记录在 `evidence/release-1.2.0-2026-10-18/`；未连接真实仪器，硬件和工业验收仍需独立完成。

### 多实例与 Windows 文件替换复验

runner/SDK 在未显式指定服务 URL 时继承当前 GUI 的 WIPARSE_API_BIND，避免测试误连其它实例；显式参数和 WIPARSE_URL 保持优先。FPGA 状态文件的原子替换针对 Windows 短暂占用增加有界重试，持续失败保留旧文件并报错，不退回先删除目标文件。上述问题由实际发行 GUI 联调检出，修正后重新运行五项模拟和附件跳转验证。

## 1.2.1 通用表单与协同验收

字段顺序、中文显示名、宽度、常用/高级分层、常量隐藏、引用选择、规则摘要及当前详情遵循 [通用声明契约](HUB_UI_FIELDS_CONTRACT.md)。不因显示页面补写可选默认值或修正非法常量。文件浏览和检查由受控后台进程计算实际SHA256，保存相对配置目录路径；改变文件或配置后必须重新预检。全局AI设置可从校验区进入，凭据不进入用例。

配置型插件统一使用工作区执行栏，原宿主头部不再重复运行/预检按钮。历史结果标注“上次”，配置变化显示待测试；预检和保存状态与历史产品结论分开。暂停仅在当前项归档清理后生效，等待暂停不是冻结采集时钟。

本轮真实20项配置、COM与Dongle UART、DC与Dongle复位及错误路径的验收证据以 `evidence/release-1.2.1-2026-10-18/` 最终摘要为准，阶段测试不视为最终发布通过。实机计时、合格适配器与目标最长老化另行验收。

### 启动与停止边界

短暂配置互斥仅在尚未进入操作时有界等待，恢复异常、过期绑定和业务错误不会重放。run等待锁期间及调用插件前检查Stop；插件在取得租约前后、复位前继续检查，不能在启动后删除新Stop。

预检是独立校验生命周期，可以写验证元数据，但不会复位或采集激励；残留Stop不阻断预检，也绝不被预检删除。run始终保守尊重现有Stop。常规界面及自动化的 ui.test_tool.run 由GUI在接受新启动、spawn前准备旧停止记录。直接Node CLI未经过受控新启动准备时，旧Stop也会取消运行；不要为绕过恢复状态自行删除租约或锁。无代际令牌时不宣称能区分预检期间的新旧Stop；取消该预检使用进程控制。
