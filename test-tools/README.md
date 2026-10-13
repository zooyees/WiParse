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

## 1.1.30 报告与演示数据

内置插件报告统一提供原始数据与附件入口。示波器/串口监控 0.6.1 使用相对 ISF/截图/PDF 链接，整个 run 目录移动后仍可查看；无线充电自动检验 0.1.1 提供逐用例检查 JSON、原始回放、覆盖率、配置限值、异常处置与哈希清单入口。

发布包包含 `projects/default/tests/ScopeSerialDemo/runs/20261013_231510/`，演示 PING、正常功率传输和 VIN 欠压后关闭 PWM。所有波形、截图和串口日志均为合成数据，PDF 未生成，不能据此给产品 PASS。启动发布包后在“测试报告”选择 default 并刷新，展开 ScopeSerialDemo 和该运行时间。JSON/JSONL 可内嵌查看；报告链接普通单击打开，旧 GUI 需要先更新。
