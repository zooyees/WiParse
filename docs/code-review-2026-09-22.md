# WiParse 项目代码审查报告

审查日期：2026-09-22

项目版本：1.1.26

基准提交：`1ed322cf757251b60db4b2e71ec234b76b7f6d68`

审查职责：只审查、验证和生成报告，不修改业务代码。

## 1. 总体结论

**系统具备合理的模块划分和性能优化基础，但存在影响数据完整性、长期稳定性及多设备响应速度的具体问题。建议优先解决串口数据丢失和持久化错误不可见的问题，再开展持续采集与多设备并发验收。**

| 维度 | 评估 | 主要依据 |
|---|---|---|
| 系统架构 | 基础分层合理，运行时隔离不足 | core / GUI / CLI 分离；但业务调度依赖 GUI，所有仪器共用一个工作线程 |
| 稳定性 | 存在需要整改的问题 | 串口溢出丢弃发生在落盘前；写盘错误被忽略；事件订阅无容量限制 |
| 流畅性 | 有优化基础，压力场景存在风险 | 已有后台加载、解码、LOD；GUI 消费队列缺少完整时间预算，慢设备会阻塞其他设备任务 |
| 自动化可靠性 | 存在语义缺口 | 请求超时后仍可能执行；全局正则导致触发结果错误 |

本次列出 **10 项问题：2 项 P1、8 项 P2**。P1 表示应优先处理的数据完整性问题；P2 表示有明确触发条件的稳定性、响应或恢复问题。优先级为本次审查判断，并非运行故障统计。

## 2. 范围与证据边界

重点追踪了 GUI 主循环、HTTP 请求桥接、串口采集到落盘及测试判定、仪器工作线程、事件订阅、配置保存、波形后台解码与共享数据结构、Node 触发器和更新脚本。另抽查了 SQLite、插件安装路径约束、更新包校验及 MCP HTTP 客户端。

这是以关键路径为中心的静态审查，不是对全部源文件逐行审计，也不是性能基准或完整安全审计。没有连接实际硬件、启动项目 GUI、执行更新或修改设备状态；未执行 Rust 全量构建和测试，也未验证当前 dist 二进制与源码的一致性。报告不声称已测得帧率、内存增长速率或硬件故障率。

工作区存在未跟踪的日志、项目数据和发布包。审查过程中还观察到 `bus_decode/digital.rs`、`bus_decode/mod.rs`、`bus_decode/uart.rs` 出现其他来源的修改；本次未修改这些文件，也不对其并发变更后的最终状态作背书。以下问题对应审查时读取的代码，后续修改可能使行号变化。

## 3. 架构现状

主要调用关系：CLI / MCP / Node 插件 → GUI 内嵌 HTTP API → 无状态处理函数或 GUI 请求队列 → 页面业务状态 → 串口 / 仪器工作线程 → UI、日志及证据包。

值得保留的设计包括：

- 核心协议、驱动、配置、数据库集中在 core，GUI 和 CLI 分开组织。
- 仪器主要操作异步投递，API 返回 accepted / job_id，避免在 GUI 中直接等待设备完成。
- 波形加载和总线解码已有后台任务；波形坐标使用 `Arc<[f64]>`，本次未发现解码调度处的 clone 会深复制整个采样数组。
- 实时显示有 5,000 行 FIFO，数据分析有 LOD 和缓存机制，SQLite 开启 WAL。
- 更新包有 SHA-256 校验，插件安装目录有 ID / 版本字符串约束。

维护性方面，`instrument_control.rs` 约 9,587 行、`waveform_analysis.rs` 约 5,032 行、`test_tool.rs` 约 4,383 行；仪器文件同时承载 UI、API、任务调度和执行逻辑。文件大本身不是运行故障，但会增加状态一致性验证和回归审查成本。建议按设备会话、命令调度、展示状态拆分，并把业务控制接口从页面对象中提取出来。

## 4. 问题明细

### R01 · P1：串口积压淘汰发生在持久化及测试消费之前

位置：[serial_tool.rs:1535](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:1535)、[serial_tool.rs:1571](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:1571)、[serial_tool.rs:1595](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:1595)、[serial_tool.rs:946](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:946)。

每帧最多取 400 行进入 batch，其余收到的行进入 pending；pending 超过 1,000 行就从头删除。随后仅对 batch 执行 `ingest_live_batch`、实时文件写入和 API 发布。证据包写入及 LiveBrief 更新同样位于 ingest 路径。

**触发条件：** GUI 暂停响应后积累大量行，或生产速率持续超过 GUI 消费速率。例如 pending 初始为空、单次收到 2,000 行时，400 行进入 batch，1,600 行进入 pending，其中 600 行会在后续消费前被删除。

**影响：** 删除的不只是显示历史，还包括实时日志、证据包和测试判定所需输入；短暂出现的触发报文可能永久漏掉。此处未记录丢弃计数或将测试标记为数据不完整。

**建议：** 将可靠采集、持久化和测试消费放到显示淘汰之前；GUI 只消费允许丢弃的展示副本。若采集层确实溢出，应显式报告数据缺口并影响测试有效性。

验证状态：静态路径确认；未进行真实串口压力复现。

### R02 · P1：日志与证据写入失败被忽略

位置：[serial_tool.rs:493](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:493)、[serial_tool.rs:528](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:528)、[serial_tool.rs:946](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:946)。

`append_to_live_file` 对创建失败直接返回，对 `writeln!` 返回值使用 `let _`；flush / close 也忽略错误。证据包的逐行、事件和测量写入结果同样被丢弃。

**触发条件：** 磁盘满、目标目录不可写、移动盘断开或其他持久化 I/O 失败。

**影响：** UI 可以继续显示数据，但相关错误没有在这些调用点传播到录制状态和测试判定，用户可能在缺少证据的情况下认为录制正常。

**建议：** 建立明确的 recording_error 状态；记录错误原因并通知 UI/API。证据不完整时应阻止将测试当作完整有效结果交付，同时保留可恢复的数据。

验证状态：静态确认；未实施磁盘满或拔盘测试。

### R03 · P2：串口和 API 队列清空缺少每帧总预算

位置：[serial_tool.rs:1521](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/serial_tool.rs:1521)、[backend/stateful.rs:10](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/stateful.rs:10)、[app.rs:961](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/app.rs:961)。

串口的 400 行预算只限制进入 batch 的数量，`while let Ok(ev) = rx.try_recv()` 仍持续读到队列为空，extras 也在读完后才限长。API 请求同样在 GUI update 中通过 while 循环清空，没有请求数或耗时上限。

**触发条件：** 大量积压，或消费期间生产者持续补入事件 / 请求。

**影响：** 单帧处理工作量无法由当前预算约束；窗口交互和绘制可能被延迟，临时 extras 也可能明显增大。不能仅凭当前代码给出卡顿时长。

**建议：** 对 GUI 队列消费设置总条数和时间预算，剩余工作下一帧继续；控制状态与高频数据显示分离。该修改必须配合 R01，不能通过进一步丢弃原始数据解决。

验证状态：静态确认。

### R04 · P2：慢事件订阅者和请求并发缺少资源上限

位置：[backend/events.rs:35](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/events.rs:35)、[backend/events.rs:40](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/events.rs:40)、[backend/mod.rs:207](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/mod.rs:207)、[backend/mod.rs:314](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/mod.rs:314)。

每个事件订阅使用 unbounded 通道，publish 给每个订阅者复制字符串；事件线程同步写 socket。invoke 和 events 请求各自新建线程，所审查的入口没有并发配额。

**触发条件：** 客户端保持连接但读取缓慢，事件持续产生；或客户端重复创建大量请求 / 订阅。

**影响：** 慢订阅者的队列可持续积压，增加内存占用；大量连接增加线程和其他资源消耗。localhost 部署也可能被有缺陷的自动化客户端触发。

**建议：** 限制订阅数、请求并发和单订阅缓冲容量；溢出时断开慢客户端或明确发送 gap 信息；为网络写入设置超时或取消机制。

验证状态：静态确认；未测量实际内存曲线。

### R05 · P2：所有仪器共享一个串行工作线程

位置：[instrument_control.rs:414](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/instrument_control.rs:414)、[instrument_control.rs:4417](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/instrument_control.rs:4417)、[instrument_control.rs:4470](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/instrument_control.rs:4470)、[instrument_control.rs:4585](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/instrument_control.rs:4585)。

构造时仅启动一个 worker_loop，持有所有设备会话；扫描、命令、测量、截图、波形读取及 Shutdown 都按同一队列顺序处理。

**触发条件：** 某台仪器响应慢、通信超时，或正在读取大波形 / 执行耗时命令。

**影响：** 后续发给其他设备的操作必须等待。例如示波器读取期间，电源输出关闭命令也会排队。该问题是设备响应隔离不足，不等于所有仪器 I/O 都直接阻塞 GUI。

**建议：** 每个设备独立串行执行器，保持同一会话内 SCPI 顺序；不同设备并行调度。对取消 / 停止设计明确优先级与可中断边界。

验证状态：静态确认；尚需多设备实测等待上限。

### R06 · P2：HTTP 请求超时后仍可能执行有副作用的操作

位置：[backend/dispatch.rs:80](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/dispatch.rs:80)、[backend/stateful.rs:12](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/stateful.rs:12)、[backend/mod.rs:56](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/mod.rs:56)。

stateful 请求入队后等待 15 秒。超时仅返回错误，PendingRequest 中没有截止时间或取消标记，GUI 之后仍可取出并执行；回复发送失败也不会撤销已产生的操作。

**触发条件：** GUI 长时间繁忙，调用端收到 timeout 后重试。

**影响：** 原请求可能在恢复后执行，重试可能导致重复发送或重复创建任务；调用方无法仅凭超时判断操作是否发生。

**建议：** 未开始请求执行前检查 deadline / cancellation；已开始的命令返回可查询任务 ID，并对需要重试的有副作用操作提供幂等键。

验证状态：静态确认；未阻塞正在运行的 GUI 做实验。

### R07 · P2：配置非原子保存，加载失败静默使用默认值

位置：[config.rs:657](D:/windlink/windlink/WiParse-R/crates/wiparse-core/src/config.rs:657)、[main.rs:98](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/main.rs:98)、[app.rs:276](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/app.rs:276)。

保存直接 `fs::write` 覆盖 config.json；启动 `load_config().unwrap_or_default()`，GUI 保存处忽略错误。

**触发条件：** 覆盖写入途中异常终止、磁盘写失败，或用户配置包含无效 JSON / 字段类型。

**影响：** 可能留下空文件或半写入配置；下次启动静默恢复默认，之后的保存可能覆盖原配置，增加定位和恢复难度。

**建议：** 临时文件写完并同步后原子替换，保留最近可用备份；加载失败明确提示并保留损坏文件；保存错误可见。

验证状态：静态确认。

### R08 · P2：带 g / y 标志的正则产生跨行状态，错误触发上升沿

位置：[line-triggers.mjs:50](D:/windlink/windlink/WiParse-R/test-tools/lib/line-triggers.mjs:50)。

compilePredicate 复用同一个 RegExp，直接调用 `re.test(line)`，没有重置 lastIndex。对 g / y 正则，前一次 test 会影响下一行匹配。

**实际复现：** 使用 `{id: "hit", pattern: "ASK", flags: "g"}` 创建默认上升沿分类器，连续输入四次 ASK，结果为 `["hit", null, "hit", null]`；按连续匹配只产生一次上升沿的语义，期望为 `["hit", null, null, null]`。

**影响：** 连续匹配行会被错误判断为条件下降再上升，重复触发动作；level 模式也可能漏判。

**建议：** 每次 test 前重置 lastIndex，或明确禁止 g / y 标志；包含条件和排除条件均需覆盖。

验证状态：已通过直接导入项目模块完成最小复现。现有相关测试 5/5 通过，未覆盖此条件。

### R09 · P2：更新替换失败时缺少自动回滚

位置：[apply-update.ps1:24](D:/windlink/windlink/WiParse-R/packaging/update/apply-update.ps1:24)、[update/mod.rs:187](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/update/mod.rs:187)。

GUI 嵌入并使用该更新脚本。脚本先将 WiParse.exe 改名为 .bak，再逐文件复制新版本；ErrorActionPreference 为 Stop，但没有失败时恢复旧版本的处理。其它文件也没有作为整体进行事务替换。

**触发条件：** 改名之后复制失败、文件被占用、磁盘满，或新版本启动失败。

**影响：** 安装目录可能缺少可启动主程序或混合新旧文件；虽然存在单个 exe 备份，但需要人工恢复，不能保证全部组件版本一致。

**建议：** 完整 staging 校验、版本目录切换或完整备份恢复；捕获复制与启动失败并回滚，保留更新日志。

验证状态：静态确认；没有实际执行更新。

### R10 · P2：事件断线重连不补发历史，任务完成通知可能丢失

位置：[backend/events.rs:40](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/events.rs:40)、[backend/mod.rs:284](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/backend/mod.rs:284)。

接口接收 since_seq，但 subscribe 明确忽略历史回放，只从实时流开始，hello 仅给出当前序号。事件流本身无法补回断线期间的消息。

**触发条件：** 依赖事件流获取异步完成通知的调用方短暂断线，并以旧序号重连。

**影响：** 断线期间的完成 / 错误事件无法通过该流恢复；若调用方没有独立状态查询与补偿策略，可能等待超时或重复下发操作。此结论不意味着所有现有客户端都依赖该恢复能力。

**建议：** 明确声明实时流不支持重放并返回 gap 标志；关键任务结果保留在可查询状态表，或提供有限事件回放缓冲。

验证状态：静态确认。

## 5. 已执行验证

| 验证 | 结果 |
|---|---|
| 关键路径源码与调用关系检查 | 完成，见上述问题位置 |
| `node --test test-tools/test/line-triggers.test.mjs` | 5 项通过，0 项失败 |
| 直接导入 createLineClassifier 的全局正则复现 | 确认出现重复上升沿 |
| Rust 编译 / 全量单元测试 | 未执行 |
| 实际串口 / VISA / USB 长时间压力测试 | 未执行 |
| GUI 帧时间、内存、磁盘故障实验 | 未执行 |

上述测试通过仅覆盖现有触发器用例，不能据此判断整个项目通过稳定性验收。

## 6. 建议整改顺序与验收标准

1. **先保证数据与证据完整性：R01、R02。** 使用可计数输入，比较接收、持久化、测试消费数量；允许展示淘汰，但任何采集缺口必须可见。模拟写盘失败，确认录制与测试状态报告错误。
2. **再保证响应边界：R03—R06。** 注入大量输入、慢订阅者、慢仪器和延迟 GUI；观察内存是否有界、GUI 是否继续响应、其他设备控制是否及时、过期请求是否被拒绝。具体延迟目标应由工位需求确定。
3. **完善恢复和自动化语义：R07—R10。** 验证配置半写、更新中断、全局正则和事件断线场景；确认能够恢复旧配置 / 版本、避免错误触发，并能查询任务最终状态。

建议在修复优先问题后，开展连续采集、多设备同时工作、大波形加载与交互叠加的长时间测试，记录帧时间分位数、内存峰值、丢弃数、任务排队耗时和写盘错误。当前静态证据足以支持整改，但不足以替代该验收。
