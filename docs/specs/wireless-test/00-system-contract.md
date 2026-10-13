# 无线充电自动测试系统：总目标与跨任务接口契约

日期：2026-10-11；规格版本：S1，待联合评审冻结。上位设计：[系统方案 R2](../../wireless-charging-automated-test-system-2026-10-10.md)。本组规格描述开发目标，拟议接口不代表当前软件/固件已实现。

## 1. 交付文件与职责

1. [WiParse 软件功能修正目标规格书](01-wiparse-software-spec.md)：通用设备服务、采集、时钟关联、分析、判定基础设施和 GUI。
2. [插件开发规格书](02-plugin-development-spec.md)：产品配置、测试矩阵、状态机/规则、刺激编排、验收判断与报告。
3. [WiParse-Dongle 功能开发规格书](03-dongle-development-spec.md)：硬件捕获、UART/I2C/DAP 接口、设备时钟、缓冲、健康和同步。

本文件为三份任务共同依赖。跨任务变更需同时修改契约、模拟器和兼容测试；不得由某个任务自行改变字段、结果语义或时序假设。整体流程：批准配置 → 安全预检 → 独占设备 → Dongle ready/示波器 armed → 刺激 → 采集与保护 → 安全停止 → 分析/判定 → 提交可追溯结果。

硬件保护独立于这三个软件任务，工位急停、限流、断能、触点/残压反馈和带电接线验证为外部必需交付；由硬件/工位负责人落实。DUT 遥测与事件 GPIO 由产品固件负责人落实。适配器/RX/夹具/探头/仪表由测试负责人落实。三份规格不能把这些依赖视为已完成。

## 2. 完整需求追踪

| 总需求 | 必须交付的系统行为 | WiParse任务 | 插件任务 | Dongle任务 | 外部依赖 |
|---|---|---|---|---|---|
| SYS-01 适配器兼容性 | 协商、电气、持续充电、回退/恢复及多口共享矩阵 | SW-02/03/07/09 | PL-02/05/09 | DG-02/04/05 | 协议源、真实适配器/线缆、互锁切换 |
| SYS-02 VIN/VBRG | 请求/反馈/物理值区分、稳态与瞬态控制判定 | SW-04/06/08 | PL-03/06 | DG-02/03/05 | 物理电压电流测量、DUT字段 |
| SYS-03 PWM/PING/死区 | burst频率、PT轨迹、双桥臂双向实际VGS死区 | SW-05/06 | PL-07 | DG-04/05 | 原始同步波形、差分探头、deskew |
| SYS-04 充电过程 | 分层状态、会话/周期、异常识别及恢复 | SW-04/08 | PL-03/06/08 | DG-02/03/04 | 可控RX、手机/电池状态可观测性 |
| SYS-05 通信质量 | 输入协议与Qi分别统计，分母、误码/超时/质量 | SW-07/08 | PL-08/09 | DG-02/04 | 原始信号/分析仪、协议依据 |
| SYS-06 异常及时捕获 | 独立保护、预触发、时间预算、盲区/丢失可见 | SW-01/03/04/05 | PL-04/10 | DG-04/05/06/07 | 独立保护、同步线、scope触发能力 |
| SYS-07 顺畅可靠执行 | 独占、完成确认、超时取消、有限恢复 | SW-01/03/10 | PL-01/04/10 | DG-01/06/07 | 输出读回和安全反馈 |
| SYS-08 快速报错处理 | 首个异常、关联证据、处置卡、隔离/恢复 | SW-08/10/11 | PL-10/11 | DG-06/07 | 操作员与风险评审 |
| SYS-09 开发验证/上市 | 发布回归、真实兼容、长稳、保护、热/FOD | SW-09/12 | PL-02/09/12 | DG-08 | 正式规范、温控/位置/FOD设备 |
| SYS-10 生产与追溯 | 测量系统、金/坏样、覆盖率、版本锁定 | SW-08/09/12 | PL-11/12 | DG-01/08 | 校准、质量批准和产线接口 |

需求为双向追踪：每项结果保存 SYS、SW/PL/DG 需求ID和具体case/check ID；测试缺项必须在覆盖报告显式列出。

## 3. 共同数据模型

所有 schema 有独立 `schema_version`。单位采用 SI（V/A/W/s/Hz/°C）；缺失值为 null 并附原因，禁止补零。原始字段值/单位保留以便排查换算。命名、枚举和 JSON Schema 在 WiParse 仓库 `test-tools/schemas/wireless-test/` 作为唯一契约源维护，固件提供对应二进制定义与生成器；这是拟开发目录。

| 模型 | 必需字段与语义 |
|---|---|
| RunContext | run_id、attempt_id、project/test_id、DUT硬件/固件/序列号、profile/limits/station/suite hashes、软件/解析器/固件版本 |
| DeviceBinding | role、物理serial/model/firmware、能力及verified状态、校准/探头、临时device_id；相同实体的DAP/UART/I2C归属相同dongle_serial |
| CommandContext | run/step/command_id、lease_token、session_epoch、deadline；命令与任务完成、实际反馈分开 |
| EventRecord | source、boot_id、capture_session_id、stream_id、seq、device_tick、tick_hz、host_monotonic_ns、UTC、event_type、payload、quality |
| TimeMapping | 两时钟标识、有效范围、offset/drift、同步方式、残差与不确定度、连续/断裂状态 |
| CaptureRecord | capture_id、触发事件、原始文件、channel map、采集ID、sample interval、模式/点数、pre/post窗、deskew、缺失与盲区 |
| MeasurementResult | metric_id、channel/window、value/unit、统计与极值、quality、uncertainty、算法版本、evidence_refs；不含产品专属硬编码限值 |
| CheckResult | SYS/case/check IDs、limits_ref/hash、measurement refs、状态窗/屏蔽窗、product_verdict、reason_code |
| ErrorRecord | error_id/code、source/severity、run/step/device/capture、时间、首个异常关联、观测/期望、自动动作、recovery_id、evidence |

三类序号明确区分：UART原始块序号、IO事件序号、DUT遥测帧序号。IO seq按stream单调递增，重复记录可去重但不可静默填补缺口。原始tick宜用十进制字符串或明确64位编码，防止 JS Number 超过2^53精度丢失；JSON输出不得把64位tick当浮点数。

结果维度：`execution_status=completed|error|cancelled|interrupted`；每项 `product_verdict=PASS|FAIL|INCONCLUSIVE|SKIP`；cancelled/interrupted时保留已确认FAIL。ERROR是执行错误，不冒充产品判定。总体只有必需项全部PASS才Pass；已确认FAIL优先保留；无FAIL而必需项不确定/缺测则INCONCLUSIVE。SKIP须适用性依据和批准，未完成不能伪装不适用。

## 4. 跨任务接口与责任

拟议Host接口族：`device.capabilities`、`device.lease.*`、`job.get/cancel`、`capture.*`、`dongle.capture.*`、`stream.subscribe`、`time.mapping.*`、`wave.analyze`、`protocol.decode`、`check.evaluate`。实施时允许评审调整命名，但三任务同步。旧instrument/serial/ui API兼容适配；插件不直接打开USB/VISA或竞争现有串口连接。

统一job状态：queued/running/succeeded/failed/cancelled/unknown。完成信息持久可查询，事件是通知，不能因丢事件永远等待。重复command_id返回同任务；同ID不同payload拒绝。fencing在设备服务执行前检查，超时command不能滞留队列后执行。已经送到硬件的命令超时需查反馈，无法确认则安全处理。

Dongle ICD控制接口族：capabilities、clock.info、capture.configure/start/status/stop、stream.read/subscribe、health、sync.mark（有输出时）、session控制；并保留现有UART/I2C/DAP兼容。Host→Dongle可行性和USB描述符由DG任务主责，Host适配由SW主责；PL只使用规范化接口。

generic波形算法、协议帧、阈值评估引擎归SW；具体产品状态图、预期消息序列、限值、故障解释与用例归PL；硬件边沿/数据采集与健康归DG。DUT传感器反馈不是外部校准物理测量。

## 5. 时间、质量与安全共同约束

各case批准时间预算：最短脉冲/间隔、burst/持续速率、capture精度、主机可见延迟、预后触发窗、保护上限、盲区允许值。保护上限按器件/系统风险确定，Windows不作硬实时保证。可用性初始目标为正常批准负载P99事件可见≤250ms、100ms健康检查/500ms失联告警并禁止新刺激；未实测前只是待批准目标。

安全保护先行；波形冻结可并行，导出/报告不得阻塞保护。单次触发只对齐起点，长期漂移需参考时钟或周期同步校正。无同步/overflow/截断/时钟断裂/探头未知等质量异常沿Event→Capture→Measurement→Check传播，不可在后续丢弃。

非必需数据损失允许以显式降级继续；必需数据受损禁止新刺激并按配置安全停止，已知FAIL保留。默认不自动切USB/WiFi、不Halt、不SCAN DUT工作总线、不清read-to-clear寄存器、不在未知输出状态重试。

## 6. 联合里程碑与交付门槛

| 门 | 交付 | 通过条件 |
|---|---|---|
| G0 契约冻结 | 三规格、JSON schemas、Dongle ICD、限值模板、风险/接线与能力清单 | 时钟/字段/枚举/错误/独占语义明确；外部依赖有负责人；未定安全限值阻止验收运行 |
| G1 离线贯通 | Host服务、Dongle模拟器、插件smoke、金/坏/缺失数据库 | 三端正常与故障数据一致；无空数据PASS；结果/目录可回放 |
| G2 实物最小闭环 | 1个产品、1台已验scope、Dongle、受控源/RX | VIN/VBRG、PING/PT、fault同步、停止与断线可靠；没有盲目重试 |
| G3 全功能验证 | 实际VGS死区、协议/真实适配器矩阵、热/位置/FOD、长稳 | SYS-01至10必需范围覆盖，缺设备项明确不通过完整验收 |
| G4 工业批准 | 故障注入、测量系统分析、压力/72h初始soak、冻结包与SOP | 保护/时限实测、缺失可见、恢复可控、必要项无未批准缺测，质量负责人批准 |

每任务交付源码/构建说明、自动测试与模拟器、版本化契约、验收证据、操作/故障SOP、兼容/迁移记录。正式齐套包列出Host版本、插件版本、Dongle固件/硬件版本及支持设备；单任务完成不等于系统工业验收完成。

## 7. 变更与待定项

待G0冻结：产品拓扑/限值、IO路数/电平/带宽/定时器、Dongle USB实际吞吐、触发输出与隔离、示波器型号/原始点数/触发能力、测试负载与存储预算、生产节拍、DUT遥测和I2C总线模式。负责人分别为产品、DG、SW、测试/质量。未定参数不得自动用示例值放行。

需求变更采用CR编号、影响SYS/SW/PL/DG清单、契约版本/兼容矩阵及必要回归。安全与接口冲突联合评审，禁止局部规格悄悄削减整体覆盖。正式协议条款引用沿用上位设计的官方来源，并按产品声明版本冻结。
