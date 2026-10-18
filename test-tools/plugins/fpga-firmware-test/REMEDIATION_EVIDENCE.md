# FPGA 0.4.0 整改证据清单

2026年10月18日。对应原评估报告 F01–F16。此清单区分软件回归、静态改进、联合验收及未完成验证；不能据此推导硬件或长期老化已验收。

运行：在仓库根目录，将 TEMP/TMP 指向工作区 tmp/fpga-upgrade-temp，再执行 `node --test test-tools/plugins/fpga-firmware-test/test/*.test.mjs`。最新完整输出：`tmp/fpga-plugin-evidence-final.log`。真实 Hub/runner 五项成功与两项失败子集在 hub-integration.test.mjs 中执行；20项真实业务配置由软件工程统一重跑和绑定最终产物。

本轮最新软件回归：142/142通过，其中 fault-matrix.test.mjs 为112项；Node v24.15.0，总耗时约29.7秒。持久日志与摘要位于插件 evidence/plugin-regression-2026-10-18.log、.json，JSON记录日志及关键运行源码SHA-256、版本、命令与范围。此记录替代此前30/103/135项阶段计数，包含架构复核提出的直接reject、并发取消及发行联调启动Stop遗漏；不替代宿主最终EXE、压力和硬件验收。

|编号|当前证据|仍需验证或边界|
|---|---|---|
|F01|upgrade.test：非法组合/正则由 schema 和运行判定双重拒绝；rule-contract 为类型化分支来源|未枚举所有非法输入组合；不得以测试代替输入校验|
|F02|upgrade.test的capture回归；fault-matrix对复位/日志/源波形/清理逐项覆盖deadline、Stop、迟返回、unknown；两项run断言后项不启动、未知状态保留租约并拒绝恢复。补充复位/日志/截图/源波形/清理直接及迟reject，确认成功/unknown；同步throw、确认等待时阻断新动作、取消本身reject；真实日志/波形并发一支reject+另一支pending的联动取消，明确静止业务错误仍取消其在途兄弟分支。取消触发后迟返回不能改成成功|清理在Stop时仍执行有界安全确认；矩阵使用受控适配器、调度时钟和真实取消计时器；真实硬件取消未验收|
|F03|upgrade.test：截图后 Stop 不再调用源波形；plugin.test：项内停止不启动后项|真实仪器停止耗时待测|
|F04|upgrade.test：请求 UART1 不能误选同 Dongle UART0|真实 DAP/UART 物理身份关联待适配器验收|
|F05|upgrade.test：坏 matrix 在取租约前拒绝；取租约后文件失败 release 恰好一次。fault-matrix补充直接/并发reject后unknown时cleanup/release调用为0，确认静止后才清理和释放；cleanup reject仍保留已得FAIL及日志|未对所有文件操作逐点注入；未知设备状态保留租约并要求恢复|
|F06|upgrade.test：failed 复测使用真实历史文件、新 attempt 和不可变内容哈希；崩溃恢复不被旧完成记录掩盖|宿主历史选择到最终 EXE 的操作闭环由软件工程验收；旧0.3历史无内容哈希不强行兼容|
|F07|index 只保留最近1000条摘要，完整 attempt JSONL 索引增量落盘；日志按1024行批量写并同步；输入哈希缓存随文件stat变化失效|仅有500项发现测试，没有500项执行/目标最长老化/20万行日志RSS、吞吐、Stop和定时偏差证据。此项为部分整改，不能宣布关闭|
|F08|upgrade.test已有FAIL保留；fault-matrix对result、启动/完成journal、index、启动/终态matrix、matrix-final、项/最终report、suite、最终state分别注入ENOSPC/EPERM；journal另注入部分写入。断言suite不能PASS、独立状态未完成、恢复后新run通过且旧证据保留；完整损坏记录拒绝恢复|路径持续不可写时不能强写旧matrix；以独立matrix-final/run-state/suite标识未完成，恢复后preflight重建。仅允许忽略一个未终止的journal尾记录，完整损坏行继续阻断；不代表整盘掉电/文件系统损坏已验收|
|F09|20个文件已迁移为{id}.json并有迁移审计；ui-contract验证20项schema；Hub五项成功和失败子集回归；软件工程已反馈20PASS/62链接|最终20项证据和交付包由软件工程复核；模拟结果不代表IP硬件通过|
|F10|typed oneOf规则中文分支；ui.fields顺序/宽度/高级区；required_widgets；规则摘要和真正引用选择声明；ui-contract加载宿主schema/UI|九分组、第二份COM/DC配置、空白创建与切换规则的真实渲染/操作待宿主反馈|
|F11|RULES/METRICS共享目录；flattenRules递归采集依赖；upgrade.test嵌套测量获得输入；未安装custom_adapter不进入可选分支|尚未提供通用第三方规则注册插件API；新增指标仍需目录及适配器契约更新|
|F12|upgrade.test策略跳过不保留PASS和inputs交集；plugin.test公网阻断；fault-matrix对超时/transport抛错/无效响应、公开及本地空allowlist、无交集输入逐项断言，真实run中required异常降为INCONCLUSIVE、已知FAIL保持|本地显式allowlist现同样生效；未配置本地allowlist沿用固定安全字段。模板/脱敏配置版本校验仍待补充；AI仅为建议|
|F13|ui-contract：拒绝MHz/%非规范单位、错误DC角色；rule-contract单位目录；reset分别记录duration和未知uncertainty(null)|不做隐式单位换算；真实DC/Dongle时间误差和不确定度未测；能力/确认方式矩阵尚不完整|
|F14|assertInputs将实际冻结snapshot与testMatrix.values原输入哈希绑定；运行复核固件/配置；plugin.test配置变化阻断|Hub与独立discover仍是两条实现；隐藏/禁用/坏JSON/重复ID/大小限制完整一致性测试未补齐，不宣布已统一核心|
|F15|每秒heartbeat发布elapsed；报告含中文名称、身份、递归规则/限值、存在的证据链接；ui-contract验证嵌套子规则限值、转义和无日志不造链接|本轮修复了递归报告子项丢失；长采集实际刷新延迟与预计剩余时间未完成用户操作验收|
|F16|test-tools/package.json默认test及test:fpga包含插件目录；本轮完整插件回归输出独立保存|先前全量npm100/101，唯一junction EPERM属于沙箱；本清单不将其改写为全量通过。目标Node18和发行Node组合仍由发布方验证|

引用控件以配置根为源：`reset.power_role`、`reset.dongle_role`、递归规则 `resource_role` → instruments；规则 `capture` → waveform_captures。由宿主提供通用 reference_selector，允许手工输入并标记未解析，不静默修改已有值。角色kind、通道兼容及实际文件验证仍由插件语义校验/预检承担。固件文件声明hash_field，由宿主显式检查填写只读SHA-256；预检仍复核文件。全局AI入口声明ai_settings。

发行联调追加启动Stop竞态：旧run()在取得租约后无条件删除Stop，可能吞掉GUI spawn之后、锁等待/初始化期间写入的当前请求。已移除删除，acquire前检查，未取到租约的取消不调用release。新专项覆盖启动前、受控模拟外部短锁等待、acquire等待、首次reset前、acquire返回null同时Stop、残留Stop和外部显式新启动准备；断言零复位/采集、Stop保留、正常取消且无租约泄漏。原插件夹具仅模拟启动者阻塞，host_lock_wait_simulated=true保留。

真实宿主补充证据：仓库相对路径 evidence/release-1.2.1-2026-10-18/hub-lock-retry-tests.log，6/6通过。此为锁重试、停止、时限/不可重试边界、真实runner生命周期和endpoint处理六项总范围，不是六个独立Stop测试。真实互斥由父进程持有，观察BUSY_WAIT后同步释放；验证预检保留旧Stop、CLI run带旧Stop拒绝且不创建run、等待锁时新Stop拒绝进入插件、显式prepare后正常运行。日志SHA-256及runner/retry/config-store/hub-runner测试四份源码哈希已记录到startup_stop_followup.real_runner_acceptance（status=passed，path_base=repository_root）。架构已只读核对并确认此缺口在受控软件范围关闭。

架构已只读核对135/105日志及四份关键源码哈希，并独立复核直接/并发reject四种确认场景，本次新增F02/F05缺口在受控软件范围关闭。软件工程反馈112个源帧场景及规则创建/切换/嵌套指针交互通过；末轮确认标题、超时标签和隐藏空timing声明之后，UI契约3/3通过，证据摘要另记该后续检查及源码哈希。

宿主写入和Cargo已停止；本轮仅更新插件证据和文档。启动Stop补充的142项及真实宿主6项通过并获架构确认后，重新生成PLUGIN_FREEZE.json并停止写入。最后20项正反例、EXE与组合门禁交给软件工程。未完成项须由联合发布明确验收范围，不能用源码清单替代发布门禁。
