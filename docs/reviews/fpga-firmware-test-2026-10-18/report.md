# FPGA固件测试插件综合评估报告

评审日期：2026年10月18日。对象：当前工作区的 fpga-firmware-test 0.3.1、通用 Hub 对接、配置 schema、演示配置及测试。仓库 HEAD 为 ce25f584f81afa49a89f0a405e5b1a8674b374b4；插件和部分宿主文件仍是未提交内容，因此结论针对本次文件快照，而不是该提交本身。

当前插件具备配置驱动、确定性规则、独立证据归档和显式模拟的工程框架，适合继续做交互演示和受控开发验证。**不建议现状作为可靠硬件测试或长期老化工具交付**：规则存在可复现的无效PASS路径，停止、超时、租约清理和恢复流程尚未闭环。真实硬件适配器也没有通过当前runner提供可直接使用的注入路径。

现有12项测试在工作区临时目录下全部通过，但新增复现证实了测试未覆盖的缺陷。测试通过与产品就绪需要分别判断。

## 评估方法和适用范围

检查了插件全部lib、入口、schema生成源、schema/UI schema、manifest、12项测试，以及宿主runner、ConfigStore、run-binding、matrix和GUI workspace的相关代码。执行现有测试和独立最小复现，未访问真实示波器、DC电源、COM设备或Dongle，未操作或截图桌面UI。易用性中的交互结论基于实现结构；实际排版、点击效率和操作负担仍需用户操作验收。

仅新增本目录的报告与复现材料，不修复插件、不覆盖其他任务改动，也未重命名已有20项演示配置。

优先级定义：P1为影响判定正确性、安全停止、资源回收或核心恢复的发布阻断；P2为需要计划修复的体验、一致性、覆盖和扩展能力问题。它们是本次工程评审分级，不是外部认证等级。

## 三个维度的总体评价

|维度|当前成熟度|已有价值|主要短板|
|---|---|---|---|
|易用性|可演示，日常配置仍需工程知识|中文分组、只读源码、时间轴、仪器选择、通用表单、虚拟化列表和模拟证据|20项样例与Hub文件名契约不一致；规则不是按类型引导；恢复选项缺闭环；长任务进度与报告信息不足|
|拓展性|模块边界较清晰，能力注册机制不足|schema/UI与执行分离；资源、规则、固件、AI、采集各有模块；适配器版本明确|递归规则与采集计划不一致；schema/校验/执行/报告多处手工同步；宿主与插件重复发现；没有指标与单位目录|
|稳定性|短流程模拟可重复，硬件和长时运行未就绪|原子状态、配置/固件哈希、独立attempt、严格UTF-8、原始字节留存、证据路径/哈希校验、非PASS质量门禁|无效规则PASS；调用无运行时deadline；Stop边界遗漏；UART误选；租约异常泄漏；恢复与老化资源上界不足|

不以平均分掩盖判定缺陷：即使交互和模块划分达标，F01未修复也应阻止正式发布。

## 应继续保留的设计

1. 硬件模式默认不会悄悄回退到模拟；模拟的suite、截图、CSV和报告有明确标识。适合保留为独立演示/回归能力。
2. firmware.download=true被明确阻断；不因已有ProbeFlash就假定下载链路可靠。固件身份未验证、industrial_approved=false的表达是正确边界。
3. 规则与AI建议分离；AI不参与确定性PASS聚合。日志截断、UTF-8解码失败和数值坏质量不会按当前正常规则获得PASS。
4. 每个attempt单独归档，固件、配置、限值均保存哈希或快照；截图和源波形经路径jail与哈希验证。
5. 通用Hub已提供配置并发冲突、事务/审计、活动run锁、项目jail、矩阵协议校验和虚拟化表格的实现基础。此次发现的问题并不意味着这些宿主能力全部缺失。
6. 已有12项测试包含真实Hub/runner的五项模拟成功和子集失败场景，比单纯mock入口更有价值。需要继续扩大异常和边界覆盖。

## 问题清单

|编号|优先级|维度|问题|
|---|---|---|---|
|F01|P1|稳定性|规则结构不完整仍可签发 PASS|
|F02|P1|稳定性|执行超时主要依赖适配器自律|
|F03|P1|稳定性|截图完成后没有 Stop 检查就开始源波形读取|
|F04|P1|稳定性|UART 通道号被资源匹配逻辑忽略|
|F05|P1|稳定性|取得资源租约后存在未覆盖的异常清理路径|
|F06|P1|易用性与稳定性|恢复模式没有形成可用的宿主闭环|
|F07|P1|稳定性与拓展性|老化执行的内存与同步 I/O 没有稳定上界|
|F08|P1|稳定性|归档失败后当前行可能永久保留 running|
|F09|P2|易用性|20项演示配置与当前Hub文件名契约不兼容|
|F10|P2|易用性与拓展性|规则表单仍是通用字段集合|
|F11|P2|拓展性|支持的规则语法与执行能力尚未统一|
|F12|P2|易用性与稳定性|AI 的选择输入与 required 语义没有完整执行|
|F13|P2|拓展性与稳定性|指标单位和复位角色缺少统一能力校验|
|F14|P2|稳定性|配置工作区与插件重复维护发现和冻结规则|
|F15|P2|易用性|进度和报告不足以支持长时间运行与复盘|
|F16|P2|稳定性与拓展性|插件测试没有接入默认测试命令|

## 问题依据和改进要求

### F01 规则结构不完整仍可签发 PASS

**优先级：P1；维度：稳定性。**

代码依据：[rules.mjs:8](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/rules.mjs:8)；[generate-schemas.mjs:8](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/generate-schemas.mjs:8)。

验证：动态复现：空 all 子规则组与缺少 pattern 的 log_regex 均通过完整 JSON schema 和 validateCase，最终 product_verdict=PASS。后者对无关日志产生14次空匹配。

影响：用户从编辑器新增规则而未补齐条件，或导入语义不完整规则时，可能得到没有实际验证意义的 PASS。这是发布前必须解决的判定正确性问题。

改进：规则 schema 改为按 type 分支的 oneOf：log_contains 必填非空 text，正则规则必填有效且有业务意义的 pattern；all/any 至少一个子规则，not 恰好一个；数值/文件规则约束各自输入和引用。运行判定层再次拒绝非法结构。

验收：上述两例保存或预检必须失败；非法规则直接进入 evaluate 也不得返回 PASS。组合规则、空文本、缺失引用和非法匹配次数分别有回归测试。

### F02 执行超时主要依赖适配器自律

**优先级：P1；维度：稳定性。**

代码依据：[scheduler.mjs:53](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/scheduler.mjs:53)；[scheduler.mjs:62](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/scheduler.mjs:62)。

验证：动态复现：捕获 timeout_s=0.001，适配器不返回；40毫秒后 capture 仍 pending。静态检查发现单项 timeout_s 没有转成执行 deadline，也没有预算透传到所有硬件调用。

影响：接口卡住会无限阻塞当前项，Promise.allSettled 继续等全部任务；即便收到 Stop，也无法在悬挂调用之外推动安全清理。现有 README 的有界 I/O 要求是契约，尚非运行时保证。

改进：明确适配器每次调用的 deadline、取消信号、timeout/error 结果和不可中断 I/O 完成确认。调度器维护单项总预算，适配器提供有界取消和设备状态确认；不能单纯 Promise.race 后让后台硬件继续执行。

验收：对截图、源波形、复位、日志、清理分别注入永久不返回/迟返回故障；可预测地结束或进入显式恢复状态，且不启动下一项、不遗留后台设备访问。

### F03 截图完成后没有 Stop 检查就开始源波形读取

**优先级：P1；维度：稳定性。**

代码依据：[scheduler.mjs:55](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/scheduler.mjs:55)。

验证：动态复现：adapter.capture 返回前设置 stopped=true，后续 waveformSource 仍被调用1次。

影响：用户停止后又启动新的昂贵仪器 I/O，停止延迟可能增加数十秒，违背停止优先的调度边界约定。

改进：每个设备动作前后检查停止和 deadline；截图已完成且用户停止时保留截图，取消尚未开始的源波形读取。任一采集任务出错时通知同项其他可取消任务停止。

验收：在截图结束、源波形开始前注入 Stop，源波形调用计数必须为0；既有截图保留，当前项ABORTED，后项不启动。

### F04 UART 通道号被资源匹配逻辑忽略

**优先级：P1；维度：稳定性。**

代码依据：[resource-resolver.mjs:6](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/resource-resolver.mjs:6)。

验证：动态复现：配置 dongle_serial=D1、uart_channel=1，唯一候选为同一Dongle的通道0，解析器仍将通道0选中。

影响：可能把另一 UART 的日志作为 DUT 证据；若多通道都枚举出来，也可能形成无谓的 ambiguous 错误。只匹配物理Dongle还不足以确定逻辑通道。

改进：把物理身份和通道身份分开：dongle_serial 和 uart_channel 均必须精确匹配，baud等配置项通过配置通道应用；探针 serial 与 UART dongle_serial 应归一到共同 physical_identity。

验收：同Dongle的通道0/1分别匹配，缺少请求通道必须RESOURCE.MISSING；DAP/UART身份错配必须在预检阻断。

### F05 取得资源租约后存在未覆盖的异常清理路径

**优先级：P1；维度：稳定性。**

代码依据：[index.mjs:102](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:102)；[index.mjs:106](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:106)。

验证：动态复现：成功预检后将 matrix 写成损坏JSON，再运行；acquire调用1次，release调用0次。matrix读取位于取得租约之后、try/finally之前。

影响：异常退出可遗留端口、仪器或工位占用；硬件适配器一旦接入，这会直接影响下一次运行。停止文件删除等操作也处于同一保护空档。

改进：取得租约后立即进入最外层try/finally；提前读取/校验不依赖租约的输入；资源释放与归档失败分别记录，清理不应被其他异常跳过。

验收：对租约取得后的每个文件操作注入异常，release均恰好执行一次；释放失败有恢复记录，并阻止未知状态设备继续执行。

### F06 恢复模式没有形成可用的宿主闭环

**优先级：P1；维度：易用性与稳定性。**

代码依据：[index.mjs:100](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:100)；[index.mjs:101](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:101)；[runner.mjs:502](D:/windlink/windlink/WiParse-R/test-tools/runner.mjs:502)。

验证：动态复现：从显式fresh预检改为unfinished，原manifest触发RUN.PREFLIGHT_STALE；重新预检后，历史suite的manifest_sha256又与新manifest不同，触发RUN.RESUME_MANIFEST_REQUIRED。当前runner没有注入previousFpgaRun。

影响：UI虽然提供failed/unfinished选项，普通用户不能完成“选择历史run→新attempt→保留原FAIL”的恢复流程。即使自定义调用绕过参数冲突，套件PASS仍按原始完整选择集×loop_count计数，子集重试容易最终INCONCLUSIVE。

改进：分开不可变测试内容manifest与本次执行计划；resume/retry不改变原内容哈希，通过parent_run_id和previous_attempt_id关联。宿主提供历史run选择并加载可恢复journal；最终统计按本次应执行attempt集合计算。

验收：通过真实runner完成fresh→中断→unfinished、FAIL→failed复测；保留旧证据、重新占用与复位，报告明确父子run，子集全通过能按定义得到正确结论。

### F07 老化执行的内存与同步 I/O 没有稳定上界

**优先级：P1；维度：稳定性与拓展性。**

代码依据：[index.mjs:148](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:148)；[log-capture.mjs:11](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/log-capture.mjs:11)；[common.mjs:12](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/common.mjs:12)。

验证：静态确认：所有attempt完整result及log.text、lines常驻results，最终再次序列化整套suite；JSONL按每行打开、写入、fsync和关闭；每项执行前重复校验整个冻结集合。

影响：loop_count最高100000、最多500项时，内存随attempt和日志量持续增长；高吞吐日志逐行同步刷盘会阻塞JS事件循环，影响同项波形定时。500项发现成功不能证明500项或长时老化稳定。

改进：结果增量落盘，内存只保留计数和索引，报告按需读取；日志采用流式批量写入及周期持久化，原始字节与JSONL并行存档；固件按哈希去重缓存，定义复核频率与变更失效策略。

验收：开展500项及目标最长老化压力测试，给出RSS上限、吞吐、定时偏差和磁盘增长；覆盖20万行、长单行日志和大固件，确认Stop与刷新延迟受控。

### F08 归档失败后当前行可能永久保留 running

**优先级：P1；维度：稳定性。**

代码依据：[index.mjs:147](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:147)；[index.mjs:152](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:152)。

验证：静态确认：result.json写入或attempt-events追加异常会跳过row终态赋值，外层catch只写errors.jsonl；finally清除active_item_id并发布matrix，但不终结running行。最终matrix写入失败还可阻断后续suite报告。

影响：磁盘满、权限变化或证据文件损坏时，磁盘matrix和套件错误结论不一致，用户看到任务已结束而某行仍TESTING。结果存储越异常，越需要可靠的故障状态。

改进：为每项建立先写running再终态的journal，错误路径总是尝试发布ERROR/ABORTED；将设备清理、状态提交、报告生成隔离。无法落盘时保留清晰的外层错误并在恢复时重建。

验收：对result、journal、matrix、report分别注入ENOSPC/EPERM，验证不残留running终态，不覆盖既有FAIL，重启后可重建状态。

### F09 20项演示配置与当前Hub文件名契约不兼容

**优先级：P2；维度：易用性。**

代码依据：[config-store.mjs:58](D:/windlink/windlink/WiParse-R/test-tools/lib/hub/config-store.mjs:58)；[01-pll_clock.json:5](D:/windlink/windlink/WiParse-R/recipes/fpga-firmware-test/cases/01-pll_clock.json:5)。

验证：动态核对：20/20文件的basename均不等于配置id，例如01-pll_clock.json内id为fpga_01_pll_clock。插件discover接受，但Hub scan将其标记invalid。

影响：此前生成的20项只能通过插件侧发现校验，不能等同于当前Hub可编辑和可预检。直接选择该目录查看效果会出现invalid行。这是演示交付缺陷，修复成本低。

改进：通过通用config.import导入到新仓库，或在保留内容的前提下受控重命名为{id}.json；示例生成器也使用宿主API生成，增加完整Hub验收。

验收：Hub config.list返回20项且errors均为空；可逐项编辑、保存、勾选并成功进行模拟预检。

### F10 规则表单仍是通用字段集合

**优先级：P2；维度：易用性与拓展性。**

代码依据：[fpga-test-case.ui.json:108](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/schemas/fpga-test-case.ui.json:108)；[workspace.rs:2297](D:/windlink/windlink/WiParse-R/crates/wiparse-gui/src/test_tool/workspace.rs:2297)。

验证：静态确认：UI声明rule_builder，但宿主workspace.rs没有该widget的专门分支；规则schema用单一object列出所有字段，只有id/type/required必填。宿主支持oneOf，但规则没有利用。

影响：新增规则后需要自行添加text、pattern、capture、unit等可选字段；切换type后无明确字段约束和清理。新建、复制复杂IP规则对用户仍偏工程化，F01也更容易由正常操作触发。

改进：优先用oneOf按规则类型驱动现有通用表单；提供启动日志、频率、占空比等模板；为规则引用、指标、单位、上下限提供可选列表与即时预览。

验收：不查看JSON即可从空白创建有效日志/数值/组合规则；切换类型仅显示相关字段，必填缺失时明确定位。需做实际UI操作验收。

### F11 支持的规则语法与执行能力尚未统一

**优先级：P2；维度：拓展性。**

代码依据：[index.mjs:128](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:128)；[config-loader.mjs:43](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/config-loader.mjs:43)。

验证：静态确认：规则校验和evaluate可递归，但instrument_measure_range的采集只扫描顶层；放入all/any/not后不会触发measure。custom_adapter列在schema枚举中却被无条件阻断。规则引用没有统一的采集依赖图。

影响：用户把已有规则组织成组合后可能从可运行变成缺证据；新增协议、寄存器或测量方式需要同时修改schema、loader、采集器、判定器和报告，容易遗漏。

改进：建立受版本控制的规则注册表，提供schema、dependencies、capability_check、evaluate、report字段；递归编译规则为执行依赖，未安装能力不出现在可选项中或明确置灰。

验收：顶层与嵌套的同一测量规则获得相同输入和结论；注册新指标/适配器时有契约测试，不允许配置任意执行路径。

### F12 AI 的选择输入与 required 语义没有完整执行

**优先级：P2；维度：易用性与稳定性。**

代码依据：[ai-adapter.mjs:14](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/ai-adapter.mjs:14)；[ai-adapter.mjs:12](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/ai-adapter.mjs:12)；[index.mjs:136](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:136)。

验证：静态确认：发送字段取决于provider allow_fields，未与用例inputs取交集；仅处理required AI的error，公网未授权返回skipped并不触发降级。redaction_profile和prompt_template_id主要作为标识，未建立模板和脱敏策略版本校验。

影响：用户选择的AI输入范围与实际发送字段可能不一致；明确要求AI完成的用例，AI被跳过时仍可能保留PASS。当前不发送原始日志是优点，但策略不能只靠字段名称约定。

改进：取用例inputs、provider allowlist和固定安全字段三者交集；required=true时所有非completed结果明确INCONCLUSIVE（已有FAIL保持）；验证profile/模板版本并记录凭据引用、策略版本和跳过原因。

验收：分别覆盖AI关闭、未授权、超时、格式错误、空allowlist和inputs限制；验证AI始终不能提升确定性结论，且实际发送字段可追溯。

### F13 指标单位和复位角色缺少统一能力校验

**优先级：P2；维度：拓展性与稳定性。**

代码依据：[config-loader.mjs:42](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/config-loader.mjs:42)；[config-loader.mjs:14](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/config-loader.mjs:14)；[scheduler.mjs:39](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/scheduler.mjs:39)。

验证：静态确认：数值单位主要检查非空和引用相等，没有metric→量纲映射；复位只验证角色存在，未强制dc_power绑定dc_source；部分确认类型由宽泛schema共用。timing_uncertainty_ms把整个复位耗时当作不确定度。

影响：单位或角色错误可能到运行期才暴露；频率Hz/MHz、占空比ratio/%等新指标扩展容易产生解释歧义。复位耗时不能直接表征capture_t0精度。

改进：增加指标目录、规范单位及换算边界，校验角色kind和确认方式兼容性；分别记录命令耗时、读回周期、稳定窗口与t0不确定度，明确时间基准。

验收：非法单位、角色和确认方式在控件级阻断；Hz/MHz和ratio/%有确定转换或明确拒绝；真实DC/Dongle测量给出时间误差证据。

### F14 配置工作区与插件重复维护发现和冻结规则

**优先级：P2；维度：稳定性。**

代码依据：[config-loader.mjs:50](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/lib/config-loader.mjs:50)；[config-store.mjs:44](D:/windlink/windlink/WiParse-R/test-tools/lib/hub/config-store.mjs:44)；[index.mjs:25](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:25)。

验证：静态确认：Hub和插件分别扫描仓库；隐藏/disabled文件、文件名/id、schema校验范围和选择集处理不一致。Hub快照模式比较snapshot与ctx.testMatrix.values，但没有直接比较冻结plugin config与同一来源值。

影响：同目录可能在两层显示不同的项或不同错误；反复扫描提高I/O。Hub在预检完成时会复核manifest，已降低一般配置变更竞态，因此不能把所有重复扫描直接认定为已发生错用配置。

改进：Hub提供唯一版本化冻结配置输入；插件仅编译语义、固件和限值，保存原输入哈希与有效配置哈希。独立运行也复用相同发现/验证核心，避免两套约定。

验收：隐藏、禁用、坏JSON、重复ID、大小限制、筛选和外部变化在两条入口一致；快照明确绑定到实际执行内容。

### F15 进度和报告不足以支持长时间运行与复盘

**优先级：P2；维度：易用性。**

代码依据：[index.mjs:126](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:126)；[index.mjs:157](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/index.mjs:157)。

验证：静态确认：进度在启动/采集/结束节点跳变，采集中不持续更新elapsed_s；套件报告仅显示item_id而非中文name，未直接展示冻结身份和全部子规则，日志关闭仍生成日志链接；Markdown没有统一转义。

影响：长时间源波形读取时缺少等待原因和预计剩余时间；复杂组合规则、实际限值和DUT追溯需要进一步打开JSON。报告易被规则id中的竖线/换行破坏表格。

改进：增加独立低频heartbeat和当前动作、计划/实际时点、已用/剩余时间；报告统一模板展示中文名称、身份、复位事件、完整递归规则和限值来源/哈希，仅输出存在证据链接并转义文本。

验收：长采集期间每秒级可见进度；无日志/失败/组合规则/特殊字符报告均可读，所有链接存在，历史attempt可跳转。

### F16 插件测试没有接入默认测试命令

**优先级：P2；维度：稳定性与拓展性。**

代码依据：[package.json:16](D:/windlink/windlink/WiParse-R/test-tools/package.json:16)；[plugin.test.mjs:54](D:/windlink/windlink/WiParse-R/test-tools/plugins/fpga-firmware-test/test/plugin.test.mjs:54)。

验证：静态确认：npm test仅匹配test/*.test.mjs与lib/*.test.mjs，不包含plugins/fpga-firmware-test/test/*.test.mjs。本次12项单独运行通过，但现有测试不覆盖F01至F08等关键路径。

影响：日常运行默认测试可能遗漏全部插件专用回归；500项测试只覆盖发现顺序，不能证明500行UI性能或长时执行可靠。

改进：把插件测试显式加入项目CI和发布门禁；用例按规则、时间、资源、存储、恢复和UI契约分层。TEMP位置可配置，Windows临时目录限制和真实产品失败分开呈现。

验收：默认CI确实执行插件测试，且故障注入和本报告复现转为断言测试；目标Node18及发行版Node环境均完成验证。

## 硬件集成和产品边界

当前runner构造ctx时没有fpgaAdapter或previousFpgaRun，硬件模式因此会在标准入口返回HOST.ADAPTER_REQUIRED。适配器接口定义本身是合理的，但它不等于已实现了DC电源、普通COM、Dongle UART、截图、源波形和恢复的硬件执行。需要宿主任务提供可信设备适配及错误分类，插件再做端到端验收；不应通过跳过能力检查来获得演示成功。

模拟transport固定输出BOOT_OK/BOOT_ERROR、100kHz等数据，并不是通用IP仿真器。20项IP样例的成功日志标记各不相同，部分要求100MHz或20kHz，且声明了固定DUT身份；即便修正文件名，也不能保证直接选择simulation得到20项PASS。用于“查看效果”应准备与模板语义匹配的独立模拟数据集，明确展示PASS、FAIL、ERROR、INCONCLUSIVE和停止状态。硬件配置与模拟配置应保持可识别的边界。

本版不实现固件下载是已确认范围，并非本报告要求补齐的缺陷；多DUT并行也不属于目标，不建议为提高扩展性而引入并行调度。

## 测试结果和证据

环境为Windows PowerShell与Node v24.15.0。首次直接运行插件全部测试，结果9通过、3失败；3个失败均在沙箱TEMP内rename时发生EPERM，两个发生于Hub写锁、一个发生于atomic测试初次写入。将TEMP和TMP仅对当前测试进程改为工作区tmp/fpga-review-temp后，同一套12项测试全部通过。不能把首次环境失败直接认定为插件算法回归，也不能以重跑通过替代Windows共享访问和异常存储测试。

复跑命令为 node --test test-tools/plugins/fpga-firmware-test/test/*.test.mjs。工作区临时目录下总耗时约16.7秒；500项发现测试约4.56秒。它只说明该机器、该文件系统和小固件fixture的这一次运行，不是性能基准或UI流畅度结论。

|验证范围|本次结果|结论边界|
|---|---|---|
|插件现有单元与集成测试|12/12通过|短流程、mock与显式模拟覆盖|
|空all规则|schema和跨字段检查均无错误，结论PASS|已复现判定缺陷|
|缺pattern的log_regex|schema和跨字段检查均无错误，结论PASS|已复现判定缺陷|
|UART通道选择|请求通道1选中通道0|已复现资源匹配缺陷|
|截图后Stop|仍调用源波形1次|已复现停止边界缺陷|
|捕获调用不返回|1ms配置超时后40ms仍pending|插件层无deadline保证，硬件适配器仍需负责有界I/O|
|从fresh恢复unfinished|原预检STALE；新预检不匹配旧manifest|已复现恢复绑定冲突|
|租约后matrix损坏|acquire=1，release=0|已复现异常清理遗漏|
|20项样例文件名|20/20与id不一致|插件discover成功不等于Hub可用|
|真实硬件、500行桌面交互、长时老化|未执行|不得据本次评审宣称已验收|

同目录证据文件：

- [test-results.txt](test-results.txt)：12项测试复跑原始结果。
- [reproduction-results.json](reproduction-results.json)：最小复现的结构化结果。
- [reproduce.mjs](reproduce.mjs)：可重新执行的复现脚本，命令 node docs/reviews/fpga-firmware-test-2026-10-18/reproduce.mjs。
- [source-snapshot.json](source-snapshot.json)：评审源文件的SHA-256快照，用于后续修复后核对版本。

复现脚本中的模拟硬件全部为内存适配器；在工作区tmp下创建独立fixture并在结束时删除自己的fixture，不接触真实仪器，也不覆盖20项演示配置。

## 改进顺序和任务边界

### 第一阶段 消除错误结论和停止风险

优先修复F01、F03、F04、F05及F08：规则语义、Stop动作边界、UART身份、租约保护和异常终态。并将本报告的复现改为正式断言回归。此阶段由插件任务主导，宿主协助错误显示与恢复入口；验收应先看错误能否被明确阻断，再看正常演示是否继续通过。

### 第二阶段 接通有界硬件执行和恢复

由宿主任务完成可信fpga-host适配器、串口所有权/原字节游标、deadline和取消/超时确认；插件修复F02和F06，采用不可变内容manifest加独立执行计划。完成普通COM、Dongle、DC和示波器的实机故障注入，覆盖断线、超时、端口外部占用、Stop、磁盘满和进程退出。只有此阶段闭环后才适合受控硬件试用。

### 第三阶段 降低日常配置成本

修正20项示例文件名并通过Hub导入；按规则类型重构schema，增加IP模板、单位选择和引用下拉；补齐进度、递归规则报告及历史attempt导航。至少由一名不熟悉JSON的测试用户完成“新建→选择仪器→添加规则→预检→模拟运行→定位失败→复测”操作，无需修改源码。

### 第四阶段 验证长时规模与扩展机制

结果、日志流式持久化，增加规则/指标能力注册和统一发现核心；把插件回归纳入默认CI。使用目标500项规模与预期最长老化时长设定性能预算，量测内存、磁盘、延迟和恢复行为。性能门槛应由目标工位与业务时序确定，不能用本次小数据集执行时间替代。

## 发布建议

可以继续交付带清晰模拟标识的演示版，用于讨论表单、列表和证据结构；演示配置应先修复F09。受控硬件试用前必须消除P1问题并完成适配器实机验收。长期老化还需给出内存和持久化上界、可靠恢复与故障注入结果。当前固定industrial_approved=false应继续保持，正式质量放行能力另行验收。
