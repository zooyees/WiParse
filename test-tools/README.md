# WiParse Test Tools (Node plugins)

Plugin-style scripts for the GUI **集成测试 / Testing Hub** panel (config key still `test_tool`).  
Drive **WiParse GUI HTTP API** and/or **CLI**.  
See **[PLUGIN_SPEC.md](./PLUGIN_SPEC.md)** for the **2026-09-11 / GUI 1.1.10** contract (generic `device` params, preflight `suggested_params`, path colocation, HTTP invoke catalog). This is the document to hand to another developer or AI writing a plugin.

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

```powershell
cd test-tools
npm test
node marketplace.mjs list --json
```

Cloud server: [`services/testing-hub-marketplace`](../services/testing-hub-marketplace).
