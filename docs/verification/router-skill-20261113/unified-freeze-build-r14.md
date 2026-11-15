# r14 统一冻结及独立默认构建

构建前生成 `source-manifest-unified-r14-prebuild.json`，涵盖 700 个已跟踪或非忽略的源码、测试、patched winit、图标、波形样本、MCP、打包与构建配置文件，清单 SHA-256 `7b4a52d7aff143e846948b482a00510d75f40e886b5b5f34512b79ee142ca0a8`。清单同时记录 Git HEAD、操作系统、Rust、Cargo、Node、Python 版本和默认关闭的监控构建变量；用户配置与运行数据未纳入。

在全新 `target/default-unified-freeze-r14` 目录，以监控变量未设置、`--locked --offline --release` 编译 GUI、CLI 和服务；`v11-unified-default-build-r14.log` 记录成功，耗时 12 分 25 秒。构建后 `source-manifest-unified-r14-postbuild-check.json` 比较同一范围，新增、删除、内容变化均为零。

| 构建结果 | SHA-256 |
| --- | --- |
| GUI | `ba24d952e4d3b99defaa42a722b686c5b4e741bb516759ad2c57b8353e1085ab` |
| CLI | `a0274a80a6f9e53e292086fb83b8a8bb1bd724bbe463f90e4c0d86ce55a6d033` |
| 服务 | `cdbce38b270e5e859c54e18fc316383f0a13ce87e602e78cc3fb21507ffa67d7` |

此独立目录产物与先前目录的哈希不同；构建路径和调试信息等可能影响二进制，未完成差异归因或可复现构建验证，不能声称二进制可复现。清单未逐项哈希外部 Rust 注册表缓存、工具链或系统 SDK；Cargo.lock、离线模式和版本记录不替代它们的完整供应链快照。构建前后比对不能排除中途变化后又恢复的情况。

计划 V10 明文要求相同任务、模型和数据的质量、时延及已知 Token/费用对照，并未要求新增 Token 分段计量软件。本次没有扩大源码范围；现有评估仍缺独立样本、完整匹配基线与 P50/P95，不宣称成本或性能达标。桌面、真实台架、千问及受审阅监控生产验收仍未完成；未打包或部署。
