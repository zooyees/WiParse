# 任务规格书2：无线充电自动检验插件开发

日期：2026-10-11；版本S1；主责：测试自动化/插件开发；协作：产品硬件/固件、WiParse、Dongle、质量。依赖[共同契约](00-system-contract.md)、[软件规格](01-wiparse-software-spec.md)、[Dongle规格](03-dongle-development-spec.md)。目标覆盖上位方案R2所有测试领域，不能仅交付示波器抓图插件。

## 1. 产品和插件边界

插件ID建议 `wireless-charge-validation`；沿用Testing Hub `preflight/run/stop`、overlay、report_name、outputs与项目/用例/run目录。插件编排用例并维护产品规则，通过WiParse服务访问仪表/Dongle，不直接占USB/VISA、另起串口、点GUI或复制Rust解析器。

支持研发smoke、发布regression、真实interop、长稳soak、生产production档；以TX为默认，RX或电池链路按product_profile添加。scope没有raw、多通道/校准不全、产品限值未批准、缺硬件保护时，工业验收档阻止运行；开发档可以显式降级但不得出完整合格报告。

## 2. 功能工作包

| ID/级别 | 开发要求 | 验收 |
|---|---|---|
| PL-01/P0 | 合同兼容的manifest/生命周期/overlay、设备role、schema检查和预检 | preflight不分run、不刺激；stop不建run；run结果位于批准项目；ok不等于产品PASS |
| PL-02/P0 | product/limits/station/catalog/scenario/suite独立版本配置；smoke/回归矩阵与覆盖 | 批准限值不可被普通overlay悄改；高风险/历史故障强制纳入，不被pairwise丢弃 |
| PL-03/P0 | 输入供电、TX无线会话、可观测RX/电池三层状态图，时间/证据置信度 | 日志PT而物理无功率识别不一致；EPT按原因/场景；无电池证据不能宣称CC/CV合格 |
| PL-04/P0 | 有界编排状态机、armed确认、刺激与记录、取消/回滚、独占管理 | 每阶段有条件/deadline，安全停止与恢复；缺ready不刺激；不靠固定sleep代替状态 |
| PL-05/P0基础、P1完整 | ADP01–09输入协议/真实适配器/线缆/多口/回退兼容 | 合同+物理V/I+充电+恢复共同通过；source模拟正确，固定DC不能冒充协议互操作 |
| PL-06/P0 | VIN/VBRG请求/反馈/物理值、启动/纹波/调压/过冲/残压/循环行为 | 状态门控和独立极值；VIN/VBRG映射按拓扑；断电/反复启动能定位 |
| PL-07/P0基础、P1完整 | PING burst与PT频率/占空比、双桥臂双向VGS死区、门极/电流异常规则 | 不将idle计周期，不将扫频误判抖动；量测质量不足输出INCONCLUSIVE |
| PL-08/P0基础、P1完整 | Qi和输入协议分层质量、消息顺序/合同/CE/RP/响应和静默 | 分母/观测覆盖明确；没有参考不声称BER；日志/物理/独立分析仪交叉证据 |
| PL-09/P1 | CHG01–09功率/效率、手机SOC/热/位置/FOD/移除/重启/保护/长稳与A/B | 测量边界和负载明确；不同次波形不冒充同事件；FOD按正式夹具/规范依据 |
| PL-10/P0 | 统一故障处置、首个异常、证据选择、重试attempt、隔离/恢复 | 已确认FAIL不被重试/执行ERROR抹掉；首次异常与后果关联；自动恢复有边界 |
| PL-11/P0基础、P1完整 | 结构化结果、覆盖报告、manifest、Markdown、波形/日志/指标证据 | 必需未测项可见；每check可定位窗口/限值版本；可离线重判并保留原结果 |
| PL-12/P1/P2 | 金/坏样回放、重复性/再现性、生产短筛检与发布门禁 | 历史问题有用例，工业G4通过；生产节拍和误判/漏判经试产验证 |

完整兼容与长期/保护项是最终范围，不因P0只做一个金样工位而从总体目标删除。

## 3. 配置与目录

```text
wireless-charge-validation/
  plugin.json / index.mjs / station.json
  products/<id>/{profile,limits,states}.json
  catalogs/{adapters,receivers,cables}.json
  scenarios/{adapter,power,pwm,charge,communication,protection}/
  suites/{smoke,regression,interop,soak,production}.json
  lib/{orchestrator,rules,report,recovery}.mjs
  tests/fixtures/                # 明确正常、异常与缺失结果
```

profile记录TX/RX、功率/拓扑/线圈、VIN/VBRG、协议声明、遥测/事件映射；limits记录批准单位、状态/window、阈值/持续、guard band、来源/条款、版本/批准状态；station记录设备/探头/校准/同步/保护/停止顺序及能力；catalog记录真实对象/条件；scenario记录刺激/观察/期望/证据/恢复；suite引用case矩阵/重复/覆盖/停止策略。

每case必须有：ID、SYS需求映射、目标、适用条件、required capabilities、风险和保护、设备roles、初态、步骤/deadlines、capture计划、checks/limits refs、质量条件、cleanup/recovery、报告字段。产品/协议正式限值由产品负责人批准；无值/null不当作无限制。

参数表单以产品、suite、DUT序列号、固件、适配器/RX、工位、次数为主。工程高级选项不开放绕过lease/保护；覆盖限值必须形成新的评审版本。手机人工换位/SOC准备只在SAFE_READY暂停，状态超时可取消且健康/保护继续。

## 4. 流程与异常控制

preflight验证连接/身份/能力/权限、profile与limits、残压/输出、磁盘容量、DUT协议、时钟/采集完整性；不产生长测试loop或改变危险输出。run冻结配置和分配run/attempt，按共同执行状态推进。等待以单调时钟计算deadline；任务完成可查询，lost event不能无限wait。

适配器切换按批准break-before-make/放电/触点唯一确认；DUT重新启动先重新武装scope和capture。命令unknown不重复刺激，调用SW安全状态流程；保护异常立即停，不先导出波形。负载关闭/桥停止/输入关断顺序由拓扑批准，不使用全产品同一顺序。

错误按共同ErrorRecord和处置表分类；产品故障、工位故障、数据质量、算法未支持独立呈现。瞬时读取允许有界retry；功率/切换/故障注入需唯一command_id。每次复测为独立attempt，保留首次失败与统计。危险事件/重复工位错误隔离，人工复位后重新预检。

## 5. 检验与报告规则

调用SW共用分析，插件只定义状态窗和产品判据。频率分burst/稳态/控制调频段；deadtime双向逐次换向，保留最坏值；状态质量/异常屏蔽显式；绝对保护不屏蔽。效率使用同时间窗外部计量，手机RP不替代校准测量。

报告输出：summary.md、checks.json、coverage.json、errors.jsonl、events/metrics、configuration hashes、artifact manifest。目录按共同项目规范，原始波形和日志由SW服务落盘且manifest关联。case result须包含执行/产品双状态、首次失败、重试/盲区/未支持/质量；总体归并遵循共同规则。

问题识别卡显示现象、第一观测时间、前后状态、VIN/VBRG/协议/复位原因、证据、候选因果、建议复测步骤；不要将规则猜测写为已确认根因。A/B对比冻结工况、仪器配置与算法版本，不比较不同手机温度/SOC混杂样本。

## 6. 验收与交付

单元/回放：各SYS至少正常/故障/缺数据用例；case适用性、limits、单位、状态错位、seq断裂、命令unknown、取消每阶段、首因/结果归并。模拟器：SW job迟到/重复/失败与DG overflow/boot变化。实物：受控源/RX与真实适配器验证，故障注入及独立测量对照，长稳和统计样本。

交付全部插件源码/config/schema refs、使用说明、测试对象与矩阵、正式限值审批记录、schema兼容版本、回放库/预期、测试/故障/安全恢复SOP、结果样例与齐套版本表。可先做无刺激dry-run：验证矩阵/接口/目录/缺项，并明确dry-run非产品Pass。

完成定义：PL01–12逐项证据齐全、SYS无缺归属；所需SW/DG功能或外部设备缺失显式列为阻塞项；工业/生产档需通过共同G4，不以开发档“能跑完”代替。
