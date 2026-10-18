# FPGA 插件接入评审与效果验证

日期：2026-10-18；宿主 WiParse 1.2.0；插件 fpga-firmware-test 0.3.1。

检查插件开发工位交付后，保留业务与宿主的责任边界，完成以下修正。

|问题|修正及检查|
|---|---|
|声明 configure 但未导出函数|导出业务校验入口，使用真实配置工作区加载与保存|
|业务预检哈希代替宿主绑定|确认 ctx.testMatrix 清单，运行使用冻结快照、选择集和预检 revision|
|矩阵字段缺失、状态大写|对齐通用矩阵，校验 plugin/run/attempt/progress/elapsed/loop_index、选择集统计及项目内结果链接|
|只依赖注入 mock，无法实际从 runner 演示|增加必须显式选择的 simulation adapter；硬件模式继续拒绝不完整 adapter|
|老化提前结束可能误判整套 PASS|未完成预期项数时保持不确定结论，清理异常独立记录|
|模拟标记在通用结果保存时丢失|结果与 run.json 保留 simulation_only；报告和截图可见标注|
|自定义 GUI 端口下可能误连旧实例|runner/SDK 继承当前 WIPARSE_API_BIND，显式 URL 优先；实际 GUI HTTP 验证通过|
|Windows 状态文件替换偶发 EPERM|保留原子替换，增加600ms以内有界重试；持续占用保留旧文件并明确失败|
|相对目录可能被不同工作目录解释|保存相对设置，运行边界统一解析，案例固件相对 config_dir；包内报告均使用相对链接|

模拟验证通过真实 Hub/runner 生命周期，而非直接调用插件函数：五项配置得到五项 PASS；只勾选两项且注入失败时，两项 FAIL、三项 SKIPPED。报告包含每项结果 JSON、截图、源波形 CSV、串口日志，共二十个证据链接。单元回归另覆盖停止、时间窗、缺失/坏质量指标、配置与固件变化、资源及清理失败。

使用发行包时进入集成测试，选择 FPGA 固件测试插件，配置仓库保持默认相对路径。选择“模拟”执行模式后先预检再运行；包内已经包含五项历史模拟报告，亦可直接在测试报告工具打开。只有五项配置时仅显示五项，500 是上限。不要将模拟 PASS 用作真实产品放行依据。

尚未完成真实硬件验收：fpgaAdapter 仍须落实并验证原始串口字节游标、设备租约、完整时间窗、仪器 ACK、电压读回、复位释放、安全断电与异常恢复。未提供 adapter 时硬件模式明确失败；本次发行不声称已经满足这些要求。

复验入口：`scripts/verify-hub-release.ps1` 检查实际 GUI HTTP、预检运行、五行矩阵、证据打开及相对设置保留；插件测试位于 `test-tools/plugins/fpga-firmware-test/test/`。运行证据见 `evidence/release-1.2.0-2026-10-18/`。

实际 egui 渲染预览：[配置表单](ui-testing-hub-1.2.0/fpga-1440-0.png)、[五项矩阵](ui-testing-hub-1.2.0/fpga-1440-1.png)、[窄窗口矩阵](ui-testing-hub-1.2.0/fpga-640-1.png)。
