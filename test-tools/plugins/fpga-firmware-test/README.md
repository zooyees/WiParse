# FPGA固件测试 0.5.0

插件实现和模拟宿主测试；硬件运行需要 `wiparse.fpga-host/1` 适配器。未提供适配器时以 `HOST.ADAPTER_REQUIRED` 阻断，不能将模拟执行当作硬件验收。固件下载保留接口，`download=true` 始终阻断，无 ProbeFlash 调用。

0.5.0需要WiParse>=1.2.2。工程基本信息、DUT适用约束及仪器绑定统一保存在当前project/test/plugin共享实例中，station.json只保留插件工位策略。测试项使用schema/2，仅保存独立名称、ID、启停、顺序/标签、单项超时和固件/复位/采集/规则；日志通过resource_role选择共享COM或Dongle角色，不能重复填写或覆盖绑定。

旧schema/1不会自动执行或自动迁移。请从工程设置预览迁移差异、共享候选和逐项角色映射，明确提交后由宿主在同一事务备份并写入。DUT约束不同或同一角色存在不同绑定时阻断，不能取首项覆盖。原20项示例仍保留为迁移源；专用测试仓库已验证原10项COM、10项Dongle及5项DC复位的保真迁移。实际用户数据必须经用户提交。

运行只解析ctx.projectConfig冻结实例，记录原用例hash、共享revision/hash、有效配置hash及本轮实际身份来源。修改共享使预检失效，恢复不能套用新绑定。result/2同时携带scope、run/item、数字attempt、UUID attempt_id、raw config hash及共享元数据；矩阵行携带相同UUID，缺少来源的旧结果不能当作当前通过。

配置仓库建议 `{data_root}/recipes/fpga-firmware-test/cases`，固件和 limits 与 cases 同级。只扫描当前层 JSON，忽略隐藏、下划线和 disabled 文件，最多500项、每项1 MiB、合计32 MiB。所有文件经 realpath 限制在 `repositoryRoot`（默认 cases 父目录）。运行前须成功预检并保持运行参数一致；配置或固件内容变化立即中止后续项。

配置入口由宿主加载 `schemas/fpga-test-case.schema.json` 和 UI schema 提供；JSON 仅用于持久化、导入导出和只读预览。`generate-schemas.mjs` 是 schema/manifest 可重复生成源。跨字段校验导出 `validateCase`，返回控件路径和错误码。当前无内置 custom adapter，引用它会明确阻断。limit_ref 为相对配置文件的 `../limits/file.json#entry-id`。数值指标要求 quality=good 且单位精确匹配，缺失和坏质量保持 INCONCLUSIVE。所有结果固定未经工业审批。

运行参数支持 selected_ids（逗号分隔或宿主数组）、名称/标签过滤、循环、老化时长、失败即停。老化时长到达后停止启动新项，当前项仍完成。单次运行一个 DUT、一个工位；资源租约必须由宿主跨进程互斥。每项每轮独立 UUID attempt，证据永不覆盖、默认不删除。

## 硬件宿主适配契约（实机验收待完成）

宿主调用生命周期入口，注入 `ctx.args`、`ctx.layout.test_dir`（或 testDir）、`ctx.layout.run_dir`、可选 repositoryRoot、fpgaStation，以及 `ctx.fpgaAdapter`。

adapter.version = `wiparse.fpga-host/1`；capabilities 必须包含 host-adapter.mjs 的全部能力。方法均 async，失败必须抛异常，不能返回伪成功。

| 方法 | 输入/结果及宿主责任 |
|---|---|
| devices() | 设备数组，kind、device_id、稳定 serial/resource/model/port/dongle_serial；不得仅返回临时 ID |
| preflight({config,resources,identity,station}) | 检查设备可用、角色能力、工位忙、外部串口占用、timeout预算、指标支持、safe power、AI provider配置；失败抛具体错误码 |
| storage({minimum_free_bytes,estimate,items,loops}) | 检查可写及预计证据量，剩余空间不足阻断；运行各 I/O 同样检查空间 |
| acquire({station_id,run_id,resources}) | 原子跨进程工位/资源独占，串口从终端移交；返回不透明租约或 false；必须记录起始游标和交接事件 |
| release(lease) | 释放全部资源，恢复终端所有权；须支持异常恢复租约 |
| sourceOutput(resource,channel,enabled) | 映射 SourceOutput，确认设备 ACK |
| voltage(resource,channel) | 返回读回电压数值，设备不支持应预检阻断 |
| probeReset(resource,{hardware:true}) | 映射 ProbeReset，返回时确认复位释放 |
| collectLog({resource,config,start_mono_ms,end_mono_ms,dir,stopped}) | 从租约的原始字节游标采集严格时间窗口；分页≤2000并持续排空环形缓冲；返回 raw_base64、lines（单调时间戳/UTC/原字节映射）、start_cursor、end_cursor、dropped_lines、window_covered、complete、truncated。插件保存 raw/jsonl/log 并严格 UTF-8 解码 |
| capture({resource,config,dir,stopped}) | 映射 instrument.capture，真实保存截图到 dir/shots，返回 {screenshot:{path,sha256}} |
| waveformSource({resource,config,dir,stopped}) | 保存原始源波形到 dir/waves；返回 {artifact:{path,sha256},metrics:{CH1:{frequency_hz:{value,unit,quality}}}} |
| cleanup({config,resources,safe_power_state}) | 无论停止/异常均安全清理，DC电源进入工位安全状态；返回可审计结果 |
| ai({profile,payload,audit}) | 可选；宿主凭据库解析 credential_ref，白名单 provider transport；只接收安全 allowlist 字段。返回结构化 recommendation/confidence |

所有宿主硬件 I/O 必须实施配置 timeout_s 和单项剩余预算，超时须等不可中断 I/O 完成/确认停止后才返回错误，禁止通过 Promise.race 遗留后台硬件任务。日志与捕获使用同一单调时钟域；可注入 fpgaClock 用于模拟。后台进程崩溃的工位租约、串口恢复及正在运行 attempt 转 ERROR 由宿主恢复协调器负责，不能跳过重新占用和复位。

matrix 使用 `wiparse.test-matrix/1`，原子写入、递增 revision，最多500行，同时写旧 status.json。宿主负责虚拟化表格、选择状态持久化、分页、项目 jail。配置工作区 CRUD、原子保存/冲突检测、活动配置只读锁、审计和预检失效由宿主通用编辑器负责，本插件不修改宿主。schema/UI schema 是插件侧声明，尚须宿主确认控件协议后完成视觉验收。

恢复使用 resume_mode=failed/unfinished 和 resume_from（历史 suite-result.json 或run目录），也支持可信宿主注入 ctx.previousFpgaRun。以 content_sha256 冻结配置、固件、限值与身份，允许重新预检和选择新的执行计划；新attempt关联原attempt，旧证据保持原位。活动进程或未知硬件状态必须先完成安全恢复。

AI 默认关闭。公网默认无外发许可，跳过 provider；只发送规则数值和波形数值摘要，不发送日志、身份、固件或原始波形。advisory 结果独立归档，不提升确定性结论。required AI 错误只能使非 FAIL 降为 INCONCLUSIVE。当前插件不自行读取密钥或访问模型 endpoint。

## 验证

`node --test test-tools/plugins/fpga-firmware-test/test/*.test.mjs`

测试包含配置边界、下载阻断、源波形重叠、规则证据质量、AI外发门禁和模拟宿主循环归档。需要后续真实硬件验收：DC确认延迟、COM/Dongle连续采集及溢出、不可中断I/O停止、源波形时间容差、磁盘满和掉电恢复、500行UI性能。未宣称上述硬件或宿主UI已验收。

## WiParse 1.2.1 接入与显式模拟

已接入通用 Hub 的 configure、配置快照、预检绑定、选择集和小写状态矩阵。配置路径 `recipes/fpga-firmware-test/cases` 相对 data_root；案例固件路径相对 config_dir，便携包移动后重新预检即可。新版宿主以矩阵勾选集为准，旧 selected_ids 参数仅兼容旧调用。

默认 `execution_mode=hardware`，未提供满足上述契约的 fpgaAdapter 时返回 HOST.ADAPTER_REQUIRED，绝不自动转为模拟。检查效果时必须手动选择 `simulation`，可使用发行包内五项案例。simulation_scenario=fail 用于验证失败显示；模拟截图/CSV/日志和报告均明确标识模拟，simulation_only=true、industrial_approved=false。placeholder 固件不是真实 FPGA 固件。

自动回归：`node --test test-tools/plugins/fpga-firmware-test/test/*.test.mjs`。测试包含真实 Hub/runner 的五项模拟运行和两项失败选择集，不代表物理串口时序、探针复位、电源安全或波形量测准确度已经验收。

## 0.4.0 升级

本版增加精简卡片与分组表单、工作区执行控制、测试项之间的安全暂停/继续、规则worker超时、精确UART通道选择、恢复内容哈希与历史运行来源、增量报告和有界attempt摘要。详见 `docs/FPGA_PLUGIN_UPGRADE_0.4.0.md`。

硬件适配器新增 `cancelAndConfirm({operation_id,method,reason}) -> {quiescent:true}` 契约；没有确认时禁止继续设备动作并保留租约等待人工恢复。`guardedAdapter`传入的signal和deadline必须由真实transport执行；不能仅以“已收到取消请求”冒充设备已空闲。

配置与样例升级后需要重新预检。旧0.3.x运行未保存content_sha256，不能冒充可恢复的0.4.0运行，仍可查看原证据或建立新测试。

取消一旦触发，迟返回不能替代取消确认；设备状态未确认安全时保留租约，阻断后项及恢复运行。清理预算默认10秒，工位可通过正数 cleanup_timeout_ms 调整。Stop 期间仍执行有界清理，以确认安全状态。

启动Stop边界：插件run绝不删除停止文件，也不按mtime、随机run_id或错误码猜测新旧。外部启动者只有在确认无活动运行、接受本次新启动之前，才可显式准备并清理旧Stop；此后出现的Stop属于当前启动/执行过程。GUI在spawn前执行此准备；独立CLI若未明确准备，残留Stop将保守取消，不自动继续。插件在acquire前检查Stop，已存在则不取租约；等待acquire期间或首次reset前收到Stop，不复位/采集，已取得的租约仍正常释放。等待配置短锁的Stop-aware退出属于宿主runner职责，不能在插件初始化时清掉该请求。

传输 Promise 直接或迟延 reject、同步 throw 均默认视为设备状态未确认，须有界调用 cancelAndConfirm；仅可信适配器明确附带 quiescent=true 的业务错误可跳过本动作取消。同项任一采集失败会取消仍在途的其他采集，确认期间不接受新设备动作；任一确认失败则禁止清理动作及租约释放。取消确认必须代表设备已停止，不能仅代表取消消息已发送。

最终归档以 suite-result.json 和 run-state.json 共同确认，只有 archive_complete=true 且状态 completed 才表示流程完成。归档失败保留旧证据并进入 error/finalizing；unfinished 恢复会重新执行归档不完整运行的已选项，新attempt关联旧记录。磁盘持续不可写时，某个旧 matrix 快照可能仍显示 running，需以 run-state/matrix-final/suite 的错误或未完成状态识别，并在存储恢复后重新预检重建；不承诺在不可写文件上强制写成终态。
