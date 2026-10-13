# 波形查看 Ctrl+滚轮闪退分析（2026-09-23）

## 触发路径

用户确认触发操作为 Ctrl+滚轮横向缩放。源码路径：`WaveformAnalysisPanel::plot_area` 允许 `egui_plot` 直接缩放 X 轴；绘图库在 `Plot::show` 闭包返回后根据滚轮修改 `PlotTransform`，然后立刻绘制坐标网格。本应用原先在闭包尾部调用 `clamp_view_bounds`，且此时 `plot_ui.plot_bounds()` 仍是上一帧视窗，因此无法限制这一次滚轮生成的边界。

在绘图库 `egui_plot 0.31.0` 中，缩放公式为 `center + (endpoint-center)/factor`；网格绘制随后对坐标轴范围调用 `f64::clamp(min,max)`。如缩放产生无效范围（上下界倒序、相等或非有限），Rust 的 `f64::clamp` 会 panic。频繁放大、异常大滚轮增量、过窄时间范围会提高风险。应用自己的边界检查原先发生在绘图库处理缩放之后，保护不到这个绘制阶段。

本机 `D:\software\WiParse\logs\crash.log` 和仓库 `dist\logs\crash.log` 均保存了 2026-08-02 的旧 panic：`min > max`，下界 `0.0005969900000000002`、上界 `0.0005969899999999999`，与无效的极窄时间范围一致。不过日志不是本次用户闪退时生成的，不能仅凭它断言本次故障只有这一原因。`D:\software\WiParse\logs\wiparse.log` 最近更新于 2026-09-23 14:52，未出现新的 Rust panic；查询最近两天 Windows Application Error / WER 未找到 WiParse.exe 记录。尚未拿到本次闪退所用波形和 exe 路径。

本机另有两个版本：`D:\software\WiParse\WiParse.exe` 修改于 2026-09-23 08:30，SHA256 前缀 `17245627`；仓库 `dist\WiParse.exe` 修改于 14:14，SHA256 前缀 `199B97D0`。程序路径会影响问题是否出现在此前打包版本，不能把两者混作同一二进制。

## 源码修复

- 关闭 `egui_plot` 内部的 Ctrl+滚轮缩放，将滚轮倍率交由应用计算，在调用 `set_plot_bounds` 前执行已有的数据范围和最大倍率限制。
- 新增 `zoom_x_around`：保持鼠标所在时间点的缩放锚点，同时拒绝无效输入和塌缩边界。
- `clamp_view_bounds` 遇到无效范围时恢复数据范围；上帧视窗非有限或顺序错误时不进入可见区间缓存键与后台重采样。
- 未改变 Y 轴手动缩放、工具栏缩放和一般波形绘制的交互。

## 验证

- `cargo test -p wiparse-gui --bin wiparse-gui`：107 passed，0 failed。新增回归连续放大 1000 次，确认边界合法且被限制到最大倍率。
- `cargo test -p wiparse-core wave_display`：12 passed，0 failed，覆盖可见区间包络、均匀采样及时间桶逻辑。
- `git diff --check`：通过。

这验证了已确认的边界保护缺陷和修复。尚缺用户实际崩溃文件、启动程序路径和新的崩溃堆栈；因此对“本次闪退”的归因仍为高概率，而不是已经逐次重现。当前修改只在源码，之前打包的 ZIP 和 EXE 尚未重建。
