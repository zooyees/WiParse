# FPGA固件测试插件 0.4.0 升级说明

日期：2026年10月18日。适用于 WiParse 1.2.0 工作区。本次升级保留其他任务已有改动，未连接任何真实仪器，未启用固件下载。

## 页面与卡片

测试项卡片只显示测试名称和简短状态。ID、版本及完整标签移到悬停详情；名称超长时截断并保留完整提示。卡片使用统一留白、圆角、边框和选中高亮，状态按通过、失败、运行等区分颜色，列表继续使用虚拟化滚动。

编辑器增加分组导航，不再要求滚动整份配置寻找波形或规则。规则采用类型化 oneOf 表单，普通 COM 与 Dongle UART 使用不同配置分支。右侧检查面板显示易懂的资源名称、配置问题和下一步操作。已有的“文件名与ID不一致”配置可点击修复按钮，操作沿用宿主的版本冲突检查、原子保存和审计事务。

20项IP样例已迁移为 `{id}.json`，迁移记录在 recipes/fpga-firmware-test/.filename-migration.jsonl。其他数据目录可运行 `node scripts/upgrade-fpga-demo.mjs <配置仓库根目录>`；脚本只重命名，不改变测试内容，不覆盖冲突目标。

## 执行控制

工作区所有标签页均提供预检、启动测试、暂停、继续、停止按钮，并显示选择数量及当前执行状态。未保存配置或预检失效时不能启动。执行仍通过原有 runner，资源独占、配置快照和预检绑定继续生效。

暂停采用安全边界语义：点击暂停后先显示“等待当前项完成后暂停”；当前项的复位、采集、判定、归档与清理完成后才暂停，不能冻结正在进行的采集窗口。暂停时保留资源与配置锁，可继续或停止；暂停时间不计入老化运行时长。若当前项已是最后一项，则直接完成测试。

通用接口为 `ui.test_tool.run.control`，参数包含 plugin、run_id、action（pause或resume）。宿主校验活动run、请求状态和项目路径。插件通过 manifest 的 `ui.execution_controls` 声明控制文件与 between_items 语义，宿主不硬编码FPGA业务。未声明安全暂停能力的插件不会获得可用的暂停按钮。停止继续使用宿主已有的独立 Stop 通道，不受配置后台校验队列阻塞。

## 判定与执行可靠性

- 规则类型、必填字段、组合规则数量、正则空匹配、匹配次数及波形单位统一校验。空all、缺pattern和不合法规则不再得到PASS。
- 规则在可终止的worker内执行，有独立预算；灾难性正则不再阻塞runner主线程和Stop。
- UART资源解析精确匹配Dongle物理身份和UART通道，不能用通道0代替配置要求的通道1。
- 截图后开始源波形读取前再次检查Stop。
- 硬件调用接收operation_id、signal、deadline；超时或停止必须由cancelAndConfirm确认quiescent后才能继续。无法确认时标记恢复必需，保留硬件租约并禁止后续设备动作。真实hardware适配器未实现该契约时明确阻断，不自动回退模拟。
- matrix在取得租约前读取；租约取得后的文件操作受finally保护。归档失败不再留下最终running行，已有确定性FAIL保留，其他异常结论为INCONCLUSIVE。
- 报告递归展示组合规则、中文测试名称、身份、实际限值与来源，文本正确转义，只输出存在的证据链接。

## 规模与恢复

日志JSONL分批写入并批量持久化，避免逐行fsync阻塞采集。运行中对冻结输入使用文件身份、纳秒级修改时间、ctime和长度校验缓存；每个run重新建立缓存，内容改变后必须重新验证哈希。

每项证据独立保存，Markdown和attempt索引增量追加。suite-result.json最多内嵌最近1000个attempt摘要，全部attempt保留在attempt-index.jsonl和各自result.json中；通过attempt_count与results_truncated明确表达，不删除历史证据。原始日志仍由适配器按窗口返回；完整流式传输和目标工位的最长老化内存预算仍需硬件集成阶段压测。

恢复把不可变测试内容哈希与本轮执行参数分开。选择failed或unfinished后，通过resume_from选择历史suite-result.json或历史run目录；配置、固件、限值、DUT身份和工位策略保持一致才能建立新attempt。新结果关联parent_run_id与previous_attempt_id。运行中写入run-state与attempt-started日志；选择崩溃run目录时从已完成索引及开始记录恢复，重新占用、重新复位后执行，不从半段采集继续。活动run和未知设备状态的run会被阻断。

## AI与模拟

AI输入取用例inputs、provider allowlist及插件安全字段的交集；required AI没有完成时只能得到INCONCLUSIVE，已有FAIL不被提升。公网未授权仍不发送数据。凭据不写入配置和报告。

模拟运输使用独立、明确的固定IP样例数据，不从用户修改的限值推导“通过数值”。20个IP使用各自成功日志标记，PLL与PWM提供固定频率/占空比模拟指标。模拟结果保持simulation_only=true、industrial_approved=false；修改限值仍可能使模拟规则失败。

## 验收与限制

端到端脚本：`node scripts/verify-fpga-upgrade.mjs <新的临时数据目录>`。它复制20项样例到独立仓库，仅缩短模拟采集时序，经Hub API创建、真实runner预检与运行，检查矩阵和报告证据链接；不会修改原20项配置。

插件回归已接入默认 npm test。测试结果与界面截图见同目录相关输出及 docs/ui-fpga-plugin-0.4.0。验收统计以最终执行日志为准。

硬件适配器、DC/Dongle复位精度、COM高吞吐/环形缓冲、仪器断线和掉电后的物理安全恢复仍需实机验收。固件HEX/BIN下载继续明确阻断。宿主和插件仍各自保留独立发现入口，已加强快照内容绑定，但未把通用发现完全合并。此版本不支持多DUT并发，也不提供工业放行审批。
