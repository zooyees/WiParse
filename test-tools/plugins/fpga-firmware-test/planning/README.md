# 工程共享配置：阶段 A 交接

2026年10月18日。依据 docs/HUB_PROJECT_CONFIG_CONTRACT.md；目标插件0.5.0、宿主最低1.2.2。此目录是未激活的协议草案，不改变当前manifest、旧20项数据或运行解析。上轮发布冻结已因新需求进入下一轮开发，不能直接复用其清单打包。

project.schema.proposal.json / project.ui.proposal.json 已通过现有宿主loadSchema/loadUi。重新生成草案与只读统计：`node test-tools/plugins/fpga-firmware-test/planning/build-project-proposal.mjs`；只写本planning目录。旧配置逐文件哈希、按项差异和值分组见 legacy-config-differences.json。

|范围|字段与处理|
|---|---|
|工程基本信息|name、description；不替代每项名称|
|工程DUT约束|model、serial_required、board_revision、fpga_model、fixture_id、station_id；不是实际身份输入|
|工程仪器|角色→kind/selector/required；COM/Dongle UART、探针、示波器、电源等精确身份常用，高级串口参数与过滤集中|
|逐项精简属性|name/id/enabled/order/tags/timeout_s；宿主确认ui.item_properties，移除基本信息页签|
|逐项业务|firmware/reset/timing/log_capture/waveform_captures/verdict；不保留逐项dut/instruments覆盖|
|本轮输入|实际dut_serial、board_revision、fpga_model、fixture_id、station_id、operator；不由共享默认值覆盖|

统计：20个独立名称和ID；DUT六字段只有1组；仪器3组、7个冲突叶路径。奇数10项为Dongle UART且无电源；02/06/10/14/18为COM3并有电源；04/08/12/16/20为COM3且无电源。探针、示波器、波特率一致。复位Dongle15项/DC5项；超时均120仍保留逐项。缺失电源字段也如实列作差异，不能按第一项覆盖或悄悄删掉5项需求。

单一旧log角色含两个互斥绑定，因此automatic_migration_allowed=false、shared_candidate=null。拟议显式log_com/log_dongle共享角色与逐项log_capture.resource_role引用，等待架构确认后再实现迁移；草案中仅预留两个角色结构，不表示已批准自动合并。此方式只选择已保存工程角色，不允许在用例另写绑定值。若架构不接受，必须明确处理冲突或拆分工程。

宿主确认：纯planProjectMigration输入raw value[]，projectConfig为null或已保存完整实例；返回ok/conflicts/project_value/items。宿主同一事务验证候选共享+新用例、保存备份、写入、审计和可恢复journal，计划hash绑定全部旧byteshash、共享scope/revision/hash及schemahash，活动运行拒绝迁移。已迁移项重复执行不产生新迁移；真实配置冲突不能用演示值统一。

冻结接口：ctx.projectConfig完整快照；manifest.project_config元数据、testMatrix.project_config_snapshot完整实例；禁止运行回读可变工程文件。需要记录原用例hash、共享hash、有效配置hash、本轮身份与实际角色来源，恢复内容hash包含共享，改共享使预检及当前历史结论过期。reference_selector保留widget.source_path，并使用widget.source_scope=project_config；宿主注入_project_config_root。矩阵元数据与报告采用同一共享来源。

当前独立纯解析模块lib/project-config.mjs尚未接入主入口，5项针对scope/hash、禁止覆盖、引用缺失、不可变解析和共享变化的单元验证通过。后续将以新共享协议替换运行来源，并保留全部Stop/取消/租约回归；由软件工程统一编译和EXE验收，先EXE预览，用户确认后才ZIP。

阶段B/C进展：上述文字保留为阶段A交接记录，角色映射已获架构批准，0.5主声明与运行来源已接入，真实Hub迁移/回滚/恢复专项及完整160项通过。当前实现及证据以../PROJECT_UPGRADE_0.5.0.md和新版本evidence摘要为准；此目录proposal JSON仍为历史草案，不是运行入口。
