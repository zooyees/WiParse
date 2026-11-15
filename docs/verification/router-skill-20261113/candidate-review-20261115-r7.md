# Router / Skill 候选验证记录（r7）

源码清单：`source-manifest-candidate-20261115-r7.json`，清单 SHA-256 `e43a4df8da802386f64c429e3697d55c279ea58c28e83a77630a8ac950825098`。验证后核对 449 个受清单覆盖的源码、测试、评测和打包文件，没有新增、删除或散列变化。该清单不覆盖运行数据和文档，不代表独立构建环境已完全冻结。

| 项目 | 当前结果和证据 |
| --- | --- |
| 编译 | r1 GUI 测试编译有 5 处错误，修复后 r2 `v01-candidate-check-20261115-r2.log` 通过；核心测试编译错误在 r3 修正。后续 GUI、核心和服务测试构建均成功。 |
| Core | r4 在可访问本机回环的环境中 419 通过、2 忽略，见 `v01-core-authorized-r4.log`；此前沙箱内网络失败和历史证据错误保留在 `v01-core-candidate-r3.log`。r7 相对 r4 的核心源码没有变化。 |
| GUI | r5 串行 580 通过、1 失败、9 忽略，见 `v01-gui-authorized-r5.log`；失败是来源切换通知的测试断言，r6 定向复验通过。r7 串口 AI 相关 158 通过、1 忽略，管理参数恢复用例及测试控制 4 项通过。r4 并行全量有失败，包含共享工作线程饱和导致的报告读取失败；不能把并行运行当成通过。 |
| Service / CLI | r6 测试通过，服务 17 项；CLI 无测试，见 `v01-service-cli-r6.log`。 |
| 插件 / 无线 | r7 Node 测试 289 通过、2 跳过，见 `v01-plugins-r7.log`。跳过项依赖已投入运行的真实台站；不能作为无线 live 验收。 |
| DeepSeek / Promptfoo | 相同合成输入 r6 理解和续问通过、拆解失败；失败时完整扫描已得出两次转换，但不合法的管理调用使任务停止。r7 返回可修正错误并提供管理 schema 后，拆解场景复测通过。详见 `v08-promptfoo-r6.log`、`v08-promptfoo-r7.log` 和对应轨迹。r7 拆解单场景服务端累计 71,428 Token（含缓存输入），未证明成本低或整体性能改善；修复后的其他两项未重新进行付费实测。 |
| 千问 | 未配置凭据，三个真实模型场景未执行，不能比较真实模型表现。 |
| 桌面、硬件及发布 | 自动布局测试不替代完整桌面人工验收；computer-use 的 node_repl 初始化（含重置后重试）返回 `failed to write kernel assets: The system cannot find the path specified. (os error 3)`，桌面操作未开始。真实仪表、可信无线台站、物理安全反馈、安装升级和回退尚无本候选验收证据。默认关闭受审阅自动监控生产门禁；启用构建尚未验证，不得因测试钩子通过而开启。 |

本轮修正还包括：来源发生变化时取消当前手动任务并暂停待执行队列；历史证据可查看但不能作为当前证据引用；串口测试控制仅限本任务对应运行，并要求具体操作确认。测试结果只证明记录的范围，不代表工业级完整验收。`tools/agent-eval` 没有 npm test 脚本，误执行的命令失败已记录在 `v01-promptfoo-unit-r2.log`；Promptfoo 使用其实际评测入口执行。

Broker 定向 14 项、派发门禁定向 8 项通过，详见 `v03-broker-r7.log`、`v06-dispatch-gate-r7.log`；环境记录见 `environment-candidate-r7.json`。受设备和桌面环境限制，V07、V09 和 V11 不能标记完整通过。

## V01–V11 状态

| 项目 | 状态 | 依据或未完成内容 |
| --- | --- | --- |
| V01 编译与回归 | 部分通过 | Core、GUI、Service 和插件结果见上表；GUI r7 仅对改动相关串口区域定向复验，不能将 r5 全量结果标作 r7 全量通过。 |
| V02 注册和路由 | 部分验证 | Core 与串口测试覆盖注册、路由和权限，未单独整理所有计划矩阵场景的逐项证据。 |
| V03 权限与对抗 | 部分验证 | Broker 14 项、测试控制 4 项通过；完整威胁矩阵及组合覆盖待逐项核对。 |
| V04 任务和证据 | 部分验证 | Core 及 GUI 自动测试覆盖历史证据和报告读取；未对所有五个技能逐一完成独立实测样本比对。 |
| V05 无线时序 | 部分通过 | 插件的本地场景通过；真实台站的两项跳过，不能证明实际 I2C 和物理时序。 |
| V06 控制和恢复 | 部分验证 | 门禁与 Broker 自动用例通过；完整真实设备断线及崩溃后的物理读回没有证据。 |
| V07 桌面交互 | 阻断 | 桌面工具初始化失败；测试中的布局和键盘模拟覆盖不能替代真实窗口操作。 |
| V08 模型和外发 | 部分验证 | DeepSeek r6 三场景、r7 拆解单项；千问无凭据，修复后其他场景和本地模型未实测。 |
| V09 真实硬件 | 阻断 | 未连接并验证目标台架、仪表、FT4222 和安全反馈。 |
| V10 性能和质量 | 未完成 | 有同一合成样本的模型 Token 和时延记录；没有足够独立样本、P50/P95 或一致基线，不能宣称性能或费用收益。 |
| V11 发布准备 | 未完成 | 发布配置编译检查通过；桌面验收、安装升级和回退未完成，暂不打包或部署。 |

发布配置编译日志 `v11-release-build-r7-retry.log` 显示 GUI、CLI 和服务构建完成（7 分 53 秒）；这只是构建检查，不是可发布包。GUI SHA-256 `85437e3cfa0986b082754c95305a1e659b3883055a490ce11d3e3da5ba5d02fe`；CLI SHA-256 `c51dad8ce5665212f6abcd691bdfd09531d74748d719616c8d95cd5f0c875d3e`；服务 SHA-256 `929409d7fd2b5a097984a7de99360a4724512a11526b63957ae213a473ce9ff3`。测试宿主 SHA-256 `14319786e7ef8c8a0a27caf868d4dc43911db65fe781275dba45428795b16366`。构建使用默认关闭的受审阅自动监控门禁。

V02 的现有自动证据包括 Core 的 `router_handles_domains_combinations_missing_context_and_explicit_priority`、`registry_rejects_unknown_and_duplicate_selection`、`unknown_source_never_qualifies_for_automatic_routing`；V04 的 GUI r5 串行证据包括 `selected_log_skill_uses_real_file_and_host_before_completing_plan`、`selected_qi_skill_reads_decoded_isf_frames_before_completion`、`selected_report_skill_reads_demo_markdown_pages_before_completion`、`selected_wave_skill_measures_wfm_before_completing_plan` 和 `charging_skill_requires_log_events_and_sourced_waveform`。这些用例支持上述部分验证判断，但不替代逐项真实数据验收或 r7 全量 GUI 实测。
