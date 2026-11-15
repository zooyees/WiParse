# r14 受审阅自动监控实验验证

源码为 `source-manifest-candidate-20261115-r14.json`，清单 SHA-256 `e7d832b4d692a0b8fc5d51967dde7df28a1f2cd24dc6300d21fb49842e8b32c3`。实验只用隔离构建目录、本机模拟 HTTP 和模拟工作线程，未连接真实设备，未打包或部署。

测试构建显式设置 `WIPARSE_REVIEWED_MONITOR_VERIFIED=1`；`v08-monitor-enabled-gate-r14.log` 检查编译时和运行时值一致，`v08-monitor-enabled-approval-r14.log` 检查批准前没有收到请求、批准后收到冻结样本，`v06-monitor-enabled-gate-r14.log` 八项检查来源、撤权、过期、认领及未知结果。`v03-monitor-revoke-r14.log`、`v03-monitor-scope-r14.log` 分别检查撤权和范围变化。测试后 `v08-monitor-default-restored-r14.log` 确认恢复默认关闭的测试构建。

在 `target/monitor-experiment-r14` 用 `--locked --offline --release` 编译启用配置的 GUI，构建成功（11 分 18 秒），日志 `v11-monitor-experimental-build-r14.log`，GUI SHA-256 `f829f1bdb488b6a8556ab62b065df9a239e9118af7384210bec09b03740cdba0`；该构建没有覆盖默认产物。测试开关和局部模拟不能证明实验生产程序的桌面确认、真实串口、物理安全或无线 live 能力；这些仍需单独验证，默认门禁保持关闭。
