# r7 路由、权限、技能和 Token 补充核对

## 注册和路由矩阵

以下均是自动测试证据，未经过独立真实模型盲评。Core 的 r4 到 r7 源码一致；GUI r5 的通用工程 Agent 部分到 r7 没有修改，串口入口改动已在 r7 定向验证。

| 情况 | 已有证据 |
| --- | --- |
| 显式选择顺序与优先级 | `explicit_assessment_preserves_every_selection_and_order`、`router_handles_domains_combinations_missing_context_and_explicit_priority`。 |
| 未知来源、缺少条件、候选与组合 | `unknown_source_never_qualifies_for_automatic_routing`、`cross_domain_candidates_remain_suggestions_without_inventing_a_selection`。 |
| 冲突和否定 | `domain_exclusions_do_not_suppress_unrelated_reads`、`later_explicit_exclusion_is_not_hidden_by_an_earlier_mention`、`conceptual_prefix_does_not_discard_a_followup_analysis_request`。 |
| 注册与兼容 | `registry_rejects_unknown_and_duplicate_selection`、`contract_text_rejects_blank_and_oversized_fields`、GUI 的 `old_test_execution_skill_version_requires_new_task`。 |
| 测试执行范围 | `test_execution_skill_includes_owned_run_controls` 及 `v03-test-control-r7.log` 的 4 项。 |

测试名称和结果在 `v01-core-authorized-r4.log`、`v01-gui-authorized-r5.log`、`v03-test-control-r7.log`；它们不证明所有自然语言任务都能正确路由。

## 权限与威胁组合

`v03-broker-r7.log` 覆盖项目、来源、授权伪造，确认过期、取消、参数与运行绑定变更，仪表会话替换，以及并发入队；`v06-dispatch-gate-r7.log` 覆盖来源/配置变化、确认超时、认领后未知和禁止重放。`v01-gui-authorized-r5.log` 中的 `injected_report_cannot_execute_unconfirmed_engineering_device_action`、`stale_source_cross_tool_and_forged_internal_requests_are_rejected` 及 `platform_registry_rejects_raw_commands_arbitrary_files_and_extra_fields` 覆盖部分注入与越权；`v01-serial-agent-r7.log` 复验了串口入口。以上为自动和模拟环境，不代表所有攻击组合或真实设备均已验证。

## 五个技能与真实数据

GUI 串行回归中日志、波形、Qi、报告和充电保护各有通过用例，代码使用真实宿主工具，但模型由本地模拟服务提供：日志是仓库合成 fixture，Qi 是合成帧，报告/矩阵是临时创建的模拟文件，充电保护使用合成数据；波形用仓库 `sample_waveforms/Tektronix_WFM/tek_4ch_sine_square_tri_saw.wfm`，样本的采集与标定来源尚未独立确认。不得把模拟结果称为产品实测。

另对本机 `D:\software\WiParse\sample_waveforms\SCP\Normal.isf` 只读执行原生解析测试，文件 SHA-256 `0c030f7d5ac283aeba2aab8678d6cfecf482060b2064f6d1246bf087acd559ad`，结果 60 个 SCP 数据帧、218 个非数据帧，PING 不计入数据帧，见 `v04-scp-normal-r7.log`。这是 SCP 波形及 GUI 报文分类验证，不是 Qi 技能、完整 AI 诊断、设备台架或协议标准合规验收。此前对话中的 `D:\software\WiParse\sample\_waveforms\SCP\Normal.isf` 路径在本机不存在；这里没有把两个路径当作同一文件。

## Token 消耗

r7 任务拆解请求实际 7 次模型响应，服务报告输入 68,836、输出 2,592、合计 71,428 Token，其中缓存输入 49,152。轨迹说明模型有一次状态识别规则不完整后进行了修正，也进行了计划和证据操作；它仅说明本次会话，不能直接推广为平均值。原生工具请求的完整工具 schema 与有限的历史轮次被重复携带，当前代码在 `serial_tool/ai_chat.rs` 和 `ai/tools.rs` 中可核对；缓存输入也不等同于免费。

这些是高成本的可疑来源，但现有轨迹没有按请求拆解 system、schema、历史、工具回执的 Token 分布，无法证明任一因素的贡献或保证裁剪后不损失分析质量。建议在合成数据与同一模型配置上增加本地、无密钥的请求分段计量，再对照任务正确率、证据覆盖、P50/P95 与费用验证压缩；不能只凭这一个样例声称优化收益。本轮没有为了降低数字删掉任务证据或模型可调用能力。

### r8 去重试验与撤回

曾试验将重复出现的初始证据替换为引用。r8 串口回归首次失败是既有调用链要求完整初始证据，其后的 8 项失败为同一个测试锁中毒；DeepSeek 拆解场景也未通过，因事件规则参数无效后任务直接中止，续问场景通过。不能证明拆解失败由去重引起，但由于不能保证质量，已撤回去重；r8 的失败日志和轨迹保留，不将 Token 数字作为收益结论。

日志规则参数无效直接中止的问题独立存在；r9 将手动本地只读参数验证错误作为未执行结果返回模型修正，禁止实际访问非法路径的约束仍保留。定向用例和串口 AI 159 项通过；DeepSeek 拆解复测通过、累计 79,055 Token（输入 76,605、缓存输入 57,472），本次模型没有触发日志规则错误，故真实规则修复链仍以定向测试为证据。r10 进一步保持本地预览的既有同步拒绝行为，需单独验证；不能使用 r7 构建物冒充当前源码。
