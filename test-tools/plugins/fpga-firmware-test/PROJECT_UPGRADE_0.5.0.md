# 0.5.0 工程共享配置交接

2026年10月18日。宿主最低版本1.2.2，工程协议wiparse.project-config/1，逐项schema wiparse.fpga-test-case/2，结果wiparse.fpga-result/2。最终EXE、源帧、真实20项及发行动态脚本由软件工程统一验收；先提供EXE预览，用户确认后才ZIP。

## 保存位置与编辑入口

工程共享实例由宿主按data_root/project/test/plugin隔离，存于当前testDir/.hub/plugin_id/project-config.json；插件安装目录station.json不保存各工程可变设置。共享页维护工程名称/说明、DUT适用约束和仪器角色；单项保留name/id/enabled/order/tags/timeout_s的精简属性入口，以及固件/复位/日志/波形/规则/AI六组业务配置。单项没有dut/instruments覆盖层。

角色名称可扩展。工程里保存log_com/log_dongle/power等完整设备绑定，log_capture.resource_role仅引用其中一个日志角色；reset.power_role/dongle_role及测量规则同样引用工程角色。端口、UART通道、波特率、设备序列号与地址不在用例重复填写。类型不兼容、角色缺失、UART通道缺失和实际设备不唯一均拒绝预检；不会按名称猜设备或覆盖绑定。

## 旧20项迁移

recipes/fpga-firmware-test/cases保留原schema/1作为迁移源，独立原始测试夹具也保留在test/fixtures/legacy-demo-cases.json。原20个不同名称/ID不合并；DUT六字段相同，仪器有三组、七个原差异叶路径。映射预览将旧log明确提为log_com或log_dongle，共享power仅由需要的DC项引用；保留10COM、10Dongle及5DC/15Dongle复位组合。

planProjectMigration为纯函数，无设备或文件权限，不含时间/随机值。预览包括源差异、候选角色、逐项旧→新引用映射及未解决冲突；同一目标角色不同绑定、DUT差异、与已保存工程冲突、未解析引用均阻断。不是取第一项覆盖；不是把真实配置统一为演示值。

宿主要求用户明确commit，按全部旧用例byteshash/共享scope+revision+hash/schemahash进行CAS；同一事务写备份、全部升级项及共享实例，审计并可回滚/恢复。重复执行已经升级的配置返回0项迁移。测试仅在专用临时仓库显式commit；不表示用户已确认迁移实际数据。

## 运行来源及历史

插件只接受ctx.projectConfig完整冻结快照，核对canonical envelope hash及project/test/plugin来源；禁止运行回读可变共享文件。由共享DUT和本项引用编译有效配置，仅解析操作需要的角色。保存raw-case.snapshot.json、有效config.snapshot.json、project-config.snapshot.json及配置manifest。

matrix/result/suite携带project_config revision/sha256/schema_hash；原始用例字节hash用于与仓库版本关联，有效配置hash覆盖真实解析的角色与限值。报告列明共享版本、实际角色绑定及来源、本轮身份逐字段来源。显式身份参数不会被共享默认值覆盖；仅在明确simulation模式下填充缺省身份并标明simulation_default或project_config.simulation_constraint。

恢复content hash包含共享元数据。工程绑定、适用约束或共享版本变化将阻断旧预检/恢复，不把新值套入旧attempt。历史缺共享hash的结果只作历史，当前状态由宿主判断TBT/过期，不能冒充新配置PASS。

result/2强制scope={project_id,test_id,plugin_id}、run_id、item_id、numeric attempt、UUID attempt_id、config_sha256及project_config。matrix行从running起携带同一UUID；result_ref在该UUID目录。宿主应同时校验run/item/numeric/UUID/raw/sharedhash与test路径边界，迟返回与旧attempt不能串入当前项详情。

## 可靠性边界

保持启动后绝不删除Stop、停止前不复位采集、取消确认期间阻断新动作、未知硬件状态保留租约、已得FAIL不被归档错误提升、项间安全暂停/继续等既有回归。报告初始化前先写冻结恢复输入，避免报告头写失败导致无可恢复manifest。

同scope跨仓库互斥、共享活run保护、事务恢复、矩阵主从与草稿并发由宿主实现。插件专项包含纯迁移/解析、真实Hub显式迁移备份/幂等/冲突/CAS、替换失败回滚及rollback失败后journal恢复、共享变更运行/恢复拒绝、scope与结果来源；最终结果以新版本持久日志为准，旧0.4冻结不代表本轮交付。

没有真实硬件取消、精确复位时间不确定度或目标最长老化验收；模拟运输明确标识，不修改限值制造通过。固件下载仍为阻断的保留接口。

## 交接验证与冻结

完整插件回归160/160通过，包含112项故障矩阵与18项共享配置新增专项。持久日志与40份runtime/test/schema/UI/manifest源码哈希在evidence/plugin-regression-0.5.0-2026-10-18.log/.json。架构只读核对全部源码及日志哈希、160/112/18范围，并独立对原20项做纯内存预览/解析，确认逐项名称、超时、复位和完整日志绑定保真、源文件hash不变；未代替重新执行160项或最终EXE验收。

插件源码以PLUGIN_FREEZE.json冻结。host_final_acceptance仍为pending，最终20项正/负/子集、工程设置/矩阵/精简属性页面与发行EXE由软件工程统一绑定新证据；旧0.4发布记录不能替代本轮。冻结后插件/recipes/持久证据停止写入，后续改动须先解除冻结并重建相应证据。先EXE预览，用户确认前不ZIP。
