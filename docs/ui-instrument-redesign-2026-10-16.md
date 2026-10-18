# 仪表控制页面评审与重构

日期：2026-10-16。交付版本：1.1.31。

## 评审结论

上一轮调整仍沿用“大卡片套长表单”的结构。主要问题不是圆角或颜色，而是缺少稳定的导航和视觉中心：

| 问题 | 原表现 | 本轮处理 |
|---|---|---|
| 设备导航过重 | 每台设备重复显示地址、连接和断开，列表滚动后难以找设备 | 54 px 设备行，只保留类型、型号和连接提示；连接表单集中到未连接设备详情 |
| 阅读层级不清 | 设定值、实测值、说明文字几乎同等字号 | 模型标题、分组标题、参数行、40 px 主读数及24 px 次读数分级 |
| 多通道堆叠 | 电源所有通道纵向展开，示波器堆叠四个通道卡片 | 使用当前通道选择器，参数与实测值共同跟随所选通道 |
| 控件排列随内容漂移 | 标签上置、横排按钮和嵌套卡片混用 | 参数组统一42 px行高，标签与右侧数值对齐，使用共享设计令牌 |
| 高级功能压过主流程 | 网络参数、保护、擦除等混入日常操作 | 保护、维护按需展开；网络参数移入设备信息；探针按操作任务切换 |
| 空间分配机械 | 固定四宫格，控制台与主操作等高 | 宽屏采用参数/读数双栏，窄屏纵向阅读；SCPI、波形、日志使用独立视口 |
| 画风与主软件不统一 | 数字孪生总览包含炫光、装饰线和持续动画 | 默认总览采用同主题设备摘要，不再使用装饰性数字孪生画面 |

## 设计依据与本地适配

参考 [Apple HIG：Layout](https://developer.apple.com/design/human-interface-guidelines/layout) 的层级、对齐与分组原则，以及 [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars) 的紧凑导航原则。

实现仍使用 WiParse 的 egui 组件和统一主题。沿用 `theme::Tokens` 的深浅配色、蓝色强调色、字体和控件圆角；没有引入另一套应用级主题、模糊玻璃效果或 Apple 专用字体依赖。参数行、分段选择、开关、动作按钮、设备行和读数由仪表模块内共享组件负责。

## 各类页面

| 页面 | 默认控制台 | 次级任务 |
|---|---|---|
| 示波器 | 常用采集操作、当前CH、“垂直通道/时基触发”参数、实测值和屏幕预览 | 截图、波形读取及导出、SCPI诊断 |
| 直流电源 | 当前CH的电压与限流、明确的输出开/关指令、实测V/A/W | 当前CH的OVP/OCP按需展开；采样趋势、通信诊断 |
| 电子负载 | 工作模式、目标值、输入开/关、实测V/A/W | 采样趋势、通信诊断 |
| 万用表 | 功能、量程、NPLC、突出实测值 | 趋势、设备信息；不展示其他测量功能的旧值 |
| 调试探针 | “连接/烧录/内存/RTT”各自的参数和动作 | 接收记录；复位和擦除单独展开 |
| USB桥 | “SPI/I2C/GPIO”各自的参数和事务操作 | 接收记录、芯片与连接信息 |
| WiParse-Dongle | “DAP/UART/I2C”，DAP再按操作任务切换 | 网络地址和端口集中到设备信息，保留链路查询 |
| 通用SCPI | 完整宽度的命令控制台 | 设备信息；不臆造未声明的测量能力 |
| 设备总览/未连接 | 设备摘要、查找、已发现设备连接入口 | 未连接页保留手动地址输入和发现资源选择 |

## 状态与操作约束

- 切换设备、CH或任务只是导航，不自动向仪表发送输出或参数命令。
- 电源“应用设定”只发送该CH的电压和限流指令，不附带开启输出。开启与关闭保留独立入口，避免依赖缓存状态的单个翻转开关。
- 设定值、最近实测值和输出请求分离。没有数据时显示“—”，不以设定值充当实测；有时间记录时展示上次采样时间。
- 错误与SCPI记录按设备隔离；保留读数不表示故障后当前状态。复位、擦除、写入没有自动重试。
- 保留原有驱动、协议、Job异步派发及示波器切换时的延迟重绘保护。本轮没有开展实物仪表/DUT验收。

## 图像复核

图中的设备与读数为明确标注的模拟展示夹具，用于评审布局，不是实测或产品合格数据。

| 页面 | 深色 | 浅色 |
|---|---|---|
| 直流电源 | [查看](ui-instrument-redesign-2026-10-16/source-dark.png) | [查看](ui-instrument-redesign-2026-10-16/source-light.png) |
| 示波器 | [查看](ui-instrument-redesign-2026-10-16/scope-dark.png) | [查看](ui-instrument-redesign-2026-10-16/scope-light.png) |
| 万用表 | [查看](ui-instrument-redesign-2026-10-16/meter-dark.png) | [查看](ui-instrument-redesign-2026-10-16/meter-light.png) |
| Dongle UART | [查看](ui-instrument-redesign-2026-10-16/dongle-dark.png) | [查看](ui-instrument-redesign-2026-10-16/dongle-light.png) |
| 电子负载 | [查看](ui-instrument-redesign-2026-10-16/load-dark.png) | [查看](ui-instrument-redesign-2026-10-16/load-light.png) |
| 调试探针 | [查看](ui-instrument-redesign-2026-10-16/probe-dark.png) | [查看](ui-instrument-redesign-2026-10-16/probe-light.png) |
| USB桥 | [查看](ui-instrument-redesign-2026-10-16/bridge-dark.png) | [查看](ui-instrument-redesign-2026-10-16/bridge-light.png) |
| 通用SCPI | [查看](ui-instrument-redesign-2026-10-16/generic-dark.png) | [查看](ui-instrument-redesign-2026-10-16/generic-light.png) |
| 设备总览 | [查看](ui-instrument-redesign-2026-10-16/overview-dark.png) | [查看](ui-instrument-redesign-2026-10-16/overview-light.png) |

烧录、内存、RTT、I2C、GPIO及示波器触发参数的逐页预览同目录保存。正常模式的设备总览也已开放，不依赖调试开关，打开总览不会创建模拟设备。

布局回归覆盖八类设备和总览、三工作区、各接口/任务子页、中英文、深浅主题及640×360、1100×720、1920×1080三种尺寸，共432组，另覆盖32组未连接页面。真实鼠标事件回归检查CH选择与命令派发的对应关系。完整发布验证结果见包内 `docs/verification/release-1.1.31.json`。
