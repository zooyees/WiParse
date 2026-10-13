# 另一台电脑上的 WiParse Windows 异常分析

用户截图显示：应用程序日志事件 1000，`WiParse.exe` 版本 `1.1.26.0`，路径 `D:\SOFTWARE\WiParse-Rust\WiParse.exe`，故障模块为 `WiParse.exe`，模块偏移 `0x9e7124`，异常码 `0xC0000409`，进程 ID `0x238c`，报告 ID `569a026c-7ec9-457e-a1fa-e6a99cd1aa88`。两张截图内容相同，不能算作两次独立故障。

另一工位的转储检查补充：转储位于 `C:\Users\Administrator\AppData\Local\CrashDumps\WiParse.exe.9100.dmp`（约 41 MB，16:44:11），异常参数为 `7`，即 `FAST_FAIL_FATAL_APP_EXIT`；截图给出的近顶层返回地址全在 `WiParse.exe`，首地址 `+0x9e7124`。这与 Windows 上 Rust 标准库 `abort_internal()` 执行 `int 0x29`、传入子码 7 的实现吻合。因此应将其理解为进程执行了主动快速失败路径，而非认定为栈缓冲区溢出。仍需调用者的符号/反汇编，无法仅凭子码断定哪个业务模块触发。

该工位还检查了目标机日志：`D:\SOFTWARE\WiParse-Rust\logs\wiparse.log` 中 14:49:57 启动到 16:44:16 再启动之间没有 ERROR、WARN 或 panic，配置指向同目录的 `crash.log` 但文件未生成。当前仓库的 release 配置没有设置 `panic=abort`；而正常 Rust panic 的 `std::panic::set_hook` 在 abort/unwind 策略下都会先运行。故“普通 Rust panic 后按 release 配置直接 abort”不是现有证据最支持的解释。可能的路径包括显式 abort、不可恢复的运行时故障、panic hook 未执行/未能写入，以及外部原生组件引发的终止；目前不能排序确认。

当前工作机不存在截图中的 `D:\SOFTWARE\WiParse-Rust\WiParse.exe`，其本机 Windows 事件与 `dist/logs` 不是该故障的证据。仓库发布配置去除了符号；仅凭版本号和 `0x9e7124` 偏移不能映射到源码。必须先取得报错电脑上 EXE 的 SHA-256，再与发布产物比较；如果不同，还需该确切二进制和对应构建信息。当前发布包 `dist/WiParse.exe` 的 SHA-256 是 `A1076CECA988B824321157E3F4F523BB2AACAEB62EDCD170E32D74245A9F0314`。

诊断脚本：`scripts/collect-windows-crash.ps1`。在报错电脑上运行时指定截图中的 EXE 路径与报告 ID，生成 JSON，其中包括原有日志和转储路径。若没有现成转储，需在报错电脑上单独启用 Windows Error Reporting 的应用专用 LocalDumps，再复现一次；这涉及注册表修改，脚本本身不会执行。转储文件可能包含应用内存中的敏感数据，分享前需确认处理方式。

尚缺：报错时的用户操作、目标 EXE 哈希、匹配的 PDB 或可解析调用者的转储分析。版本号 `1.1.26.0` 跨多次构建不变；截图故障发生于 16:44，而本机最新发布包生成于 17:50，故不能把后者当作崩溃二进制。当前不能把原因归结为之前修复的 Ctrl+滚轮缩放问题，也不能认定为本次 UI 改动引入。

## 当前代码审查与修复

检查了工作区的显式 abort、panic 策略、Windows 回调、波形显示与后台解码路径。工作区自身未调用 `process::abort()`；依赖中的 Rayon/polling 在不可恢复的回调异常下有主动 abort 路径，但通常先产生 panic hook 记录，且现有证据没有指向它们。Rust 标准库 Windows `abort_internal()` 使用子码 7，内存分配失败等路径可绕过普通 panic hook，因此无 `crash.log` 与资源耗尽相容，但并非证明。

发现并修复一项明确缺陷：波形页原来每帧先复制完整的解码帧、字节、位和 chip 列表，再按当前可见范围跳过绘制。DDSSS 的 chip 列表可远多于屏幕像素，反复缩放/重绘会产生没有上限的临时分配和绘图图元。现改为先按可见时间范围筛选，再按绘图宽度限制标记数量，仍保留选中帧。加入 10 万标记的限量及可见性回归测试。此项是可验证的资源风险修复，不能宣称已复现目标机的精确终止点。

诊断可靠性同时改进：panic hook 先把位置和信息写入 `crash.log`，再尝试耗内存的回溯捕获；启动后记录当前页面，波形加载和协议解码记录规模摘要。release 构建保留匹配 PDB 的调试行号，以便后续转储定位。目标机原始转储仍缺失，不能用新构建的 PDB 解析旧构建偏移。

验证：GUI 108 项测试通过；带调试行号的 release 构建成功。新 ZIP 完整性检查通过（122 个条目），包内 GUI 与 `dist/WiParse.exe`、`target/release/wiparse-gui.exe` 的 SHA-256 一致：`5bccc5561d1f16fdf506b4cfedc349b21b96938059b2a743adfc8f040b47b765`。匹配 PDB 保存在 `dist/WiParse.pdb`，其哈希记录于 `dist/WiParse.symbols.json`，不放入运行 ZIP。ZIP SHA-256：`4752c48973c6217fd22364e880a6ad3a4ae505ff837a01cfbe858a610de5c942`。
