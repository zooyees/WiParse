# FPGA 固件测试示例

`cases/` 中的 20 个文件是可运行的 `wiparse.fpga-test-case/2` 示例。DUT 约束和仪器角色只写在同级的 `project.json`（`wiparse.fpga-project/1`）里。插件清单的工程种子是 `{config_dir}/../project.json`，由宿主放进未保存草稿；保存并选择运行模式之后，预检读取的是 `ctx.projectConfig`，不直接读种子文件。

`wiparse.fpga-test-case/1` 只作为外部旧数据的迁移来源，用 `planProjectMigration` 预览差异后再由宿主提交。本目录里的示例已经迁完，打开配方后不需要再迁一次。未迁移的 schema/1 数据直接预检会失败。

默认 `execution_mode` 仍是 `hardware`。查看效果时要先保存工程，再显式选择模拟。DEMO 仪器身份和占位固件不代表硬件验收。

配方诚实标签：

- `log-selftest`（18 项）：复位后用 UART/Dongle 日志 token 判定功能；示波器截图可作旁证，不启用源波形量测。
- `analog-scope`（`fpga_01_pll_clock`、`fpga_09_pwm`）：在日志 token 之外用示波器源波形实测频率/占空比。宿主只发布解析得到的 metrics，不编造数值；测不到则 INCONCLUSIVE。

证据在 attempt 目录的 `logs/`、`waves/`、`shots/`。Hub 按插件 `outputs` 打开这些视图。`PLUGIN_FREEZE.hardware_accepted` 保持 false，直到实机联合项通过。
