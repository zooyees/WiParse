# 任务规格书3：WiParse-Dongle数据采集与事件捕获开发

日期：2026-10-11；版本S1；主责：Dongle硬件/固件；协作：WiParse、产品固件、测试/夹具。依赖[共同契约](00-system-contract.md)，配套[软件规格](01-wiparse-software-spec.md)与[插件规格](02-plugin-development-spec.md)。现有UART/I2C/DAP为基线；IO捕获与新增同步/性能目标须开发和实测验证。

## 1. 目标与边界

Dongle作为DUT实时数据入口：UART遥测、I2C受控快照、DAP/RTT开发诊断、IO硬件边沿事件，统一身份/时钟/质量，为WiParse与示波器共同时间轴提供证据。Dongle不替代示波器模拟VGS/死区/功率测量，也不承担唯一安全保护。

保持现有DAP MI_00、UART MI_01、I2C MI_03路径兼容；新增endpoint/interface/TCP端口在完整USB/ICD评审后分配，不能据局部接口表占用未知资源。USB主链路、WiFi可选，测试中不静默自动切换。

## 2. 功能工作包

| ID/级别 | 功能 | 验收 |
|---|---|---|
| DG-01/P0 | capabilities、序列号/硬件/固件版本、boot/session、接口与性能信息、版本协商 | 复合通道可合并同一实体；未知/不支持配置拒绝；新旧Host行为可预测 |
| DG-02/P0 | UART配置/透明兼容+采集块、顺序/tick/seq、framing/parity/overrun计数 | 跨块帧/二进制不破坏；CRC/长度/seq/质量完整；最高批准波特率连续与burst验证 |
| DG-03/P0基础、P1批量 | I2C master RPC兼容、事务ID/开始结束时间、busy/timeout/取消/白名单与快照 | 禁止测试模式无条件SCAN工作总线；read-to-clear提示；I2C阻塞不阻塞UART/IO |
| DG-04/P0 | 多路IO上升/下降/双边沿捕获、初始level、可配置过滤、公共timer硬件capture | 最窄合法脉冲/最密边沿/最大burst实测；先硬件锁tick，再异步处理；不把ISR时刻冒充边沿 |
| DG-05/P0时间、P1输出 | 公共64位时钟/回绕、clock.info、同步捕获、可选硬件marker/trigger输出 | 周期同步/误差/漂移可度量；无输出时capability明确false；长采集不串boot |
| DG-06/P0 | 有界buffer/DMA/stream、分通道调度、溢出计数/区间、传输完整性/健康 | 最大批准并发无静默丢失；超能力可见告警，队列水位/发送与捕获计数可核对 |
| DG-07/P0 | session/deadline/配置读回/状态/幂等、安全默认/断线行为/watchdog | 复位输出安全、捕获状态可查询；过期写入不执行；watchdog不被通信任务饿死 |
| DG-08/P0回归、P1完整 | DAP/RTT兼容、并发/模式约束、电气与同步校准、可靠性验证 | capture期间烧录/Halt等互斥；RTT负载可量化；USB/网络/回绕/压力/72h联合测试 |

所有IO映射、路数、电平、隔离、最大速率及精度是G0待冻结参数，产品需求和实测决定，不承诺未知硬件无法实现的能力。

## 3. ICD和数据包

ICD至少定义magic/帧长/类型/schema版本、dongle identity、boot/session、stream/seq、tick/计数单位、payload格式、quality、CRC规则（若采用）、字节序/对齐、最大包长、分片/重组/超时、错误回复和向前兼容。长度/CRC错需重同步且统计，不把损坏包继续解为正常事件。Host规范化字段按共同契约。

IO record：pin、edge、level、capture_tick、seq、quality；UART block：bytes、first/last接收tick、block seq、线路错误、截断标志；I2C transaction：addr/op、request_id、start/end tick、结果/长度、busy/timeout及是否实际开始。UART块tick不等于DUT帧生成时间，后者依靠DUT内嵌tick及同步。

控制：capabilities/clock.info、capture.configure（pins/edges/filter/buffer/transport）、start（新session）、status（真实状态/计数）、stop（停止边界/最后seq）、health、sync.mark（输出存在时）。命令包含id/session epoch/deadline，重发同id返回原结果，不重新产生marker/启动刺激；不同payload同id报冲突。

停止后发送终止记录和final counters，Host据最后seq核对尾部完整性；异常断线没有final不假定完整。设备tick用64位整数二进制，Host转JSON安全字符串。共同tick_hz与epoch记录，计数器扩展回绕逻辑在IRQ禁止/跨核/读写竞争下验证。

## 4. 实时采集与资源隔离

IO优先硬件capture+DMA，UART DMA/ring；I2C/DAP/网络低优先级或隔离任务。中断只搬运必要状态，不能做网络发送、格式化日志或等待锁。控制回复/健康与数据流分配有界资源，Host消费变慢不能使watchdog或保护反馈失效。

IO过滤记录配置与生效边界，既验证噪声拒绝又验证最短待检异常不会被过滤掉。如某pin只能软件ISR捕获，capability声明software_timestamp和实测误差，插件必须按精度约束选择。

带宽预算：Σ(event_rate×record_bytes)+UART有效字节+封装+其它流；容量≥最坏服务停顿×峰值速率×批准裕量，无法满足则缩短window/本地统计/限速并通过Host显式批准。例四路各200k edge/s、16B/事件约12.8MB/s，仅用于证明需预算，非设备指标。8N1 2Mbaud有效字节理论约200kB/s，实际需同时测USB/CPU/内存/网络。

overflow必须包含stream、first/last失效tick范围或范围未知标志、lost计数/unknown、cause、水位；不能精确定位时诚实标unknown。计数器自身不能溢出后归零伪装正常。预触发buffer可环形覆盖正常旧数据，但冻结失败/故障窗口要可见。

I2C master总线占用预算、最大事务/重试/恢复时间有界；当前不能保证多主机就拒绝工作总线接入。DAP烧录/Halt/Reset在正常性能capture模式禁止或需安全状态确认；RTT/内存读取非停机仍需测DUT总线影响。G0确认MCU/USB端点/内存资源是否支持所有目标同时运行，冲突模式在capabilities声明。

## 5. 同步与电气硬件

推荐DUT同步/FAULT/状态信号共享至Dongle与scope，记录路由/电平/阈值与线长。单点marker对齐起点，长期需周期marker/参考时钟。trigger_out若由ISR翻转须声明延迟分布；高精度尽量硬件timer路由。marker输出默认关闭且不会误接DUT危险控制脚；触发脉宽/极性/drive strength和负载限制读回。

输入只接兼容数字电平，声明绝对额定/阈值、保护/ESD/隔离与接地、上拉配置；线圈、VBRG、桥节点、高侧gate不可直连。测试接入不应反向供电DUT；DUT断电时夹具和IO行为需验证。watchdog/可选安全输出初态与断线状态经风险评审；独立保护机构另行交付。

## 6. 验收方法

脉冲发生器+独立示波器验证捕获误差/最短pulse/边沿间距/最大burst/过滤；start/end/周期同步测漂移残差。UART已知随机二进制/跨包/奇偶/线路错误/最高负载对照总字节与seq。I2C NACK/busy/超时/恢复及事务取消对照逻辑分析。DAP/RTT与IO/UART并发做影响评估。

故障注入USB拔插/WiFi切换/Host停读/队列溢出/Dongle复位/boot变化/回绕/重复过期命令/长I2C timeout/固件任务饿死；必须统计和时间断裂可见，不把旧数据当新。工作capture时阻止危险DAP操作。72h初始联合soak覆盖最大批准并发及故障注入，不用无故障空跑代替。

提交硬件原理/引脚/额定与路由说明、固件源码/构建/升级回滚、完整ICD与golden数据/模拟器、吞吐/精度/buffer/电气实测报告、模式兼容矩阵、Host接入样例、故障/校准SOP。DG开发需与SW contract test和PL实际capture流程联调。

完成定义：DG01–08验收数据可追溯，性能标称来自批准负载实测，未知精度能力不对外声称确定；公共schema和session/clock/quality端到端一致；无静默丢失、不能独立保护的限制明确。G4联合通过后才批准工业档。
