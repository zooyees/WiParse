# Tektronix 3 Series MDO（含 MDO3014）SCPI 控制指令手册（中文整理）

> **来源**：`docs/3-MDO-Oscilloscope-Programmer-Manual-077149800.pdf`（Tektronix *3 Series MDO Oscilloscopes Programmer Manual*，文档号 077-1498-00）
>
> **整理方式**：按手册 **Command Groups** 功能分组表提取全部指令；功能说明译为中文；Syntax 保留手册原文（SCPI 大写为最短合法缩写）。
>
> **翻译说明**：P0 优先分组（文件系统、硬拷贝、水平、数学等）为完整中文；其余分组为短语机译，必要时保留英文关键词并以 `【设置/查询】` 等标签标明类型。个别超长指令名可能因 PDF 换行被截断，请以手册正文 Syntax 为准。
>
> **用途**：支撑 WiParse 将 MDO3014 全量遥控能力接入 **PC GUI** 与 **CLI/AI**。

## 1. 阅读约定

| 记号 | 含义 |
|------|------|
| `ACQuire:MODe` | 最短可写 `ACQ:MOD`（大写必写，小写可选） |
| `{A\|B\|C}` | 枚举参数，三选一 |
| `<NR1>` / `<NR3>` / `<QString>` | 整数 / 浮点 / 引号字符串 |
| 指令名以 `?` 结尾 | 查询（Query） |
| `CH<x>` / `REF<x>` / `MEAS<x>` / `MATH[1]` | 通道/参考/测量/数学波形占位符 |

## 2. WiParse 集成优先级（MDO3014）

| 优先级 | 分组 | 典型 CLI / GUI 用途 |
|--------|------|---------------------|
| P0 | Hard Copy / Save and Recall | `scope shot` 截屏 PNG、保存/加载 setup |
| P0 | Waveform Transfer | `CURVe?` + `DATa:SOUrce` / `WFMOutpre` 数值波形 |
| P0 | Horizontal / Vertical / Acquisition | 时基、通道开关/伏格、RUN/STOP |
| P0 | File System | 远程读写文件、工作目录 |
| P1 | Trigger | 边沿等触发源、耦合、电平 |
| P1 | Measurement / Cursor | 自动测量与光标读数给 AI |
| P2 | Math / Zoom / Display | 运算波形、缩放、显示风格 |
| P2 | Status and Error / Miscellaneous | `*IDN?`、`*OPC?`、错误队列 |
| P3 | Bus / Search / RF / Power / AFG / ARB… | 按测试场景逐步覆盖 |

## 3. 分组目录

1. **采集 (Acquisition)** — 14 条
2. **事件动作 (Act on Event)** — 2 条
3. **任意函数发生器 (AFG)** — 16 条
4. **别名 (Alias)** — 5 条
5. **任意波形 (ARB)** — 9 条
6. **总线解码 (Bus)** — 86 条
7. **校准与诊断 (Calibration/Diagnostic)** — 29 条
8. **配置 (Configuration)** — 41 条
9. **光标 (Cursor)** — 37 条
10. **显示 (Display)** — 26 条
11. **数字万用表 (DVM)** — 12 条
12. **邮件 (Email)** — 6 条
13. **以太网 (Ethernet)** — 12 条
14. **文件系统 (File System)** — 17 条
15. **硬拷贝/截屏 (Hard Copy)** — 9 条
16. **水平时基 (Horizontal)** — 12 条
17. **标记 (Mark)** — 15 条
18. **数学运算 (Math)** — 15 条
19. **自动测量 (Measurement)** — 42 条
20. **杂项与 IEEE488.2 (Miscellaneous)** — 52 条
21. **电源分析 (Power)** — 140 条
22. **射频分析 (RF)** — 73 条
23. **保存与调用 (Save/Recall)** — 20 条
24. **搜索 (Search)** — 129 条
25. **状态与错误 (Status/Error)** — 17 条
26. **触发 (Trigger)** — 192 条
27. **垂直通道 (Vertical)** — 65 条
28. **视频画面 (Video Picture)** — 15 条
29. **波形传输 (Waveform Transfer)** — 42 条
30. **缩放 (Zoom)** — 7 条

**合计：30 组，1157 条指令（分组表汇总并去重）。**

---

## 1. 采集 (Acquisition)

手册原名：*Acquisition Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ACQuire?` | 返回采集参数。 | `ACQuire?` |
| `ACQuire:FASTAcq` | 设置或查询 快速采集功能. | `ACQuire:FASTAcq` |
| `ACQuire:FASTAcq:PALEtte` | 设置（或查询） 快速采集模式使用的调色板. | `ACQuire:FASTAcq:PALEtte`<br>`ACQuire:FASTAcq:PALEtte?` |
| `ACQuire:FASTAcq:STATE` | 打开或关闭快速采集模式，或查询该模式状态。 | `ACQuire:FASTAcq:STATE {0\|1\|OFF\|ON}`<br>`ACQuire:FASTAcq:STATE?` |
| `ACQuire:MAGnivu` | 本指令设置 MagniVu 功能。 | `ACQuire:MAGnivu {<NR1>\|OFF\|ON}`<br>`ACQuire:MAGnivu?` |
| `ACQuire:MAXSamplerate?` | 返回 最大实时采样率。 | `ACQuire:MAXSamplerate?` |
| `ACQuire:MODe` | 设置 示波器全部模拟通道波形的采集模式. | `ACQuire:MODe {SAMple\|PEAKdetect\|HIRes\|AVErage\|ENVelope}`<br>`ACQuire:MODe?` |
| `ACQuire:NUMACq?` | 返回已发生的采集次数。 | `ACQuire:NUMACq?` |
| `ACQuire:NUMAVg` | 本指令设置 平均波形所用的采集次数。 | `ACQuire:NUMAVg <NR1>`<br>`ACQuire:NUMAVg?` |
| `ACQuire:NUMEnv` | 本指令控制 包络次数 (when 采集 mode is set to ENVelope using ACQuire:MODe). | `ACQuire:NUMEnv {<NR1>\|INFInite}`<br>`ACQuire:NUMEnv?` |
| `ACQuire:SEQuence:CURrent?` | 返回 序列中目前已完成的采集次数。 | `ACQuire:SEQuence:CURrent? Returns <NR1> is an integer that specifies the number of acquisitions in the sequence` |
| `ACQuire:SEQuence:NUMSEQuence` | Sets or returns the 序列使用的采集次数。 | `ACQuire:SEQuence:NUMSEQuence <NR1>`<br>`ACQuire:SEQuence:NUMSEQuence?` |
| `ACQuire:STATE` | 启动或停止采集系统。 | `ACQuire:STATE {OFF\|ON\|RUN\|STOP\|<NR1>}`<br>`ACQuire:STATE?` |
| `ACQuire:STOPAfter` | 本指令设置 采集为连续还是单次序列 Act on Event Co mmand Group UsetheActonEventcommandsto Actonceacertaintypeof Eventhashappened. | `ACQuire:STOPAfter {RUNSTop\|SEQuence}`<br>`ACQuire:STOPAfter?` |

## 2. 事件动作 (Act on Event)

手册原名：*Act on Event*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ACTONEVent:NUMACQs` | 设置（或查询） number of 采集s to complete for the event type ACQCOMPLete. | `ACTONEVent:NUMACQs <NR1>`<br>`ACTONEVent:NUMACQs?` |
| `ACTONEVent:REPEATCount` | 【设置】Sets or returns the number of events to run. | `ACTONEVent:REPEATCount <NR1>`<br>`ACTONEVent:REPEATCount?` |

## 3. 任意函数发生器 (AFG)

手册原名：*AFG Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `AFG:AMPLitude` | 设置（或查询） AFG amplitude in volts, peak to peak. | `AFG:AMPLitude <NR3>`<br>`AFG:AMPLitude?` |
| `AFG:FREQuency` | 设置（或查询） AFG frequency. | `AFG:FREQuency <NR3>`<br>`AFG:FREQuency?` |
| `AFG:FUNCtion` | 设置（或查询）哪个 AFG function to execute. | `AFG:FUNCtion`<br>`AFG:FUNCtion?` |
| `AFG:HIGHLevel` | 设置（或查询） AFG high level, in volts. | `AFG:HIGHLevel <NR3>`<br>`AFG:HIGHLevel?` |
| `AFG:LEVELPreset` | 【设置或查询】Sets (or queries) the AFG preset levels to values that correspond to the logic standard specified by the argument The presets set the vertical controls for AMPLitude, OFFSet , HIGHLevel, and LOWLevel. | `AFG:LEVELPreset {CMOS_5_0V\|CMOS_3_3V\|CMOS_2_5V\|ECL\|TTL\|USER}`<br>`AFG:LEVELPreset?` |
| `AFG:LOWLevel` | 设置（或查询） AFG low level, in volts. | `AFG:LOWLevel <NR3>`<br>`AFG:LOWLevel?` |
| `AFG:NOISEAdd:PERCent` | 设置（或查询） AFG additive noise level as a percentage. | `AFG:NOISEAdd:PERCent <NR3>`<br>`AFG:NOISEAdd:PERCent?` |
| `AFG:NOISEAdd:STATE` | 设置（或查询） AFG additive noise state. | `AFG:NOISEAdd:STATE {0\|1\|OFF\|ON}`<br>`AFG:NOISEAdd:STATE?` |
| `AFG:OFFSet` | 设置（或查询） AFG offset, in volts. | `AFG:OFFSet <NR3>`<br>`AFG:OFFSet?` |
| `AFG:OUTPut:LOAd:IMPEDance` | 设置（或查询） AFG output load impedance. | `AFG:OUTPut:LOAd:IMPEDance {FIFty\|HIGHZ}`<br>`AFG:OUTPut:LOAd:IMPEDance?` |
| `AFG:OUTPut:STATE` | 设置（或查询） AFG output state. | `AFG:OUTPut:STATE {0\|1\|OFF\|ON}`<br>`AFG:OUTPut:STATE?` |
| `AFG:PERIod` | 设置（或查询） period of the AFG 波形, in seconds. | `AFG:PERIod <NR3>`<br>`AFG:PERIod?` |
| `AFG:PHASe` | 设置（或查询） AFG phase. | `AFG:PHASe <NR3>`<br>`AFG:PHASe?` |
| `AFG:PULse:WIDth` | 设置（或查询） AFG pulse width, in seconds. | `AFG:PULse:WIDth <NR3>`<br>`AFG:PULse:WIDth?` |
| `AFG:RAMP:SYMmetry` | 设置（或查询） AFG ramp symmetry as a percentage. | `AFG:RAMP:SYMmetry <NR3>`<br>`AFG:RAMP:SYMmetry?` |
| `AFG:SQUare:DUty` | 设置（或查询） AFG duty cycle, as a percentage. | `AFG:SQUare:DUty <NR3>`<br>`AFG:SQUare:DUty?` |

## 4. 别名 (Alias)

手册原名：*Alias Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ALIas:CATalog?` | 【返回/查询】Returns a list of the currently defined alias labels。 | `ALIas:CATalog?` |
| `ALIas:DEFine` | 【操作】Assigns a sequence of program messages to an alias label。 | `ALIas:DEFine <QString><,>{<QString>\|<Block>}` |
| `ALIas:DELEte:ALL` | 【操作】Deletes all existing aliases。 | `ALIas:DELEte:ALL` |
| `ALIas:DELEte[:NAMe]` | 【控制】Removes a specified alias。 | `ALIas:DELEte[:NAMe] <QString>` |
| `ALIas[:STATE]` | 本指令设置 alias state。 | `ALLEv?` |

## 5. 任意波形 (ARB)

手册原名：*ARB Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `AFG:ARBitrary:ARB<x>:DATE?` | 【返回/查询】Returns the date that the data in the specified arbitrary waveform slot 1-4 was saved. | `AFG:ARBitrary:ARB<x>:DATE?`<br>`AFG:ARBitrary:ARB<x>:LABel <QString>`<br>`AFG:ARBitrary:ARB<x>:LABel?` |
| `AFG:ARBitrary:ARB<x>:LABel` | 设置（或查询） 波形 label for arbitrary 波形 slots 1- 4. | `AFG:ARBitrary:ARB<x>:DATE?`<br>`AFG:ARBitrary:ARB<x>:LABel <QString>`<br>`AFG:ARBitrary:ARB<x>:LABel?` |
| `AFG:ARBitrary:ARB<x>:TIMe?` | 【返回/查询】Returns the time that the data in the specified arbitrary waveform slot was saved. | `AFG:ARBitrary:ARB<x>:DATE?`<br>`AFG:ARBitrary:ARB<x>:LABel <QString>`<br>`AFG:ARBitrary:ARB<x>:LABel?` |
| `AFG:ARBitrary:EMEM:FUNCtion?` | 返回当前ly selected arbitrary 波形 pre-defined function. | `AFG:ARBitrary:EMEM:FUNCtion?` |
| `AFG:ARBitrary:EMEM:GENerate` | 【控制】This command generates the arbitrary waveform function specified by the enumeration argument, with the number of points optionally specified by the NR1 argument. To query the arbitrary waveform function set by this command, use AFG:ARBitrary:EMEM:FUNCtion? | `AFG:ARBitrary:EMEM:GENerate`<br>`AFG:ARBitrary:EMEM:GENerate?` |
| `AFG:ARBitrary:EMEM:NUMPoints?` | 返回数量： points in the AFB arbitrary 波形 edit memory. | `AFG:ARBitrary:EMEM:NUMPoints?` |
| `AFG:ARBitrary:EMEM:POINTS` | 【设置】Specifies which points to load into the AFG arbitrary waveform edit memory. | `AFG:ARBitrary:EMEM:POINTS <BlockWfmInDTO> \|<NrfWfmInDTO>`<br>`AFG:ARBitrary:EMEM:POINTS?` |
| `AFG:ARBitrary:EMEM:POINTS:BYTEORDer` | 【设置】This command specifies the byte order for the :AFG:ARBitrary:EMEM:POINTS? query when the :AFG:ARBitrary:EMEM:ENCDG is set to BINary and when binary block data is sent for the :AFG:ARBitrary:EMEM:POINTS command. | `AFG:ARBitrary:EMEM:POINTS:BYTEORDer <LSB> \|<MSB>`<br>`AFG:ARBitrary:EMEM:POINTS:BYTEORDer?` |
| `AFG:ARBitrary:EMEM:POINTS:ENCdg` | 【设置】This command specifies the data encoding format for the AFG:ARBitrary:EMEM:POINTS query (either ASCII or binary). | `AFG:ARBitrary:EMEM:POINTS:ENCdg {ASCii\|BINary}`<br>`AFG:ARBitrary:EMEM:POINTS:ENCdg?` |

## 6. 总线解码 (Bus)

手册原名：*Bus Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `BUS?` | 【返回/查询】Returns the parameters for each serial (if installed) and parallel bus. | `BUS?` |
| `BUS:B<x>:ARINC429A:BITRate` | 本指令设置 bit rate for the ARINC429 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:ARINC429A:DATA:FORMAT` | 【设置】This command specifies the size of the DATA field in an ARINC429 packet. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:ARINC429A:POLarity` | 【控制】This command sets the ARINC429 bus polarity to normal or inverted. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:ARINC429A:SOUrce` | 【设置】This command specifies the source for differential input for the ARINC429 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:BITDelay` | 本指令设置 number of delay bits for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:BITOrder` | 本指令设置 bit order for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:CHANnel:SIZe` | 本指令设置 number of bits per 通道 for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:CLOCk:POLarity` | 本指令设置 clock polarity for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:CLOCk:SOUrce` | 本指令设置 clock source 波形 for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:DATa:POLarity` | 本指令设置 data polarity for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:DATa:SIZe` | 本指令设置 number of bits per word for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:DATa:SOUrce` | 本指令设置 data source 波形 for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:DISplay:FORMat` | 本指令设置 显示 format for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:FRAME:SIZe` | 本指令设置 number of 通道s in each frame for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:FRAMESync:POLarity` | 本指令设置 frame sync polarity for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:FRAMESync:SOUrce` | 本指令设置 frame sync source 波形 for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:TYPe` | 本指令设置 audio format (type) for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:WORDSel:POLarity` | 本指令设置 word select polarity for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:AUDio:WORDSel:SOUrce` | 本指令设置 word select source 波形 for the AUDIO bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:FD:BITRate` | 【设置】This command specifies the bit rate for the data phase of a CAN FD packet。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:FD:STANDard` | 【设置】This command specifies the CAN FD standard: ISO (11898-1:2015) or non-ISO (Bosch:2012)。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:BITRate` | 本指令设置 bit rate for the CAN bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:PRObe` | 本指令设置 probing method for the CAN bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:SAMPLEpoint` | 【设置】This command specifies the sample point (in %) to sample during each bit period。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:STANDard` | 【设置】This command specifies CAN or CAN FD bust standard: CAN 2.0 or CAN FD。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:CAN:SOUrce` | 本指令设置 CAN bus data source。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:DISplay:FORMat` | 【设置】Sets the display format for the numerical information in the specified bus waveform。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:DISplay:TYPe` | 【设置】Sets the display type for the specified bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:FLEXray:BITRate` | 本指令设置 bit rate for the FlexRay bus signal。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:FLEXray:CHannel` | 本指令设置 FlexRay bus ID format。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:FLEXray:SIGnal` | 本指令设置 FlexRay bus standard。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:FLEXray:SOUrce` | 本指令设置 FlexRay bus data source。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:I2C:ADDRess:RWINClude` | 【设置】Sets and returns whether the read/write bit is included in the address。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:I2C{:CLOCk\|:SCLk}:SOUrce` | 本指令设置 I2C bus SCLK source。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:I2C{:DATa\|:SDAta}:SOUrce` | 本指令设置 I2C bus SDATA source。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LABel` | 本指令设置 波形 label for the specified bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:BITRate` | 本指令设置 bit rate for the LIN bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:IDFORmat` | 本指令设置 LIN bus ID format。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:POLarity` | 本指令设置 LIN bus polarity。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:SAMPLEpoint` | 【设置】This command specifies the point to sample during each bit period, as a percent, for the LIN bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:SOUrce` | 本指令设置 LIN bus data source。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:LIN:STANDard` | 本指令设置 LIN bus standard。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:MIL1553B:POLarity` | 【控制】This command sets the MIL-STD-1553 bus polarity to normal or inverted. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:MIL1553B:RESPonsetime:MAXimum` | 【设置】This command specifies the maximum response time to a valid command issued for the MIL-STD-1553 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:MIL1553B:RESPonsetime:MINimum` | 【设置】This command specifies the minimum response time to a valid command issued for the MIL-STD-1553 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:MIL1553B:SOUrce` | 【设置】This command specifies the source for differential input for the MIL-STD-1553 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:PARallel:BIT<x>:SOUrce` | 本指令设置 bit source for the parallel bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:PARallel:CLOCk:EDGE` | 本指令设置 clock edge for the parallel bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:PARallel:CLOCk:ISCLOCKed` | 【设置】This command specifies the state of the clock function for the parallel bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:PARallel:CLOCk:SOUrce` | 本指令设置 clock source 波形 for the parallel bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:PARallel:WIDth` | 【设置】This command specifies the number of bits to use for the width of the parallel bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:POSition` | 本指令设置 position of the bus 波形 on the 显示. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:BITRate` | 本指令设置 bit rate for the RS-232 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:DATABits` | 【设置】This command specifies the number of bits in the data frame for the RS-232 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:DELIMiter` | 【设置】This command specifies the delimiting value for a packet on the RS-232 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:DISplaymode` | 本指令设置 显示 mode for the RS-232 bus (frame or packet). | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:PARity` | 本指令设置 parity for the RS-232 bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:POLarity` | 本指令设置 polarity for the RS-232C bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:RX:SOUrce` | 本指令设置 RX source 波形 for the RS-232 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:RS232C:TX:SOUrce` | 本指令设置 TX source 波形 for the RS-232 bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:BITOrder` | 本指令设置 bit order for the SPI bus。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI{:CLOCk\|:SCLk}:POLarity` | 本指令设置 SCLK polarity for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI{:CLOCk\|:SCLk}:SOUrce` | 本指令设置 SCLK source for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:DATa{:IN\|:MISO}:POLarity` | 本指令设置 MISO polarity for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:DATa{:IN\|:MISO}:SOUrce` | 本指令设置 MISO source for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:DATa{:OUT\|:MOSI}:POLarity` | 本指令设置 MOSI polarity for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:DATa{:OUT\|:MOSI}:SOUrce` | 本指令设置 MOSI source for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:DATa:SIZe` | 【设置】This command specifies the number of bits per word (data size) for the specified SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:FRAMING` | 本指令设置 type of framing to use for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI:IDLETime` | 本指令设置 idle time, in seconds, for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI{:SELect\|:SS}:POLarity` | 本指令设置 polarity for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:SPI{:SELect\|:SS}:SOUrce` | 本指令设置 source 波形 for the SPI bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:STATE` | 本指令设置 on/off state of the bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:TYPe` | 本指令设置 bus type。 | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:USB:BITRate` | 本指令设置 bit rate for the USB bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:USB:PRObe` | 本指令设置 type of probe connected to the USB bus. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:USB:SOUrce:DIFFerential` | 【设置】This command specifies the source waveform for the eUSB bus when using a differential probe. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:USB:SOUrce:DMINus` | 本指令设置 source 波形 for the USB bus D- input. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:B<x>:USB:SOUrce:DPLUs` | 本指令设置 source for the USB D+ input. | `BUS:B<x>:ARINC429A:BITRate {LOW\|HI\|<NR1>}`<br>`BUS:B<x>:ARINC429A:BITRate?`<br>`BUS:B<x>:ARINC429A:DATA:FORMAT {DATA\|SDIDATA\|SDIDATASSM}` |
| `BUS:LOWerthreshold:CH<x>` | 本指令设置 lower threshold for each 通道. BUS:LOWerthreshold{:MATH\|: MATH1} 本指令设置 lower threshold for the 数学波形. | `BUS:LOWerthreshold:CH<x> {<NR3>\|ECL\|TTL}`<br>`BUS:LOWerthreshold:CH<x>?` |
| `BUS:LOWerthreshold:REF<x>` | 【控制】This command sets the lower threshold for each reference waveform. | `BUS:LOWerthreshold:REF<x> {<NR3>\|ECL\|TTL}`<br>`BUS:LOWerthreshold:REF<x>?` |
| `BUS:THReshold:CH<x>` | 本指令设置 threshold for a 通道. | `BUS:THReshold:CH<x> {ECL\|TTL\|<NR3>}`<br>`BUS:THReshold:CH<x>?` |
| `BUS:THReshold:D<x>` | 本指令设置 threshold for a digital 通道. | `BUS:THReshold:D<x> {<NR3>\|ECL\|TTL}`<br>`BUS:THReshold:D<x>?` |
| `BUS:UPPerthreshold:CH<x>` | 本指令设置 upper threshold for each 通道. BUS:UPPerthreshold{:MATH\|: MATH1} 本指令设置 upper threshold for the 数学波形. | `BUS:UPPerthreshold:CH<x> {<NR3>\|ECL\|TTL}`<br>`BUS:UPPerthreshold:CH<x>?` |
| `BUS:UPPerthreshold:REF<x>` | 【控制】This command sets the upper threshold for each reference waveform. | `BUS:UPPerthreshold:REF<x> {<NR3>\|ECL\|TTL}`<br>`BUS:UPPerthreshold:REF<x>?` |

## 7. 校准与诊断 (Calibration/Diagnostic)

手册原名：*Calibration and Diagnostic Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `*CAL?` | Instructs the 示波器 to perform self-校准 and returns the 示波器 self 校准 status. | `*CAL?` |
| `CALibrate:FACtory:STATus?` | 返回 出厂 校准 status value 保存d in nonvolatile memory. | `CALibrate:FACtory:STATus?` |
| `CALibrate:FACtory:STATus:AFG?` | 【返回/查询】Returns th e factory calibration status for the Arbitrary Function Generator portion of the instrument, if present. | `CALibrate:FACtory:STATus:AFG?` |
| `CALibrate:FACtory:STATus:RF?` | 【返回/查询】Returns the factory calibration status value saved in nonvolatile memory for the RF portion of the oscilloscope. Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:FACtory:STATus:RF? Returns PASS:showstheRFportionofthefactorycalibrationhassucceeded.` |
| `CALibrate:FACtory:STATus:SCOPE?` | 返回 出厂 校准 status value 保存d in nonvolatile memory for the non-RF portion of the 示波器. | `CALibrate:FACtory:STATus:SCOPE? Returns PASS:showsthenon-RFportionofthefactorycalibrationhassucceeded.` |
| `CALibrate:INTERNal` | 【启停】Starts a signal path compensation。 | `CALibrate:INTERNal` |
| `CALibrate:INTERNal:STARt` | 【启停】Starts the internal signal path calibration. | `CALibrate:INTERNal:STARt` |
| `CALibrate:INTERNal:STATus?` | 返回当前 status of the internal signal path 校准. | `CALibrate:INTERNal:STATus?` |
| `CALibrate:INTERNal:STATus:RF?` | 【控制】This query returns the status of the last SPC run for the RF portion of the instrument: (doesn't include the analog channels). Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:RESults:SPC:RF? Returns This query will return one of the following:INIT indicatestheRFportionoftheinstrumenthasnotbeencalibrated.` |
| `CALibrate:INTERNal:STATus:SCOPE?` | 【控制】This query returns the status of the last SPC run for the oscilloscope portion of the instrument (doesn't include the RF portion). | `CALibrate:RESults:SPC:SCOPE? Returns This query will return one of the following:INIT indicatestheoscilloscopeportiono ftheinstrumenthasnotbeencalibra ted.` |
| `CALibrate:RESults?` | 【返回/查询】Returns the status of all calibration subsystems without performing an SPC operation. | `CALibrate:RESults?` |
| `CALibrate:RESults:FACtory?` | 返回 status of internal and 出厂 校准. | `CALibrate:RESults:FACtory?` |
| `CALibrate:RESults:FACtory:AFG?` | 【控制】This query returns the factory calibration status for the Arbitrary Function Generator portion of the instrument, if present. | `CALibrate:RESults:FACtory:AFG?` |
| `CALibrate:RESults:FACtory:RF?` | 【控制】This query returns the factory calibration status for the RF portion of the instrument, if present. Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:RESults:FACtory:RF?` |
| `CALibrate:RESults:FACtory:SCOPE?` | 本查询返回 出厂 校准 status for the 示波器 (doesn't include RF or AFG) of the instrument. | `CALibrate:RESults:FACtory:SCOPE?` |
| `CALibrate:RESults:SPC?` | 【返回/查询】Returns the results of the last SPC operation. | `CALibrate:RESults:SPC?` |
| `CALibrate:RESults:SPC:RF?` | 【控制】This query returns the status of the last SPC run for the RF portion of the instrument (doesn't include analog channels) . This query is synonymous with CALibrate:INTERNal:STATus:RF? Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:INTERNal:STATus:RF? Returns This query will return one of the following:INIT indicatestheRFportionoftheinstrumenthasnotbeencalibrated.` |
| `CALibrate:RESults:SPC:SCOPE?` | 【控制】This query returns the status of the last SPC run for the oscilloscope portion of the instrument (doesn't include the RF portion). This query is synonymous to CALibrate:INTERNal:STATus:SCOPE? | `CALibrate:RESults:SPC:SCOPE?` |
| `CALibrate:RF` | 【控制】This command begins the RF calibration process. Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:RF` |
| `CALibrate:RF:STARt` | 【控制】This command is identical to CALIBRATE:RF. Available on mixed domain oscilloscope models with RF enabled. | `CALibrate:RF:STARt` |
| `CALibrate:RF:STATus?` | 本查询返回 status of the last RF 校准. Available on mixed domain 示波器 models with RF enabled. | `CALibrate:RF:STATus?` |
| `DIAg:LOOP:OPTion` | 【设置】Sets the self-test loop option. | `DIAg:LOOP:OPTion {ALWAYS\|FAIL\|ONFAIL\|ONCE\|NTIMES}` |
| `DIAg:LOOP:OPTion:NTIMes` | 【设置】Sets the self-test loop option to run N times. | `DIAg:LOOP:OPTion:NTIMes <NR1>`<br>`DIAg:LOOP:OPTion:NTIMes?` |
| `DIAg:LOOP:STOP` | 【控制】Stops the self-test at the end of the current loop. | `DIAg:LOOP:STOP` |
| `DIAg:RESUlt:FLAg?` | 【返回/查询】Returns the pass/fail status from the last self-test sequence execution. | `DIAg:RESUlt:FLAg?` |
| `DIAg:RESUlt:LOG?` | 【返回/查询】Returns the internal results log from the last self-test sequence execution. | `DIAg:RESUlt:LOG?` |
| `DIAg:SELect` | 【控制】Runs self tests on the specified system subsystem. | `DIAg:SELect` |
| `DIAg:SELect:<function>` | 【设置】Specifies which of the subsystems will be tested when the DIAg:STATE EXECute command is run. | `DIAg:SELect:<function>` |
| `DIAg:STATE` | Sets the 示波器 operating state. | `DIAg:STATE {EXECute\|ABORt}` |

## 8. 配置 (Configuration)

手册原名：*Configuration Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `CONFIGuration:ADVMATH?` | 【控制】This query returns a boolean value to indicate whether the instrument supports the advanced math feature. | `CONFIGuration:ADVMATH?` |
| `CONFIGuration:AFG?` | 【控制】Indicates whether or not the arbitrary function generator hardware is present, and the arbitrary function generation feature is enabled. | `CONFIGuration:AFG? Returns 1 indicatesthatthearbitraryfunctiongeneratorhardwareispresentandt heAFG` |
| `CONFIGuration:ANALOg:BANDWidth?` | 本查询返回 maximum bandwidth for analog 通道s. | `CONFIGuration:ANALOg:BANDWidth?` |
| `CONFIGuration:ANALOg:MAXBANDWidth?` | 返回最大 bandwidth for analog 通道s. | `CONFIGuration:ANALOg:MAXBANDWidth?` |
| `CONFIGuration:ANALOg:MAXSAMPLERate?` | 本查询返回 maximum 采样率 for analog 通道s. | `CONFIGuration:ANALOg:MAXSAMPLERate?` |
| `CONFIGuration:ANALOg:NUMCHANnels?` | 本查询返回 number of analog 通道s. | `CONFIGuration:ANALOg:NUMCHANnels? Returns <NR1>` |
| `CONFIGuration:ANALOg:RECLENS?` | 本查询返回 a list of supported 记录长度s for analog 通道s. | `CONFIGuration:ANALOg:RECLENS?` |
| `CONFIGuration:ANALOg:VERTINVert?` | 【控制】This query returns a boolean value to indicate whether the instrument supports the vertical invert feature for analog channels. | `CONFIGuration:ANALOg:VERTINVert?` |
| `CONFIGuration:APPLications:CUSTOMMask?` | 【控制】Indicates whether the Custom Mask test feature is present and enabled. | `CONFIGuration:APPLications:CUSTOMMask? Returns 1 indicatesthattheCustomMasktestfeatureispresentandenabled.` |
| `CONFIGuration:APPLications:LIMITMask?` | 【控制】This query returns a boolean value to indicate whether the instrument supports the mask/limit test application feature. | `CONFIGuration:APPLications:LIMITMask?` |
| `CONFIGuration:APPLications:POWer?` | 【控制】This query returns a boolean value to indicate whether the optional power application feature is present. | `CONFIGuration:APPLications:POWer?` |
| `CONFIGuration:APPLications:VIDPIC?` | 【控制】Indicates whether the Video Picture feature is present and enabled. | `CONFIGuration:APPLications:VIDPIC?` |
| `CONFIGuration:ARB?` | 【控制】Indicates whether or not the arbitrary function generator hardware is present, and the user-defined arbitrary waveform generation feature is enabled. Note that this is different than the。 | `CONFIGuration:ARB?` |
| `CONFIGuration:AUXIN?` | 【控制】This query returns a boolean value to indicate whether the instrument has an Aux Input connector. | `CONFIGuration:AUXIN?` |
| `CONFIGuration:BUSWAVEFORMS:ARINC429A?` | 【控制】This query returns a boolean value to indicate whether the optional ARINC429 bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:ARINC429A?` |
| `CONFIGuration:BUSWAVEFORMS:AUDIO?` | 【控制】This query returns a boolean value to indicate whether the optional audio bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:AUDIO?` |
| `CONFIGuration:BUSWAVEFORMS:CAN?` | 【控制】This query returns a boolean value to indicate whether the optional CAN bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:CAN?` |
| `CONFIGuration:BUSWAVEFORMS:CANFD?` | 【控制】This query returns a boolean value to indicate whether the optional CAN FD bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:CANFD?` |
| `CONFIGuration:BUSWAVEFORMS:ETHERNET?` | 【控制】This query returns a boolean value to indicate whether the optional Ethernet triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:ETHERNET?` |
| `CONFIGuration:BUSWAVEFORMS:FLEXRAY?` | 【控制】This query returns a boolean value to indicate whether the optional FlexRay bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:FLEXRAY?` |
| `CONFIGuration:BUSWAVEFORMS:I2C?` | 【控制】This query returns a boolean value to indicate whether the optional I2C bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:I2C?` |
| `CONFIGuration:BUSWAVEFORMS:LIN?` | 【控制】This query returns a boolean value to indicate whether the optional LIN bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:LIN?` |
| `CONFIGuration:BUSWAVEFORMS:MIL1553B?` | 【控制】This query returns a boolean value to indicate whether the optional MIL-STD-1553 bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:MIL1553B?` |
| `CONFIGuration:BUSWAVEFORMS:NUMBUS?` | 本查询返回 number of bus 波形. | `CONFIGuration:BUSWAVEFORMS:NUMBUS? Returns <NR1>` |
| `CONFIGuration:BUSWAVEFORMS:PARallel?` | 【控制】This query returns a boolean value to indicate whether the parallel bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:PARallel?` |
| `CONFIGuration:BUSWAVEFORMS:RS232C?` | 【控制】This query returns a boolean value to indicate whether the optional RS232 bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:RS232C?` |
| `CONFIGuration:BUSWAVEFORMS:SPI?` | 【控制】This query returns a boolean value to indicate whether the optional SPI bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:SPI?` |
| `CONFIGuration:BUSWAVEFORMS:USB?` | 【控制】This query returns a boolean value to indicate whether the USB bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:USB?` |
| `CONFIGuration:BUSWAVEFORMS:USB:HS?` | 【控制】This query returns a boolean value to indicate whether the high-speed USB bus triggering and analysis feature is present. | `CONFIGuration:BUSWAVEFORMS:USB:HS?` |
| `CONFIGuration:DIGITAl:MAGnivu?` | 【控制】This query returns a boolean value to indicate whether the instrument supports the MagniVu feature for digital channels. If there are no digital channels, the value returned is 0. | `CONFIGuration:DIGITAl:MAGnivu?` |
| `CONFIGuration:DIGITAl:MAXSAMPLERate?` | 本查询返回 maximum 采样率 for digital 通道s, in samples per second. | `CONFIGuration:DIGITAl:MAXSAMPLERate?` |
| `CONFIGuration:DIGITAl:NUMCHANnels?` | 本查询返回 number of digital 通道s. | `CONFIGuration:DIGITAl:NUMCHANnels? Returns <NR1>` |
| `CONFIGuration:DVM?` | 【控制】Indicates whether the Digital Voltmeter hardware is present. and the DVM feature is enabled. | `CONFIGuration:DVM? Returns 1 indicatesthattheDigitalVoltmeterha rdwareispresentandtheDVMfeatu reis` |
| `CONFIGuration:EXTVIDEO?` | 【控制】This query returns a boolean value to indicate whether the optional extended video trigger features are present. | `CONFIGuration:EXTVIDEO?` |
| `CONFIGuration:HISTOGRAM?` | 【控制】This query returns a boolean value to indicate whether the waveform histogram feature is present. | `CONFIGuration:HISTOGRAM?` |
| `CONFIGuration:NETWORKDRIVES?` | 本查询返回 a boolean value to indicate whether 网络驱动器s are present. | `CONFIGuration:NETWORKDRIVES?` |
| `CONFIGuration:NUMMEAS?` | 本查询返回 number of periodic 测量s. | `CONFIGuration:NUMMEAS?` |
| `CONFIGuration:REFS:NUMREFS?` | 本查询返回 number of 参考 波形. | `CONFIGuration:REFS:NUMREFS?` |
| `CONFIGuration:RF:BANDWidth?` | 【控制】This query returns the maximum bandwidth, in hertz, for RF channels. If there are no RF channels, the value returned is 0. | `CONFIGuration:RF:BANDWidth? Returns Floatingpointnumberthatrepresents thebandwidth,inHz,fortheRFchan nel.` |
| `CONFIGuration:RF:NUMCHANnels?` | 本查询返回 number of RF 通道s present. | `CONFIGuration:RF:NUMCHANnels?` |
| `CONFIGuration:ROSC?` | 【控制】This query returns a boolean value to indicate whether the instrument has an external reference oscillator (ROSC) input. | `CONFIGuration:ROSC? Returns <NR1> = 1 if a ROSC input is present.`<br>`<NR1> = 0 if a ROSC input is not present.` |

## 9. 光标 (Cursor)

手册原名：*Cursor Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `CURSor?` | 【返回/查询】Returns curso r settings。 | `CURSor?` |
| `CURSor:DDT?` | 返回 光标 deltaY/deltaT (dY/dT) readout。 | `CURSor:DDT?` |
| `CURSor:FUNCtion` | 本指令设置 光标 type。 | `CURSor:FUNCtion {SCREEN\|WAVEform\|OFF}`<br>`CURSor:FUNCtion?` |
| `CURSor:HBArs?` | 返回hba r 光标 settings。 | `CURSor:HBArs?` |
| `CURSor:HBArs:DELT` | a? 返回hbars 光标s 垂直 difference。 | `CURSor:HBArs:DELT` |
| `CURSor:HBArs:POSITION<x>` | 本指令设置 hbar 光标<x> 垂直 position。 | `CURSor:HBArs:POSITION<x> <NR3>`<br>`CURSor:HBArs:POSITION<x>?` |
| `CURSor:HBArs:UNIts` | 返回h bar 光标 units。 | `CURSor:HBArs:UNIts {BASE\|PERcent}`<br>`CURSor:HBArs:UNIts?` |
| `CURSor:HBArs:USE` | Sets the 水平 bar 光标 测量 scale, for use with ratio 光标s。 | `CURSor:HBArs:USE {CURrent\|HALFgrat}` |
| `CURSor:MODe` | 本指令设置 whether 光标s move in unison or separately。 | `CURSor:MODe {TRACk\|INDependent}`<br>`CURSor:MODe?` |
| `CURSor:SOUrce` | 本指令设置 光标 source, which can be one of 通道s 1–4, 参考 波形 1–4, 数学波形, bus 1–4, digital 通道s 0– 15 (Requires option 3-MSO installed). | `CURSor:SOUrce {CH1\|CH2\|CH3\|CH4\|REF1\|REF2\|REF3\|REF4\|MATH`<br>`CURSor:SOUrce?` |
| `CURSor:VBArs?` | 本指令设置 position of 垂直 bar 光标s。 | `CURSor:VBArs?` |
| `CURSor:VBArs:ALTERNATE<x>?` | 返回 alternate readout for the 波形 (Vbar) 光标s。 | `CURSor:VBArs:ALTERNATE<x>?` |
| `CURSor:VBArs:DELTa?` | 返回 水平 difference between vbar 光标s。 | `CURSor:VBArs:DELTa?` |
| `CURSor:VBArs:HPOS<x>?` | 返回 垂直 value of the specified 垂直 bar tick。 | `CURSor:VBArs:HPOS<x>?` |
| `CURSor:VBArs:POSITION<x>` | 本指令设置 vbar 光标<x> 水平 position。 | `CURSor:VBArs:POSITION<x> <NR3>`<br>`CURSor:VBArs:POSITION<x>?` |
| `CURSor:VBArs:UNIts` | 本指令设置 水平 units for vbar 光标s。 | `CURSor:VBArs:UNIts {SEConds\|HERtz\|DEGrees\|PERcent}`<br>`CURSor:VBArs:UNIts?` |
| `CURSor:VBArs:USE` | Sets the 垂直 bar 光标 测量 scale。 | `CURSor:VBArs:USE` |
| `CURSor:VBArs:VDELTa?` | 返回 垂直 difference between the two 垂直 bar 光标 ticks。 | `CURSor:VBArs:VDELTa?` |
| `CURSor:XY:POLar:RADIUS:DELta?` | 【返回/查询】Returns the difference between the cursors X radius and the cursor Y radius。 | `CURSor:XY:POLar:RADIUS:DELta?` |
| `CURSor:XY:POLar:RADIUS:POSITION<x>?` | 返回 polar radius of the specified 光标。 | `CURSor:XY:POLar:RADIUS:POSITION<x>?` |
| `CURSor:XY:POLar:RADIUS:UNIts?` | 【返回/查询】Returns the polar radius units。 | `CURSor:XY:POLar:RADIUS:UNIts?` |
| `CURSor:XY:POLar:THETA:DELta?` | 返回 XY 光标 polar coordinate delta。 | `CURSor:XY:POLar:THETA:DELta?` |
| `CURSor:XY:POLar:THETA:POSITION<x>?` | 返回 光标 X or 光标 Y polar coordinate。 | `CURSor:XY:POLar:THETA:POSITION<x>?` |
| `CURSor:XY:POLar:THETA:UNIts?` | 返回 光标 polar coordinate units。 | `CURSor:XY:POLar:THETA:UNIts?` |
| `CURSor:XY:PRODUCT:DELta?` | 【返回/查询】Returns the difference between the cursors X position and cursor Y position。 | `CURSor:XY:PRODUCT:DELta?` |
| `CURSor:XY:PRODUCT:POSITION<x>?` | 返回 position of the X or Y 光标 used to calculate the X × Y 光标 测量。 | `CURSor:XY:PRODUCT:POSITION<x>?` |
| `CURSor:XY:PRODUCT:UNIts?` | 返回 XY 光标 product units。 | `CURSor:XY:PRODUCT:UNIts?` |
| `CURSor:XY:RATIO:DELta?` | 【返回/查询】Returns the ratio of the difference between the cursor X position and cursor Y position。 | `CURSor:XY:RATIO:DELta?` |
| `CURSor:XY:RATIO:POSITION<x>?` | 返回 X or Y position for the specified 光标。 | `CURSor:XY:RATIO:POSITION<x>?` |
| `CURSor:XY:RATIO:UNIts?` | 返回 X and Y 光标 units for the ratio 测量。 | `CURSor:XY:RATIO:UNIts?` |
| `CURSor:XY:READOUT` | 本指令设置 XY 光标 readout selection. | `CURSor:XY:READOUT {RECTangular\|POLARCord\|PRODuct\|RATio}`<br>`CURSor:XY:READOUT?` |
| `CURSor:XY:RECTangular:X:DELta?` | 【返回/查询】Returns the cursor X delta value in rectangular coordinates。 | `CURSor:XY:RECTangular:X:DELta?` |
| `CURSor:XY:RECTangular:X:POSITION<x>` | 本指令设置 光标 X rectangular coordinates。 | `CURSor:XY:RECTangular:X:POSITION<x> <NR3>`<br>`CURSor:XY:RECTangular:X:POSITION<x>?` |
| `CURSor:XY:RECTangular:X:UNIts?` | 【返回/查询】Returns the Cursor X rectangular units。 | `CURSor:XY:RECTangular:X:UNIts?` |
| `CURSor:XY:RECTangular:Y:DELta?` | 【返回/查询】Returns The cursor Y delta value in rectangular coordinates。 | `CURSor:XY:RECTangular:Y:DELta?` |
| `CURSor:XY:RECTangular:Y:POSITION<x>` | > 本指令设置 光标 Y rectangular coordinate。 | `CURSor:XY:RECTangular:Y:POSITION<x> <NR3>`<br>`CURSor:XY:RECTangular:Y:POSITION<x>?` |
| `CURSor:XY:RECTangular:Y:UNIts?` | 返回 光标 Y rectangular units。 | `CURSor:XY:RECTangular:Y:UNIts?` |

## 10. 显示 (Display)

手册原名：*Display Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `DISplay?` | 返回current 显示 settings。 | `DISplay?` |
| `DISplay:CLOCk` | 本指令设置 显示 of the date/time stamp。 | `DISplay:CLOCk {ON\|OFF\|<NR1>}`<br>`DISplay:CLOCk?` |
| `DISplay:CONFIGure:READOut` | 【控制】Configures or returns readout backgrounds. | `DISplay:CONFIGure:READOut{NORMal ! TRANSParent}`<br>`DISplay:CONFIGure:READOut?` |
| `DISplay:DIGital:ACTIVity` | 【设置】Sets or returns the state of the digital channel monitor display. | `DISplay:DIGital:ACTIVity {0\|1\|OFF\|ON}`<br>`DISplay:DIGital:ACTIVity?` |
| `DISplay:DIGital:HEIght` | 本指令设置 number of available digital 波形 position slots. | `DISplay:DIGital:HEIght {SMAll\|MEDium\|LARge}`<br>`DISplay:DIGital:HEIght?` |
| `DISplay:GRAticule` | 本指令设置 type of graticule that is 显示ed。 | `DISplay:GRAticule {CROSSHair\|FRAme\|FULl\|GRId\|SOLid}`<br>`DISplay:GRAticule?` |
| `DISplay:INTENSITy?` | 返回全部 显示 intensity settings。 | `DISplay:INTENSITy?`<br>`DISplay:INTENSITy:BACKLight {LOW\|MEDium\|HIGH}`<br>`DISplay:INTENSITy:BACKLight?` |
| `DISplay:INTENSITy:BACKLight` | 本指令设置 backlight intensity for the 显示。 | `DISplay:INTENSITy:BACKLight {LOW\|MEDium\|HIGH}`<br>`DISplay:INTENSITy:BACKLight?` |
| `DISplay:INTENSITy:BACKLight:AUTODim:ENAble` | 【设置】Sets or returns the state of the display auto-dim feature. The default is enabled. | `DISplay:INTENSITy:BACKLight:AUTODim:ENAble {OFF\|ON\|0\|1}`<br>`DISplay:INTENSITy:BACKLight:AUTODim:ENAble?` |
| `DISplay:INTENSITy:BACKLight:AUTODim:TIMe` | 【设置】Sets or returns the amount of time, in minutes, to wait for no UI activity before automatically dimming the display. | `DISplay:INTENSITy:BACKLight:AUTODim:TIMe <NR1>`<br>`DISplay:INTENSITy:BACKLight:AUTODim:TIMe?` |
| `DISplay:INTENSITy:GRAticule` | 本指令设置 graticule intensity for the 显示。 | `DISplay:INTENSITy:GRAticule <NR1>`<br>`DISplay:INTENSITy:GRAticule?` |
| `DISplay:INTENSITy:WAVEform` | 本指令设置 intensity of the 波形。 | `DISplay:INTENSITy:WAVEform <NR1>`<br>`DISplay:INTENSITy:WAVEform?` |
| `DISplay:PERSistence` | 本指令设置 显示 persistence for analog 波形. This affects the 显示 only. | `DISplay:PERSistence {<NR3>\|CLEAR\|AUTO\|INFInite\|OFF}`<br>`DISplay:PERSistence?` |
| `DISplay:STYle:DOTsonly` | 【开关】This command turns on or off the dots-only mode for the waveforms displayed in the time domain. | `DISplay:STYle:DOTsonly {ON\|OFF\|<NR1>}`<br>`DISplay:STYle:DOTsonly?` |
| `DISplay:TRIGFrequency` | 【开关】This command switches the trigger frequency readout on or off. | `DISplay:TRIGFrequency {OFF\|ON\|0\|1}`<br>`DISplay:TRIGFrequency?` |
| `DISplay:XY` | This command turns 开或关 the XY 显示 mode. | `DISplay:XY {OFF\|TRIGgered}`<br>`DISplay:XY?` |
| `DISplay:XY:WITHYT` | 【设置】Sets or returns the state of simultaneous display of the XY and YT waveforms when in TRIGgered XY display mode. When both are displayed, the YT waveform is displayed in the upper graticule, and the XY waveform is displayed in the lower graticule. | `DISplay:XY:WITHYT {0\|1\|OFF\|ON}`<br>`DISplay:XY:WITHYT?` |
| `MESSage` | 【设置或查询】Sets or queries message box (screen annotation) parameters。 | `MESSage`<br>`MESSage?` |
| `MESSage:BOX` | 本指令设置 coordinates of the message box。 | `MESSage:BOX <X1>,<Y1>[,<X2>,<Y2>]`<br>`MESSage:BOX?` |
| `MESSage:CLEAR` | 【控制】Clears the contents of the message box. | `MESSage:CLEAR` |
| `MESSage:SHOW` | 本指令设置 contents of the message box。 | `MESSage:SHOW <QString>`<br>`MESSage:SHOW?` |
| `MESSage:STATE` | 【控制】Controls the display of the message box。 | `MESSage:STATE {OFF\|ON\|<NR1>}`<br>`MESSage:STATE?` |
| `MESSage:MESSAGE1<x>:BOX` | 本指令设置 coordinates of the message box。 | `MESSage:MESSAGE1<x>:BOX <X1>,<Y1>[,<X2>,<Y2>]`<br>`MESSage:MESSAGE1<x>:BOX?`<br>`MESSage:MESSAGE1<x>:CLEAR` |
| `MESSage:MESSAGE1<x>:CLEAR` | 【控制】Clears the contents of the message box。 | `MESSage:MESSAGE1<x>:BOX <X1>,<Y1>[,<X2>,<Y2>]`<br>`MESSage:MESSAGE1<x>:BOX?`<br>`MESSage:MESSAGE1<x>:CLEAR` |
| `MESSage:MESSAGE1<x>:SHOW` | 本指令设置 contents of the message box。 | `MESSage:MESSAGE1<x>:BOX <X1>,<Y1>[,<X2>,<Y2>]`<br>`MESSage:MESSAGE1<x>:BOX?`<br>`MESSage:MESSAGE1<x>:CLEAR` |
| `MESSage:MESSAGE1<x>:STATE` | 【控制】Controls the display of the message box。 | `MESSage:MESSAGE1<x>:BOX <X1>,<Y1>[,<X2>,<Y2>]`<br>`MESSage:MESSAGE1<x>:BOX?`<br>`MESSage:MESSAGE1<x>:CLEAR` |

## 11. 数字万用表 (DVM)

手册原名：*DVM Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `DVM` | 【控制】Resets the Digital Voltmeter measurements and history。 | `DVM {RESET}` |
| `DVM:AUTORange` | 设置（或查询） auto range state for the Digital Voltmeter. | `DVM:AUTORange {0\|1\|OFF\|ON}`<br>`DVM:AUTORange?` |
| `DVM:DISPLAYSTYle` | 设置（或查询） 显示 style for the Digital Voltmeter. | `DVM:DISPLAYSTYle {FULl\|MINimum}`<br>`DVM:DISPLAYSTYle?` |
| `DVM:MEASUrement:FREQuency?` | 【返回/查询】Returns the current frequency value for the Digital Voltmeter. | `DVM:MEASUrement:FREQuency?` |
| `DVM:MEASUrement:HIStory:AVErage?` | 【返回/查询】Returns the average readout value for the Digital Voltmeter function over the history period. | `DVM:MEASUrement:HIStory:AVErage?` |
| `DVM:MEASUrement:HIStory:MAXimum?` | 【返回/查询】Returns the maximum readout value for the DVM function over the history period. | `DVM:MEASUrement:HIStory:MAXimum?` |
| `DVM:MEASUrement:HIStory:MINImum?` | 【返回/查询】Returns the minimum readout v alue for the DVM function over the history period. | `DVM:MEASUrement:HIStory:MINImum?` |
| `DVM:MEASUrement:INFMAXimum?` | 【返回/查询】Returns the maximum DVM reado ut value over the entire time that the DVM has been on since the last change using the DVM:MODe or DVM:SOUrce commands or DVM RESET . | `DVM:MEASUrement:INFMAXimum?` |
| `DVM:MEASUrement:INFMINimum?` | 【返回/查询】Returns the minimum readout value of the DVM function over the entire time that the DVM has been on since the last change using the DVM:MODe or DVM:SOUrce commands or。 | `DVM:MEASUrement:INFMINimum?` |
| `DVM:MEASUrement:VALue?` | 【返回/查询】Returns the DVM readout va lue (the large displayed value at the top of the DVM screen). | `DVM:MEASUrement:VALue?` |
| `DVM:MODe` | 【设置】Specifies (or queries) the mode to use for the Digital Voltmeter (ACRMS, ACDCRMS, DC, Frequency, or OFF). | `DVM:MODe {ACRMS\|ACDCRMS\|DC\|FREQuency\|OFF}`<br>`DVM:MODe?` |
| `DVM:SOUrce` | 设置（或查询） source for the Digital Voltmeter: Channel 1 - 4. | `DVM:SOUrce {CH1\|CH2\|CH3\|CH4}`<br>`DVM:SOUrce?` |

## 12. 邮件 (Email)

手册原名：*Email Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `EMAIL:SETUp:FROMADDRess` | 【设置或查询】Sets (or queries) the sender’s email address for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands. | `EMAIL:SETUp:FROMADDRess <QString>`<br>`EMAIL:SETUp:FROMADDRess?` |
| `EMAIL:SETUp:HOSTALIASNAMe` | 【设置或查询】Sets (or queries) the email host alias name for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands. | `EMAIL:SETUp:HOSTALIASNAMe <QString>`<br>`EMAIL:SETUp:HOSTALIASNAMe?` |
| `EMAIL:SETUp:SMTPLOGIn` | 【设置】Sets or returns the email SMTP server login ID for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands. | `EMAIL:SETUp:SMTPLOGIn <QString>`<br>`EMAIL:SETUp:SMTPLOGIn?` |
| `EMAIL:SETUp:SMTPPASSWord` | 【设置】Sets the email SMTP server login password for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands. | `EMAIL:SETUp:SMTPPASSWord <QString>` |
| `EMAIL:SETUp:SMTPPort` | 【设置】Sets or returns the email SMTP server port number for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands。 | `EMAIL:SETUp:SMTPPort <NR1>`<br>`EMAIL:SETUp:SMTPPort?` |
| `EMAIL:SETUp:SMTPServer` | 【设置】Sets or returns the email SMTP server DNS name for the common server setup information that is shared between the Act on Event commands and the Hardcopy Email commands. | `EMAIL:SETUp:SMTPServer <QString>`<br>`EMAIL:SETUp:SMTPServer?` |

## 13. 以太网 (Ethernet)

手册原名：*Ethernet Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ETHERnet:DHCPbootp` | 本指令设置 network initialization 搜索 for a DHCP/BOOTP server。 | `ETHERnet:DHCPbootp` |
| `ETHERnet:DNS:IPADDress` | 【设置】This command specifies the network Domain Name Server (Dns) IP address。 | `ETHERnet:DNS:IPADDress <QString>`<br>`ETHERnet:DNS:IPADDress?` |
| `ETHERnet:DOMAINname` | 本指令设置 network domain name。 | `ETHERnet:DOMAINname` |
| `ETHERnet:ENET:ADDress?` | 返回 Ethernet address value assigned to the 示波器。 | `ETHERnet:ENET:ADDress?` |
| `ETHERnet:GATEWay:IPADDress` | 本指令设置 remote interface gateway IP address。 | `ETHERnet:GATEWay:IPADDress <QString>`<br>`ETHERnet:GATEWay:IPADDress?` |
| `ETHERnet:HTTPPort` | 本指令设置 remote interface HTTP port value。 | `ETHERnet:HTTPPort <QString>`<br>`ETHERnet:HTTPPort?` |
| `ETHERnet:IPADDress` | 本指令设置 IP address assigned to the 示波器。 | `ETHERnet:IPADDress <QString>`<br>`ETHERnet:IPADDress?` |
| `ETHERnet:NAME` | 本指令设置 network name assigned to the 示波器。 | `ETHERnet:NAME <QString>`<br>`ETHERnet:NAME?` |
| `ETHERnet:PASSWord` | 本指令设置 Ethernet access password。 | `ETHERnet:PASSWord <new>`<br>`ETHERnet:PASSWord?` |
| `ETHERnet:PING` | 【控制】Causes the oscilloscope to ping the gateway IP address。 | `ETHERnet:PING EXECute` |
| `ETHERnet:PING:STATus?` | 【返回/查询】Returns the results from pinging the gateway IP address。 | `ETHERnet:PING:STATus?` |
| `ETHERnet:SUBNETMask` | 设置或查询远程接口子网掩码。 | `ETHERnet:SUBNETMask <QString>`<br>`ETHERnet:SUBNETMask?` |

## 14. 文件系统 (File System)

手册原名：*File System Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `FILESystem?` | 返回文件系统相关设置。 | `FILESystem?` |
| `FILESystem:COPy` | 将指定文件复制为新文件。 | `FILESystem:COPy {<source QString>,<destination QString>}` |
| `FILESystem:CWD` | 设置或查询文件系统当前工作目录。 | `FILESystem:CWD {<new working directory path>}` |
| `FILESystem:DELEte` | 删除指定文件或目录。 | `FILESystem:DELEte <file path>` |
| `FILESystem:DIR?` | 返回当前目录内容列表。 | `FILESystem:DIR?` |
| `FILESystem:FORMat` | 格式化指定驱动器。 | `FILESystem:FORMat` |
| `FILESystem:FREESpace?` | 返回当前驱动器剩余空间（字节）。 | `FILESystem:FREESpace?` |
| `FILESystem:LDIR?` | 返回文件夹内所有文件与目录的分号分隔列表。 | `FILESystem:LDIR?` |
| `FILESystem:MKDir` | 创建新目录。 | `FILESystem:MKDir <directory path>` |
| `FILESystem:MOUNT:AVAILable?` | 返回可用于挂载网络驱动器的盘符列表（逗号分隔）。 | `FILESystem:MOUNT:AVAILable?` |
| `FILESystem:MOUNT:DRIve` | 挂载由引号字符串指定的网络驱动器。 | `FILESystem:MOUNT:DRIve <Qstring>` |
| `FILESystem:MOUNT:LIST?` | 返回已挂载网络驱动器列表（盘符、服务器、路径、类型）；无挂载时返回空字符串。 | `FILESystem:MOUNT:LIST?` |
| `FILESystem:READFile` | 读取指定文件内容并通过当前接口返回。 | `FILESystem:READFile <QString>` |
| `FILESystem:REName` | 将已有文件重命名为新名称。 | `FILESystem:REName <old file path>,<new file path>` |
| `FILESystem:RMDir` | 删除指定目录。 | `FILESystem:RMDir <directory path>` |
| `FILESystem:UNMOUNT:DRIve` | 卸载由引号字符串指定的网络驱动器。 | `FILESystem:UNMOUNT:DRIve QString` |
| `FILESystem:WRITEFile` | 将指定块数据写入示波器当前工作目录中的文件。 | `FILESystem:WRITEFile <file path>, <data>` |

## 15. 硬拷贝/截屏 (Hard Copy)

手册原名：*Hard Copy Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `HARDCopy` | 将屏幕显示内容发送到所选打印机。 | `HARDCopy {STARt}` |
| `HARDCopy:ACTIVeprinter` | 设置或查询当前活动打印机。 | `HARDCopy:ACTIVeprinter {<NR1>\|<name>}`<br>`HARDCopy:ACTIVeprinter?` |
| `HARDCopy:INKSaver` | 切换 InkSaver：在白底上打印彩色波形与格线。 | `HARDCopy:INKSaver {ON\|OFF\|<NR1>}`<br>`HARDCopy:INKSaver?` |
| `HARDCopy:LAYout` | 设置或查询硬拷贝页面方向。 | `HARDCopy:LAYout {PORTRait\|LANdscape}`<br>`HARDCopy:LAYout?` |
| `HARDCopy:PREVIEW` | 预览应用 InkSaver 调色板后的当前屏幕内容。 | `HARDCopy:PREVIEW {ON\|OFF\|<NR1>}` |
| `HARDCopy:PRINTer:ADD` | 向可用打印机列表添加网络或电子邮件打印机。 | `HARDCopy:PRINTer:ADD`<br>`<name>,<server>,<address>\|<emailaddress>` |
| `HARDCopy:PRINTer:DELete` | 从可用打印机列表移除网络打印机。 | `HARDCopy:PRINTer:DELete <name>` |
| `HARDCopy:PRINTer:LIST?` | 显示当前已定义的打印机列表。 | `HARDCopy:PRINTer:LIST?` |
| `HARDCopy:PRINTer:REName` | 重命名可用打印机列表中的网络或电子邮件打印机。 | `HARDCopy:PRINTer:REName` |

## 16. 水平时基 (Horizontal)

手册原名：*Horizontal Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `HORizontal?` | 返回水平系统相关设置。 | `HORizontal?` |
| `HORizontal:DELay:MODe` | 设置或查询水平延迟模式。 | `HORizontal:DELay:MODe {OFF\|ON\|<NR1>}`<br>`HORizontal:DELay:MODe?` |
| `HORizontal:DELay:TIMe` | 设置或查询水平延迟时间。 | `HORizontal:DELay:TIMe` |
| `HORizontal:DIGital:RECOrdlength:MAGnivu?` | 返回 MagniVu 数字采集的记录长度。 | `HORizontal:DIGital:RECOrdlength:MAGnivu?` |
| `HORizontal:DIGital:RECOrdlength:MAIn?` | 返回主数字采集的记录长度。 | `HORizontal:DIGital:RECOrdlength:MAIn?` |
| `HORizontal:DIGital:SAMPLERate:MAGnivu?` | 返回 MagniVu 数字采集的采样率。 | `HORizontal:DIGital:SAMPLERate:MAGnivu?` |
| `HORizontal:DIGital:SAMPLERate:MAIn?` | 返回主数字采集的采样率。 | `HORizontal:DIGital:SAMPLERate:MAIn?` |
| `HORizontal:POSition` | 设置水平位置（百分比；延迟关闭时使用）。 | `HORizontal:POSition <NR3>`<br>`HORizontal:POSition?` |
| `HORizontal:PREViewstate?` | 返回显示系统预览状态。 | `HORizontal:PREViewstate?` |
| `HORizontal:RECOrdlength` | 设置或查询记录长度。 | `HORizontal:RECOrdlength <NR1>`<br>`HORizontal:RECOrdlength?` |
| `HORizontal:SAMPLERate?` | 查询采样率（秒）。 | `HORizontal:SAMPLERate?` |
| `HORizontal:SCAle` | 设置或查询水平时基（秒/格）。 | `HORizontal:SCAle <NR3>`<br>`HORizontal:SCAle?` |

## 17. 标记 (Mark)

手册原名：*Mark Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `MARK` | 【控制】Move to the next or previous mark on the waveform or returns all learnable settings from the mark commands。 | `MARK {NEXT\|PREVious}`<br>`MARK?` |
| `MARK:CREATE` | 【操作】Creates a mark on a particular waveform or all wavefor ms in a column。 | `MARK:CREATE {CH1\|CH2\|CH3\|CH4\|MATH\|REF1` |
| `MARK:DELEte` | 【操作】Deletes a mark on a particular waveform, all wavefor ms in a column, or all marks。 | `MARK:DELEte` |
| `MARK:FREE?` | 【返回/查询】Returns how many marks are free to be used。 | `MARK:FREE?` |
| `MARK:SAVEALL` | 【控制】This command saves all current marks on waveforms i n the time domain to an internal memory location. (This is equivalent to pressing the “Save All Marks" button in the Search button menu on the front panel.) In order to retrieve the information, use the query form of MARK:USERLIST. | `MARK:SAVEALL TOUSER` |
| `MARK:SELected:END?` | 【返回/查询】Returns the end of the selected mark, in terms of 0 to 100% of the waveform。 | `MARK:SELected:END?` |
| `MARK:SELected:FOCUS?` | 【返回/查询】Returns the focus of the selected mark, in terms o f 0 to 100% of the waveform。 | `MARK:SELected:FOCUS?` |
| `MARK:SELected:MARKSINCOLumn?` | 【返回/查询】Returns how many marks are in the current zoom pixel column。 | `MARK:SELected:MARKSINCOLumn?` |
| `MARK:SELected:OWNer?` | 【返回/查询】Returns the owner of the selected mark。 | `MARK:SELected:OWNer? Returns <QString> is the owner of the mark.` |
| `MARK:SELected:SOURCe?` | 返回 source 波形 of the selected mark。 | `MARK:SELected:SOURCe?` |
| `MARK:SELected:STARt?` | 【返回/查询】Returns the start of the selected mark, in terms of 0 to 100% of the waveform。 | `MARK:SELected:STARt?` |
| `MARK:SELected:STATE?` | 返回 开或关 state of the selected mark。 | `MARK:SELected:STATE?` |
| `MARK:SELected:ZOOm:POSition?` | 【返回/查询】Returns the position of the selected mark, in terms of 0 to 100% of the upper window。 | `MARK:SELected:ZOOm:POSition?` |
| `MARK:TOTal?` | 【返回/查询】Returns how many marks are used。 | `MARK:TOTal?` |
| `MARK:USERLIST` | 【控制】The command creates a single user mark on a waveform in the time domain. The arguments consist of an enumeration specifying the source waveform, followed by 7 time mark parameters. You can create up to 1,024 marks. To save all the marks to memory, use the command。 | `MARK:USERLIST`<br>`MARK:USERLIST?` |

## 18. 数学运算 (Math)

手册原名：*Math Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `MATH[1]?` | 返回数学波形的定义与相关设置。 | `MATH[1]?` |
| `MATH[1]:AUTOSCale` | 设置或查询数学波形自动垂直缩放状态。 | `MATH[1]:AUTOSCale?`<br>`MATH[1]:AUTOSCale {0\|1\|OFF\|ON}` |
| `MATH[1]:DEFine` | 以文本字符串设置当前数学函数表达式。 | `MATH[1]:DEFine <QString>`<br>`MATH[1]:DEFine?` |
| `MATH[1]:HORizontal:POSition` | 设置 FFT 或（非实时）数学参考波形的水平显示位置。 | `MATH[1]:HORizontal:POSition <NR3>`<br>`MATH[1]:HORizontal:POSition?` |
| `MATH[1]:HORizontal:SCAle` | 设置 FFT 或双波形数学的水平显示时基。 | `MATH[1]:HORizontal:SCAle <NR3>`<br>`MATH[1]:HORizontal:SCAle?` |
| `MATH[1]:HORizontal:UNIts` | 返回数学波形水平单位。 | `MATH[1]:HORizontal:UNIts` |
| `MATH[1]:LABel` | 设置或查询数学波形标签。 | `MATH[1]:LABel <QString>`<br>`MATH[1]:LABel?` |
| `MATH[1]:SPECTral:MAG` | 设置数学字符串中频谱幅度的单位。 | `MATH[1]:SPECTral:MAG {LINEAr\|DB}`<br>`MATH[1]:SPECTral:MAG?` |
| `MATH[1]:SPECTral:WINdow` | 设置数学波形频谱输入数据的窗函数。 | `MATH[1]:SPECTral:WINdow`<br>`MATH[1]:SPECTral:WINdow?` |
| `MATH[1]:TYPe` | 设置数学波形类型（DUAL / FFT / ADVanced / SPECTRUM），需配合 MATH:DEFine。 | `MATH[1]:TYPe {DUAL\|FFT\|ADVanced\|SPECTRUM}`<br>`MATH[1]:TYPe?` |
| `MATH[1]:VERTical:POSition` | 设置当前数学类型的垂直位置。 | `MATH[1]:VERTical:POSition <NR3>`<br>`MATH[1]:VERTical:POSition?` |
| `MATH[1]:VERTical:SCAle` | 设置当前数学类型的垂直标度。 | `MATH[1]:VERTical:SCAle <NR3>`<br>`MATH[1]:VERTical:SCAle?` |
| `MATH[1]:VERTical:UNIts` | 返回数学波形垂直单位。 | `MATH[1]:VERTical:UNIts?` |
| `MATHVAR?` | 返回数学表达式中使用的全部数值变量。 | `MATHVAR?` |
| `MATHVAR:VAR<x>` | 设置可在数学表达式中使用的数值变量 VAR<x>。 | `MATHVAR:VAR<x> <NR3>`<br>`MATHVAR:VAR<x>?` |

## 19. 自动测量 (Measurement)

手册原名：*Measurement Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `MEASUrement?` | 返回全部 测量 parameters。 | `MEASUrement?` |
| `MEASUrement:CLEARSNapshot` | Removes the 测量 snapshot 显示。 | `MEASUrement:CLEARSNapshot` |
| `MEASUrement:GATing` | 本指令设置 测量 gating。 | `MEASUrement:GATing {OFF\|SCREen\|CURSor}`<br>`MEASUrement:GATing?` |
| `MEASUrement:IMMed?` | 返回全部 immediate 测量 setup parameters。 | `MEASUrement:IMMed?` |
| `MEASUrement:IMMed:DELay?` | 【返回/查询】Returns information about the immediate delay measurement。 | `MEASUrement:IMMed:DELay?` |
| `MEASUrement:IMMed:DELay:DIRection` | 本指令设置 搜索 direction to use for immediate delay 测量s。 | `MEASUrement:IMMed:DELay:DIRection {BACKWards\|FORWards}`<br>`MEASUrement:IMMed:DELay:DIRection?` |
| `MEASUrement:IMMed:DELay:EDGE<x>` | 本指令设置 slope of the edge used for immediate delay “from” and “to” 波形 测量s。 | `MEASUrement:IMMed:DELay:EDGE<x> {FALL\|RISe}`<br>`MEASUrement:IMMed:DELay:EDGE<x>?` |
| `MEASUrement:IMMed:SOUrce<x>` | 本指令设置 “from” source for all single 通道 immediate 测量s 本指令设置 source to measure “to” for two-通道 测量s。 | `MEASUrement:IMMed:SOUrce<x>`<br>`MEASUrement:IMMed:SOUrce<x>?` |
| `MEASUrement:IMMed:TYPe` | 本指令设置 type of the immediate 测量。 | `MEASUrement:IMMed:TYPe`<br>`MEASUrement:IMMed:TYPe?` |
| `MEASUrement:IMMed:UNIts?` | 返回 units of the immediate 测量。 | `MEASUrement:IMMed:UNIts? Returns This query returns one of the following strings.` |
| `MEASUrement:IMMed:VALue?` | 返回 value of the immediate 测量。 | `MEASUrement:IMMed:VALue?` |
| `MEASUrement:INDICators?` | 返回全部 测量 indicator parameters。 | `MEASUrement:INDICators?` |
| `MEASUrement:INDICators:HORZ<x>?` | 返回 position of the specified 水平 测量 indicator。 | `MEASUrement:INDICators:HORZ<x>?` |
| `MEASUrement:INDICators:NUMHORZ?` | 返回数量： 水平 测量 indicators currently being 显示ed。 | `MEASUrement:INDICators:NUMHORZ?` |
| `MEASUrement:INDICators:NUMVERT?` | 返回数量： 垂直 测量 indicators currently being 显示ed。 | `MEASUrement:INDICators:NUMVERT?` |
| `MEASUrement:INDICators:STATE` | 本指令设置 state of visible 测量 indicators。 | `MEASUrement:INDICators:STATE {OFF\|M EAS<x>}`<br>`MEASUrement:INDICators:STATE?` |
| `MEASUrement:INDICators:VERT<x>?` | 返回 value of the specified 垂直 测量 indicator。 | `MEASUrement:INDICators:VERT<x>?` |
| `MEASUrement:MEAS<x>?` | 返回全部 测量 parameters。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:COUNt?` | 【返回/查询】Returns the number of values accumulated since the last statistical reset。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:DELay?` | 返回 delay 测量 parameters for the specified 测量。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:DELay:DIRection` | 本指令设置 搜索 direction to use for delay 测量s。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:DELay:EDGE<x>` | 本指令设置 slope of the edge to use for delay “from” and “to” 波形 测量s MEASUrement:MEAS<x>:。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MAXimum?` | 【返回/查询】Returns the maximum value found since the last statistical reset。 | `MAXimum?` |
| `MEASUrement:MEAS<x>:MEAN?` | 【返回/查询】Returns the mean value accumulated since the last statistical reset MEASUrement:MEAS<x>:。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MINImum?` | 返回 minimum value found since the last statistical reset MEASUrement:MEAS<x>: SOUrce<x> 本指令设置 “from” source for all single 通道 immediate 测量s 本指令设置 source to measure “to” for two-通道 测量s。 | `MINImum?` |
| `MEASUrement:MEAS<x>:STATE` | 本指令设置 whether the specified 测量 slot is computed and 显示ed。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:STDdev?` | 【返回/查询】Returns the standard deviation of values accumulated since the last statistical reset。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:TYPe` | 本指令设置 测量<x> type。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:UNIts?` | 返回测量<x> units。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:MEAS<x>:VALue?` | 返回 value of 测量<x>。 | `MEASUrement:MEAS<x>:COUNt?`<br>`MEASUrement:MEAS<x>:DELay?`<br>`MEASUrement:MEAS<x>:DELay:DIRection {BACKWards\|FORWards}` |
| `MEASUrement:METHod` | 本指令设置 method used for calculating 参考 levels。 | `MEASUrement:METHod {Auto\|HIStogram\| MINMax}`<br>`MEASUrement:METHod?` |
| `MEASUrement:REFLevel?` | 返回当前 参考 level parameters。 | `MEASUrement:REFLevel?` |
| `MEASUrement:REFLevel:ABSolute:HIGH` | 本指令设置 top 参考 level for rise time。 | `MEASUrement:REFLevel:ABSolute:HIGH <NR3>`<br>`MEASUrement:REFLevel:ABSolute:HIGH?` |
| `MEASUrement:REFLevel:ABSolute:LOW` | 本指令设置 low 参考 level for rise time。 | `MEASUrement:REFLevel:ABSolute:LOW <NR3>`<br>`MEASUrement:REFLevel:ABSolute:LOW?` |
| `MEASUrement:REFLevel:ABSolute:MID<x>` | 本指令设置 mid 参考 level for the specified 通道 in absolute volts。 | `MEASUrement:REFLevel:ABSolute:MID<x> <NR3>`<br>`MEASUrement:REFLevel:ABSolute:MID<x>?` |
| `MEASUrement:REFLevel:METHod` | 本指令设置 method for assigning high and low 参考 levels。 | `MEASUrement:REFLevel:METHod {ABSolute\|PERCent}`<br>`MEASUrement:REFLevel:METHod?` |
| `MEASUrement:REFLevel:PERCent:HIGH` | 本指令设置 top 参考 percent level for rise time。 | `MEASUrement:REFLevel:PERCent:HIGH <NR3>`<br>`MEASUrement:REFLevel:PERCent:HIGH?` |
| `MEASUrement:REFLevel:PERCent:LOW` | 本指令设置 low 参考 percent level for rise time。 | `MEASUrement:REFLevel:PERCent:LOW` |
| `MEASUrement:REFLevel:PERCent:MID<x>` | 本指令设置 mid 参考 level for the specified 通道 in percent。 | `MEASUrement:REFLevel:PERCent:MID<x> <NR3>`<br>`MEASUrement:REFLevel:PERCent:MID<x>?` |
| `MEASUrement:STATIstics` | 【控制】Clears or returns all of the statistics accumulated for all period measurements (MEAS1 through MEAS4)。 | `MEASUrement:STATIstics?`<br>`MEASUrement:STATIstics RESET`<br>`MEASUrement:STATIstics:MODe {OFF\|ALL}` |
| `MEASUrement:STATIstics:MODe` | 切换测量 statistics 开或关。 | `MEASUrement:STATIstics:MODe {OFF\|ALL}`<br>`MEASUrement:STATIstics:MODe?` |
| `MEASUrement:STATIstics:WEIghting` | 【控制】Controls the responsiveness of the mean and standard deviation to waveform changes。 | `MEASUrement:STATIstics:WEIghting <NR1>`<br>`MEASUrement:STATIstics:WEIghting?` |

## 20. 杂项与 IEEE488.2 (Miscellaneous)

手册原名：*Miscellaneous Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `LOCation?` | 【控制】This query returns the application license location. < x> can be slot number 1–4. APPLication:LICENSE:SLOT<x>: TRANSFER You can use this command to transfer an option license from the option to internal memory in the oscilloscope, and transfer it back. APPLication:LICENSE:SLOT<x>:。 | `LOCation?` |
| `TYPe?` | 【控制】This query returns the application license type of the option that is currently inserted in the specified application option slot. | `TYPe?` |
| `APPLication:TYPe` | 【控制】When a mask/limit or power test application option is installed, one of the associated test types is always selected by default. This command allows the test type to be changed from the default. | `APPLication:TYPe {POWer\|LIMITMask\|VIDPic\|ACTONEVent\|NONe}`<br>`APPLication:TYPe?` |
| `AUTOSet` | 【设置】Sets the vertical, horizontal and trigger controls to provide a stable display of the appropriate waveform. This is equivalent to pressing the front panel Autoset button。 | `AUTOSet {EXECute\|UNDo}` |
| `AUTOSet:ENAble` | 【使能】Enables or disables the autoset feature。 | `AUTOSet:ENAble {OFF\|ON\|0\|1}`<br>`AUTOSet:ENAble?` |
| `AUXOut:SOUrce` | 本指令设置 source for the auxiliary-out port. | `AUXOut:SOUrce {ATRIGger\|MAIn\|REFOut\|EVENT\|AFG}`<br>`AUXOut:SOUrce?` |
| `CLEAR` | Clears 采集s, 测量s, and 波形. | `CLEAR` |
| `CLEARMenu` | 【控制】Clears the current menu from the display。 | `CLEARMenu` |
| `DATE` | 本指令设置 date 显示ed by the 示波器。 | `DATE <QString>`<br>`DATE?` |
| `*DDT` | 【设置】This command specifies the commands that will be executed by the group execute trigger DESkew Causes the deskew values for all channels to be set to the recommended values。 | `*DDT {<Block>\|<QString>}`<br>`*DDT?` |
| `ETHERnet:LXI:LAN:PASSWord:ENABle` | 【控制】This command controls whether LXI (LAN eXtensions for Instrumentation) is password protected. | `ETHERnet:LXI:LAN:PASSWord:ENABle {0\|1\|ON\|OFF}`<br>`ETHERnet:LXI:LAN:PASSWord:ENABle?` |
| `ETHERnet:LXI:LAN:PASSWord:ESCOPEENABle` | 【控制】This command controls whether to use the LX I password for e*Scope (effectively equal to enabling password protection for e*Scope). | `ETHERnet:LXI:LAN:PASSWord:ESCOPEENABle {0\|1\|ON\|OFF}`<br>`ETHERnet:LXI:LAN:PASSWord:ESCOPEENABle?` |
| `ETHERnet:LXI:LAN:RESET` | 【控制】This command resets the LXI local area network. The items which this command reset include: DHCP/BOOTP , mDNS and DNS-SD, e*Scope password protection, LXI password protection, and e*Scope and LXI password. | `ETHERnet:LXI:LAN:RESET` |
| `ETHERnet:LXI:LAN:SERVICENAMe` | 【设置】This command specifies the mDNS service name used for the LXI interface. | `ETHERnet:LXI:LAN:SERVICENAMe?`<br>`ETHERnet:LXI:LAN:SERVICENAMe QString` |
| `ETHERnet:LXI:LAN:STATus?` | 本查询返回 LXI network status. | `ETHERnet:LXI:LAN:STATus?` |
| `ETHERnet:NETWORKCONFig` | 本指令设置 Ethernet network Configuration setting. | `ETHERnet:NETWORKCONFig {AUTOmatic\|MANual}`<br>`ETHERnet:NETWORKCONFig?` |
| `FPAnel:HOLD` | 【控制】This command is used to emulate the push-and-hold feature of the Cursor button. | `FPAnel:HOLD` |
| `FPAnel:PRESS` | 【控制】Simulates the action of pressing a specified front-panel button。 | `FPAnel:PRESS <button>` |
| `FPAnel:TURN` | 【控制】Simulates the action of turning a specified front-panel control knob。 | `FPAnel:TURN <knob>,<n>` |
| `GPIBUsb:ADDress?` | 返回当前 GPIB address。 | `GPIBUsb:ADDress?` |
| `GPIBUsb:ID?` | 【返回/查询】Returns the identi fication string of the connected adaptor option and firmware version HEADer\|:HDR This command specifies the Response Header Enable State。 | `GPIBUsb:ID?` |
| `ID?` | 【返回/查询】Returns the instrument identi fication data similar to that returned by the *IDN? IEEE488.2 common query, including the addition of any enabled application options. However, it does not include the instrument serial number. | `ID?` |
| `*IDN?` | 【返回/查询】Returns the same information as the ID? command except the data is formatted according to Tektronix Codes & Formats。 | `*IDN?` |
| `LANGuage` | 本指令设置 user interface 显示 language。 | `LANGuage` |
| `LOCk:ALL` | 【禁用】Disables the front panel, mouse, and touchscreen。 | `LOCk:ALL` |
| `LOCk:FPanel` | 【使能】Enables or disables the front panel buttons and knobs。 | `LOCk:FPanel {LOCKed\|UNLOCKed}`<br>`LOCk:FPanel?` |
| `LOCk:MOUse` | 【使能】Enables or disables the mouse。 | `LOCk:MOUse {LOCKed\|UNLOCKed}`<br>`LOCk:MOUse?` |
| `LOCk:NONe` | 【使能】Enables the front panel, mouse, and touchscreen。 | `LOCk:NONe` |
| `LOCk:TOUCHscreen` | 【使能】Enables or disables the touchscreen。 | `LOCk:TOUCHscreen {LOCKed\|UNLOCKed}`<br>`LOCk:TOUCHscreen?` |
| `*LRN?` | 返回a listing of 示波器 settings。 | `*LRN?` |
| `NEWpass` | 【控制】Changes the password for user protected data。 | `NEWpass <QString>` |
| `PASSWord` | 【使能】Enables the *PUD and NEWpass set commands。 | `PASSWord` |
| `PAUSe` | 【控制】This command causes the interface to pause the specified number of seconds before processing any other commands. | `PAUSe <NR3>` |
| `RRB:STATE` | 【控制】This command returns or sets the state of the Results Readout bar (RRB)。 | `RRB:STATE {<NR1>\|OFF\|ON}`<br>`RRB:STATE?` |
| `REBOOT` | 【控制】Performs a reboot of the instrument after a short delay. | `REBOOT` |
| `REM` | 设置a comment, which is ignored by the 示波器。 | `REM <QString>` |
| `ROSc:SOUrce` | 【设置】This command specifies the source for the time base reference oscillator. The reference oscillator locks to this source. Depending on the command argument that you specify, you can use an external reference or use the i nternal crystal oscillator as the time base reference. | `ROSc:SOUrce {INTERnal\|EXTernal}`<br>`ROSc:SOUrce?` |
| `ROSc:STATE?` | 【控制】This query returns an enumeration value that indicates the lock state of the reference oscillator specified by the ROSc:SOUrce command. | `ROSc:STATE?` |
| `SET?` | 返回a listing of 示波器 settings。 | `SET?` |
| `SOCKETServer:ENAble` | 【控制】This command enables or disables the socket server which supports a Telnet or other TCPIP socket connection to send commands and queries to the instrument. | `SOCKETServer:ENAble {ON\|OFF\|<NR1>}`<br>`SOCKETServer:ENAble?` |
| `SOCKETServer:PORT` | 【控制】This command sets the TCPIP port for the socket server connection. | `SOCKETServer:PORT <NR1>`<br>`SOCKETServer:PORT?` |
| `SOCKETServer:PROTOCol` | 【控制】This command sets the protocol for the socket server. TEKSecure Initializes both waveform and setup memories。 | `SOCKETServer:PROTOCol {TERMinal\|NONe}`<br>`SOCKETServer:PROTOCol?` |
| `TOTaluptime?` | 【返回/查询】Returns the total number of hours that the oscilloscope has been turned on since the nonvolatile memory was last programmed。 | `TOTaluptime?` |
| `*TRG` | 【控制】Performs the group execute trigger (GET)。 | `*TRG` |
| `*TST?` | 【控制】Tests the interface and returns the status。 | `*TST?` |
| `USBTMC?` | 【返回/查询】Returns the USBTMC information used by the USB hosts to determine the instrument interfaces. | `USBTMC?` |
| `USBTMC:PRODUCTID:DECimal?` | 本查询返回 product ID number as a decimal. | `USBTMC:PRODUCTID:DECimal?` |
| `USBTMC:PRODUCTID:HEXadecimal?` | 本查询返回 product ID number as a decimal. | `USBTMC:PRODUCTID:HEXadecimal?` |
| `USBTMC:SERIALnumber?` | 本查询返回 serial number of the 示波器. | `USBTMC:SERIALnumber?` |
| `USBTMC:VENDORID:DECimal?` | 本查询返回 vendor ID number as a decimal. | `USBTMC:VENDORID:DECimal?` |
| `USBTMC:VENDORID:HEXadecimal?` | 【控制】This query returns the vendor ID number as a hexadecimal value. The hexadecimal vendor ID for Tektronix instruments is 0x699. | `USBTMC:VENDORID:HEXadecimal?` |
| `VERBose` | 本指令设置 verbose state。 | `VERBose {OFF\|ON\|<NR1>}` |

## 21. 电源分析 (Power)

手册原名：*Power Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `APPLication:TYPe` | 【控制】When a mask/limit or power test application option is installed, one of the associated test types is always selected by default. This command allows the test type to be changed from the default. | `APPLication:TYPe {POWer\|LIMITMask\|VIDPic\|ACTONEVent\|NONe}`<br>`APPLication:TYPe?` |
| `POWer:CURRENTSOurce` | 本指令设置 current source for the power application。 | `POWer:CURRENTSOurce {CH1\|CH2\|CH3\|CH4\|REF1\|REF2\|REF3\|REF4}`<br>`POWer:CURRENTSOurce?` |
| `POWer:DISplay` | 本指令控制 whether or not to 显示 the power test results. | `POWer:DISplay {OFF\|ON\|0\|1}`<br>`POWer:DISplay?` |
| `POWer:QUALity:VCRESTfactor?` | 本查询返回 测量 for the voltage crest factor. | `POWer:QUALity:VCRESTfactor?` |
| `POWer:GATESOurce` | 本指令设置 gate source for the power application。 | `POWer:GATESOurce` |
| `POWer:GATing` | 本指令设置 power application gating。 | `POWer:GATing {OFF\|SCREen\|CURSor}`<br>`POWer:GATing?` |
| `POWer:HARMonics:DISplay:SELect` | 本指令设置 harmonics to be 显示ed when the harmonics standard is None。 | `POWer:HARMonics:DISplay:SELect {ODD\|EVEN\|ALL}`<br>`POWer:HARMonics:DISplay:SELect?` |
| `POWer:HARMonics:DISplay:TYPe` | 本指令设置 显示 type for harmonics tests。 | `POWer:HARMonics:DISplay:TYPe {GRAph\|TABle}`<br>`POWer:HARMonics:DISplay:TYPe?` |
| `POWer:HARMonics:FREQRef` | 本指令设置 frequency 参考 波形 for harmonics tests。 | `POWer:HARMonics:FREQRef`<br>`POWer:HARMonics:FREQRef?` |
| `POWer:HARMonics:FREQRef:FIXEDFREQValue` | 本指令设置 fixed 参考 frequency value for harmonics 测量s。 | `POWer:HARMonics:FREQRef:FIXEDFREQValue <NR3>`<br>`POWer:HARMonics:FREQRef:FIXEDFREQValue?` |
| `POWer:HARMonics:IEC:CLAss` | 本指令设置 filtering class for IEC harmonics。 | `POWer:HARMonics:IEC:CLAss {A\|B\|C1\|C2\|C3\|D}`<br>`POWer:HARMonics:IEC:CLAss?` |
| `POWer:HARMonics:IEC:FILter` | 【设置】This command specifies the enabled state for filtering of IEC harmonics。 | `POWer:HARMonics:IEC:FILter` |
| `POWer:HARMonics:IEC:FUNDamental` | 本指令设置 fundamental current for IEC harmonics。 | `POWer:HARMonics:IEC:FUNDamental <NR3>`<br>`POWer:HARMonics:IEC:FUNDamental?` |
| `POWer:HARMonics:IEC:GROUPing` | 【设置】This command specifies the enabled state for grouping of IEC harmonics。 | `POWer:HARMonics:IEC:GROUPing {OFF\|ON\|1\|0}`<br>`POWer:HARMonics:IEC:GROUPing?` |
| `POWer:HARMonics:IEC:INPUTPOWer` | 【设置】Sets of returns the class D input power for IEC harmonics。 | `POWer:HARMonics:IEC:INPUTPOWer <NR3>`<br>`POWer:HARMonics:IEC:INPUTPOWer?` |
| `POWer:HARMonics:IEC:LINEFREQuency` | 本指令设置 line frequency for the IEC standard。 | `POWer:HARMonics:IEC:LINEFREQuency <NR1>`<br>`POWer:HARMonics:IEC:LINEFREQuency?` |
| `POWer:HARMonics:IEC:OBSPERiod` | 本指令设置 IEC observation period。 | `POWer:HARMonics:IEC:OBSPERiod <NR3>`<br>`POWer:HARMonics:IEC:OBSPERiod?` |
| `POWer:HARMonics:IEC:POWERFACtor` | 本指令设置 power factor for IEC harmonics。 | `POWer:HARMonics:IEC:POWERFACtor <NR3>`<br>`POWer:HARMonics:IEC:POWERFACtor?` |
| `POWer:HARMonics:MIL:FUNDamental:CALCmethod` | 本指令设置 测量 method for the MIL harmonics fundamental frequency。 | `POWer:HARMonics:MIL:FUNDamental:CALCmethod {MEAS\|USER}`<br>`POWer:HARMonics:MIL:FUNDamental:CALCmethod?` |
| `POWer:HARMonics:MIL:FUNDamental:USER:CURrent` | 本指令设置 RMS amperes for User calculation method。 | `POWer:HARMonics:MIL:FUNDamental:USER:CURrent <NR3>`<br>`POWer:HARMonics:MIL:FUNDamental:USER:CURrent?` |
| `POWer:HARMonics:MIL:LINEFREQuency` | 【设置】This command specifies the line frequency for MIL-STD-1399 Section 300A harmonics tests。 | `POWer:HARMonics:MIL:LINEFREQuency <NR1>`<br>`POWer:HARMonics:MIL:LINEFREQuency?` |
| `POWer:HARMonics:MIL:POWERLEVel` | 【设置】This command specifies the power level for MIL-STD-1399 Section 300A harmonics tests。 | `POWer:HARMonics:MIL:POWERLEVel {LOW\|HIGH}`<br>`POWer:HARMonics:MIL:POWERLEVel?` |
| `POWer:HARMonics:NR_HARMonics` | 【设置】Sets of returns the number of harmonics (a value in the range of 20 to 400) when the harmonics standard is NONe POWer:HARMonics:RESults:HAR<1-400>:FREQuency? Returns the frequency of the harmonic POWer:HARMonics:RESults:HAR<1-400>:IECMAX? The IEC standard specifies harmonics measurements to be computed in windows of time, with each time window being nominally 200 ms. This returns the maximum of the RMS magnitude of the harmonic, computed across successive 200 ms time windows within an observation period entered by the user POWer:HARMonics:RESults:HAR<1-400>:LIMit? The IEC and MIL standards specify a limit for each harmonic magnitude. Returns the limit in absolute units, or as a percentage of the fundamental as specified by the standard. IEC Class C (Table 2) and MIL standards specif y the limit as a percentage of the fundamental POWer:HARMonics:RESults:HAR<1-400>:PHASe? Returns the phase of the harmonic in degrees. The phase is measured relative to the zero-crossing of the reference waveform. W hen there is no reference waveform, the phase is relative to the fundamental component POWer:HARMonics:RESults:HAR<1-400>:RMS:ABSolute? Returns the RMS magnitude of the harmonic expressed in absolute units POWer:HARMonics:RESults:HAR<1-400>:RMS:PERCent? Returns the RMS magnitude of the harmonic expressed as a percentage of the fundamental POWer:HARMonics:RESults:HAR<1-400>:TEST:IEC:CLASSALIMit? Returns PASS, FAIL or NA. specifies if the IEC Class A higher harmonic limit (and conditions) are met POWer:HARMonics:RESults:HAR<1-400>:TEST:IEC:NORMAL? Returns PASS, FAIL or NA. specifies if the Normal IEC harmonic limits are met POWer:HARMonics:RESults:HAR<1-400>:TEST:IEC:POHCLIMit? Returns PASS, FAIL or NA. specifies if the higher harmonic limit (and conditions) for the 21st and higher order odd harmonics are met POWer:HARMonics:RESults:HAR<1-400>:TEST:MIL:NORMAL? Returns the test result for the specified harmonic for the MIL-STD-1399 Section 300A testing standard。 | `POWer:HARMonics:NR_HARMonics <NR3>`<br>`POWer:HARMonics:NR_HARMonics?` |
| `POWer:HARMonics:RESults:IEC:FUNDamental?` | 【返回/查询】Returns the IEC fundamental frequency。 | `POWer:HARMonics:RESults:IEC:FUNDamental?` |
| `POWer:HARMonics:RESults:IEC:HARM3ALTernate?` | 【返回/查询】Returns the IEC harmonics test result for the 3rd harmonic: PASS, FAIL or NA。 | `POWer:HARMonics:RESults:IEC:HARM3ALTernate? Returns PASS, FAIL,o rNA.` |
| `POWer:HARMonics:RESults:IEC:HARM5ALTernate?` | 【返回/查询】Returns the IEC harmonics test result for the 5th harmonic: PASS, FAIL or NA。 | `POWer:HARMonics:RESults:IEC:HARM5ALTernate? Returns PASS, FAIL,o rNA.` |
| `POWer:HARMonics:RESults:IEC:POHC?` | 返回 IEC POHC 测量。 | `POWer:HARMonics:RESults:IEC:POHC?` |
| `POWer:HARMonics:RESults:IEC:POHL?` | 返回 IEC POHL 测量。 | `POWer:HARMonics:RESults:IEC:POHL?` |
| `POWer:HARMonics:RESults:IEC:POWer?` | 返回 IEC input power 测量。 | `POWer:HARMonics:RESults:IEC:POWer?` |
| `POWer:HARMonics:RESults:IEC:POWERFactor?` | 返回 IEC power factor 测量。 | `POWer:HARMonics:RESults:IEC:POWERFactor?` |
| `POWer:HARMonics:RESults:PASSFail?` | 【返回/查询】Returns the overall harmonics test result: PASS, FAIL or NA。 | `POWer:HARMonics:RESults:PASSFail?` |
| `POWer:HARMonics:RESults:RMS?` | 【返回/查询】Returns the root mean square value of the source waveform。 | `POWer:HARMonics:RESults:RMS?` |
| `POWer:HARMonics:RESults:SAVe` | 【控制】Saves the harmonic results to the specified file in CSV format。 | `POWer:HARMonics:RESults:SAVe <String>` |
| `POWer:HARMonics:RESults:THDF?` | 【返回/查询】Returns the Total Harmonic Distortion (THD) in percentage, measured as a ratio to the RMS value of the fundamental component of the source waveform。 | `POWer:HARMonics:RESults:THDF?` |
| `POWer:HARMonics:RESults:THDR?` | 【返回/查询】Returns the Total Harmonic Distortion (THD) in percentage, measured as a ratio to the RMS value of the source waveform。 | `POWer:HARMonics:RESults:THDR?` |
| `POWer:HARMonics:SOURce` | 本指令设置 source 波形 for harmonics tests。 | `POWer:HARMonics:SOURce {VOLTage\|CURRent}`<br>`POWer:HARMonics:SOURce?` |
| `POWer:HARMonics:STANDard` | 本指令设置 standard for harmonics tests。 | `POWer:HARMonics:STANDard {NONe\|IEC\|MIL}`<br>`POWer:HARMonics:STANDard?` |
| `POWer:INDICators` | 本指令设置 state of the 测量 indicators for the power application。 | `POWer:INDICators {OFF\|ON\|0\|1}`<br>`POWer:INDICators?` |
| `POWer:MODulation:SOUrce` | 本指令设置 source 波形 for modulation tests。 | `POWer:MODulation:SOUrce` |
| `POWer:MODulation:TYPe` | 本指令设置 modulation type。 | `POWer:MODulation:TYPe`<br>`POWer:MODulation:TYPe?` |
| `POWer:QUALity:APPpwr?` | 返回 apparent power 测量。 | `POWer:QUALity:APPpwr?` |
| `POWer:QUALity:DISplay:APPpwr` | 本指令设置 显示 state for the apparent power readout。 | `POWer:QUALity:DISplay:APPpwr {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:APPpwr?` |
| `POWer:QUALity:DISplay:FREQuency` | 本指令设置 显示 state for the frequency readout。 | `POWer:QUALity:DISplay:FREQuency {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:FREQuency?` |
| `POWer:QUALity:DISplay:ICRESTfactor` | 本指令设置 显示 state for the current crest factor readout。 | `POWer:QUALity:DISplay:ICRESTfactor` |
| `POWer:QUALity:DISplay:IRMS` | 本指令设置 显示 state for the rms current (IRMS) readout。 | `POWer:QUALity:DISplay:IRMS {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:IRMS?` |
| `POWer:QUALity:DISplay:PHASEangle` | 本指令设置 显示 state for the phase angle readout。 | `POWer:QUALity:DISplay:PHASEangle {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:PHASEangle?` |
| `POWer:QUALity:DISplay:POWERFACtor` | 本指令设置 显示 state for the power factor readout。 | `POWer:QUALity:DISplay:POWERFACtor {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:POWERFACtor?` |
| `POWer:QUALity:DISplay:REACTpwr` | 本指令设置 显示 state for the reactive power readout。 | `POWer:QUALity:DISplay:REACTpwr {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:REACTpwr?` |
| `POWer:QUALity:DISplay:TRUEpwr` | 本指令设置 显示 state for the true power readout。 | `POWer:QUALity:DISplay:TRUEpwr {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:TRUEpwr?` |
| `POWer:QUALity:DISplay:VCRESTfactor` | 本指令设置 显示 state for the voltage crest factor readout。 | `POWer:QUALity:DISplay:VCRESTfactor {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:VCRESTfactor?` |
| `POWer:QUALity:DISplay:VRMS` | 本指令设置 显示 state for the rms voltage (VRMS) readout。 | `POWer:QUALity:DISplay:VRMS {OFF\|ON\|0\|1}`<br>`POWer:QUALity:DISplay:VRMS?` |
| `POWer:QUALity:FREQREFerence` | 本指令设置 power quality frequency 参考。 | `POWer:QUALity:FREQREFerence {VOLTage\|CURRent}`<br>`POWer:QUALity:FREQREFerence?` |
| `POWer:QUALity:FREQuency?` | 返回 frequency 测量。 | `POWer:QUALity:FREQuency?` |
| `POWer:QUALity:ICRESTfactor?` | 返回当前 crest factor 测量。 | `POWer:QUALity:ICRESTfactor?` |
| `POWer:QUALity:IRMS?` | 返回 rms current 测量。 | `POWer:QUALity:IRMS?` |
| `POWer:QUALity:PHASEangle?` | 返回 phase angle 测量。 | `POWer:QUALity:PHASEangle?` |
| `POWer:QUALity:POWERFACtor?` | 返回 power factor 测量。 | `POWer:QUALity:POWERFACtor?` |
| `POWer:QUALity:REACTpwr?` | 返回 reactive power 测量。 | `POWer:QUALity:REACTpwr?` |
| `POWer:QUALity:TRUEpwr?` | 返回 true power 测量。 | `POWer:QUALity:TRUEpwr?` |
| `POWer:QUALity:VRMS?` | 返回 rms voltage 测量。 | `POWer:QUALity:VRMS?` |
| `POWer:REFLevel:ABSolute` | 【设置】Sets the reference levels to their default unit values。 | `POWer:REFLevel:ABSolute {SETTODEFaults}`<br>`POWer:REFLevel:ABSolute:HIGH <NR3>; Ranges={D,-1e6,+1E6}`<br>`POWer:REFLevel:ABSolute:HIGH?` |
| `POWer:REFLevel:ABSolute:HIGH` | 本指令设置 top 参考 level for rise time。 | `POWer:REFLevel:ABSolute:HIGH <NR3>; Ranges={D,-1e6,+1E6}`<br>`POWer:REFLevel:ABSolute:HIGH?` |
| `POWer:REFLevel:ABSolute:LOW` | 本指令设置 low 参考 level for rise time。 | `POWer:REFLevel:ABSolute:LOW <NR3>; Ranges={D,-1e6,+1E6}`<br>`POWer:REFLevel:ABSolute:LOW?` |
| `POWer:REFLevel:ABSolute:MID<x>` | 本指令设置 mid 参考 level for 测量s。 | `POWer:REFLevel:ABSolute:MID<x> <NR3>; Ranges={D,-1e6,+1E6}`<br>`POWer:REFLevel:ABSolute:MID<x>?` |
| `POWer:REFLevel:HYSTeresis` | 本指令设置 测量 参考 level hysteresis value。 | `POWer:REFLevel:HYSTeresis` |
| `POWer:REFLevel:METHod` | 本指令设置 method used to calculate the 0% and 100% 参考 level。 | `POWer:REFLevel:METHod {ABSolute\|PERCent}`<br>`POWer:REFLevel:METHod?` |
| `POWer:REFLevel:PERCent` | 【设置】Sets the reference levels to the default percentage values。 | `POWer:REFLevel:PERCent <SETTODEFaults>`<br>`POWer:REFLevel:PERCent:HIGH <NR3>; Ranges={D,0.0,100.0}`<br>`POWer:REFLevel:PERCent:HIGH?` |
| `POWer:REFLevel:PERCent:HIGH` | 本指令设置 top 参考 percent level for rise time。 | `POWer:REFLevel:PERCent:HIGH <NR3>; Ranges={D,0.0,100.0}`<br>`POWer:REFLevel:PERCent:HIGH?` |
| `POWer:REFLevel:PERCent:LOW` | 本指令设置 low 参考 percent level for rise time。 | `POWer:REFLevel:PERCent:LOW` |
| `POWer:REFLevel:PERCent:MID<x>` | 本指令设置 mid 参考 percent level for 波形 测量s。 | `POWer:REFLevel:PERCent:MID<x> <NR3>; Ranges={D,0.0,100.0}`<br>`POWer:REFLevel:PERCent:MID<x>?` |
| `POWer:RIPPle` | Sets the 垂直 offset of the source 波形。 | `POWer:RIPPle {VERTAUTOset\|VERTDEFault}` |
| `POWer:RIPPle:RESults:AMPLitude?` | 返回 peak-to-peak ripple 测量。 | `POWer:RIPPle:RESults:AMPLitude?` |
| `POWer:RIPPle:RESults:MAX?` | 返回最大 of the peak-to-peak ripple 测量s。 | `POWer:RIPPle:RESults:MAX?` |
| `POWer:RIPPle:RESults:MEAN?` | 返回 mean of the peak-to-peak ripple 测量s。 | `POWer:RIPPle:RESults:MEAN?` |
| `POWer:RIPPle:RESults:MIN?` | 【返回/查询】Returns the minimum of the peak-to-peak ripple measurement。 | `POWer:RIPPle:RESults:MIN?` |
| `POWer:RIPPle:RESults:STDdev?` | 【返回/查询】Returns the standard deviation of the peak-to-peak ripple measurements。 | `POWer:RIPPle:RESults:STDdev?` |
| `POWer:RIPPle:SOUrce` | 本指令设置 source 波形 for ripple tests。 | `POWer:RIPPle:SOUrce {VOLTage\|CURRent}` |
| `POWer:SOA:LINear:XMAX` | 【设置】This command specifies the user XMAX value for use in linear SOA calculations。 | `POWer:SOA:LINear:XMAX <NR3>`<br>`POWer:SOA:LINear:XMAX?` |
| `POWer:SOA:LINear:XMIN` | 【设置】This command specifies the user XMIN value for use in linear SOA calculations。 | `POWer:SOA:LINear:XMIN <NR3>`<br>`POWer:SOA:LINear:XMIN?` |
| `POWer:SOA:LINear:YMAX` | 【设置】This command specifies the user YMAX value for use in linear SOA calculations。 | `POWer:SOA:LINear:YMAX <NR3>`<br>`POWer:SOA:LINear:YMAX?` |
| `POWer:SOA:LINear:YMIN` | 【设置】This command specifies the user YMIN value for use in linear SOA calculations。 | `POWer:SOA:LINear:YMIN <NR3>`<br>`POWer:SOA:LINear:YMIN?` |
| `POWer:SOA:LOG:XMAX` | 【设置】This command specifies the user XMAX value for use in log SOA calculations。 | `POWer:SOA:LOG:XMAX <NR3>`<br>`POWer:SOA:LOG:XMAX?` |
| `POWer:SOA:LOG:XMIN` | 【设置】This command specifies the user XMIN value for use in log SOA calculations。 | `POWer:SOA:LOG:XMIN <NR3>`<br>`POWer:SOA:LOG:XMIN?` |
| `POWer:SOA:LOG:YMAX` | 【设置】This command specifies the user YMAX value for use in log SOA calculations。 | `POWer:SOA:LOG:YMAX <NR3>`<br>`POWer:SOA:LOG:YMAX?` |
| `POWer:SOA:LOG:YMIN` | 【设置】This command specifies the user YMIN value for use in log SOA calculations。 | `POWer:SOA:LOG:YMIN <NR3>`<br>`POWer:SOA:LOG:YMIN?` |
| `POWer:SOA:MASK:DEFine` | 【设置】This command specifies the X (volts) and Y (Amps) coordinates of the current SOA mask。 | `POWer:SOA:MASK:DEFine <NR3>`<br>`POWer:SOA:MASK:DEFine?` |
| `POWer:SOA:MASK:MAXAmps` | 【设置】This command specifies the maximum current applied to SOA mask testing。 | `POWer:SOA:MASK:MAXAmps <NR3>`<br>`POWer:SOA:MASK:MAXAmps?` |
| `POWer:SOA:MASK:MAXVolts` | 【设置】This command specifies the maximum voltage applied to SOA mask testing。 | `POWer:SOA:MASK:MAXVolts <NR3>`<br>`POWer:SOA:MASK:MAXVolts?` |
| `POWer:SOA:MASK:MAXWatts` | 本指令设置 maximum power applied to SOA mask testing。 | `POWer:SOA:MASK:MAXWatts <NR3>`<br>`POWer:SOA:MASK:MAXWatts?` |
| `POWer:SOA:MASK:NR_Pt?` | 返回数量： mask points defined。 | `POWer:SOA:MASK:NR_Pt?` |
| `POWer:SOA:MASK:STATE` | 本指令设置 state of the mask for SOA calculations。 | `POWer:SOA:MASK:STATE {OFF\|LIMITS\|POINTS}`<br>`POWer:SOA:MASK:STATE?` |
| `POWer:SOA:MASK:STOPOnviol` | 【设置】This command specifies the enabled state of the mask stop on violation condition。 | `POWer:SOA:MASK:STOPOnviol {OFF\|ON\|0\|1}`<br>`POWer:SOA:MASK:STOPOnviol?` |
| `POWer:SOA:PLOTTYPe` | 本指令设置 SOA plot type。 | `POWer:SOA:PLOTTYPe {LOG\|LINear}`<br>`POWer:SOA:PLOTTYPe?` |
| `POWer:SOA:RESult:FAILures:QTY?` | 返回数量： failures in the test。 | `POWer:SOA:RESult:FAILures:QTY?` |
| `POWer:SOA:RESult:NUMACq?` | 返回数量： 采集s in the test。 | `POWer:SOA:RESult:NUMACq?` |
| `POWer:SOA:RESult:STATE?` | 【返回/查询】Returns the pass/fail state of the SOA test。 | `POWer:SOA:RESult:STATE? Returns PASS or FAIL.` |
| `POWer:STATIstics` | 【控制】Clears all the accumulated statistics of all measurements。 | `POWer:STATIstics {RESET}`<br>`POWer:STATIstics:MODe {OFF\|ALL}`<br>`POWer:STATIstics:MODe?` |
| `POWer:STATIstics:MODe` | 使能or disables the 显示 of the 测量 statistics。 | `POWer:STATIstics:MODe {OFF\|ALL}`<br>`POWer:STATIstics:MODe?` |
| `POWer:STATIstics:WEIghting` | 【设置】Sets the number of samples which are included for the statistics computations for mean and the standard deviation。 | `POWer:SWLoss:CONDCALCmethod {VOLTage\|RDSon\|VCEsat}`<br>`POWer:SWLoss:CONDCALCmethod?` |
| `POWer:SWLoss:CONDCALCmethod` | 【设置】This command specifies the power application switching loss conduction calculation method。 | `POWer:SWLoss:CONDCALCmethod {VOLTage\|RDSon\|VCEsat}`<br>`POWer:SWLoss:CONDCALCmethod?` |
| `POWer:SWLoss:CONDuction:ENERGY:MAX?` | 【返回/查询】Returns the maximum conduction energy for the switching loss calculation。 | `POWer:SWLoss:CONDuction:ENERGY:MAX?` |
| `POWer:SWLoss:CONDuction:ENERGY:MEAN?` | 【返回/查询】Returns the mean conduction energy for the switching loss calculation。 | `POWer:SWLoss:CONDuction:ENERGY:MEAN?` |
| `POWer:SWLoss:CONDuction:ENERGY:MIN?` | 【返回/查询】Returns the minimum conduction energy for the switching loss calculation。 | `POWer:SWLoss:CONDuction:ENERGY:MIN?` |
| `POWer:SWLoss:CONDuction:POWer:MAX?` | 【返回/查询】Returns the maximum conduction power for the switching loss calculation。 | `POWer:SWLoss:CONDuction:POWer:MAX?` |
| `POWer:SWLoss:CONDuction:POWer:MEAN?` | 【返回/查询】Returns the mean conduction power for the switching loss calculation。 | `POWer:SWLoss:CONDuction:POWer:MEAN?` |
| `POWer:SWLoss:CONDuction:POWer:MIN?` | 【返回/查询】Returns the minimum conduction power for the switching loss calculation。 | `POWer:SWLoss:CONDuction:POWer:MIN?` |
| `POWer:SWLoss:DISplay` | 本指令设置 显示 selection for switching loss results。 | `POWer:SWLoss:DISplay {ALL\|ENERGYLoss\|POWERLoss}`<br>`POWer:SWLoss:DISplay?` |
| `POWer:SWLoss:GATe:POLarity` | 本指令设置 switching loss gate polarity。 | `POWer:SWLoss:GATe:POLarity {FALL\|RISe}`<br>`POWer:SWLoss:GATe:POLarity?` |
| `POWer:SWLoss:GATe:TURNON` | 本指令设置 gate turn on level for switching loss power 测量s。 | `POWer:SWLoss:GATe:TURNON <NR3>`<br>`POWer:SWLoss:GATe:TURNON?` |
| `POWer:SWLoss:NUMCYCles?` | 【返回/查询】Returns the number of cycles counted for the switching loss calculation。 | `POWer:SWLoss:NUMCYCles? <NR3>` |
| `POWer:SWLoss:RDSon` | 【设置】This command specifies RDSON value for use in switching loss calculations when the conduction calculation method is RDSON。 | `POWer:SWLoss:RDSon <NR3>`<br>`POWer:SWLoss:RDSon?` |
| `POWer:SWLoss:REFLevel:ABSolute:GATEMid` | 本指令设置 mid voltage 参考 level used in switching loss power 测量s in volts。 | `POWer:SWLoss:REFLevel:ABSolute:GATEMid <NR3>`<br>`POWer:SWLoss:REFLevel:ABSolute:GATEMid?` |
| `POWer:SWLoss:REFLevel:ABSolute:LOWCurrent` | 本指令设置 low current 参考 level used in switching loss power 测量s in amperes。 | `POWer:SWLoss:REFLevel:ABSolute:LOWCurrent <NR3>`<br>`POWer:SWLoss:REFLevel:ABSolute:LOWCurrent?` |
| `POWer:SWLoss:REFLevel:ABSolute:LOWVoltage` | 本指令设置 low voltage 参考 level used in switching loss power 测量s in volts。 | `POWer:SWLoss:REFLevel:PERCent:GATEMid <NR3>`<br>`POWer:SWLoss:REFLevel:PERCent:GATEMid?` |
| `POWer:SWLoss:REFLevel:PERCent:GATEMid` | 本指令设置 mid voltage 参考 level used in switching loss power 测量s in percentage。 | `POWer:SWLoss:REFLevel:PERCent:GATEMid <NR3>`<br>`POWer:SWLoss:REFLevel:PERCent:GATEMid?` |
| `POWer:SWLoss:REFLevel:PERCent:LOWCurrent` | 本指令设置 low current 参考 level used in switching loss power 测量s in percentage。 | `POWer:SWLoss:REFLevel:PERCent:LOWCurrent <NR3>`<br>`POWer:SWLoss:REFLevel:PERCent:LOWCurrent?` |
| `POWer:SWLoss:REFLevel:PERCent:LOWVoltage` | 本指令设置 low voltage 参考 level used in switching loss power 测量s in percentage。 | `POWer:SWLoss:REFLevel:PERCent:LOWVoltage <NR3>`<br>`POWer:SWLoss:REFLevel:PERCent:LOWVoltage?` |
| `POWer:SWLoss:TOFF:ENERGY:MAX?` | 【返回/查询】Returns the maximum Toff energy switching loss calculation。 | `POWer:SWLoss:TOFF:ENERGY:MAX?` |
| `POWer:SWLoss:TOFF:ENERGY:MEAN?` | 【返回/查询】Returns the mean Toff energy switching loss calculation。 | `POWer:SWLoss:TOFF:ENERGY:MEAN?` |
| `POWer:SWLoss:TOFF:ENERGY:MIN?` | 【返回/查询】Returns the minimum Toff energy switching loss calculation。 | `POWer:SWLoss:TOFF:ENERGY:MIN?` |
| `POWer:SWLoss:TOFF:POWer:MAX?` | 【返回/查询】Returns the maximum Toff power switching loss calculation。 | `POWer:SWLoss:TOFF:POWer:MAX?` |
| `POWer:SWLoss:TOFF:POWer:MEAN?` | 【返回/查询】Returns the mean Toff power switching loss calculation。 | `POWer:SWLoss:TOFF:POWer:MEAN?` |
| `POWer:SWLoss:TOFF:POWer:MIN?` | 【返回/查询】Returns the minimum Toff power switching loss calculation。 | `POWer:SWLoss:TOFF:POWer:MIN?` |
| `POWer:SWLoss:TON:ENERGY:MAX?` | 【返回/查询】Returns the maximum Ton energy switching loss calculation。 | `POWer:SWLoss:TON:ENERGY:MAX?` |
| `POWer:SWLoss:TON:ENERGY:MEAN?` | 【返回/查询】Returns the mean Ton energy switching loss calculation。 | `POWer:SWLoss:TON:ENERGY:MEAN?` |
| `POWer:SWLoss:TON:ENERGY:MIN?` | 【返回/查询】Returns the minimum Ton energy switching loss calculation。 | `POWer:SWLoss:TON:ENERGY:MIN?` |
| `POWer:SWLoss:TON:POWer:MAX?` | 【返回/查询】Returns the maximum Ton power switching loss calculation。 | `POWer:SWLoss:TON:POWer:MAX?` |
| `POWer:SWLoss:TON:POWer:MEAN?` | 【返回/查询】Returns the mean Ton power switching loss calculation。 | `POWer:SWLoss:TON:POWer:MEAN?` |
| `POWer:SWLoss:TON:POWer:MIN?` | 【返回/查询】Returns the minimum Ton power switching loss calculation。 | `POWer:SWLoss:TON:POWer:MIN?` |
| `POWer:SWLoss:TOTal:ENERGY:MAX?` | 【返回/查询】Returns the maximum total energy switching loss calculation。 | `POWer:SWLoss:TOTal:ENERGY:MAX?` |
| `POWer:SWLoss:TOTal:ENERGY:MEAN?` | 【返回/查询】Returns the mean total energy switching loss calculation。 | `POWer:SWLoss:TOTal:ENERGY:MEAN?` |
| `POWer:SWLoss:TOTal:ENERGY:MIN?` | 【返回/查询】Returns the minimum total energy switching loss calculation。 | `POWer:SWLoss:TOTal:ENERGY:MIN?` |
| `POWer:SWLoss:TOTal:POWer:MAX?` | 【返回/查询】Returns the maximum total power switching loss calculation。 | `POWer:SWLoss:TOTal:POWer:MAX?` |
| `POWer:SWLoss:TOTal:POWer:MEAN?` | 【返回/查询】Returns the mean total power switching loss calculation。 | `POWer:SWLoss:TOTal:POWer:MEAN?` |
| `POWer:SWLoss:TOTal:POWer:MIN?` | 【返回/查询】Returns the minimum total power switching loss calculation。 | `POWer:SWLoss:TOTal:POWer:MIN?` |
| `POWer:SWLoss:VCEsat` | 【设置】This command specifies VCESAT value for use in switching loss calculations when the conduction calculation method is VCESAT。 | `POWer:SWLoss:VCEsat <NR3>`<br>`POWer:SWLoss:VCEsat?` |
| `POWer:TYPe` | 本指令设置 power application 测量 type。 | `POWer:TYPe {NONe\|QUALity\|SWITCHingloss\|SOA`<br>`POWer:TYPe?` |
| `POWer:VOLTAGESOurce` | 【设置】This command specifies the voltage source for the power application RF Command Gro up The Tektronix 3 Series MDO models havea built-in RF input, in addition to analoganddigitalchannels,whichallo wsyoutodisplay,measure,perfor mmath onandanalyz ebothtimeandfrequencydomainsignalswithoneinstrument. The 3 Series MDO allows frequency domain measurements but does not offer time-correlateddisplayandmeasurementoptionsforRFtraces. TheRFcommandsareconcentratedintheRFCommandGroup,butalsoappear inotherco mmandgroups,includingSaveandRec all,WaveformTransfer,Trigger and Search. Frequency Domain Trace Types。 | `POWer:VOLTAGESOurce {CH1\|CH2\|CH3\|CH4\|REF1\|REF2\|REF3\|REF4}`<br>`POWer:VOLTAGESOurce?` |
| `The3SeriesMDOsupportsfourfrequencywaveformtypes:Thefrequencydomainwindowprovides` | 【控制】supportforfourspectrumtraces,wh ich may be turned on and off independently. 1. RFNormaltrace: Eachacquisitionisdiscardedasnewdataisacquired. 2. RFMaxHoldtrace: Themaximumdatav aluesareaccumulatedovermultiple acquisitions of the RF Normal trace. 3. RFMinHoldtrace: Theminimumdatavaluesareaccumulatedovermultiple acquisitions of the RF Normal trace. 4. RFAveragetrace: Datafrom theRFNo rmal traceisaveragedovermultiple acquisitions. This is true power averaging, which occurs before the log conversion. Eachpowerof2averagereducesthedisplayednoiseby3dB. Acquisition Stages RFacquisitionstravelthroughtheir ownsignalpathbeforebeingdigitiz edbythe oscilloscope. This signal path includes a combination of analog amplification, attenuation, filteringand down-conversion, dependingontheparameters setby the user(frequency, span, reference level, and so forth.) The 3 Series MDO uses one frequency band (up to 3 GHz depending on the model and installed options). Specifying the Reference Level and Resolution Bandwidth (RBW) Settings The 3 Series MDO reference level is adjustable from –140 dBm to +20 dBm. Attenuation is set automatically withthe reference level. The RBW settingi s adjustabledownto20Hz. Bydefault,th eRBWtracksspaninautomaticmodei n a 1000:1 ratio; this ratio is adjustable. Detection Types MDO instruments calculate Fast Fourier Transform calculations (FFTs) with a 1,000 to ~2,000,000 point output, depending on the acquisition settings.I t then reduces that FFT output into a 750 pixel-wide display. This means that approximately 1 to 2,000 FFT points get compressed into each pixel column. There are four choices as to how this compression is done: +peak, sample, average, and -peak. | `The3SeriesMDOsupportsfourfrequencywaveformtypes:Thefrequencydomainwindowprovides` |
| `Spectrogram` | 【控制】Display The spectrogram is a graph of frequency domain traces over time. It provides an intuitive display that is useful for monitoring slowly changing RF events, and for identifying low amplitude signals too subtle for the eye to catch ina regularspectrumdisplay. Thex-axisshowsfrequency,andthey-axisshow stime. Amplitude is represented by the color of the trace. Cold colors (blue, green) indicatelowamplitude,andhotcolors(red,yellow)indicatehighamplit ude. Spectrogramslices aregenerated bytaking each spectrum and flipping itonits edge,sothatitisonepixelrowtall. Eachnewacquisitiona ddsanothersliceatthe bottom of the spectrogram, and the previous acquisitions(slices) move upone row;youcanthennavigatebackwardsthroughthehistoryofthespectrogra mby selecting slice numbers to view. (The spectrogram slice trace is displayeda s the RF Normal trace.) Spectrum Mode: Triggered and Free Run WhenTriggeredmodeisselected,youcancontrolalltriggersettings, inc luding NormalandAutotriggering. WhenFreeRunmodeisselected,theoscillosco pe generates RF acquisitions as fast as possible. When the oscilloscope displays bothtime and frequency domain waveforms, then theinstrument’striggersystemisincontroloftheTriggeredmodean dthe RF acquisitions. Using Markers in the Frequency Domain for Measurement and Analysis For frequency domain measurements, up to 11automaticmarkersareavailable to assist with quickly identifying the frequency and amplitude of peaks inthe spectrumbaseduponuser thresholdand excursionsettings. Ifmorepeaksm eet thecriteriathanthedesirednumbero fmarkers,thenthehighestamplitud epeaks areshown. Twomanualmarkersarealsoavailableformeasuringnon-peakar eas ofinterest,andtomeasureNoiseDensityandPhaseNoise. Ifmanualmarker sare off,thereferencemarkerisautomatic allyplacedonthehighestamplitud epeak. Withmanualmarkerson,thereferencemarkerbecomesthe“A”manualmarker . Automatic peak markers are on by default. Each automaticmarker hasa readout associated with it. These can be absolute or delta readouts. An absolute markerreadoutshows the actual frequency and amplitudeoftheassociatedmarker. A deltamarkerreadoutshowsthefrequ ency and amplitude of the automatic markers relative to the Reference Marker. The Reference Marker’s readout indicates absolute frequency and amplitude, regardless of the readout type. (It ismarked on the display with a red R in a triangle.) ThemarkermeasurementreadoutsareabsoluteindBmorrelativ etothe referencemarkerin dBc(dBbelow carrieramplitude). Thethresholdandexcursionsettingsde finewhichpeaksaremarkedautomat ically. Thethresholdisaminimumamplitudeth atasignalmustcrosstobeavalidpe ak. If the threshold is lower, more peaks will tend to qualify for markers. If the thresholdishi gher,fewerpeakstendtoqualifyformarkers. Theexcursionishow farasignalneedstofallinamplitude betweenmarkedpeakstobeanotherva lid peak. Iftheexcursionislow,morepeakswilltendtoqualifyformarkers. I fthe excursionishigh,fewerpeakswilltendtoqualifyformarkers. Whenthetwomanualmarkersareturnedon,theReferenceMarkerisnolonger automaticallyattachedtothehighe stamplitudepeak. Itcannowbemovedt oany desired location. This enables easy measurement of any part of the spectrum, as well as delta measurements to any part of the spectrum. This also lets you measureno n-peakspectralcontentofinterest. Thereadoutsformanualmarkers indicatefrequency,amplitudeandnoise(justlikeautomaticmarkerread outs). Taking Automatic Measurements in the Frequency Domain Youcantakethreeautomaticmeas urementsinthefrequencydomain:1. Channel Power (CP) — The total power within the bandwidth, defined by the Channel Width. 2. Adjacent Channel Power Ratio (ACPR) — The power in the main channel andtheratioofchannelpowertomainpower,fortheupperandlowerhalveso f each adjacent channel. 3. Occupied Bandwidth (OBW) — The bandwidth that contains the specified percentage ofpower within the analysis bandwidth. Transferring and Saving RF Trace Information YoucanperformwaveformtransfercommandsandqueriesusingRFtraces. (S ee。 | `Spectrogram` |

## 22. 射频分析 (RF)

手册原名：*RF Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `MARKER:M<x>:AMPLitude:ABSolute?` | 【控制】This query returns the actual amplitude (vertical) value of the either of the two manual markers that are available for frequency domain traces, in dBm. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:M<x>:AMPLitude:DELTa?` | 【控制】This query returns the delta amplitude (vertical) value of either of the two manual markers that are available for frequency domain traces, in relation to the Reference Marker. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:M<x>:FREQuency:ABSolute` | 【设置】This command specifies the actual frequency (horizontal) value of either of the two manual markers that are available for frequency domain traces. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:M<x>:FREQuency:DELTa?` | 【控制】This query returns the delta frequency (horizontal) value of either of the two manual markers that are available for frequency domain traces, in relation to the Reference Marker. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:M<x>:NOISEDensity?` | 【控制】, This command returns the noise density of the RF_NORMal trace at the specified marker position in <RF Units>/Hz units, where <RF Units> are the units specified by the command RF:UNIts. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:M<x>:PHASENoise?` | 【控制】This command returns the phase noise of the RF_NORMal trace at the specified marker position in dBc/Hz units. | `MARKER:M<x>:AMPLitude:ABSolute?`<br>`MARKER:M<x>:AMPLitude:DELTa?`<br>`MARKER:M<x>:FREQuency:ABSolute <NR3>` |
| `MARKER:MANual` | 【开关】This command switches on or off the manual markers a and b that are available for frequency domain traces. | `MARKER:MANual` |
| `MARKER:PEAK:EXCURsion` | 【设置】This command specifies the excursion value, in user-selected units, for the frequency domain trace automatic peak markers. You can select the units with the command RF:UNIts. | `MARKER:PEAK:EXCURsion <NR3>`<br>`MARKER:PEAK:EXCURsion?` |
| `MARKER:PEAK:MAXimum` | 【设置】This command specifies the maximum number of frequency domain trace peaks that should have automatic markers associated with them. This can be a number between 1 and 11. | `MARKER:PEAK:MAXimum <NR1>`<br>`MARKER:PEAK:MAXimum?` |
| `MARKER:PEAK:STATE` | 【开关】This command switches on or off the automat ic peak markers that are available for frequency domain traces. The default is 1 (on). There are up to 11 automatic markers. | `MARKER:PEAK:STATE {OFF\|ON\|0\|1}`<br>`MARKER:PEAK:STATE?` |
| `MARKER:PEAK:THReshold` | 【设置】This command specifies the threshold value, in the same vertical units as the source waveform, of the automatic peak markers available for frequency domain traces. (Use the RF:UNIts to specify the units.)。 | `MARKER:PEAK:THReshold <NR3>`<br>`MARKER:PEAK:THReshold?` |
| `MARKER:REFERence` | 【控制】This command changes the Center Frequency to the frequency indicated by the Reference Marker, in effect moving the Reference Marker to the center of the screen. | `MARKER:REFERence` |
| `MARKER:REFERence:AMPlitude?` | 【控制】This query returns the amplitude (vertical) value of the Reference Marker in dBm when markers are turned on (using the command MARKER:PEAK:STATE or MARKER:MANual). | `MARKER:REFERence:AMPlitude?` |
| `MARKER:REFERence:FREQuency?` | 【控制】This query returns the frequency of the Reference Marker when the frequency domain trace markers have been turned on (using either the command MARKER:PEAK:STATE or MARKER:MANual). | `MARKER:REFERence:FREQuency?` |
| `MARKER:TYPe` | 【设置】This command specifies the marker type (either DELTa or ABSolute) to use when the automatic markers for the frequency domain traces are turned on. To turn on the automatic markers,。 | `MARKER:TYPe {DELTa\|ABSolute}`<br>`MARKER:TYPe?` |
| `RF:DETECTionmethod:RF_MINHold` | 【设置】This command specifies the detection method the oscilloscope should use when creating an RF Min Hold trace in the frequency domain. The Min Hold trace displays the smallest value throughout the acquisition history at each trace point. | `RF:DETECTionmethod:RF_MINHold`<br>`RF:DETECTionmethod:RF_MINHold?` |
| `RF:DETECTionmethod:RF_NORMal` | 【设置】This command specifies the detection method the oscilloscope should use when creating an RF Normal trace in the frequency domain. The Normal trace displays the most recently acquired sample at each trace point. | `RF:DETECTionmethod:RF_NORMal`<br>`RF:DETECTionmethod:RF_NORMal?` |
| `RF:FREQuency` | 【设置】This command specifies the center frequency of the RF acquisition system. The center frequency range varies with the model:RF:LABel This command specifies a general label for the RF frequency domain traces. RF:MEASUre:ACPR:ADJACENTPAIRs When the RF measurement type has been set to ACPR, the frequency domain displays a Main channel in the center (Ch:Main), and a side channel group on either side of the Main Channel. There can be either 1, 2 or 3 channels within each side group; this command specifies that number. (Lower Area 1, 2 and 3 would be on the left side of the Main channel; Upper Area 1, 2 and 3 would be on the right side). To set the measurement type to ACPR, use the command。 | `RF:FREQuency <NR3>`<br>`RF:FREQuency?` |
| `RFMEASUre:TYPe` | 【控制】ACPR . | `RF:MEASUre:ACPR:ADJACENTPAIRs[ 1\|2\|3]`<br>`RF:MEASUre:ACPR:ADJACENTPAIRs?` |
| `RF:MEASUre:ACPR:CHANBW` | 【控制】This command con figures the measurement bandwidth to use for the Main channel, as well as the adjacent side channels, when performing ACPR measurements using a frequency domain trace. The RF measurement type must first be set to ACPR using the command。 | `RF:MEASUre:ACPR:CHANBW <NR3>`<br>`RF:MEASUre:ACPR:CHANBW?` |
| `RF:MEASUre:ACPR:CHANSPACing` | 【设置】This command specifies the center-to-center spacing between the Main channel and adjacent channels when performing ACPR measurements using a frequency domain trace. (The RF measurement type must be set to ACPR using the command RF:MEASUre:TYPe.) Note that if the channel spacing is adjusted to be more narrow than the channel bandwidth, then the oscilloscope will automatically decrease the channel bandwidth. | `RF:MEASUre:ACPR:CHANSPACing <NR3>`<br>`RF:MEASUre:ACPR:CHANSPACing?` |
| `RF:MEASUre:ACPR:UA1DB?` | 【控制】This query measures a ratio between the first upper side channel and the Main channel when performing ACPR measurements using a frequency domain trace. The power in the adjacent channel is equivalent to the power in the main channel (dBm) added to the power ratio (dB) of the adjacent channel. (The RF measurement type must be set to ACPR using the command RF:MEASUre:TYPe.)。 | `RF:MEASUre:ACPR:UA1DB?` |
| `RF:MEASUre:ACPR:UA2DB?` | 【控制】This query measures a ratio between the second upper side channel and the Main channel when performing ACPR measurements using a frequency domain trace. The power in the adjacent channel is equivalent to the power in the main channel (dBm) added to the power ratio (dB) of the adjacent channel. (The RF measurement type must be set to ACPR using the command RF:MEASUre:TYPe.)。 | `RF:MEASUre:ACPR:UA2DB?` |
| `RF:MEASUre:ACPR:UA3DB?` | 【控制】This query measures a ratio between the third upper side channel and the Main channel when performing ACPR measurements using a frequency domain trace. The power in the adjacent channel is equivalent to the power in the main channel (dBm) added to the power ratio (dB) of the adjacent channel. (The RF measurement type must be set to ACPR using the command RF:MEASUre:TYPe.)。 | `RF:MEASUre:ACPR:UA3DB?` |
| `RF:MEASUre:CP:CHANBW` | 【设置】This command specifies the channel bandwidth to use when the RF measurement type has been set to Channel Power (CP) using the command RF:MEASUre:TYPe. | `RF:MEASUre:CP:CHANBW <NR3>`<br>`RF:MEASUre:CP:CHANBW?` |
| `RF:MEASUre:CP:POWer?` | 【控制】This query returns the total channel power within the displayed channel bandwidth, when the RF measurement type has been set to CP (using the command RF:MEASUre:TYPe). | `RF:MEASUre:CP:POWer?` |
| `RF:MEASUre:OBW:CHANBW` | 【设置】This command specifies the Analysis Bandwidth to use, when the measurement type has been set to OBW (using the command RF:MEASUre:TYPe). Note that the span automatically increases or decreases to be 10% more than the Analysis Bandwidth (providing some room around the signal of interest). | `RF:MEASUre:OBW:CHANBW <NR3>`<br>`RF:MEASUre:OBW:CHANBW?` |
| `RF:MEASUre:OBW:LOWERFreq?` | 【控制】This query returns the lower frequency threshold (on the display, the white line to the left bracketing OBW power). The RF measurement type must be set to OBW using the command RF:MEASUre:TYPe. | `RF:MEASUre:OBW:LOWERFreq?` |
| `RF:MEASUre:OBW:PERCENTdown` | 【设置】This command specifies the percentage of total power within the Analysis Bandwidth (the OBW power) such that half of the remaining power will be below the OBW:LOWERFreq level and the other half of the remaining power will be above the OBW:UPPERFreq level. | `RF:MEASUre:OBW:PERCENTdown <NR3>`<br>`RF:MEASUre:OBW:PERCENTdown?` |
| `RF:MEASUre:OBW:POWer?` | 【控制】This query returns the total channel power within the occupied bandwidth, when the RF measurement type has been set to OBW (using the command RF:MEASUre:TYPe). | `RF:MEASUre:OBW:POWer?` |
| `RF:MEASUre:OBW:UPPERFreq?` | 【控制】This query returns the upper frequency threshhold (on the display, the white line to the right bracketing OBW power). The RF measurement type must be set to OBW using the command RF:MEASUre:TYPe. | `RF:MEASUre:OBW:UPPERFreq?` |
| `RF:MEASUre:TYPe` | 【设置】This command specifies the RF measurement type: Channel Power, Adjacent Channel Power Ratio, Occupied Bandwidth, or none. | `RF:MEASUre:TYPe` |
| `RF:POSition` | 【设置】This command specifies the vertical position for the frequency domain traces. The vertical position is the location of the Reference Level with respect to the top of the graticule, in divisions. The lower limit is –10 divisions. The upper limit is +10 divisions. | `RF:POSition` |
| `RF:PRObe:AUTOZero` | 【控制】This command executes the attached probe’s AutoZero function, for probes that support this feature. | `RF:PRObe:AUTOZero EXECute` |
| `RF:PRObe:CALibrate` | 【控制】This command executes a calibration or initialization for a probe attached to the RF input, if the probe is calibratable. | `RF:PRObe:CALibrate {EXECute\|INITialize}` |
| `RF:PRObe:CALibrate:CALIBRATABLe?` | 【控制】This query returns a boolean value that indicates whether the attached probe is calibratable. | `RF:PRObe:CALibrate:CALIBRATABLe?` |
| `RF:PRObe:CALibrate:STATE?` | 本指令返回 校准 state of the probe connected to the RF input. | `RF:PRObe:CALibrate:STATE? Returns DEFAULT — Not calibrated` |
| `RF:PRObe:COMMAND` | 【控制】This command sets the state of the probe control specified with the first argument to the state specified with the second argument. | `RF:PRObe:COMMAND <QString>, <QString>` |
| `RF:PRObe:DEGAUss` | 【控制】This command starts a degauss/AutoZero cycle on a TekVPI current probe attached to the RF input. | `RF:PRObe:DEGAUss EXECute` |
| `RF:PRObe:DEGAUss:STATE?` | 【控制】This command returns the state of the probe degauss for the RF input. | `RF:PRObe:DEGAUss:STATE? Returns NEEDED indicatestheprobeshouldbedegaussedbeforetakingmeasurements.` |
| `RF:PRObe:FORCEDRange` | 【设置】This command specifies the range of a TekVPI probe attached to the RF input. | `RF:PRObe:FORCEDRange <NR3>`<br>`RF:PRObe:FORCEDRange?` |
| `RF:PRObe:GAIN` | 【设置】This command specifies the scale factor for the probe attached to the RF input. | `RF:PRObe:GAIN <NR3>`<br>`RF:PRObe:GAIN?` |
| `RF:PRObe:ID:SERnumber?` | 【控制】This query returns the serial number of the probe attached to the RF input. | `RF:PRObe:ID:SERnumber?` |
| `RF:PRObe:ID:TYPe?` | 本查询返回 type of probe attached to the RF input. | `RF:PRObe:ID:TYPe?` |
| `RF:PRObe:PREAmp:MODe` | 【设置】Sets or returns the user selected mode for an RF pre-amp connected to the RF input. BYPass or AUTO。 | `RF:PRObe:PREAmp:MODe` |
| `RF:PRObe:PREAmp:STATus?` | 【返回/查询】Returns the actual state of the RF pre-amp connected to the RF input. NONe, ON,o r BYPass。 | `RF:PRObe:PREAmp:STATus?` |
| `RF:PRObe:RESistance?` | 【控制】This query returns the input resistance of the probe attached to the RF input, if the probe supports it (otherwise, it returns 0.0). The RF input is 50 Ω impedance. | `RF:PRObe:RESistance?` |
| `RF:PRObe:SIGnal` | 【设置】This command specifies the input bypass setting of a TekVPI probe attached to the RF input. The probe must support input bypass. | `RF:PRObe:SIGnal` |
| `RF:PRObe:UNIts?` | 【控制】This query returns a quoted string that describes the units of measure for the probe attached to the RF input. | `RF:PRObe:UNIts?` |
| `RF:RBW` | 【设置】This command specifies the resolution bandwidth (RBW) setting when the RBW mode has been set to MANUAL (using the command RF:RBW:MODe). The resolution bandwidth is the width of the narrowest measurable band of frequencies in a frequency domain trace. The RBW is adjustable down to 20Hz. By default, the RBW tracks the span value in a 1000:1 ratio. | `RF:RBW <NR3>`<br>`RF:RBW?` |
| `RF:RBW:MODe` | 【设置】This command specifies the resolution bandwidth (RBW) mode, either automatic or manual. | `RF:RBW:MODe {AUTO\|MANual}`<br>`RF:RBW:MODe?` |
| `RF:REFLevel` | 【控制】This command sets the Reference Level of the RF input. The Reference Level can either be specified as a numeric floating point value, or set automatically. | `RF:REFLevel` |
| `RF:RF_AMPlitude:LABel` | 【设置】This command specifies the label for the RF Amplitude vs. Time trace. | `RF:RF_AMPlitude:LABel <QString>`<br>`RF:RF_AMPlitude:LABel?` |
| `RF:RF_AMPlitude:VERTical:POSition` | 【设置】This command specifies the vertical position of the RF Amplitude vs. Time trace. The position value determines the vertical graticule location at which the trace is displayed. Increasing the position value of a waveform causes the waveform to move up. Decreasing the position value causes the waveform to move down. The minimum is -50 divisions and the maximum is 50 divisions with a resolution of 0.02 divisions. | `RF:RF_AMPlitude:VERTical:POSition <NR3>`<br>`RF:RF_AMPlitude:VERTical:POSition?` |
| `RF:RF_AMPlitude:VERTical:SCAle` | 【设置】This command specifies the vertical scale for the RF Amplitude vs. Time trace. For a signal with constant amplitude, increasing the scale c auses the waveform to be displayed smaller. Decreasing the scale causes the waveform to be displayed larger. | `RF:RF_AMPlitude:VERTical:SCAle <NR3>`<br>`RF:RF_AMPlitude:VERTical:SCAle?` |
| `RF:RF_AVErage:COUNt?` | 【控制】This query returns the number of RF traces that have been accumulated to create an RF Average frequency domain trace. | `RF:RF_AVErage:COUNt?` |
| `RF:RF_AVErage:NUMAVg` | 【设置】This command specifies the number of acquisitions to be used when creating an RF Average frequency domain trace, which displays the average of values from multiple acquisitions at each trace point. The default is 16. The range is 2 – 512, in exponential increments. | `RF:RF_AVErage:NUMAVg <NR1>`<br>`RF:RF_AVErage:NUMAVg?` |
| `RF:RF_PHASe:REFERence:DEGrees` | 【设置】Sets or returns the phase, in degrees, at the trigger point for the RF_PHASe time domain trace. | `RF:RF_PHASe:REFERence:DEGrees` |
| `RF:RF_PHASe:WRAP:DEGrees` | 【设置】Sets or returns the number of degrees to wrap the RF_PHASe time domain trace. | `RF:RF_PHASe:WRAP:DEGrees <NR3>`<br>`RF:RF_PHASe:WRAP:DEGrees?` |
| `RF:RF_PHASe:WRAP:STATE` | 【设置】Sets or returns the state of the phase wrap control for the RF_PHASE time domain trace. | `RF:RF_PHASe:WRAP:STATE <Boolean>`<br>`RF:RF_PHASe:WRAP:STATE?` |
| `RF:RF_V_TIMe:BANDWidth` | 【设置】Sets or returns the RF versus time bandwidth as an NR3 value in Hz. | `RF:RF_V_TIMe:BANDWidth`<br>`RF:RF_V_TIMe:BANDWidth?` |
| `RF:SCAle` | 【设置】This command specifies the overall vertical scale setting of the frequency domain window. The lower limit is 0.1 dB/division. The upper limit is 100dB/division. The vertical scale is adjustable in a 1–2–5 sequence. | `RF:SCAle <NR3>`<br>`RF:SCAle?` |
| `RF:SPAN` | 【设置】This command specifies the span setting. The span is the range of frequencies that can be observed around the center frequency. This is the width of the frequency domain trace, which is equal to the stop frequency minus the start frequency. | `RF:SPAN <NR3>`<br>`RF:SPAN?` |
| `RF:SPANRbwratio` | 【设置】This command specifies the ratio of the span to the resolution bandwidth (RBW) that will be used when the RBW Mode is set to AUTO. (In order to set the RBW Mode to AUTO, use the command RF:RBW:MODe.)。 | `RF:SPANRbwratio <NR3>`<br>`RF:SPANRbwratio?` |
| `RF:SPECTRogram` | 【控制】Clears the spectrogram. | `RF:SPECTRogram {CLEAR}` |
| `RF:SPECTRogram:NUMSLICEs?` | 【控制】This query returns the number of spectrogram slices that are currently being rendered. A spectrogram slice is a section of the spectrogram representing one interval, or slice, of time in the spectrogram record. | `RF:SPECTRogram:NUMSLICEs?` |
| `RF:SPECTRogram:SLICESELect` | 【设置】This command specifies the spectrogram slice number that is to be displayed. Allowable slice numbers range from 0 to –327 in full-screen mode, and 0 to –147 in split-screen mode. (The range is negative because the numbering starts with the latest slice (0) and proceeds backwards in time.) The slice can only be selected or changed when acquisitions have been stopped. As soon as acquisitions start again, the slice number is reset to 0 (the latest slice). Attempts to select a slice number outside of range, or when acquisitions are running, are ignored. The query form returns the currently selected spectrogram slice. To use this command, first turn on the spectrogram ( RF:SPECTRogram:STATE). Then query the number of slices ( RF:SPECTRogram:NUMSLICEs?). Stop the acquisition when you’ve reached the number of desired slices. Then select the slice to display (RF:SPECTRogram:SLICESELect ). Each slice of the spectrogram corresponds to a single RF acquisition. The FFT samples the entire spectrum for the incoming signal (at the rate with which new spectrums are acquired). The newest spectrum is on the bottom edge of the spectrogram, and the oldest is on the top edge. When the oscilloscope is stopped, you can scroll “back in time” through the spectrogram。 | `RF:SPECTRogram:SLICESELect <NR1>`<br>`RF:SPECTRogram:SLICESELect?` |
| `RF:UNIts` | 【设置】This command specifies the vertical units to be used in all RF-related absolute logarithmic amplitudes. | `RF:UNIts {DBM\|DBUW\|DBMV\|DBUV\|DBMA\|DBUA}`<br>`RF:UNIts?` |
| `RF:WINdow` | 【设置】This command specifies which window will be used for the windowing function, which is only used for the three time domain RF traces (Amplitude vs. Time, Frequency vs. Time and Phase vs. Time). The default window is Kaiser. | `RF:WINdow` |
| `SELect:RF_AVErage` | 【开关】This command switches the RF Average trace display on or off in the frequency domain graticule. | `SELect:RF_AVErage {OFF\|ON\|0\|1}`<br>`SELect:RF_AVErage?` |
| `SELect:RF_MAXHold` | 【开关】This command switches the frequency domain Max Hold trace display on or off in the frequency domain graticule. | `SELect:RF_MAXHold {OFF\|ON\|0\|1}`<br>`SELect:RF_MAXHold?` |
| `SELect:RF_MINHold` | 【开关】This command switches the frequency domain Min Hold trace display on or off in the frequency domain graticule. | `SELect:RF_MINHold {OFF\|ON\|0\|1}`<br>`SELect:RF_MINHold?` |
| `SELect:RF_NORMal` | 【开关】This command switches the frequency domain Normal trace display on or off in the frequency domain graticule. | `SELect:RF_NORMal {OFF\|ON\|0\|1}`<br>`SELect:RF_NORMal?` |

## 23. 保存与调用 (Save/Recall)

手册原名：*Save and Recall Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `FACtory` | Resets the 示波器 to 出厂 default settings。 | `FACtory` |
| `*RCL` | Recalls 保存d 示波器 settings。 | `*RCL <NR1>` |
| `RECAll:SETUp` | Recalls 保存d 示波器 settings。 | `RECAll:SETUp {FACtory\|<NR1>\|<file path>}` |
| `RECAll:SETUp:DEMO3<x>` | 【控制】This command recalls one of the 6 specified built-in demonstration setups of RF functionality. <x> can be 1 through 6. The demonstrations include 1. Multiple Peaks, 2. spectrogram, 3. VCO/PLL Turn On, 4. ASK Modulation, 5. Frequency Hop and 6. Capture BW. | `RECAll:SETUp:DEMO3<x>` |
| `RECAll:WAVEform` | 【控制】This command (no query form) recalls a stored waveform to a reference memory location, and, for instruments with the arbitrary wavefor m feature, to arbitrary waveform edit memory (EMEM). Only the first waveform in the .CSV file is recalled for multiple waveform .CSV files. Recall of digital waveforms (D0 through D15) is not supported. | `RECAll:WAVEform <Source>, <Destination>` |
| `*SAV` | 【控制】Stores the state of the oscilloscope to a specified memory location。 | `*SAV <NR1>` |
| `SAVe:ASSIgn:TYPe` | 本指令设置 assignment of the 保存 button。 | `SAVe:ASSIgn:TYPe {IMAGe\|WAVEform\|SETUp}`<br>`SAVe:ASSIgn:TYPe?` |
| `SAVe:EVENTtable:{BUS<x>\|B<x>}` | 【控制】Saves event table data from bus<x> to a specified file。 | `SAVe:EVENTtable:{BUS<x>\|B<x>} <file path>` |
| `SAVe:IMAGe` | 【控制】Saves a capture of the screen image to the specified file。 | `SAVe:IMAGe` |
| `SAVe:IMAGe:FILEFormat` | 【设置】This command specifies the file format to use for saving screen images. The file format is not automatically determined by the file name extension. You need to choose a file name with an extension which is consistent with the selected file format。 | `SAVe:IMAGe:FILEFormat {PNG\|BMP\|TIFf}`<br>`SAVe:IMAGe:FILEFormat?` |
| `SAVe:IMAGe:INKSaver` | 本指令设置 current ink保存r setting for the SAVe:IMAGe command。 | `SAVe:IMAGe:INKSaver {OFF\|ON\|0\|1}`<br>`SAVe:IMAGe:INKSaver?` |
| `SAVe:IMAGe:LAYout` | 本指令设置 layout to use for 保存d screen images。 | `SAVe:IMAGe:LAYout {LANdscape\|PORTRait}`<br>`SAVe:IMAGe:LAYout?` |
| `SAVe:SETUp` | 【控制】Saves the state of the oscilloscope to a specified memory location or file。 | `SAVe:SETUp {<file path>\|<NR1>}` |
| `SAVe:WAVEform` | 【控制】This command saves the specified waveform to the specified destination reference memory slot, or saves the specified waveform(s) to the specified destination file. The same function can be accomplished from the front panel Menu->save waveform menu. The type of file saved is dependent upon the SAVe:WAVEform:FILEFormat command. | `SAVe:WAVEform <Source>,<Destination>` |
| `SAVe:WAVEform:FILEFormat` | 【设置】This command specifies the format for saving waveforms. This command specifies the file format to be used when saving waveforms — either an internal format, .ISF, or an external comma-delimited spreadsheet format, .CSV, that includes waveform header and timing information. | `SAVe:WAVEform:FILEFormat {INTERNal\|SPREADSheet}`<br>`SAVe:WAVEform:FILEFormat?` |
| `SAVe:WAVEform:FILEFormat:RF_BB_IQ` | 本指令设置 文件格式 for saving the RF baseband I & Q data. The default format is TIQ. | `SAVe:WAVEform:FILEFormat:RF_BB_IQ {TIQ\|MATLAB}`<br>`SAVe:WAVEform:FILEFormat:RF_BB_IQ?` |
| `SAVe:WAVEform:GATIng` | 设置whether 保存 波形 operations should 保存 the entire 波形 or a specified portion of the 波形。 | `SAVe:WAVEform:GATIng {NONe\|CURSors\|SCREEN}`<br>`SAVe:WAVEform:GATIng?` |
| `SETUP1<x>:DATE?` | 返回 date when the specified 示波器 setup was 保存d。 | `SETUP1<x>:DATE?`<br>`SETUP1<x>:LABel <Qstring>`<br>`SETUP1<x>:TIMe?` |
| `SETUP1<x>:LABel` | 本指令设置 specified 示波器 setup label。 | `SETUP1<x>:DATE?`<br>`SETUP1<x>:LABel <Qstring>`<br>`SETUP1<x>:TIMe?` |
| `SETUP1<x>:TIMe?` | 【返回/查询】Returns the time when the specified oscilloscope setup was saved Search Comman dG r o u p Thesearchcommandsletyouanalyzeyoursourcewaveformrecordforcondit ions specifiedbyasearch’scriteria. Once thesecriteriaarematched,theosci lloscope places a search mark at that location in the waveform record. You can then navigateorsavethemarks. (Seepage2-28,。 | `SETUP1<x>:DATE?`<br>`SETUP1<x>:LABel <Qstring>`<br>`SETUP1<x>:TIMe?` |

## 24. 搜索 (Search)

手册原名：*Search Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `SEARCH?` | 返回全部 搜索-related settings。 | `SEARCH?` |
| `SEARCH:SEARCH<x>:COPy` | 复制the 搜索 criteria to the 触发, or the 触发 criteria to the 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:LIST?` | 【控制】This query returns a list of all automatically created search marks on waveforms in the time domain (leaving out any manually created marks). These automatic marks are created using a search command. The entries returned are in the form of an enumeration representing the source waveform, followed by 7 time mark parameters. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:STATE` | 【控制】S e t st h es e a r c hs t a t et oo no ro f f。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TOTal?` | 返回 total number of matches for 搜索 <x>。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS?` | 【控制】Queries the SEARCH:SEARCH<x>:TRIGger:A:BUS settings. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger` | 【控制】:A:BUS:B<x>:ARINC429A:CONDition This command sets the condition to use when searching on ARINC429 bus data (word start, label, matching data, word end, or error). | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:ARINC429A:LABel:VALue` | 【设置】This command specifies the low value to use when searching on the ARINC429 bus label field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:ARINC429A:SDI` | 【设置】This command specifies the SDI portion of the packet data to be used when searching on ARINC429 bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:ARINC429A:SSM` | 【设置】This command specifies the SSM portion of the packet data to be used when searching on ARINC429 bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:CONDition` | 【控制】This command sets the condition (start of frame or matching data) to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:DATa:HIVALue` | 【控制】This command sets the upper word value to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:DATa:OFFSet` | 【控制】This commands sets the data offset value to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:DATa:QUALifier` | 【控制】This command sets the quali fier (<, >, =, <=, >=, not =, in range, out of range) to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:DATa:VALue` | 【控制】This command sets the lower word value to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:AUDio:DATa:WORD` | 【控制】This command sets the alignment of the data (left, right or either) to be used to search on audio bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:CONDition` | 【控制】This command sets the condition (start of frame, frame type, identifier, matching data, end of frame, missing ACK field, bit-stuffing error, form error, any error, CAN FD BRS bit, or CAN FD ESI bit) to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:DATa:DIRection` | 【控制】This command sets the data direction (read, write or either) to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:DATa:OFFSet` | 本指令设置 data offset for CAN data 搜索es. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:DATa:QUALifier` | 【控制】This command sets the quali fier (<, >, =, not =, <=) to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:DATa:SIZe` | 【控制】This command sets the length of the data string, in bytes, to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:DATa:VALue` | 【控制】This command sets the binary data value to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:FD:BRSBIT` | 【控制】This command sets the value (don’t care, 1, or 0) to be used to search for CAN FD BRS bits. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:FD:ESIBIT` | 【控制】This command sets the value (don’t care, 1, or 0) to be used to search for CAN FD ESI bits. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN:FRAMEtype` | 【控制】This command sets the frame type (data, remote, error or overload) to be used to search on CAN bus data. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN{: IDentifier\|:ADDRess}:MODe This command sets the addressing mode (standard or extended format) to be used to search on CAN bus data. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:CAN{: IDentifier\|:ADDRess}:VALue This command sets the binary address value to be used to search on CAN bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:CONDition` | 【设置】This command specifies the condition to use when searching on FlexRay bus data (start of frame, frame type, ID, cycle count, header, data, ID and data, EOF, error). | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:HIVALue` | 【设置】This command specifies the upper data value of the range to be used when searching on the FlexRay bus cycle count field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:QUALifier` | 【设置】This command specifies the quali fier (<, >, =, <=, >=, not =, in range, out of range) to use when searching on the FlexRay bus cycle count field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:VALue` | 【设置】This command specifies the low data value to be used when searching on the FlexRay bus cycle count field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:DATa:HIVALue` | 【设置】This command specifies the high value to use when searching on the FlexRay bus data field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:DATa:OFFSet` | 【设置】This command specifies the offset of the data string in bytes to be used when searching on the FlexRay bus data field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:DATa:QUALifier` | 【设置】This command specifies the qualifier (<, >, =, <=, >=, not =, in range, out of range) to use when searching on the FlexRay bus data field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:DATa:SIZe` | 【设置】This command specifies the length of the data string, in bytes, to use when searching on the FlexRay bus data field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:DATa:VALue` | 【设置】This command specifies the low value to use when searching on the FlexRay bus data field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:EOFTYPE` | 【设置】This command specifies which end of file type to use (static, dynamic or any) when searching on the FlexRay bus EOF field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:ERRTYPE` | 本指令设置 error type to use when 搜索ing on the FlexRay bus signal. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:HIVALue` | 【设置】This command specifies the high value to use when searching on the FlexRay bus frame ID field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:QUALifier` | 【设置】This command specifies the quali fier to use when searching on the FlexRay bus frame ID field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:VALue` | 【设置】This command specifies the low value to use when searching on the FlexRay bus frame ID field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:FRAMEType` | 【设置】This command specifies the frame type (normal, payload, null, sync or startup) to use when searching on FlexRay bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:HEADer:CRC` | 【设置】This command specifies the CRC portion of the binary header string to be used when searching on FlexRay bus data. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:HEADer:CYCLEcount` | 【设置】This command specifies to use the cycle count portion of the binary header string when searching on the FlexRay bus header. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:HEADer:FRAMEID` | 【设置】This command specifies to use the frame ID portion of the binary header string when searching on the FlexRay bus header. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:HEADer:INDBits` | 【设置】This command specifies to use the indicator bits portion of the binary header string when searching on the FlexRay bus header. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:FLEXray:HEADer:PAYLength` | 【设置】This command specifies to use the payload length portion of the binary header string when searching on the FlexRay bus header. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:ADDRess:MODe` | 本指令设置 I2C address mode to 7 or 10-Bit。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:ADDRess:TYPe` | 本指令设置 I2C address type to I2C special addresses。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:ADDRess:VALue` | 本指令设置 binary address string to be used for I2C 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:CONDition` | 本指令设置 搜索 condition for I2C 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:DATa:DIRection` | 本指令设置 I2C 搜索 condition to be valid on a READ, WRITE or either。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:DATa:SIZe` | 本指令设置 length of the data string in bytes to be used for I2C 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:I2C:DATa:VALue` | 【控制】This command speci fie st h eb i n a r yd a t as t r i n gt ob eu s e df o rI 2 C search。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:CONDition` | 本指令设置 搜索 condition for a LIN 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:DATa:HIVALue` | 本指令设置 binary data string。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:DATa:QUALifier` | 本指令设置 LIN data quali fier。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:DATa:SIZe` | 本指令设置 length of the data string in bytes。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:DATa:VALue` | 本指令设置 binary data string used for a LIN 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:ERRTYPE` | 本指令设置 error type used for a LIN 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:LIN:IDenti` | 【控制】fier:VALue This command specifies the binary address string used for LIN search SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:HIVALue When the MIL-STD-1553 bus search condition is set to COMMAND, and the quali fier is set to INrange or OUTrange, this command specifies the upper limit of the range for the remote terminal address field. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:QUALi` | 【控制】fier When the MIL-STD-1553 bus search condition is set to COMMAND, this command specifies the quali fier to be used with the remote terminal address field. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:VALue When the MIL-STD-1553 bus search condition is set to COMMAND, and the quali fier is set to LESSthan, MOREthan, EQual, UNEQual, LESSEQual or MOREEQual, this command specifies the value of the 5–bit remote terminal address to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:COUNt When the MIL-STD-1553 bus search condition is set to COMMAND, this command specifies the bit pattern for the 5–bit Word Count/Mode Code sub-address field that is to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:PARity When the MIL-STD-1553 bus search condition is set to COMMAND, this command specifies the Command word parity that is to be used in the search. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:CONDition` | 【设置】This command specifies a word type or condition within a MIL-STD-1553 bus word to search for. SEARCH<x> is the search number, which is always 1, and B<x> SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:DATa:PARity When the MIL-STD-1553 bus search condition is set to DATa,t h i s command specifies the data parity bit to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:DATa:VALue When the MIL-STD-1553 bus search condition is set to DATa,t h i s command specifies the data binary pattern to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:ERRTYPE When the MIL-STD-1553 bus search condition is set to ERRor, this command specifies the signaling error type to be used in the search: Parity, Sync, Manchester or Data. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:HIVALue When the MIL-STD-1553 bus search condition is set to STATus, and the quali fier is set to INrange or OUTrange, this command specifies the upper limit for the 5 bit remote terminal address field of the Status word. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:VALue When the MIL-STD-1553 bus search condition is set to STATus, and the quali fier is set to LESSthan, MOREthan, EQual, UNEQual, LESSEQual or MOREEQual, this command specifies the value of the 5–bit remote terminal address to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:QUALifier When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the quali fier to be used with the address field. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:BCR When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the status word broadcast command received (BCR) bit value (bit 15) to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:BUSY When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the status word busy bit value (bit 16) to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:DBCA When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the status word dynamic bus control acceptance (DBCA) bit value (bit 18) to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:INSTR When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the status word instrumentation bit value (bit 10) to be used in the search. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:ME When the MIL-STD-1553 bus search condition is set to STATus, this command specifies the status word message error bit value (bit 9) to be used in the search. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:PARallel:VALue` | 【控制】This command speci fie st h eb i n a r yd a t as t r i n gt ob eu s e df o ra Parallel search。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:RS232C:CONDition` | 本指令设置 搜索 condition for an RS-232 触发。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:RS232C:RX:DATa:SIZe` | 本指令设置 length of the data string for an RS-232搜索, if the 搜索 condition is RX。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:RS232C:RX:DATa:VALue` | 【设置】This command specifies the binary data string for an RS-232 search, if the condition involves RX。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:RS232C:TX:DATa:SIZe` | 【设置】This command specifies the length of the data string to be used for an RS-232 search, if the search condition is TX。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:RS232C:TX:DATa:VALue` | 【设置】This command specifies the binary data string to be used for an RS-232 search, if the condition involves RX。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:SPI:CONDition` | 【设置】This command specifies the search condition for SPI search SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:SPI:DATa{: MISO\|:IN}:VALue This command speci fie st h eb i n a r yd a t as t r i n gt ob eu s e df o rS P I search if the search condition is MISO or MISOMOSI SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:SPI:DATa{: MOSI\|:OUT}:VALue This command specifies the binary data string for an SPI search if the search condition is MISO or MISOMOSI。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:SPI:DATa:SIZe` | 本指令设置 length of the data string in bytes to be used for SPI 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:ADDRess:HIVALue` | 本指令设置 high limit for USB address 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:ADDRess:VALue` | 本指令设置 value for USB address 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:CONDition` | 本指令设置 USB 搜索 condition. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:DATa:HIVALue` | 本指令设置 high limit for USB data 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:DATa:OFFSet` | 本指令设置 data offset for USB data 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:DATa:SIZe` | 本指令设置 number of data bytes for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:DATa:TYPe` | 本指令设置 data type for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:DATa:VALue` | 本指令设置 data value for USB data 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:ENDPoint:VALue` | 本指令设置 endpoint value for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:ERRTYPE` | 本指令设置 error type for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:HANDSHAKEType` | 本指令设置 handshake type for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SOFFRAMENUMber` | 本指令设置 SOF number for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPECIALType` | 【设置】This command specifies the special packet type for USB searches SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPLit:ET:VALue When searching on a high-speed USB split transaction, this command specifies the split transaction endpoint type value to search for. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPLit:HUB:VALue When searching on a high-speed USB split transaction, this command specifies the split transaction hub address value to search for. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPLit:PORT:VALue When searching on a high-speed USB split transaction, this command specifies the split transaction port address value to search for. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPLit:SC:VALue When searching on a high-speed USB split transaction, this command specifies whether to search for the start or complete phase of the split transaction, based on the Start/Complete bit field value. SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:SPLit:SE:VALue When searching for a high-speed USB split transaction, this command specifies the split transaction start/end bit value to search for. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:B<x>:USB:TOKENType` | 本指令设置 token type for USB 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:BUS:SOUrce` | 本指令设置 bus for a serial 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:EDGE:SLOpe` | 本指令设置 slope to be used in an edge 搜索:rising, falling or either。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:EDGE:SOUrce` | 本指令设置 source 波形 for an edge 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LEVel:CH<x>` | 【设置】Sets the threshold level to use when searching on an analog waveform. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LEVel:MATH` | Sets the threshold level to use when 搜索ing on the 数学波形. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LEVel:REF<x>` | Sets the threshold level to use when 搜索ing on a 参考 波形. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:FUNCtion` | 【设置】Specifies the logic operator to be used in a logic search. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:PATtern:INPut:CH<x>` | 本指令设置 logic operator for the logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:CH<x>` | 【设置】Specifies the logic condition to be used in a logic search when the input is an analog channel. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:CLOCk:EDGE` | 本指令设置 whether the clock edge is rise or fall for a logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:CLOCk:SOUrce` | 本指令设置 clock source de finition for logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:D<x>` | 【设置】This command specifies the criteria for a logic search to determine where to place a mark for digital channel <x>。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:MATH` | 本指令设置 Boolean logic criteria for the logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:INPut:REF<x>` | 本指令设置 Boolean logic criteria for the logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:PATtern:WHEn` | 本指令设置 condition for generating a logic pattern 搜索. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:PATtern:WHEn:LESSLimit` | 【设置】This command specifies the maximum time that the selected pattern may be true. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:PATtern:WHEn:MORELimit` | 【设置】This command specifies the minimum time that the selected pattern may be true. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:THReshold:CH<x>` | 本指令设置 通道 threshold level for a logic 搜索. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:THReshold:MATH` | 本指令设置 数学波形 threshold level for a logic 搜索. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOGIc:THReshold:REF<x>` | 本指令设置 参考 波形 threshold level for a logic 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A` | :LOWerthreshold:CH<x> 本指令设置 lower 波形 threshold level for all 通道 波形 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:LOWerthreshold:REF<x>` | 本指令设置 lower 波形 threshold level for all 参考 波形 搜索es。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:HIGHLimit` | 【设置】This command specifies the upper limit, in seconds, when searching the record for pulses whose widths are within or outside of a specified range of two values. (Use。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:LOWLimit` | 【控制】to specify the lower limit of the range.)。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:POLarity` | 本指令设置 polarity for a pulse 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:SOUrce` | 本指令设置 source 波形 for a pulse 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:WHEn` | 【设置】This command specifies to search the waveform record for pulses with a width (duration) that is less than, greater than, equal to, or unequal to a specified value (set using SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:WIDth ), OR whose widths fall outside of or within a specified range of two values (set using。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:PULSEWidth:WIDth` | 【设置】This command specifies the width setting to use, in seconds, when searching the waveform record for pulses of a certain width (duration). | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:RUNT:POLarity` | 本指令设置 polarity setting for a runt 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:RUNT:SOUrce` | 本指令设置 source setting for a runt 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:RUNT` | :WHEn 本指令设置 condition setting for a runt 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:RUNT:WIDth` | 本指令设置 width setting for a runt 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:CLOCk:EDGE` | 本指令设置 clock slope setting for a setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:CLOCk:SOUrce` | 本指令设置 clock source setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:CLOCk:THReshold` | 本指令设置 clock threshold setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:DATa:SOUrce` | 本指令设置 data source setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:DATa:THReshold` | 本指令设置 data threshold setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:HOLDTime` | 本指令设置 hold time setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:SETTime` | 本指令设置 setup time setting for an setup/hold 搜索。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:SETHold:THReshold:REF<x>` | 本指令设置 搜索 setup and hold threshold for the selected 参考 波形。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:TIMEOut:POLarity` | 【控制】When searching using the TIMEOut search type, this commands specifies the polarity to be used. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:TIMEOut:SOUrce` | 【控制】When searching using the TIMEOut search type, this command specifies the source. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:TIMEOut:TIMe` | 【控制】When searching using the TIMEOut search type, this command specifies the timeout time, in seconds. SEARCH:SEARCH<x>:TRIGger:A{: TRANsition\|:RISEFall}:DELTatime This command specifies the transition time setting for an transition search SEARCH:SEARCH<x>:TRIGger:A{: TRANsition\|:RISEFall}:POLarity This command specifies the polarity setting for a transition search SEARCH:SEARCH<x>:TRIGger:A{: TRANsition\|:RISEFall}:SOUrce This command specifies the source setting for a transition search SEARCH:SEARCH<x>:TRIGger:A{: TRANsition\|:RISEFall}:WHEn This command specifies the condition setting for a transition search。 | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `SEARCH:SEARCH<x>:TRIGger:A:TYPe` | 【设置】This command specifies the search type, ie. EDGe\|PULSEWidth\|SETHold\|RUNt\|TRANsition\|LOGIc\|TIMEOut\|BUS. | `SEARCH:SEARCH<x>:LIST?`<br>`SEARCH:SEARCH<x>:TOTal? Returns <NR1> is the total number of matches.`<br>`SEARCH:SEARCH<x>:TRIGger:A:BUS? Returns I2C specifies the Inter-IC bus.` |
| `TRIGger:A:BUS:B<x>:CAN:FD:BRSBIT` | 本指令设置 binary data value used to 搜索 on C A NF DB R Sb i t s . | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:FD:ESIBIT` | 本指令设置 binary data value used to 搜索 on C A NF DE S Ib i t s . | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |

## 25. 状态与错误 (Status/Error)

手册原名：*Status and Error Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ALLEv?` | 返回全部 ev ents and their messages。 | `ALLEv?` |
| `BUSY?` | 【返回/查询】Returns osci lloscope status。 | `BUSY?` |
| `*CLS` | 【控制】Clears status。 | `*CLS` |
| `DESE` | 本指令设置 bits in the Device Event Status 使能Register。 | `DESE <NR1>`<br>`DESE?` |
| `*ESE` | 本指令设置 bits in the Event Status 使能Register。 | `*ESE` |
| `*ESR?` | 【返回/查询】Returns the contents of the Standard Event Status Register。 | `*ESR?` |
| `EVENT?` | 【返回/查询】Returns event code from the event queue。 | `EVENT?` |
| `EVMsg?` | 【返回/查询】Returns event code, message from the event queue。 | `EVMsg?` |
| `EVQty?` | 【控制】Return number of events in the event queue。 | `EVQty?` |
| `*OPC` | 【控制】Generates the operation complete message in the standard event status register when all pending operations are finished Or returns "1" when all current operations are finished。 | `*OPC` |
| `*OPT?` | 【返回/查询】Returns a comma-separated list of installed options (not to be confused with application modules) as an arbitrary ASCII string. | `*OPT?` |
| `*PSC` | 本指令设置 power on status flag。 | `*PSC {OFF\|ON\|NR1>}`<br>`*PSC?` |
| `*PUD` | 本指令设置 a string of protected user data。 | `*PUD {<Block>\|<QString>}`<br>`*PUD?` |
| `*RST` | Resets the 示波器 to 出厂 default settings。 | `*RST` |
| `*SRE` | 本指令设置 bits in the Service Request 使能Register。 | `*SRE <NR1>`<br>`*SRE?` |
| `*STB?` | 【返回/查询】Returns the contents of the Status Byte Register。 | `*STB?` |
| `*WAI` | 【控制】Prevents the oscilloscope from executing further commands until all pending operations finish。 | `*WAI` |

## 26. 触发 (Trigger)

手册原名：*Trigger Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `TRIGger` | 【控制】Forces a trigger event to occur。 | `TRIGger?` |
| `TRIGger:A` | 【设置】Sets A trigger l evel to 50% or returns current A trigger parameters。 | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A:BUS` | 【控制】This command s pecifies the bus type to trigger on. | `TRIGger:A:BUS` |
| `TRIGger:A:BUS:B<x>:ARINC429A:CONDition` | 【控制】This command s ets the condition to use when triggering on a ARINC429 bus signal (word start, label, matching data, word end, or error). | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue` | 【设置】This command specifies the high value to use when triggering on a ARINC429 bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:DATa:QUALifier` | 【控制】This command sets the quali fier (<, >, =, <=, >=, not =, in range, out of range) to use when t riggering on a ARINC429 bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:DATa:VALue` | 本指令设置 low value to use when 触发ing on a ARINC429 bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:ERRTYPE` | 本指令设置 error type to use when 触发ing on a ARINC429 bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:LABel:HIVALue` | 【设置】This command specifies the high value to use when triggering on a ARINC429 bus label field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:LABel:QUALifier` | 【控制】This command sets the quali fier (<, >, =, <=, >=, not =, in range, out of range) to use when triggering on a ARINC429 bus label field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:LABel:VALue` | 【设置】This command specifies the low value to use when triggering on a ARINC429 bus label field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:SDI` | 【设置】This command specifies the SDI portion of the packet data to be used when triggering on ARINC429 bus data. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:ARINC429A:SSM` | 【设置】This command specifies the SSM portion of the packet data to be used when triggering on a ARINC429 bus data. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:CONDition` | 【控制】This command sets the condition (start of frame or matching data) to be used when triggering on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:DATa:HIVALue` | 【控制】This command sets the upper word value to be used when triggering on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:DATa:OFFSet` | 【控制】This command sets the data offset value to be used when triggering on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:DATa:QUALifier` | 【控制】This command sets the quali fier (<, >, =, <=, >=, not =, in range, out of range) to be used when triggering on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:DATa:VALue` | 【控制】This command sets the lower word value to be used when triggering on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:AUDio:DATa:WORD` | 【控制】This command sets the alignment of the data (left, right or either) to be used to trigger on an audio bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:CONDition` | 【控制】This command sets the condition (start of frame, frame type, identi fier, matching data, end of frame, missing ACK field, bit-stuffing error, form error, any error, CAN FD BRS bit, or CAN FD ESI bit) to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:DATa:DIRection` | 【控制】This command sets the data direction (read, write or “nocare”) to be used to search on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:DATa:OFFSet` | 本指令设置 data offset for CAN data 触发ing。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:DATa:QUALifier` | 【控制】This command sets the quali fier (<, >, =, not =, <=) to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:DATa:SIZe` | 【控制】This command sets the length of the data string, in bytes, to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:DATa:VALue` | 【控制】This command sets the binary data value to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:FD:BRSBIT` | 【控制】This command sets the value (don’t care, 1, or 0) to be used to trigger on C A NF DB R Sb i t s。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:FD:ESIBIT` | 【控制】This command sets the value (don’t care, 1, or 0) to be used to trigger on CAN FD ESI bits。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN:FRAMEtype` | 【控制】This command sets the frame type (data, remote, error or overload) to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:CAN{:IDenti` | 【控制】fier\|:ADDRess}:MODe This command sets the addressing mode (standard or extended format) to be used when triggering on a CAN bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:CONDition` | 【设置】This command specifies the condition to use when triggering on a FlexRay bus signal (start of frame, frame type, ID, cycle count, header, data, ID and data, EOF, error). | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:HIVALue` | 本指令设置 high value when 触发ing on a FlexRay bus cycle count field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:QUALifier` | 【设置】This command specifies the quali fier (<, >, =, <=, >=, not =, in range, out of range) to use when triggering on the FlexRay bus cycle count field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:CYCLEcount:VALue` | 【设置】This command specifies the low value when triggering on the FlexRay bus cycle count field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:DATa:HIVALue` | 本指令设置 high value when 触发ing on the FlexRay bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:DATa:OFFSet` | 【设置】This command specifies the offset of the data string, in bytes, when triggering on the FlexRay bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:DATa:QUALifier` | 【设置】This command specifies the quali fier (<, >, =, <=, >=, not =, in range, out of range) to use when triggering on the FlexRay bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:DATa:SIZe` | 【设置】This command specifies the length of the data string, in bytes, when triggering on the FlexRay bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:DATa:VALue` | 本指令设置 low value when 触发ing on the FlexRay bus data field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:EOFTYPE` | 【设置】This command specifies the end of file type (static, dynamic or any) when triggering on the FlexRay bus EOF field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:ERRTYPE` | 本指令设置 error type when 触发ing on the FlexRay bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:HIVALue` | 本指令设置 high value when 触发ing on the FlexRay bus frame ID field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:QUALifier` | 【设置】This command specifies the quali fier to use when triggering on the FlexRay bus frame ID field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:FRAMEID:VALue` | 本指令设置 low value when 触发ing on the FlexRay bus frame ID field. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:FRAMEType` | 【设置】This command specifies the frame type (normal, payload, null, sync or startup) when triggering on the FlexRay bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:HEADer:CRC` | 【设置】This command specifies the CRC portion of the binary header string when triggering on the FlexRay bus signal. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:HEADer:CYCLEcount` | 【设置】This command specifies the cycle count portion of the binary header string when triggering on the FlexRay bus header. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:HEADer:FRAMEID` | 【设置】This command specifies the frame ID portion of the binary header string when triggering on the FlexRay bus header. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:HEADer:INDBits` | 【设置】This command specifies the indicator bits portion of the binary header string when triggering on the FlexRay bus header. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:FLEXray:HEADer:PAYLength` | 【设置】This command specifies the payload length portion of the binary header string when triggering on the FlexRay bus header. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:ADDRess:MODe` | 本指令设置 I2C address mode to 7 or 10-bit。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:ADDRess:TYPe` | 本指令设置 I2C address type to USER。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:ADDRess:VALue` | 本指令设置 binary address string used for the I2C 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:CONDition` | 本指令设置 触发 condition for I2C 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:DATa:DIRection` | 本指令设置 I2C 触发 condition valid on a READ, WRITE, or either。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:DATa:SIZe` | 本指令设置 length of the data string in bytes to be used for I2C 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:I2C:DATa:VALue` | 本指令设置 binary data string used for I2C 触发ing。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:CONDition` | 本指令设置 触发 condition for LIN。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:DATa:HIVALue` | 本指令设置 binary data string to be used for LIN 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:DATa:QUALifier` | 本指令设置 LIN data quali fier。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:DATa:SIZe` | 本指令设置 length of the data string in bytes to be used for LIN 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:DATa:VALue` | 本指令设置 binary data string。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:ERRTYPE` | 本指令设置 error type。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:LIN:IDenti` | 【控制】fier:VALue This command specifies the binary address string used for LIN trigger TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:HIVALue When the MIL-STD-1553 bus trigger condition is set to COMMAND, and the qualifier is set to INrange or OUTrange, this command specifies the upper limit of the range for the remote terminal address field. TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:QUALifier When the MIL-STD-1553 bus trigger condition is set to COMMAND,t h i s command specifies the quali fier to be used with the remote terminal address field. TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:ADDRess:VALue When the MIL-STD-1553 bus trigger condition is set to COMMAND, and the qualifier is set to LESSthan, MOREthan, EQual, UNEQual, LESSEQual or MOREEQual, this command specifies the value of the 5–bit remote terminal address to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:COUNt When the MIL-STD-1553 bus trigger condition is set to COMMAND,t h i s command specifies the bit pattern for the 5–bit Word Count/Mode Code sub-address field that is to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:PARity When the MIL-STD-1553 bus trigger condition is set to COMMAND,t h i s command specifies the Command word parity that is to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:COMMAND:SUBADdress When the MIL-STD-1553 bus trigger condition is set to COMMAND,t h i s command specifies the 5 bit sub-address that is to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:CONDition` | 【设置】This command specifies a word type or condition within a MIL-STD-1553 bus word to trigger on. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:DATa:PARity` | 【控制】When the MIL-STD-1553 bus trigger condition is set to DATa, this command specifies the data parity bit to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:DATa:VALue` | 【控制】When the MIL-STD-1553 bus trigger condition is set to DATa, this command specifies the data binary pattern to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:ERRTYPE` | 【控制】When the MIL-STD-1553 bus trigger condition is set to ERRor, this command specifies the signaling error type to be used in the trigger: Parity, Sync, Manchester or Data. TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:HIVALue When the MIL-STD-1553 bus trigger condition is set to STATus, and the qualifier is set to INrange or OUTrange, this command specifies the upper limit for the 5 bit remote terminal address field of the Status word. TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:QUALifier When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the quali fier to be used with the address field. TRIGger:A:BUS:B<x>:MIL1553B:STATus:ADDRess:VALue When the MIL-STD-1553 bus trigger condition is set to STATus, and the qualifier is set to LESSthan, MOREthan, EQual, UNEQual, LESSEQual or MOREEQual, this command specifies the value of the 5–bit remote terminal address to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:BCR` | 【控制】When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status word broadcast command received (BCR) bit value (bit 15) to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:BUSY When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status word busy bit value (bit 16) to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:DBCA When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status word dynamic bus control acceptance (DBCA) bit value (bit 18) to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:INSTR When the MIL-STD-1553 bus trigger condition is set to STATus,t h i s command specifies the status word instrumentation bit value (bit 10) to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:ME` | 【控制】When the MIL-STD-1553 bus trigger condition is set to STATus,t h i s command specifies the status word message error bit value (bit 9) to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:SRQ` | 【控制】When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status word service request (SRQ) bit value (bit 11) to be used in the trigger. TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:SUBSF When the MIL-STD-1553 bus trigger condition is set to STATus,t h i s command specifies the status word subsystem flag bit value (bit 17) to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:STATus:BIT:TF` | 【控制】When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status word terminal flag bit value (bit 19) to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:STATus:PARity` | 【控制】When the MIL-STD-1553 bus trigger condition is set to STATus, this command specifies the status parity bit value to be used in the trigger. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:TIMe:LESSLimit` | 【控制】When the MIL-STD-1553 bus trigger condition is set to TIMe, this command specifies either the minimum remote terminal response time (RT) limit for the amount of time the terminal has to transmit, or it specifies the minimum inter-message gap (IMG). | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:TIMe:MORELimit` | 【控制】When the MIL-STD-1553 bus trigger condition is set to TIMe, this command specifies either the maximum remote terminal response time (RT) limit for the amount of time the terminal has to transmit, or it specifies the maximum inter-message gap (IMG). | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:MIL1553B:TIMe:QUALi` | 【控制】fier When the MIL-STD-1553 bus trigger condition is set to TIMe, this command specifies the trigger data time quali fier. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:PARallel:VALue` | 本指令设置 binary data string to be used for a Parallel 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:RS232C:CONDition` | 本指令设置 condition for an RS-232C 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:RS232C:RX:DATa:SIZe` | 【设置】This command specifies the length of the data string in Bytes for an RX RS-232 Trigger。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:RS232C:RX:DATa:VALue` | 本指令设置 binary data string for an RX RS-232 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:RS232C:TX:DATa:SIZe` | 本指令设置 length of the data string for a TX RS-232 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:RS232C:TX:DATa:VALue` | 本指令设置 binary data string for an RS-232 触发 if the 触发 condition involves TX。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:SPI:CONDition` | 本指令设置 触发 condition for SPI 触发ing。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:SPI:DATa{:IN\|:MISO}:VALue` | 本指令设置 binary data string to be used for SPI 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:SPI:DATa{:OUT\|:MOSI}:VALue` | 本指令设置 binary data string used for the SPI 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:SPI:DATa:SIZe` | 本指令设置 length of the data string in bytes to be used for SPI 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:ADDRess:HIVALue` | 本指令设置 high limit for the USB 触发 address。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:ADDRess:VALue` | 本指令设置 value for the USB 触发 address。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:CONDition` | 本指令设置 USB 触发 condition。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:DATa:HIVALue` | 本指令设置 high limit for the USB 触发 data。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:DATa:OFFSet` | 本指令设置 data offset for the USB 触发 data。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:DATa:SIZe` | 本指令设置 number of data bytes for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:DATa:TYPe` | 本指令设置 data type for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:DATa:VALue` | 本指令设置 data value for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:ENDPoint:VALue` | 本指令设置 endpoint value for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:ERRTYPE` | 本指令设置 error type for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:HANDSHAKEType` | 本指令设置 handshake type for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:QUALi` | fier 本指令设置 quali fier for USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SOFFRAMENUMber` | 本指令设置 SOF number for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPECIALType` | 本指令设置 special packet type for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPLit:ET:VALue` | 【控制】When triggering on a high-speed USB split transaction, this command specifies the split transaction endpoint type value to trigger on. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPLit:HUB:VALue` | 【控制】When triggering on a high-speed USB split transaction, this command specifies the split transaction hub address value to trigger on. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPLit:PORT:VALue` | 【控制】When triggering on a high-speed USB split transaction, this command specifies the split transaction port address value to trigger on. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPLit:SC:VALue` | 【控制】When triggering on a high-speed USB split transaction, this command specifies whether to trigger on the start or complete phase of the split transaction, based on the Start/Complete bit field value. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:SPLit:SE:VALue` | 【控制】When triggering on a high-speed USB split transaction, this command specifies the split transaction start/end bit value to trigger on. | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:B<x>:USB:TOKENType` | 本指令设置 token type for the USB 触发。 | `TRIGger:A:BUS:B<x>:ARINC429A:CONDition`<br>`TRIGger:A:BUS:B<x>:ARINC429A:CONDition?`<br>`TRIGger:A:BUS:B<x>:ARINC429A:DATa:HIVALue <QString>` |
| `TRIGger:A:BUS:SOUrce` | 本指令设置 source for a bus 触发。 | `TRIGger:A:BUS:SOUrce {B1\|B2\|B3\|B4}`<br>`TRIGger:A:BUS:SOUrce?` |
| `TRIGger:A:EDGE?` | 【返回/查询】Returns the source, coupling and source for the A edge trigger。 | `TRIGger:A:EDGE?` |
| `TRIGger:A:EDGE:COUPling` | 本指令设置 type of coupling for the A edge 触发。 | `TRIGger:A:EDGE:COUPling {AC\|DC\|HFRej\|LFRej\|NOISErej}`<br>`TRIGger:A:EDGE:COUPling?` |
| `TRIGger:A:EDGE:SLOpe` | 本指令设置 slope for the A edge 触发: rising, falling or either. | `TRIGger:A:EDGE:SLOpe` |
| `TRIGger:A:EDGE:SOUrce` | 本指令设置 source for the A edge 触发。 | `TRIGger:A:EDGE:SOUrce {AUX\|CH1\|CH2\|CH3\|CH4\|` |
| `TRIGger:A:HOLDoff?` | 返回 A 触发 holdoff parameters。 | `TRIGger:A:HOLDoff?`<br>`TRIGger:A:HOLDoff:TIMe <NR3>`<br>`TRIGger:A:HOLDoff:TIMe?` |
| `TRIGger:A:HOLDoff:TIMe` | 本指令设置 A 触发 holdoff time。 | `TRIGger:A:HOLDoff:TIMe <NR3>`<br>`TRIGger:A:HOLDoff:TIMe?` |
| `TRIGger:A:LEVel:AUXin` | 【设置】Sets the threshold voltage level for an Edge, Pulse Width, Runt or Rise/Fall (aka Transition, aka Slew Rate) trigger to use when triggering on the Aux Input connector signal. | `TRIGger:A:LEVel:AUXin {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LEVel:AUXin?` |
| `TRIGger:A:LEVel:CH<x>` | 【设置】Sets the threshold voltage level for an Edge, Pulse Width, Runt or Rise/Fall (aka Transition, aka Slew Rate) trigger to use when triggering on an analog channel waveform. | `TRIGger:A:LEVel:CH<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LEVel:CH<x>?` |
| `TRIGger:A:LEVel:D<x>` | 【设置】Sets the threshold voltage level for an Edge, Pulse Width, Runt or Rise/Fall (aka Transition, aka Slew Rate) trigger to use when triggering on a digital channel waveform. | `TRIGger:A:LEVel:D<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LEVel:D<x>?` |
| `TRIGger:A:LOGIc?` | 返回全部 A 触发 logic settings。 | `TRIGger:A:LOGIc?` |
| `TRIGger:A:LOGIc:CLAss` | 【控制】This command sets the class of the logic trigger (either logic or setup/hold). You also need to set the trigger type using the command TRIGger:A:TYPe. | `TRIGger:A:LOGIc:CLAss {LOGIC\|SETHold}`<br>`TRIGger:A:LOGIc:CLAss?` |
| `TRIGger:A:LOGIc:FUNCtion` | 本指令设置 logical combination of the input 通道s for the A logic 触发。 | `TRIGger:A:LOGIc:FUNCtion {AND\|NANd\|NOR\|OR}`<br>`TRIGger:A:LOGIc:FUNCtion?` |
| `TRIGger:A:LOGIc:INPut?` | 返回 logic input values for all 通道s。 | `TRIGger:A:LOGIc:INPut?`<br>`TRIGger:A:LOGIc:INPut:CH<x> {HIGH\|LOW\|X}`<br>`TRIGger:A:LOGIc:INPut:CH<x>?` |
| `TRIGger:A:LOGIc:INPut:CH<x>` | 【设置】Specifies or returns the logic setting for the specified channel。 | `TRIGger:A:LOGIc:INPut:CH<x> {HIGH\|LOW\|X}`<br>`TRIGger:A:LOGIc:INPut:CH<x>?` |
| `TRIGger:A:LOGIc:INPut:CLOCk:EDGE` | 【设置】Sets the polarity of the clock channel。 | `TRIGger:A:LOGIc:INPut:CLOCk:SOUrce {CH1\|CH2\|CH3\|CH4\|` |
| `TRIGger:A:LOGIc:INPut:CLOCk:SOUrce` | 本指令设置 通道 to use as the clock source。 | `TRIGger:A:LOGIc:INPut:CLOCk:SOUrce {CH1\|CH2\|CH3\|CH4\|` |
| `TRIGger:A:LOGIc:INPut:D<x>` | 本指令设置 logic pattern for a 触发 on digital 通道 <x>。 | `TRIGger:A:LOGIc:INPut:D<x> {HIGH\|LOW\|X}`<br>`TRIGger:A:LOGIc:INPut:D<x>?` |
| `TRIGger:A:LOGIc:INPut:RF` | 【设置】This command specifies the logic level to use when the internal RF power level is the source for a logic pattern trigger. | `TRIGger:A:LOGIc:INPut:RF {HIGH\|LOW\|X}`<br>`TRIGger:A:LOGIc:INPut:RF?` |
| `TRIGger:A:LOGIc:PATtern?` | 【返回/查询】Returns the conditions for generating an A logic pattern trigger。 | `TRIGger:A:LOGIc:PATtern?` |
| `TRIGger:A:LOGIc:PATtern:DELTatime` | 本指令设置 pattern 触发 delta time value。 | `TRIGger:A:LOGIc:PATtern:DELTatime <NR3>`<br>`TRIGger:A:LOGIc:PATtern:DELTatime?` |
| `TRIGger:A:LOGIc:PATtern:WHEn` | 本指令设置 pattern logic condition on which to 触发 the 示波器。 | `TRIGger:A:LOGIc:PATtern:WHEn`<br>`TRIGger:A:LOGIc:PATtern:WHEn?` |
| `TRIGger:A:LOGIc:THReshold:CH<x>` | 【设置或查询】Sets or queries the trigger A logic threshold voltage for the specified channel. This command specifies the threshold to use when the internal RF power level is the source for a logic trigger. It will affect all trigger types using the channel. | `TRIGger:A:LOGIc:THReshold:CH<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LOGIc:THReshold:CH<x>?` |
| `TRIGger:A:LOGIc:THReshold:D<x>` | 本指令设置 触发 A logic threshold level for the specified digital 通道. | `TRIGger:A:LOGIc:THReshold:D<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LOGIc:THReshold:D<x>?` |
| `TRIGger:A:LOWerthreshold:CH<x>` | 本指令设置 lower threshold for the 通道 selected. | `TRIGger:A:LOWerthreshold:CH<x> {ECL\|TTL\|<NR3>}`<br>`TRIGger:A:LOWerthreshold:CH<x>?` |
| `TRIGger:A:LOWerthreshold:D<x>` | 【设置】Sets the A trigger lower threshold for the digital channel selected. | `TRIGger:A:LOWerthreshold:D<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LOWerthreshold:D<x>?` |
| `TRIGger:A:LOWerthreshold{:AUX\|:EXT}` | 【设置】This command specifies the lower threshold for the Aux Input connector. | `TRIGger:A:LOWerthreshold{:AUX\|:EXT} {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:LOWerthreshold{:AUX\|:EXT}?` |
| `TRIGger:A:MODe` | 本指令设置 A 触发 mode – either AUTO or NORMAL. | `TRIGger:A:MODe {AUTO\|NORMal}`<br>`TRIGger:A:MODe?` |
| `TRIGger:A:PULse:CLAss` | 【设置】This command specifies which kind of pulse to trigger on (either runt, width, transition (rise/fall or slew rate) or timeout). You also need to set the trigger type to PULSe using the command TRIGger:A:TYPe. | `TRIGger:A:PULse:CLAss {RUNt\|WIDth\|TRANsition\|TIMEOut}`<br>`TRIGger:A:PULse:CLAss?` |
| `TRIGger:A:PULSEWidth:HIGHLimit` | 【设置】This command specifies the upper limit to use, in seconds, when triggering on detection of a pulse whose duration is inside or outside a range of two values. (Use TRIGger:A:PULSEWidth:LOWLimit to specify the lower value of the range.)。 | `TRIGger:A:PULSEWidth:HIGHLimit <NR3>`<br>`TRIGger:A:PULSEWidth:HIGHLimit?` |
| `TRIGger:A:PULSEWidth:LOWLimit` | 【设置】This command specifies the lower limit to use, in seconds, when triggering on detection of a pulse whose duration is inside or outside a range of two values. (Use TRIGger:A:PULSEWidth:HIGHLimit to specify the upper limit of the range.)。 | `TRIGger:A:PULSEWidth:LOWLimit <NR3>`<br>`TRIGger:A:PULSEWidth:LOWLimit?` |
| `TRIGger:A:PULSEWidth:POLarity` | 本指令设置 polarity for the A pulse width 触发。 | `TRIGger:A:PULSEWidth:POLarity` |
| `TRIGger:A:PULSEWidth:SOUrce` | 本指令设置 source for the pulse width 触发。 | `TRIGger:A:PULSEWidth:SOUrce` |
| `TRIGger:A:PULSEWidth:WHEn` | 【设置】This command specifies to trigger when a pulse is detected with a width (duration) that is less than, greater than, equal to, or unequal to a specified value (set using TRIGger:A:PULSEWidth:WIDth ), OR whose width falls outside of or within a specified range of two values (set using。 | `TRIGger:A:PULSEWidth:WHEn`<br>`TRIGger:A:PULSEWidth:WHEn?` |
| `TRIGger:A:PULSEWidth:WIDth` | 【设置】This command specifies the pulse width (duration), in seconds, for triggering on pulses whose widths are greater than, less than, equal to, or not equal to the specified value. | `TRIGger:A:PULSEWidth:WIDth <NR3>`<br>`TRIGger:A:PULSEWidth:WIDth?` |
| `TRIGger:A:RUNT?` | 返回当前 A runt pulse 触发 logic parameters。 | `TRIGger:A:RUNT?` |
| `TRIGger:A:RUNT:POLarity` | 本指令设置 polarity for the A pulse runt 触发。 | `TRIGger:A:RUNT:POLarity` |
| `TRIGger:A:RUNT:SOUrce` | 本指令设置 source for the A pulse 触发。 | `TRIGger:A:RUNT:SOUrce {CH1\|CH2\|CH3\|CH4}`<br>`TRIGger:A:RUNT:SOUrce?` |
| `TRIGger:A:RUNT:WHEn` | 【设置】This command specifies the type of pulse width the trigger checks for when it uncovers a runt。 | `TRIGger:A:RUNT:WHEn {LESSthan\|MOREthan\|EQual\|UNEQual\|OCCURS}`<br>`TRIGger:A:RUNT:WHEn?` |
| `TRIGger:A:RUNT:WIDth` | 本指令设置 minimum width for A pulse runt 触发。 | `TRIGger:A:RUNT:WIDth <NR3>`<br>`TRIGger:A:RUNT:WIDth?` |
| `TRIGger:A:SETHold?` | 【返回/查询】Returns settings for setup and hold violation triggering。 | `TRIGger:A:SETHold?` |
| `TRIGger:A:SETHold:CLOCk?` | 【返回/查询】Returns clock edge polarity, voltage threshold and source input for setup/hold triggering。 | `TRIGger:A:SETHold:CLOCk?` |
| `TRIGger:A:SETHold:CLOCk:EDGE` | 本指令设置 clock edge polarity for setup and hold 触发ing。 | `TRIGger:A:SETHold:CLOCk:EDGE {FALL\|RISe}`<br>`TRIGger:A:SETHold:CLOCk:EDGE?` |
| `TRIGger:A:SETHold:CLOCk:SOUrce` | 本指令设置 clock source for the A logic 触发 setup and hold input。 | `TRIGger:A:SETHold:CLOCk:SOUrce`<br>`TRIGger:A:SETHold:CLOCk:SOUrce?` |
| `TRIGger:A:SETHold:CLOCk:THReshold` | 本指令设置 clock voltage threshold for setup and hold 触发。 | `TRIGger:A:SETHold:CLOCk:THReshold {<NR3>\|TTL}`<br>`TRIGger:A:SETHold:CLOCk:THReshold?` |
| `TRIGger:A:SETHold:DATa?` | 【返回/查询】Returns the voltage threshold and data source for the setup/hold trigger。 | `TRIGger:A:SETHold:DATa?` |
| `TRIGger:A:SETHold:DATa:SOUrce` | 本指令设置 data source for the setup and hold 触发。 | `TRIGger:A:SETHold:DATa:SOUrce {CH1\|CH2\|CH3\|CH4\|`<br>`TRIGger:A:SETHold:DATa:SOUrce?` |
| `TRIGger:A:SETHold:DATa:THReshold` | 本指令设置 data voltage threshold for setup and hold 触发。 | `TRIGger:A:SETHold:DATa:THReshold` |
| `TRIGger:A:SETHold:HOLDTime` | 本指令设置 hold time for the setup and hold violation 触发ing。 | `TRIGger:A:SETHold:HOLDTime <NR3>`<br>`TRIGger:A:SETHold:HOLDTime?` |
| `TRIGger:A:SETHold:SETTime` | 本指令设置 setup time for setup and hold violation 触发ing。 | `TRIGger:A:SETHold:SETTime` |
| `TRIGger:A:SETHold:THReshold:CH<x>` | 设置或查询 threshold for the 通道。 | `TRIGger:A:SETHold:THReshold:CH<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:SETHold:THReshold:CH<x>?` |
| `TRIGger:A:SETHold:THReshold:D<x>` | 【设置】Sets the A trigger setup and hold threshold for the selected digital channel。 | `TRIGger:A:SETHold:THReshold:D<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:SETHold:THReshold:D<x>?` |
| `TRIGger:A:TIMEOut:POLarity` | 【控制】When triggering using the TIMEOut trigger type, this commands specifies the polarity to be used. | `TRIGger:A:TIMEOut:POLarity {STAYSHigh\|STAYSLow\|EITher}`<br>`TRIGger:A:TIMEOut:POLarity?` |
| `TRIGger:A:TIMEOut:SOUrce` | 【控制】When triggering using the TIMEOut trigger type, this command specifies the source. The available sources are live channels, external (or auxillary) input, and digital channels. The default is channel 1. | `TRIGger:A:TIMEOut:SOUrce {CH1\|CH2\|CH3\|CH4\|LINE\|AUX\|` |
| `TRIGger:A:TIMEOut:TIMe` | 【控制】When triggering using the TIMEOut trigger type, this command specifies the timeout time, in seconds. The default and minimum is 4.0E-9 seconds and the maximum is 8.0 seconds. The resolution is 800.0E-12 which means that the increments of time specified is 800 picoseconds. | `TRIGger:A:TIMEOut:TIMe <NR3>`<br>`TRIGger:A:TIMEOut:TIMe?` |
| `TRIGger:A:TYPe` | 【控制】This command sets the type of A trigger (edge, logic, pulse, bus or video). If you set the trigger type to LOGIc, you also need to set the logic trigger class (logic or setup/hold) using the command TRIGger:A:LOGIc:CLAss .I fy o us e t the trigger type to PULSe, you also need to set the pulse trigger class (runt, width, transition or timeout), using the command TRIGger:A:PULse:CLAss . | `TRIGger:A:TYPe {EDGe\|LOGIc\|PULSe\|BUS\|VIDeo}`<br>`TRIGger:A:TYPe?` |
| `TRIGger:A{:TRANsition\|:RISEFall}?` | 【返回/查询】Returns the delta time, polarity, and both upper and lower threshold limits for the transition time trigger。 | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A{:TRANsition\|` | 【控制】:RISEFall}:DELTatime This command specifies the delta time used in calculating the transition value。 | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A{:TRANsition\|:RISEFall}:POLarity` | 本指令设置 polarity for the A pulse transition 触发。 | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A{:TRANsition\|:RISEFall}:SOUrce` | 本指令设置 source for transition 触发. | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A{:TRANsition\|:RISEFall}:WHEn` | 【设置】This command specifies the relationship of delta time to transitioning signal。 | `TRIGger:A{:TRANsition\|:RISEFall}?`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime <NR3>`<br>`TRIGger:A{:TRANsition\|:RISEFall}:DELTatime?` |
| `TRIGger:A:UPPerthreshold:CH<x>` | 【设置】Sets the upper threshold for the channel selected。 | `TRIGger:A:UPPerthreshold:CH<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:A:UPPerthreshold:CH<x>?` |
| `TRIGger:A:VIDeo:CUSTom{:FORMat\|:TYPe}` | 【控制】This command sets the video trigger format (either interlaced or progressive) to use for triggering on video signals. | `TRIGger:A:VIDeo:CUSTom{:FORMat\|:TYPe}`<br>`TRIGger:A:VIDeo:CUSTom{:FORMat\|:TYPe}?` |
| `TRIGger:A:VIDeo:CUSTom:LINEPeriod` | 【控制】This command sets the line period for the standard under test. | `TRIGger:A:VIDeo:CUSTom:LINEPeriod <NR3>`<br>`TRIGger:A:VIDeo:CUSTom:LINEPeriod?` |
| `TRIGger:A:VIDeo:CUSTom:SYNCInterval` | 【控制】This command sets the sync interval for the standard under test to use for triggering on video signals. This is only required for BiLevel Custom. | `TRIGger:A:VIDeo:CUSTom:SYNCInterval <NR3>`<br>`TRIGger:A:VIDeo:CUSTom:SYNCInterval?` |
| `TRIGger:A:VIDeo:STANdard` | 【控制】This command sets the standard to use for triggering on video signals. This command sets the video field to use for triggering on video signals (odd, even, all fields, all lines, numeric). | `TRIGger:A:VIDeo:STANdard {NTSc\|PAL\|SECAM\|BILevelcustom\|`<br>`TRIGger:A:VIDeo:STANdard?` |
| `TRIGger:B` | 【设置】Sets the B trigger level to 50% or returns the B trigger parameters。 | `TRIGger:B?` |
| `TRIGger:B:BY` | 本指令设置 B 触发 time or event quali fiers。 | `TRIGger:B:BY {EVENTS\|TIMe}`<br>`TRIGger:B:BY?` |
| `TRIGger:B:EDGE?` | 返回B 触发 edge type parameters。 | `TRIGger:B:EDGE?` |
| `TRIGger:B:EDGE:COUPling` | 本指令设置 type of B 触发 coupling。 | `TRIGger:B:EDGE:COUPling {DC\|HFRej\|LFRej\|NOISErej}`<br>`TRIGger:B:EDGE:COUPling?` |
| `TRIGger:B:EDGE:SLOpe` | 本指令设置 B edge 触发 slope。 | `TRIGger:B:EDGE:SLOpe {RISe\|FALL}`<br>`TRIGger:B:EDGE:SLOpe?` |
| `TRIGger:B:EDGE:SOUrce` | 本指令设置 B edge 触发 source。 | `TRIGger:B:EDGE:SOUrce {CH1\|CH2\|CH3\|CH4\|AUX\|LINE\|RF}`<br>`TRIGger:B:EDGE:SOUrce?` |
| `TRIGger:B:EVENTS?` | 返回当前 B 触发 events parameter。 | `TRIGger:B:EVENTS?` |
| `TRIGger:B:EVENTS:COUNt` | 本指令设置 number of events that must occur before the B 触发 occurs。 | `TRIGger:B:EVENTS:COUNt <NR1>`<br>`TRIGger:B:EVENTS:COUNt?` |
| `TRIGger:B:LEVel` | 本指令设置 level for the B 触发。 | `TRIGger:B:LEVel {TTL\|<NR3>}`<br>`TRIGger:B:LEVel?` |
| `TRIGger:B:LEVel:CH<x>` | 本指令设置 level for the B 触发 for a speci fic 通道。 | `TRIGger:B:LEVel:CH<x> {ECL\|TTL\|<NR3>}`<br>`TRIGger:B:LEVel:CH<x>?` |
| `TRIGger:B:LEVel:D<x>` | 本指令设置 B 触发 level for digital 通道 <x>。 | `TRIGger:B:LEVel:D<x> {ECL\|TTL\|<NR3>}`<br>`TRIGger:B:LEVel:D<x>?` |
| `TRIGger:B:LOWerthreshold:CH<x>` | 本指令设置 B 触发 lower threshold for the 通道 selected。 | `TRIGger:B:LOWerthreshold:CH<x> {ECL\|TTL\|<NR3>}`<br>`TRIGger:B:LOWerthreshold:CH<x>?` |
| `TRIGger:B:LOWerthreshold:D<x>` | 设置或查询 B 触发 lower threshold for the digital 通道 selected。 | `TRIGger:B:LOWerthreshold:D<x> {<NR3>\|ECL\|TTL}`<br>`TRIGger:B:LOWerthreshold:D<x>?` |
| `TRIGger:B:STATE` | 返回当前 state of the B 触发。 | `TRIGger:B:STATE {ON\|OFF\|<NR1>}`<br>`TRIGger:B:STATE?` |
| `TRIGger:B:TIMe` | 本指令设置 B 触发 delay time。 | `TRIGger:B:TIMe` |
| `TRIGger:B:TYPe` | 本指令设置 type of B 触发。 | `TRIGger:B:TYPe?`<br>`TRIGger:B:TYPe EDGE` |
| `TRIGger:EXTernal?` | 【返回/查询】Returns all external trigger-related parameters for the probe connected to the Aux Input connector. | `TRIGger:EXTernal?` |
| `TRIGger:EXTernal:PRObe` | 【设置】This command specifies the attenuation factor value of the probe connected to the Aux Input connector. | `TRIGger:EXTernal:PRObe <NR3>`<br>`TRIGger:EXTernal:PRObe?` |
| `TRIGger:EXTernal:YUNIts?` | 返回 external 触发 垂直 (Y) units value。 | `TRIGger:EXTernal:YUNIts?` |
| `TRIGger:FREQuency?` | 返回 触发 frequency in hertz, if available。 | `TRIGger:FREQuency?` |
| `TRIGger:STATE?` | 返回当前 state of the 触发ing system。 | `TRIGger:STATE?` |

## 27. 垂直通道 (Vertical)

手册原名：*Vertical Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `AUXin?` | 【返回/查询】Returns Aux Input connector parameters。 | `AUXin?` |
| `AUXin:PRObe` | 【返回/查询】Returns all information concerning the probe attached to Aux Input connector。 | `AUXin:PRObe`<br>`AUXin:PRObe?` |
| `AUXin:PRObe:AUTOZero` | 【设置】Sets the TekVPI probe attached to the Aux Input connector to autozero。 | `AUXin:PRObe:AUTOZero EXECute` |
| `AUXin:PRObe:CALibrate:CALIBRATABLe?` | 【控制】This query indicates whether the attached probe is calibratable. | `AUXin:PRObe:CALibrate:CALIBRATABLe?` |
| `AUXin:PRObe:COMMAND` | 【设置】Sets the state of the specified probe control。 | `AUXin:PRObe:COMMAND <QString>, <QString>` |
| `AUXin:PRObe:DEGAUss` | 【启停】Starts a degauss/autozero cycle on a TekVPI current probe attached to the Aux Input connector。 | `AUXin:PRObe:DEGAUss {EXECute}` |
| `AUXin:PRObe:DEGAUss:STATE?` | 【返回/查询】Returns the degauss state of the TekVPI current probe attached to the Aux Input connector。 | `AUXin:PRObe:DEGAUss:STATE?` |
| `AUXin:PRObe:FORCEDRange` | 【设置】This command specifies the range of the TekVPI probe attached to the Aux Input connector。 | `AUXin:PRObe:FORCEDRange <NR3>`<br>`AUXin:PRObe:FORCEDRange?` |
| `AUXin:PRObe:GAIN` | 【设置】This command specifies the gain factor of the probe that is attached to the Aux Input connector。 | `AUXin:PRObe:GAIN <NR3>`<br>`AUXin:PRObe:GAIN?` |
| `AUXin:PRObe:ID:SERnumber?` | 【返回/查询】Returns the serial number of the probe that is attached to the Aux Input connector。 | `AUXin:PRObe:ID:SERnumber?` |
| `AUXin:PRObe:ID:TYPe?` | 【返回/查询】Returns the type of probe that is attached to the Aux Input connector。 | `AUXin:PRObe:ID:TYPe?` |
| `AUXin:PRObe:RESistance?` | 【返回/查询】Returns the resistance of the probe that is attached to the Aux Input connector。 | `AUXin:PRObe:RESistance?` |
| `AUXin:PRObe:SIGnal` | 【设置】This command specifies the input bypass setting on VPI probes that support input bypass。 | `AUXin:PRObe:SIGnal {BYPass\|PASS}`<br>`AUXin:PRObe:SIGnal?` |
| `AUXin:PRObe:UNIts?` | 【返回/查询】Returns the units of measure of the probe that is attached to the Aux Input connector。 | `AUXin:PRObe:UNIts?` |
| `CH<x>?` | 返回垂直 parameters for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:AMPSVIAVOLTs:ENAble` | 本指令设置 state of the amps via volts feature for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:AMPSVIAVOLTs:FACtor` | 本指令设置 amps via volts factor for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:BANdwidth` | 本指令设置 bandwidth of the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:COUPling` | 本指令设置 coupling setting for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:DESKew` | 本指令设置 deskew time for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:INVert` | 本指令设置 invert function for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:LABel` | 本指令设置 波形 label for 通道 <x>。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:OFFSet` | 本指令设置 通道 offset。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:POSition` | 本指令设置 通道 垂直 position。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe?` | 【返回/查询】Returns the gain, resistance, units, and ID of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:AUTOZero` | 【设置】Sets the TekVPI probe attached to the specified channel input to autozero。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:CALibrate` | 【控制】This command executes a calibration or initialization for the probe attached to the auxilliary input, if the probe is calibratable. | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:CALibrate:CALIBRATABLe?` | 【控制】This query returns the state of the probe attached to channel 1–4, either 0 if the probe is not calibratable, or 1 if the probe is calibratable. | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:CALibrate:STATE?` | 本查询返回 校准 state of the probe connected to the specified 通道. | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:COMMAND` | 【设置】Sets the state of the specified probe control。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:DEGAUss` | 【启停】Starts a degauss/autozero cycle on a TekVPI current probe attached to the specified channel input。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:DEGAUss:STATE?` | 【返回/查询】Returns the state of the probe degauss。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:FORCEDRange` | 本指令设置 range on a TekVPI probe attached to the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:GAIN` | 【设置】This command specifies the gain factor of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:ID?` | 【返回/查询】Returns the type and serial number of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:ID:SERnumber?` | 【返回/查询】Returns the serial number of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:ID:TYPe?` | 【返回/查询】Returns the type of probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:MODel` | 本指令设置 probe model for the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:PROPDELay` | 【设置】This command specifies the propagation delay for the probe connected to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:RECDESkew?` | 【返回/查询】Returns the recommended deskew for the probe connected to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:RESistance?` | 【返回/查询】Returns the resistance of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:SIGnal` | 本指令设置 input bypass setting of 通道 <x>TekVPI probe。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:PRObe:UNIts?` | 【返回/查询】Returns the units of measure of the probe that is attached to the specified channel。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:SCAle` | 本指令设置 垂直 scale of the specified 通道。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:TERmination` | 本指令设置 通道 input termination。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `CH<x>:YUNits` | 本指令设置 units for the specified 通道 D<x> 返回parameters for digital 通道 <x>。 | `CH<x>?`<br>`CH<x>:AMPSVIAVOLTs:ENAble {<NR1>\|OFF\|ON}`<br>`CH<x>:AMPSVIAVOLTs:ENAble?` |
| `D<x>:LABel` | 本指令设置 波形 label for digital 通道<x>。 | `D<x>`<br>`D<x>:LABel <Qstring>`<br>`D<x>:LABel?` |
| `D<x>:THReshold` | 【设置】This command specifies the logical threshold for the digital channel <x>, where x is the digital channel number D0 – D15. | `D<x>`<br>`D<x>:LABel <Qstring>`<br>`D<x>:LABel?` |
| `D<x>:POSition` | 本指令设置 垂直 position for digital 通道 <x>。 | `D<x>`<br>`D<x>:LABel <Qstring>`<br>`D<x>:LABel?` |
| `DESkew:DISplay` | 设置或查询 deskew 表显示状态。 | `DIAg:LOOP:OPTion {ALWAYS\|FAIL\|ONFAIL\|ONCE\|NTIMES}`<br>`DIAg:LOOP:OPTion:NTIMes <NR1>`<br>`DIAg:LOOP:OPTion:NTIMes?` |
| `REF<x>?` | 返回参考 波形 data for 通道 <x>。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:DATE?` | 返回 date that a 参考 波形 was stored。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:HORizontal:DELay:TIMe` | 本指令设置 水平 position of the specified 参考 波形 in percent of the 波形 that is 显示ed to the right of the center 垂直 graticule。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:HORizontal:SCAle` | 本指令设置 水平 scale for a 参考 波形。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:LABel` | 本指令设置 specified 参考 波形 label。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:TIMe?` | 返回 time that a 参考 波形 was stored。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:VERTical:POSition` | 本指令设置 垂直 position of the specified 参考 波形。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `REF<x>:VERTical:SCAle` | 本指令设置 参考 波形 垂直 scale in 垂直 units/div。 | `REF<x>?`<br>`REF<x>:DATE?`<br>`REF<x>:HORizontal:DELay:TIMe <NR3>` |
| `SELect?` | 返回information on which 波形 are 开或关 and which 波形 is selected. | `SELect?`<br>`SELect{:MATH\|:MATH1} {ON\|OFF\|<NR1>}`<br>`SELect{:MATH\|:MATH1}?` |
| `SELect:{BUS<x>\|B<x>}` | 【开关】T u r n so no ro f ft h es p e c ified bus waveform or returns whether the specified bus channel is on or off。 | `SELect:{BUS<x>\|B<x>} {OFF\|ON\|0\|1}`<br>`SELect:{BUS<x>\|B<x>}?` |
| `SELect:CH<x>` | 切换开或关 the specified 波形 or returns whether the specified 通道 is 开或关。 | `SELect:CH<x> {ON\|OFF\|1\|0}`<br>`SELect:CH<x>?` |
| `SELect:CONTROl` | 本指令设置 波形 that is selected as the implied recipient of 通道-related commands。 | `SELect:CONTROl {CH1\|CH2\|CH3\|CH4`<br>`SELect:CONTROl?` |
| `SELect:DAll` | This command turns 开或关 all digital 通道s (D0 – D15). | `SELect:DAll {ON\|OFF\|0\|1}` |
| `SELect:D<x>` | 切换on the 显示 of digital 通道 <x> and resets the 采集 SELect{:MATH\|:MATH1} 切换开或关 the 数学波形 or returns whether the 数学波形 is 开或关。 | `SELect:D<x> {<NR1>\|OFF\|ON}`<br>`SELect:D<x>?` |
| `SELect:REF<x>` | T u r n so no ro f ft h es p e c ified 参考 波形 or returns whether the specified 参考 波形 is 开或关。 | `SELect:REF<x> {ON\|OFF\|<NR1>}`<br>`SELect:REF<x>?` |

## 28. 视频画面 (Video Picture)

手册原名：*Video Picture Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `VIDPic:AUTOContrast` | 设置（或查询） video picture automatic contrast state. | `VIDPic:AUTOContrast:UPDATERate <NR1>`<br>`VIDPic:AUTOContrast:UPDATERate?` |
| `VIDPic:AUTOContrast:UPDATERate` | 设置（或查询） number of frames between automatic contrast updates。 | `VIDPic:AUTOContrast:UPDATERate <NR1>`<br>`VIDPic:AUTOContrast:UPDATERate?` |
| `VIDPic:BRIGHTNess` | 【设置或查询】Sets (or queries) the video picture brightness level as an integer percentage. | `VIDPic:BRIGHTNess <NR1>`<br>`VIDPic:BRIGHTNess?` |
| `VIDPic:CONTRast` | 设置（或查询） video picture contrast level as an integer percentage. | `VIDPic:CONTRast <NR1>`<br>`VIDPic:CONTRast?` |
| `VIDPic:DISplay` | 设置（或查询） video picture 显示 state. | `VIDPic:DISplay {0\|1\|OFF\|ON}`<br>`VIDPic:DISplay?` |
| `VIDPic:FRAMETYPe` | 设置（或查询） video picture frame type (ODD, EVEN or INTERLAced). | `VIDPic:FRAMETYPe {ODD\|EVEN\|INTERLAced}`<br>`VIDPic:FRAMETYPe?` |
| `VIDPic:LOCation:HEIght` | 设置（或查询） video picture height, in rows. | `VIDPic:LOCation:HEIght <NR1>`<br>`VIDPic:LOCation:HEIght?` |
| `VIDPic:LOCation:OFFSet` | 【设置或查询】Sets (or queries) the video picture line-to-line offset. This is the amount of additional delay time to add between lines of the video picture. | `VIDPic:LOCation:OFFSet <NR3>`<br>`VIDPic:LOCation:OFFSet?` |
| `VIDPic:LOCation:STARt:LINE` | 设置（或查询） video picture starting line number. | `VIDPic:LOCation:STARt:LINE <NR1>`<br>`VIDPic:LOCation:STARt:LINE?` |
| `VIDPic:LOCation:STARt:PIXel` | 设置（或查询） video picture starting pixel in each line。 | `VIDPic:LOCation:STARt:PIXel <NR1>`<br>`VIDPic:LOCation:STARt:PIXel?` |
| `VIDPic:LOCation:WIDth` | 设置（或查询） video picture width, in columns. | `VIDPic:LOCation:WIDth <NR1>`<br>`VIDPic:LOCation:WIDth?` |
| `VIDPic:LOCation:X` | 设置（或查询） video picture X origin location, in columns. | `VIDPic:LOCation:X <NR1>`<br>`VIDPic:LOCation:X?` |
| `VIDPic:LOCation:Y` | 设置（或查询） video picture Y origin location, in rows. | `VIDPic:LOCation:Y <NR1>`<br>`VIDPic:LOCation:Y?` |
| `VIDPic:SOUrce` | 设置（或查询） 通道 to use for the video picture source 波形. | `VIDPic:STANdard {NTSC\|PAL}`<br>`VIDPic:STANdard?` |
| `VIDPic:STANdard` | 设置（或查询）哪个 video picture standard to use (either NTSC or PAL). | `VIDPic:STANdard {NTSC\|PAL}`<br>`VIDPic:STANdard?` |

## 29. 波形传输 (Waveform Transfer)

手册原名：*Waveform Transfer Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `DATa:SOUrce` | 本指令设置 source 波形 to be transferred from the 示波器。 | `DATa:SOUrce` |
| `WFMInpre:DOMain` | 【设置】This command specifies whether the information being sent to a reference location should be treated as integer (time domain) information, or floating point (frequency domain) information, for the purposes of storing the data internally. The default is TIMe.T h i s parameter should be set before using the CURVe command to transfer a waveform from a PC to an internal reference location. | `WFMInpre:DOMain` |
| `WFMInpre:ENCdg` | 【设置】This command specifies the type of encoding of the incoming waveform data to be sent to the oscilloscope using the CURVe command. Supported types are BINary and ASCii. | `WFMInpre:ENCdg {ASCii\|BINary}`<br>`WFMInpre:ENCdg?` |
| `WFMInpre:NR_Pt` | 【设置】This command specifies the number of data points that are in the incoming waveform record to be sent to the oscilloscope using the CURVe command. | `WFMInpre:NR_Pt <NR1>`<br>`WFMInpre:NR_Pt?` |
| `WFMInpre:PT_Fmt` | 【设置】This command specifies the format of the data points to be sent to the oscilloscope using the CURVE command. This can be Y for YT format, or ENV for envelope format (min/max pairs). Regardless of the argument used, the scale, offset, and so on are interpreted similarly. When ENV is used, waveform data is interpreted as min-max pairs (the minimum value precedes the maximum for each pair); when Y is used, it is interpreted over a single point. | `WFMInpre:PT_Fmt` |
| `WFMInpre:PT_Off` | 【控制】The set form of this command is ignored. The query form always returns a 0. | `WFMInpre:PT_Off <NR1>`<br>`WFMInpre:PT_Off?` |
| `WFMInpre:REFLevel` | 【设置】This command specifies the Reference Level of the incoming waveform. This command applies only to frequency domain waveforms. The Reference Level is adjustable from 10 pico Watts (–140dBm) to 1 Watt (+30dBm). | `WFMInpre:REFLevel <NR3>`<br>`WFMInpre:REFLevel?` |
| `WFMInpre:SPAN` | 【设置】This command specifies the frequency span of the incoming RF trace. The span is the range of frequencies that can be observed around the center frequency. | `WFMInpre:SPAN <NR3>`<br>`WFMInpre:SPAN?` |
| `WFMInpre:WFMTYPe` | 【设置】This command specifies the type of waveform that is being transferred to the oscilloscope for storage in one of the REF1 — REF4 memory locations. The waveform type possibilities are the ANALOG, the RF time domain waveforms (RF_TD), or the RF frequency domain waveforms (RF_FD). The default is ANALOG. This parameter should be set before using the CURVe command to transfer a waveform from a PC to an internal reference location. The type of waveform that is being transferred in turn determines which window will display it on the instrument screen: the time domain window or frequency domain window. Both the analog and RF-TD arguments specify the time domain window; the RF_FD argument specifies the frequency domain window. | `WFMInpre:WFMTYPe {ANALOG\|RF_TD\|RF_FD}`<br>`WFMInpre:WFMTYPe?` |
| `WFMInpre:XINcr` | 【设置】This command specifies the horizontal interval between incoming waveform points sent to the oscilloscope using the CURVE command. The units can be time, in seconds, or frequency, in hertz, and can be specified or queried using the WFMInpre:XUNit command. | `WFMInpre:XINcr <NR3>`<br>`WFMInpre:XINcr?` |
| `WFMInpre:XUNit` | 【设置】This command specifies the horizontal units of the x-axis of the data points being sent to the oscilloscope using the CURVE command. This value can be in “s” or “Hz”. | `WFMInpre:XUNit <QString>`<br>`WFMInpre:XUNit?` |
| `WFMInpre:XZEro` | 【设置】This command specifies the position value of the first data point in the incoming waveform record being sent to the oscilloscope using the CURVE command. The units are determined or queried using the WFMInpre:XUNit command and are typically time, in seconds, or frequency, in hertz. This time or frequency is relative to the time or frequency of the trigger, which is always 0. Thus, the XZEro value can be negative. | `WFMInpre:XZEro <NR3>`<br>`WFMInpre:XZEro?` |
| `WFMInpre:YMUlt` | 【设置】This command specifies the vertical scale multiplying factor to be used to convert the incoming data point values being sent to the oscilloscope, from digitizing levels into the units specified by the WFMInpre:YUNit command. For one byte waveform data, there are 256 digitizing levels. For two byte waveform data there are 65,536 digitizing levels. | `WFMInpre:YMUlt <NR3>`<br>`WFMInpre:YMUlt?` |
| `WFMInpre:YOFf` | 【设置】This command specifies the vertical position of the destination reference waveform in digitizing levels. There are 25 digitizing levels per vertical division for 1-byte data, and 6400 digitizing levels per vertical division for 2-byte data. Variations in this number are analogous to changing the vertical position of the waveform. | `WFMInpre:YOFf <NR3>`<br>`WFMInpre:YOFf?` |
| `WFMInpre:YUNit` | 【设置】This command specifies the vertical units of data points in the incoming waveform record sent to the oscilloscope using the CURVE command. This can be any of several string values, depending upon the vertical units of the waveform being sent. | `WFMInpre:YUNit <QString>`<br>`WFMInpre:YUNit?` |
| `WFMInpre:YZEro` | 【设置】This command specifies the vertical offset of the destination reference waveform in units specified by the WFMInpre:YUNit command. Variations in this number are analogous to changing the vertical offset of the waveform. The WFMInpre:YMUlt, WFMInpre:YOFf, and WFMInpre:YZEro commands are used to convert waveform record values to units specified using the WFMInpre:YUNit command (YUNit units). | `WFMInpre:YZEro <NR3>`<br>`WFMInpre:YZEro?` |
| `WFMOutpre?` | 【控制】This query returns the information needed to interpret the waveform data points returned by the CURVe? query. It returns the waveform transmission and formatting parameters for the waveform specified by the DATa:SOUrce command. | `WFMOutpre?` |
| `WFMOutpre:BIT_Nr` | 【设置】This command specifies the number of bits per data point in the outgoing waveform being transferred using the CURVe? query. Changing the value of WFMOutpre:BIT_Nr also changes the values of WFMOutpre:BYT_Or and DATa:WIDth. | `WFMOutpre:BIT_Nr` |
| `WFMOutpre:BN_Fmt` | 本指令设置 format of the binary data for outgoing 波形 when。 | `WFMOutpre:BN_Fmt {RI\|RP\|FP}`<br>`WFMOutpre:BN_Fmt?` |
| `WFMOutpre:ENCdg` | 【控制】is set to BINary. The format can be RI (signed integer) or RP (positive integer) for analog channels, and FP for RF frequency domain traces. Changing the value of WFMOutpre:BN_Fmt also changes the value of DAT a:ENCdg.T h ew a v e f o r mi s specified by the DAT a:SOUrcecommand. | `WFMOutpre:ENCdg {ASCii\|BINary}`<br>`WFMOutpre:ENCdg?` |
| `WFMOutpre:BYT_Nr` | 本指令设置 data width for the outgoing 波形 specified by the。 | `WFMOutpre:BYT_Nr <NR1>`<br>`WFMOutpre:BYT_Nr?` |
| `WFMOutpre:BYT_Or` | 【设置】This command specifies which byte of outgoing binary waveform data is transmitted first (the byte order). The byte order can either be MSB (most signi ficant byte first) or LSB (least significant byte first, also known as IBM format). This speci fication only has meaning when。 | `WFMOutpre:BYT_Or {LSB\|MSB}`<br>`WFMOutpre:BYT_Or?` |
| `WFMOutpre:CENTERFREQuency?` | 【控制】This query returns the center frequency of the incoming waveform. For non-MDO models, this query always returns 0. | `WFMOutpre:CENTERFREQuency?` |
| `WFMOutpre:DOMain?` | 【控制】This query returns the domain of the outgoing waveform — either TIMe or FREQuency. If the domain is TIMe, it indicates that the data is to be treated as integer information. If the domain is FREQuency, it indicates that the data is to be treated as floating point information. | `WFMOutpre:DOMain?` |
| `DATa:ENCdg` | 【控制】command, which provides the ability to set WFMOutpre:ENCdg, WFMOutpre:BN_Fmt, and WFMOutpre:BYT_Or using a single command.)。 | `DATa:ENCdg` |
| `WFMOutpre:NR_Pt?` | 【控制】This query returns the number of data points in the waveform record that will be transmitted in response to a CURVe? query. This value is the adjusted range specified by。 | `WFMOutpre:NR_Pt?` |
| `DATA:START` | 【控制】and DATA:STOP commands. Note that the oscilloscope automatically adjusts the DATA:START and DATA:STOP values when the DATA:STOP value is less than the DATA:START value, and when the DATA:START and/or DATA:STOP value is greater than the record length of the source waveform. The adjusted DATA:START and。 | `DATa:STARt <NR1>`<br>`DATa:STARt?` |
| `DATA:STOP` | 【控制】values determine WFMOUTPRE:NR_PT. (You can use DATa:STARt and。 | `DATA:STOP` |
| `DATa:STOP` | 【控制】to transfer partial waveforms.) If the waveform specified by the DAT a:SOUrce command is not turned on, an error will be generated. | `DATa:STOP` |
| `WFMOutpre:PT_Fmt?` | 【控制】This query returns the point format of the data points in the outgoing waveform record transferred using the CURVe? query. The returned values can be Y , which indicates normal waveform points for YT format, or ENV, which indicates envelope mode format in which the data is returned as a series of min/max pairs. The minimum value precedes the maximum. The outgoing waveform is specified by the DATa:SOUrce command. The query command will time out and an error will be generated if the waveform specified by DATa:SOUrce is not turned on. | `WFMOutpre:PT_Fmt?` |
| `WFMOutpre:PT_Off?` | 【控制】This query always returns 0 if the outgoing waveform specified by DATA:SOUrce is on or displayed. | `WFMOutpre:PT_Off?` |
| `WFMOutpre:PT_ORder?` | 本查询返回 point ordering, which is always linear. | `WFMOutpre:PT_ORder?` |
| `WFMOutpre:REFLEvel?` | 【控制】This query returns the Reference Level of the outgoing waveform. It applies only to the four frequency domain waveforms (RF Normal, RF Average, RF Max Hold, and RF Min Hold). | `WFMOutpre:REFLEvel?` |
| `WFMOutpre:SPAN?` | 【控制】This query returns the frequency span of the outgoing waveform. For non-MDO models, this query always returns 0.0. The span is the range of frequencies you can observe around the center frequency. | `WFMOutpre:SPAN?` |
| `WFMOutpre:WFId?` | 【控制】This query returns a string that describes several aspects of the acquisition parameters for the source waveform, including Source, Coupling, Vertical Scale, Horizontal Scale, Record Length and Acquisition Mode. If the waveform specified by DATa:SOUrce command is not turned on, an error will be generated. | `WFMOutpre:WFId?` |
| `WFMOutpre:WFMTYPe?` | 【控制】This query returns the type of the outgoing waveform. RF_FD indicates an RF frequency domain waveform; RF_TD indicates an RF time domain waveform; ANALOG indicates Channel 1–4 or the Math waveform. The default is analog. For non-MDO models, this query always returns ANALOG. The type of waveform that is being transferred in turn determines which window will display it on the instrument screen: (the time domain window or frequency domain window). | `WFMOutpre:WFMTYPe?` |
| `WFMOutpre:XINcr?` | 【控制】This query returns the horizontal point spacing in units of time (seconds), or frequency (hertz) between data points in the waveform record transferred using the CURVe? query. This value corresponds to the sampling interval. | `WFMOutpre:XINcr?` |
| `WFMOutpre:XUNit?` | 【控制】This query indicates the horizontal units of the x-axis of the waveform record transferred。 | `WFMOutpre:XUNit?` |
| `WFMOutpre:XZEro?` | 【控制】This query returns the time coordinate, in seconds, or frequency, in hertz, of the first data point in the outgoing waveform record transferred using the CURVe? query. This time or frequency is relative to the time of the trigger, which is always 0. Thus, the XZEro time or frequency can be negative. | `WFMOutpre:XZEro?` |
| `WFMOutpre:YMUlt?` | 【控制】This query returns the vertical scale multiplying factor used to convert the waveform data point values in the outgoing waveform record from digitizing levels to the YUNit units. You can determine the units by using the WFMOutpre:YUNit query. See the description of the WFMInpre:YMUlt command to see how this scale factor is used to convert waveform sample values to volts. | `WFMOutpre:YMUlt?` |
| `WFMOutpre:YOFf?` | 【控制】This query returns the vertical position of the source waveform in digitizing levels. There are 25 digitizing levels per vertical division for 1-byte data, and 6400 digitizing levels per vertical division for 2-byte data. See the description of WFMInpre:YOFf to see how this position is used to convert waveform sample values to volts. | `WFMOutpre:YOFf?` |
| `WFMOutpre:YUNit?` | 【控制】This query returns the units of data points in the outgoing waveform record transferred。 | `WFMOutpre:YUNit?` |

## 30. 缩放 (Zoom)

手册原名：*Zoom Commands*

| 指令 | 功能说明（中文） | Syntax（手册原文，节选） |
|------|------------------|-------------------------|
| `ZOOm?` | 返回 水平 positioning and scaling of the 缩放 显示 ZOOm{:MODe\|:STATE} 本指令设置 缩放 mode。 | `ZOOm?`<br>`ZOOm{:MODe\|:STATE} {ON\|OFF\|<NR1>}` |
| `ZOOm:ZOOM<x>?` | 返回当前 水平 positioning and scaling of the 缩放 显示。 | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |
| `ZOOm:ZOOM<x>:FACtor?` | 返回 缩放 factor of the 缩放 window. <x> can only be 1。 | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |
| `ZOOm:ZOOM<x>:POSition` | 【设置】This command specifies the horizontal position of the zoom window in terms of 0 to 100% of the overview window. <x> can only be 1。 | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |
| `ZOOm:ZOOM<x>:SCAle` | 本指令设置 水平 缩放 scale of the 缩放 window. <x> can only be 1。 | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |
| `ZOOm:ZOOM<x>:STATE` | 设置or returns a trace as 缩放ed, 开或关. <x> can only be 1。 | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |
| `ZOOm:ZOOM<x>:TRIGPOS?` | 本查询返回 time relative to 触发 of the center of the 缩放 box, for the currently selected 波形. | `ZOOm:ZOOM<x>?`<br>`ZOOm:ZOOM<x>:FACtor? Returns <NR1> isanintegerthatspecifiesthezoomfactorofazoombox.`<br>`ZOOm:ZOOM<x>:POSition <NR3>` |

---

## 附录 A：IEEE 488.2 公用指令速查

| 指令 | 功能说明（中文） |
|------|------------------|
| `*IDN?` | 查询仪器识别信息（厂商、型号、序列号、固件版本）。 |
| `*RST` | 复位仪器到已知状态。 |
| `*OPC` / `*OPC?` | 操作完成：置位或查询 OPC。 |
| `*WAI` | 等待挂起操作完成后再继续。 |
| `*CLS` | 清除状态与错误队列等相关信息。 |
| `*ESE` / `*ESE?` | 设置/查询事件状态使能寄存器。 |
| `*ESR?` | 查询（并清除）标准事件状态寄存器。 |
| `*SRE` / `*SRE?` | 设置/查询服务请求使能寄存器。 |
| `*STB?` | 查询状态字节寄存器。 |
| `*TST?` | 自检并返回结果码。 |

## 附录 B：与现有 WiParse 示波器能力的对应

| WiParse 现状 | 手册指令 | 说明 |
|--------------|----------|------|
| `scope shot` / HARDCopy PNG | `SAVe:IMAGe:*` / `HARDCopy` 组 | 已实现截屏 |
| `scope wave`（规划中） | `DATa:SOUrce`、`CURVe?`、`WFMOutpre:*` | 数值波形导出 |
| 连接/`*IDN?` | Miscellaneous / Configuration | 识别 TEKTRONIX,MDO3014 |
| 文件系统（规划中） | `FILESystem:*` | CWD、读写、挂载网络盘 |

## 修订记录

| 日期 | 说明 |
|------|------|
| 2026-07-13 | 基于 077-1498-00 全文提取 Command Groups，生成中文功能说明手册。 |
| 2026-07-13 | 修复 FILESystem/HARDCopy 被误拆词、MATH[1] 漏提、硬拷贝换行断裂；补全文件系统/硬拷贝/数学分组。 |
