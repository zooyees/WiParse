# 协议解析审查与修复（2026-09-23）

本次检查覆盖当前可选的 UART、I2C、SPI、I2S、DDSSS、USB PD、SCP、UFCS，以及模拟电平转换和界面报文展示。以下是源码修复及验证结果；dist 中此前打包的程序尚未包含本次修复。

## SCP 实测

实际文件：`D:\software\WiParse\sample_waveforms\SCP\Normal.isf`。用户消息中的 `sample\_waveforms` 在本机不存在，使用查找到的同名文件。

SHA256：`0C030F7D5AC283AEBA2AAB8678D6CFECF482060B2064F6D1246BF087ACD559AD`。

三通道各 10,000,000 点，采样间隔 1 µs，时间约 -5 至 +5 秒。CH2 包含 SCP 数据；CH3 无 SCP 帧；CH4 只识别到 6 个 PING。

CH2 最终结果：278 个记录（60 个数据包、218 个 PING），0 个奇校验、CRC 或帧错误，无输出截断。例如首个读请求为 `0C 7E 12`，显示 `SCP SBRRD reg=0x7E`。没有用户提供的逐包参考报文，因此这里的“正确”依据是物理编码、奇校验、CRC 和规范命令映射一致，并非与另一台分析仪逐包比对。

定位到的主要问题：

- 通用 CMOS 阈值或按整个捕获的最大摆幅推算阈值，可能漏掉同线的 1.8 V 发端。SCP 自动阈值改为协议接收门限 VIL=0.9 V、VIH=1.3 V，手动阈值仍以伏特为单位。
- 直接用约 40 µs 的分隔脉冲估计位宽，1 µs 采样误差会累积到 9 位末尾，导致漏过后续分隔符。以 16 UI PING 优先校准，分隔符作为无 PING 时的后备；适当放宽末尾边界定位。修复时钟之前，本文件有 14 个错误记录；修复后全部消失。
- PING 获取不再只围绕默认 UI 判断，可重新获取发端时钟；新增 RESET 脉冲识别。
- 命令标签原先读写相反：纠正为 0x0C SBRRD、0x0B SBRWR、0x1C MBRRD、0x1B MBRWR。
- CRC 为多项式 0x39、初值 0；8 位 MSB first，奇校验。全字节值、混合 1.8/3.3 V、144/160/176 µs UI、量化分隔符及主动注入错误均有回归。

物理时序和命令核对依据为本地协议研究 PDF 附录 B（B.3、B.4 及图 B.19/B.20），缓存路径 `target/protocol-research/charging.pdf`。

## 其他协议

| 范围 | 审查结果与修正 | 验证 |
| --- | --- | --- |
| UART | 缺失 BREAK 确认采样不再视作持续低电平；手动波特率允许只有一个起始边沿的捕获；完整数据但未捕获停止位标记 Framing | 零/全一字节、奇偶校验、BREAK 后恢复、停止位和截断回归 |
| I2C | SDA 缺失不再补 1；STOP、重复 START 和文件尾保留不完整字节错误；重复 START 前保留半个 16 位寄存器地址并标错 | 7/10 位地址、重复 START、NAK、交换通道、缺 ACK、半寄存器及预算回归 |
| SPI | 用起点采样判断 CS，修正负时间轴下 epsilon 无效的问题；缺失数据通道采样标错；部分字保留超过 8 位的数据并标错 | 双向、双/四线、模式 0/2、LSB、负时间轴 CS、通道截断、16/24 位部分字 |
| I2S | 文件尾部分采样不再丢失；缺失 WS/DATA 采样不再补零为正常值；Philips 短时隙只消费旧通道最后一个延迟时钟，避免吞掉下一通道 MSB | Philips/Left-J 对齐、尾部截断、短时隙后下一通道恢复 |
| USB PD | 修正未分块扩展报文以 NDO 算长度的问题，按扩展头 Data Size 计算；支持最大 260 字节扩展载荷及其头和 CRC | BMC GoodCRC、CRC 错误、标准 CRC32 向量、截断、7 种 ordered set、5 V/3 A PDO、1/24/260 字节扩展载荷 |
| UFCS | 本轮所检查的控制/数据分帧与 CRC 路径未复现新缺陷；补充载荷内 0xAA 与各支持波特率测试 | 控制帧、坏 CRC、38400/57600/115200/230400、载荷包含 SOP、截断数据包 |
| DDSSS | 本轮未修改算法，既有错误保留与解扩回归通过 | 极性反转、85 kHz、浅调制、多包、码片/奇校验/校验和错误后恢复 |
| 公共电平转换 | 手动阈值的迟滞宽度不再固定套用 3.3 V CMOS 窗口，支持手动解码 1 V 逻辑摆幅 | 电压单位、地噪声与振铃既有测试，加 1 V 信号回归 |
| 界面 | Qi 语义解释限定于 DDSSS，避免其他协议字节被错误命名；充电通道提示区分 CC、SCP D-、UFCS D+；SCP 手动阈值初值 1.1 V | GUI 全量测试及编译通过 |

USB PD 扩展长度核对：[USB-IF Power Delivery 3.0 Tests v1.14 r2](https://www.usb.org/sites/default/files/Power%20Delivery%203%200%20Tests%20v1p14%20r2.pdf)，Source Tests 中 unchunked extended 的 NDO=0、Data Size 以及分块填充规则。

## 验证与复现

- `cargo test -p wiparse-core --lib`：225 passed，0 failed；1 个依赖外部文件的测试默认 ignored。
- `cargo test -p wiparse-gui --bin wiparse-gui`：106 passed，0 failed。
- 显式运行外部 SCP 测试：1 passed，0 failed。
- `git diff --check`：通过（已有文件行尾转换提示不影响检查）。

```powershell
$env:WIPARSE_SCP_CAPTURE='D:\software\WiParse\sample_waveforms\SCP\Normal.isf'
cargo test -p wiparse-core real_normal_capture_regression -- --ignored
cargo run -p wiparse-core --example inspect_bus -- scp $env:WIPARSE_SCP_CAPTURE
```

诊断工具也接受 `usbpd`、`ufcs`。外部大波形未复制进仓库。原始测试输出位于 `target/protocol-research/`。

## 验证边界

此次实测确认集中在用户提供的 SCP 文件；其他协议以现有和新增合成回归为依据，不能替代更多设备实测。USB PD 扩展消息目前展示原始载荷，不做跨包 chunk 重组和所有扩展消息字段解释；UFCS Vendor payload、SCP 私有寄存器保留原始字节。DDSSS 的波形协议解码与完整 Qi 应用层语义是不同层次。所有协议仍遵守现有 512 字节显示预算，达到上限会提示截断；长捕获应使用时间窗分段分析。
