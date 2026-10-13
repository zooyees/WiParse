# 无线充电插件实施评审

2026-10-13 发布补记：WiParse / MCP 1.1.30 与 wireless-charge-validation 0.1.1 已增加报告数据入口、JSON/JSONL 内嵌查看、报告返回、可伸缩侧栏和随包模拟场景。验收依据、SYS/SW/PL/DG 职责及下述未完成项保持不变；本次 UI 与离线回归不代表 G2—G4 工业批准。版本验证见 ../../releases/1.1.30.md。

2026-10-11，插件 0.1.0 / WiParse 1.1.29。已阅读 00-system-contract.md 与 02-plugin-development-spec.md。保持 PL 产品编排、SW 通用分析和 DG 捕获的职责边界；未更改现有 Host 方法、公共事件/量测/判定 schema 或 Dongle 二进制 ICD。G0 仍待冻结，工业批准为 false。

## 已落地与缺项

| 规格 | 本版交付 | 尚待完成 |
|---|---|---|
| PL-01 | Hub manifest、preflight/run/stop、受限 overlay、项目目录和执行/判定分离 | 实物设备 role 与服务所有权一体化 |
| PL-02 | 独立配置版本/哈希、5 suite、历史故障与高风险强制项、全矩阵缺项报告 | 批准实际产品/对象、组合矩阵和试验次数执行 |
| PL-03 | TX/输入状态规则、物理功率冲突、EPT与电池可观测门禁 | 实时状态窗识别、完整RX/电池层联动 |
| PL-04 | 有界只读回放、单用例锁、attempt对应取消、未完成显式记录 | 服务lease续期、ready/armed反馈、真实刺激、安全停止/回滚 |
| PL-05 | ADP01–09契约模板 | 正式协议源、真实适配器/线缆/多口互操作 |
| PL-06 | VIN/VBRG/过冲/残压规则引用与模拟量测判定 | 采集请求/反馈/物理值关联、独立瞬态与周期证据 |
| PL-07 | PING/PT和四方向死区检查配置及质量/窗/通道门禁 | 同步原始VGS采集、deskew读回、真实最坏换向分析接入 |
| PL-08 | Qi/输入分层计数、参考分母、覆盖/超时/顺序门禁 | 协议消息级状态/合同/CE/RP/响应检验、原始信号交叉对照 |
| PL-09 | CHG01–09全范围模板保留 | 功率效率、SOC/温度/位置/FOD/保护/长稳/A-B实物验证 |
| PL-10 | 首个异常关联、处置卡、FAIL不被执行异常/取消覆盖、无盲重试 | 保护触发和设备状态可信反馈、隔离及人工恢复确认 |
| PL-11 | 结构化报告/配置快照/覆盖/哈希manifest与离线回放 | 真正原始波形/日志归档、冻结重判保留原判定、MES导出 |
| PL-12 | 正常/坏/缺测/坏质量模拟库与回归 | 物理金坏样、MSA/GRR、72h、试产误判/漏判与G4 |

模板中的通用 integrity 指标是契约模拟占位，不应计为对应实物功能已完成。全部 SYS 有任务归属，数据库覆盖全部用例仅表示离线规则覆盖。live 固定阻止运行，不能修改 capabilities 或模拟 approved 标记绕过。

## 共用插件基础更新

标准 normalizeResult 和 run.json 保留 execution_status、product_verdict、validation_verdict、工业批准、run/attempt 标识。旧插件字段仍兼容，ok 不被改为产品判定。runner 完成/失败退出改为设置 process.exitCode，让网络句柄和 stdout 完成清理，修复本机 Windows Node 24 在 preflight 拒绝后强制退出触发的 libuv 断言。其他六个本地插件保留原有功能和版本；没有将其配置升级为工业接口或擅改旧 Dongle 协议。

## 验收路径

单元回归包含配置/路径、空集合/未批准SKIP、状态/物理冲突、分母/覆盖、质量、缺值、取消和unknown后保留首次FAIL。HTTP联调使用真实 WiParse-Service 的 check.evaluate 与实际 runner，执行四库全矩阵、预检拒绝与停止，不连接任何设备。测试日志及完整样例在 `evidence/wireless-plugin-2026-10-11/`；发行包包含源代码和使用说明，不包含 service-token.json。

下一步由 SW 提供安全协调、采集武装、共享设备和实时任务；DG 提供 ICD/模拟器/设备时钟/溢出与健康；PL 接入这些规范化方法并扩充产品消息状态机/原始证据链；产品与测试负责人批准限值、保护顺序、仪表/探头与对象。只读回放完成不等于 PL-01至12全部验收完成。
