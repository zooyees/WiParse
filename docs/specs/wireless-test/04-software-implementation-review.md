# SW规格评审与1.1.28实施状态

2026-10-13 发布补记：WiParse / MCP 1.1.30 与 wireless-charge-validation 0.1.1 已增加报告数据入口、JSON/JSONL 内嵌查看、报告返回、可伸缩侧栏和随包模拟场景。验收依据、SYS/SW/PL/DG 职责及下述未完成项保持不变；本次 UI 与离线回归不代表 G2—G4 工业批准。版本验证见 ../../releases/1.1.30.md。

日期2026-10-11。评审输入：`00-system-contract.md`、`01-wiparse-software-spec.md`、当前源码。职责以00为准，01不能单独扩大为产品插件或Dongle固件任务。

## 评审结论

系统拆分合理，但**不能判为无问题、整份规格已完成**。S1仍是待G0冻结的目标规格，有下列实际实施和验收缺项：

1. Dongle capture二进制ICD/USB通道/时钟同步定义尚未交付；SW不能自行猜测。规范化JSON事件、时钟拟合与客户端先落地，二进制接入仍依赖DG。
2. 示波器型号、armed/raw读回、通道同步与deskew证据未冻结；现有GUI仪表能力不等于已验证确定性抓取。死区算法可离线验证，实物测量不得直接放行。
3. 工位保护、断能/残压反馈与批准安全profile属于外部必需交付；SW暂不开放刺激/危险输出，不提供假安全状态。
4. 产品状态图、适配器/RX矩阵、限值和持续规则属于PL；SW提供通用数据与判定，不把现有ChargeStateTracker默认值当作新产品验收限值。
5. SW自身尚需完成：旧GUI全部设备所有权迁移、SCPI会话串行与停止优先、增量协议解码、持久run manifest与run级权限、GUI运行健康页。它们不是仅靠缺硬件即可解释的缺项，本包只完成下表的基础交付。

因此按G0/G1前置基础阶段开发并打包，工业标记保持false。**SW-01至SW-12完整验收、G1联合贯通和G4工业批准均未通过**。此处明示范围不删减01与SYS总目标，也不改变PL/DG职责。没有创建产品专属插件、没有改固件、没有执行实物刺激。

## 需求追踪与实际交付

| 需求 | 本包代码/行为 | 尚待完成与验收 |
|---|---|---|
| SW-01 | 独立Rust进程、localhost认证、数据根独占、实验串口lease/fencing；GUI可代理调用 | 全部GUI/USB/VISA设备迁移、人工控制统一owner、进程监督、安全健康协调 |
| SW-02 | serialport资源/USB身份枚举；verified=false、未知能力拒绝 | 型号/固件/选件能力profile、role绑定、校准/量程、实物认证 |
| SW-03 | 持久job、幂等冲突拒绝、从受理时起计deadline/取消查询、重启unknown且不重放、活动采集拒绝双开 | SCPI迟到响应防护、已下发硬件命令反馈与unknown恢复；长算法中途抢占 |
| SW-04 | 原始COM采集、partial证据manifest、规范化JSONL校验/质量传播、64位时钟拟合 | DG二进制ICD、USB异步队列、水位/overflow健康、I2C/IO/RTT实时捕获 |
| SW-05 | 离线多通道输入、同时间基/原始同步/deskew证据门禁 | configure/armed/status/capture型号驱动迁移及原始多通道读回验证；盲区量化 |
| SW-06 | 共用core电压min/max/pp/mean，窗口、burst周期排除，频率/周期/占空比，双向最小deadtime及负交叠；GUI统计共用核心检查 | 完整PT轨迹、瞬态/稳定时间、全统计与极值证据、VGS特征、采样/校准不确定度模型与真实参考库 |
| SW-07 | 8种协议离线批量预算与GUI显示预算分开，frames落盘/校验，错误与截断明示，observed分母 | 无预算损失的增量跨块streaming、传统Qi ASK/FSK支持矩阵、归一化业务事件 |
| SW-08 | 标量限值评估、质量/null/单位/不确定度门禁、边界guard band、聚合保留FAIL、HTTP基础错误记录 | 持续/状态门控通用规则、签名/冻结限值文件校验、SYS/case/check关联、首异常与完整ErrorRecord |
| SW-09 | 源/结果hash、项目路径检查、Windows原子替换、job台账重启恢复、partial证据 | 完整RunContext/manifest/索引、run级权限、原始文件不可变快照、压力验证 |
| SW-10 | 未验证输出控制拒绝，过期session/fencing拒绝；不盲重试 | 批准profile驱动的停机/断能读回/隔离/恢复事务与工位联调 |
| SW-11 | GUI API代理与能力目录、关于页和操作文档 | 独立运行健康/告警/处理卡/证据定位页面与DPI实测 |
| SW-12 | GUI/CLI/Service一体版本化构建与包、单元/HTTP/旧Node回归 | 每型号transcript+实物认证、驱动扩展、吞吐/时限/磁盘/kill故障、测量系统与72h soak |

当前 `check.evaluate` 是单指标纯函数，PL必须在run中关联产品状态、limits批准与整体覆盖。JSON schema只覆盖已实现的measurement/limits/check/event/job/timeMapping；其余共同RunContext/DeviceBinding/CaptureRecord/ErrorRecord模型仍需联合冻结和实施，不能宣称G0完成。

## 代码检查与修复

- 分开GUI512字节预算与离线解码预算，动态限额线程局部生效并RAII还原，截断无法伪装完整；回归验证700字节正常解码且随后GUI预算仍有效。
- 缺数据/坏质量/空检查集合无PASS；不确定度区间越界和guard band明确区分；执行不完整仍保留已确认FAIL。
- burst间歇不计成载波周期；中位数偶数样本取中间两值平均；波形非有限/倒序/错单位拒绝。双桥臂有独立参考周期测试，负死区不截为零。
- 重启恢复queued/running（兼容旧cancelling）为unknown；deadline从受理时计，排队不能延长有效期；取消请求不引入契约外公开状态。重复命令不重复执行，旧session和过期lease拒绝。
- 事件seq/tick不经过JS Number；重复/缺口/逆向/boot/session/rate变化有质量标记。时钟只在同步覆盖范围换算，残差超限或换boot/session拒绝。
- 输入hash前后校验；规范化事件与协议帧单独保存hash；项目外路径拒绝。UART中止尽量保存partial状态和实际字节，不宣称硬件时间戳或线错计数已实现。
- Windows持久化改用ReplaceFileW保持原路径原子替换，temp先sync，旧副本保留；测试发现备份只读句柄sync_all权限错误，改可写句柄后恢复/幂等测试通过。
- 请求/参数、严格参数类型/未知参数拒绝、活动任务、波形单任务、样本/文件、台账、事件记录和输出采集预算有限；不宣称性能或取消时限已达工业目标。

## 验证证据与剩余风险

证据目录：`evidence/software-spec-2026-10-11/`。Rust回归为core236通过/1外部SCP样本测试忽略、GUI115通过、service6通过。HTTP smoke覆盖认证、能力拒绝、lease、64位时间、幂等、离线频率、质量/null、路径、持久化及正常停止。Node旧测试45项通过；GUI/CLI优化构建通过，服务最终构建及包内smoke以该目录最终日志为准。

外部SCP样本测试必须设置WIPARSE_SCP_CAPTURE后专门跑；默认回归的忽略项不算通过；此次额外在本机找到 `D:\software\WiParse\sample_waveforms\SCP\Normal.isf` 并单独运行该实测回归，1项通过（278帧、60条数据帧、无帧错误），文件SHA256为 `0C030F7D5AC283AEBA2AAB8678D6CFECF482060B2064F6D1246BF087ACD559AD`。硬件吞吐/P99、保护/USB断线、72h、热/位置/FOD、仪表raw/armed均未实测。

本版主机HTTP循环仍为同步接收，慢请求/文件落盘可能阻塞其他API；源文件hash校验不能替代不可变快照；COM命名与旧GUI设备owner未统一；lease没有renew接口，超时后采集终止；VISA/输出协调未接通。以上都是工业使用阻断项，开发包不得转为上市/产线PASS依据。

## 下一阶段先后关系

1. SW继续完成设备服务迁移、会话串行/优先停止、run manifest/权限与统一错误/GUI健康页；冻结schema并补相应模拟器/故障测试。
2. DG交付版本化ICD和模拟器后，SW做二进制接入、队列/时钟/同步健康；PL交付批准profile/limits与smoke后联合G1。
3. 已验证scope与工位反馈齐套后实现SW-05/SW-10并联合G2；之后扩展指标、协议和驱动，保留SYS全覆盖表推进G3/G4。

打包仅交付当前开发版本，不能将剩余任务标记为完成。完整运行操作和接口见 [AUTOMATION-SERVICE.md](../../AUTOMATION-SERVICE.md)。
