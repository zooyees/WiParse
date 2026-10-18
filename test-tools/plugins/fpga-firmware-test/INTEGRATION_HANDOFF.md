# FPGA 插件 0.4.0 宿主交接与联合验收清单

2026年10月18日。宿主代码自本清单生成后停止由插件开发工位修改；保留已完成成果，由软件工程唯一接管。

## 构建状态

- 独立 release Cargo 会话62882已发送中断并返回exit_code=1；未产生可作为本轮交付的新版release EXE。target/release/wiparse-gui.exe的旧时间戳为2026-10-18 15:57:30，不可据此打包本轮改动。
- debug构建会话44522、GUI测试会话23103已结束，exit_code=0。debug程序只是阶段验证产物，不代表联合整改已验收。
- 独立编译程序控制验证会话10206已中断；日志为空，不能声称编译后GUI控制验收通过。该进程由验证脚本finally负责关闭。
- 不再启动任何Cargo，也不独立打包。后续唯一构建、联合验收与包冻结由软件工程承担。

## 已移交且不再写入的宿主文件

- crates/wiparse-gui/src/test_tool/workspace.rs：精简名称/状态卡片，分组导航，类型联合表单默认值与切换，文件名修复入口，中文路径/错误、统一工作区控制栏、模拟模式提示、源帧渲染测试。
- crates/wiparse-gui/src/backend/capabilities.rs、stateful.rs：ui.test_tool.run.control的能力声明与转发。
- test-tools/hub-workspace.mjs：run.control路由、matrix API控制状态、UI execution_controls元数据。
- test-tools/lib/hub/manifest.mjs、run-control.mjs、schema.mjs：项目jail内控制文件、活动run绑定/状态校验、oneOf默认值支持。
- test-tools/schemas/plugin.schema.json、test-tools/package.json：执行控制声明；插件回归进入默认测试。
- scripts/generate-fpga-hub-demo.mjs：原五项模拟截图预算从0.1秒改为1秒；不能把预算错误误认为生产超时器故障。
- scripts/verify-fpga-upgrade.mjs、verify-fpga-controls.ps1、upgrade-fpga-demo.mjs：已形成阶段验收/迁移脚本；软件工程可接收后重组统一验收。
- scripts/verify-hub-package.py：插件版本断言改读源码manifest；最终成功输出是否仍含旧写死版本仍须发布负责人核对。

## 现有通用UI契约

plugin.json.ui.execution_controls = {control_file:'{test_dir}/fpga-control.json',pause_mode:'between_items'}。

ui.test_tool.run.control输入plugin、run_id、action（pause/resume）；只接受当前活动run。控制文件状态running→pause_requested→paused→resume_requested→running。插件仅在项间安全边界暂停；Stop独立文件通道仍然有效。matrix API包装中附control，不把此包装字段写入严格matrix快照。

插件UI execution_badge声明运行模式提示，避免精简卡片后把模拟PASS当作实机结果。当前schema/UI仍有基础递归渲染降级，不能声称已经实现规则专用编辑器、通用高级设置或字段宽度契约。

## 已完成阶段验证

- 原插件与新增可靠性及Hub契约回归组合：29项通过；随后新增崩溃历史索引优先级用例所在专项文件15项全部通过（与前29项有14项重合），因此相关唯一用例30项已验证。
- 后续小变更UART选择器保留和控制文件写入前jail，2项定向回归通过。
- GUI工作区测试7项通过，包括640/1440/1920宽度、基本/仪器/规则/矩阵的源帧绘制；不是最终EXE页面验收。
- 真实20项业务配置经过独立快采样模拟Hub/runner流程：20 PASS，62个报告证据链接存在。实际固件仍是演示占位，不代表硬件IP功能被验证。
- 全量npm阶段运行100/101通过；唯一失败是当前沙箱不允许创建junction/symlink（EPERM），不能宣称全部全量测试通过。

## 夹具与证据

- 业务配置原始目录：recipes/fpga-firmware-test/cases，共20项，名称覆盖PLL/GPIO/UART/SPI/I2C/PWM/FIFO/DDR/AXI/DMA/Ethernet/CRC等。
- 文件名迁移审计：recipes/fpga-firmware-test/.filename-migration.jsonl。
- 独立模拟运输数据：test-tools/plugins/fpga-firmware-test/lib/demo-fixtures.mjs；固定日志和指标，不从用户限值反向生成PASS数值。
- 最新20项模拟验收：tmp/fpga-upgrade-acceptance-final/fpga-ui-workspace.json、tmp/fpga-upgrade-20-final.json。
- 29项组合日志：tmp/fpga-upgrade-targeted-final.log；15项可靠性日志：tmp/fpga-upgrade-regression-final.log；GUI日志：tmp/fpga-upgrade-gui-final.log。
- 源帧截图：docs/ui-fpga-plugin-0.4.0。仅对应具体已渲染状态，未覆盖全部表单和最终EXE组合。

## 待宿主完成的通用能力

1. 字段行与宽度、两列布局；common/advanced/hide与顺序元数据；不存在的可选项不再逐条显示加号。
2. 按oneOf标题选择分支，const隐藏，所有对象/可选按钮统一本地化，不能重复显示内部type常量。
3. 规则摘要列表与当前详情、数组项默认收起；证据/采集/通道引用选择器及被引用项删除提示。
4. SHA-256文件元数据计算/显示：jail、冲突、异步、文件变化失效必须由通用受控文件接口承担。
5. 全局AI配置跳转和安全profile选择；插件只引用profile与证据策略，不新增模型/密钥表单。
6. 历史PASS/当前配置有效性分离；消除原宿主头部与工作区控制栏的重复入口。
7. 明确editor capabilities/version，必需组件未实现不能默默回退后宣布体验验收通过。

## 插件后续范围与冻结

插件开发继续仅修改test-tools/plugins/fpga-firmware-test、recipes及业务测试/迁移说明；以generate-schemas.mjs为生成源，输出语义、单位、默认值、精确分支、字段顺序/层级和摘要需求。UI元数据只声明，不包含执行代码；新增声明须与宿主统一，旧宿主忽略它们不等于实现它们。

业务语义待架构确认：当前实际运行五身份字段仍按原规格要求输入，station_id还用于租约。报告提出“治具/工位可选环境记录”不能仅靠隐藏字段改变互斥/追溯语义。本轮先明确“配置约束”与“本轮输入”的来源，不静默放宽实际身份要求。

插件完成后将另写PLUGIN_FREEZE.json（全部插件/配置文件哈希、测试记录与兼容要求）。此前本清单不等同冻结，不应统一打包。

## 源码冻结交接（替代上述待办状态）

架构只读复核已确认135/135软件回归（含105项故障矩阵）的日志和四份关键源码哈希匹配，并独立核对直接/并发reject及确认成功/unknown四项。F02/F05本次新增遗漏在受控软件范围关闭。

软件工程已反馈112个源帧场景绘制及规则指针事件创建/切换/嵌套通过。最后三个UI声明问题已修正：释放确认两分支中文标题、确认超时中文标签、隐藏仅含固定常量的timing对象且保留原值参与校验。此后定向UI契约3/3通过，独立日志和生成源/生成schema/UI/test哈希记录在evidence摘要的ui_contract_followup，不将其虚增为138项唯一测试。

PLUGIN_FREEZE.json为插件及20项业务配置源码清单，排除自身，采用仓库相对路径、大小与SHA-256；包括generator、生成声明、运行代码、测试、配置和持久证据。执行 `node test-tools/plugins/fpga-firmware-test/freeze.mjs --verify` 可验证清单；任何增删改都应先与软件工程沟通解除冻结并重建证据。

冻结仅表示插件源码供联合验收使用。最后20项正反例、组合门禁、EXE和统一包由软件工程负责；最长老化及真实硬件边界仍按REMEDIATION_EVIDENCE.md保留。源码清单不确认交付包或硬件已经验收。

## 启动Stop竞态补充后的重新冻结

发行联调发现启动后Stop可能被插件初始化删除，旧冻结已解除并定点修复。插件run不再删除Stop；启动前请求不取租约，等待acquire/首次reset前请求不触发复位采集；已取得且确定静止的租约正常释放。旧Stop清理仅由外部启动者在接受新启动前显式准备；预检不删Stop，CLI未prepare则保守取消。

最新插件142/142（故障矩阵112），追加启动专项7例，连同F05定向8/8；运行源码、主日志、startup日志及UI跟进绑定均由架构核对。真实runner锁/Stop与相关helper总6/6通过，根日志 evidence/release-1.2.1-2026-10-18/hub-lock-retry-tests.log 及四份宿主源码哈希由插件证据JSON相对引用；6例是整体测试范围，不等于6例Stop。原插件模拟锁夹具标志仍保留。

架构已确认启动Stop缺口关闭（受控软件范围），此后仅更新证据、文档与冻结清单，未更改业务源码。重新冻结后的清单取代旧61文件版本，文件数以实际清单为准。交给软件工程继续发行EXE与统一包门禁；插件、recipes和持久证据重新停止写入。

## 0.5.0 新一轮工程共享整改（旧冻结已解除）

用户新要求按docs/reviews/fpga-project-matrix-optimization-2026-10-18.md实施。宿主最低1.2.2，插件0.5.0，工程协议wiparse.project-config/1、case/2、result/2。软件工程独占矩阵/草稿/共享存储与事务/runner/结果读取/编译；插件只修改业务目录、recipes说明及专项。交付改为先EXE预览，用户确认后才ZIP。

原20项源文件逐个hash与planning/legacy-config-differences.json相同，未自动迁移真实配置。已完成纯迁移规划及按共享角色解析，保真10COM/10Dongle/5DC/15Dongle复位；同角色不同绑定及DUT冲突阻断。真实Hub隔离库专项已验证显式提交、备份、CAS拒绝、幂等、替换失败回滚和rollback失败后journal恢复。共享变化阻断旧运行/恢复，真实身份不被覆盖，模拟填充逐字段标明来源。

新增声明schemas/project.schema.json/project.ui.json与主case/2、item_properties、project_config/result_details已生成；单项只保留六业务组。结果携带scope、run/item、数字attempt、matrix同UUID、raw/shared/effective hash，报告列实际来源。新主要代码为lib/project-config.mjs、project-migration.mjs、project-declarations.mjs及入口/loader/scheduler/report/simulation适配。

最新完整插件160/160通过（112故障矩阵、18新增共享专项），持久证据evidence/plugin-regression-0.5.0-2026-10-18.log及.json，源hash覆盖40份runtime/test/schema/UI/manifest。细节见PROJECT_UPGRADE_0.5.0.md。此为新轮软件证据，不能用旧142/62文件冻结替代；当前仍等待架构只读复核和软件工程最终20项/GUI/发行EXE反馈，尚未生成0.5冻结。

最新收口：架构已只读核对40份源码和160日志并独立复核原20项迁移/解析，受控软件范围未发现新增确定性阻断。证据architecture_review已更新通过及范围，host_final_acceptance保持pending。现重建0.5源码冻结并停止所有插件、recipes与持久证据写入，最终20项/页面/EXE由软件工程接管，不Cargo、不独立打包；用户确认EXE效果前不ZIP。
