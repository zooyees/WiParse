# WiParse Test Tools (Node plugins)

Plugin-style scripts for the GUI **集成测试 / Testing Hub** panel (config key still `test_tool`).  
Drive **WiParse GUI HTTP API** and/or **CLI**.  
See **[PLUGIN_SPEC.md](./PLUGIN_SPEC.md)** for the **2026-09-16 / GUI 1.1.16+** contract (`report_name` test cases, overlay `file_prefix` as optional `test_id`, generic `device` params, `outputs[]`). This is the document to hand to another developer or AI writing a plugin.

## Quick start

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin example-smoke --lifecycle preflight -- --check_api false
node runner.mjs --plugin example-smoke --lifecycle run -- --check_api false
node runner.mjs --plugin scope-serial-monitor --lifecycle preflight
node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --file_prefix MyTest
node runner.mjs --plugin scope-serial-monitor --lifecycle stop
```

GUI: open **集成测试 / Testing Hub**, pick **示波器/串口监控 (Scope & Serial Monitor)**, edit params, Preflight / Run / Stop.

Env: `WIPARSE_CLI`, `WIPARSE_URL`, `WIPARSE_DATA_ROOT`, `WIPARSE_MARKETPLACE_URL`, `WIPARSE_MARKETPLACE_ALLOW_HTTP`.

## Marketplace

Wireless charging development plugin: [wireless-charge-validation](plugins/wireless-charge-validation/README.md), WiParse 1.1.29+. It supports dry-run and simulation replay through the automation service; live stimulus and industrial acceptance are blocked. `ok` is execution success, not product PASS. Result metadata preserves `execution_status`, `product_verdict` and `validation_verdict` separately.

```powershell
cd test-tools
npm test
node marketplace.mjs list --json
```

Cloud server: [`services/testing-hub-marketplace`](../services/testing-hub-marketplace).

Local Windows server (repo root):

```powershell
..\scripts\deploy-marketplace.ps1
node marketplace.mjs catalog --url http://127.0.0.1:8787 --json
```

## 1.1.32 报告与演示数据

开发版报告导航调整（2026-10-16）：左侧仅显示“测试项目 / 运行批次 / MD 报告”。单击批次打开其 MD，箭头展开报告列表；波形、图片、记录数据和日志从正文链接打开，并可返回报告。若尚未生成 MD，显示缺少报告的提示，不会用附件或上批次的报告替代。历史目录和文件保持原样，插件仍需在 MD 中提供相关证据链接。

示波器/串口监控新报告按总报告、会话、采集轮次分级，截图通过链接打开；共用报告写入器按附件用途分节。集成测试仍负责执行和采集，报告工具负责阅读 MD 与打开证据。

内置插件报告统一提供原始数据与附件入口。示波器/串口监控 0.6.2 使用相对 ISF/截图/PDF 链接，整个 run 目录移动后仍可查看；无线充电自动检验 0.1.1 提供逐用例检查 JSON、原始回放、覆盖率、配置限值、异常处置与哈希清单入口。

发布包包含 `projects/default/tests/ScopeSerialDemo/runs/20261016_120000/`，演示 PING、正常功率传输和 VIN 欠压后关闭 PWM。所有波形、截图和串口日志均为合成数据，PDF 未生成，不能据此给产品 PASS。启动发布包后在“测试报告”选择 default 并刷新，展开 ScopeSerialDemo 和该运行时间。JSON/JSONL 可内嵌查看；报告链接普通单击打开，旧 GUI 需要先更新。


## 通用配置工作区与测试矩阵

声明 `configure` 和 `ui.mode=test_matrix` 的插件启用 schema 表单、配置仓库、预检绑定及矩阵。旧插件仍使用原参数表单。500 是上限，测试只有 5 项就只显示 5 项。

安装开发依赖：`npm ci --prefix test-tools`。打包脚本会携带 Ajv 离线依赖，运行时不下载 schema 或 npm 包。

通用示例：`hub-workspace-demo`；在其工作区导入 `test-tools/examples/hub-workspace-demo-five.config-bundle.json`，保存/预检后运行。此示例不访问仪器，不能用于产品合格判定。

宿主协议、CLI/MCP 和恢复说明见 [Testing Hub 工作区](../docs/TESTING_HUB_WORKSPACE.md)。
