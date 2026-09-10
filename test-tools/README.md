# WiParse Test Tools (Node plugins)

Plugin-style scripts for the GUI **集成测试 / Testing Hub** panel (config key still `test_tool`).  
Drive **WiParse GUI HTTP API** and/or **CLI**.  
See **[PLUGIN_SPEC.md](./PLUGIN_SPEC.md)** for the contract (lifecycle, result shape, engines, schemas).

## Quick start

```powershell
cd test-tools
node runner.mjs --list
node runner.mjs --plugin example-smoke --lifecycle preflight -- --check_api false
node runner.mjs --plugin example-smoke --lifecycle run -- --check_api false
node runner.mjs --plugin tianshu-xinwei-ask02 --lifecycle preflight
node runner.mjs --plugin tianshu-xinwei-ask02 --lifecycle run -- --file_prefix MyTest
node runner.mjs --plugin tianshu-xinwei-ask02 --lifecycle stop
```

GUI: open **集成测试 / Testing Hub**, pick a plugin (e.g. **示波器/串口监控**), edit params, Preflight / Run / Stop.

Env: `WIPARSE_CLI`, `WIPARSE_URL`, `WIPARSE_DATA_ROOT`.
