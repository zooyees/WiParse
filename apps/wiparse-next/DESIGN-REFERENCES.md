# 界面参考记录

检索日期：2026-11-06。星标数量会变化，下列为检索时 GitHub 页面展示的近似值，并非对全部 GitHub 项目的完整排名。

| 项目 | 星标 | 参考范围 |
| --- | ---: | --- |
| [Visual Studio Code](https://github.com/microsoft/vscode) | 194.9k | 长时间操作的工程工作区、导航与状态栏 |
| [shadcn/ui](https://github.com/shadcn-ui/ui) | 127.5k | 控件的间距、对齐、边框、焦点和可访问性；MIT |
| [Ant Design](https://github.com/ant-design/ant-design) | 100.1k | 数据密集的表单、表格和状态；MIT |
| [Grafana](https://github.com/grafana/grafana) | 77.7k | 工程数据、实时状态和日志的层次；AGPL-3.0，仅作视觉和交互参考 |
| [shadcn-admin](https://github.com/satnaing/shadcn-admin) | 16.6k | [任务表格实际页面](https://shadcn-admin.netlify.app/tasks)：搜索筛选区、表头、行间分隔和状态呈现；MIT |

本次重点检查了 shadcn-admin 的实际任务页面及其表格源码，提取信息层级、对齐与搜索筛选方式；WiParse 的工程工作区仍需保留 200 像素左栏、较紧凑的表格行以及选中项右侧详情。没有将参考项目的业务数据、卡片或仪表盘直接复制到产品页面。

20 项布局截图使用隔离的模拟服务与历史配置，截图上标明测试用途。它只验证页面呈现，不能证明真实设备、测试结果或性能已通过验收。
