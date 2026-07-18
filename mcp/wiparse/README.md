# WiParse MCP Server

通过 [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) 将 WiParse CLI 暴露给 Cursor / Claude 等 Agent。

## 前置条件

- Node.js 18+
- 已构建的 WiParse CLI（`dist/WiParse-CLI.exe` 或在 `PATH` 中的 `wiparse`）

## 安装与构建

```powershell
cd mcp/wiparse
npm install
npm run build
```

## 环境变量

| 变量 | 说明 |
|------|------|
| `WIPARSE_CLI_PATH` | CLI 可执行文件绝对路径（默认自动查找 `dist/WiParse-CLI.exe`） |
| `WIPARSE_CWD` | 调用 CLI 时的工作目录（配置/数据库相对路径） |

## Cursor 配置

在 Cursor Settings → MCP 中添加（路径按本机修改）：

```json
{
  "mcpServers": {
    "wiparse": {
      "command": "node",
      "args": ["D:/windlink/windlink/WiParse-R/mcp/wiparse/dist/index.js"],
      "env": {
        "WIPARSE_CLI_PATH": "D:/windlink/windlink/WiParse-R/dist/WiParse-CLI.exe",
        "WIPARSE_CWD": "D:/windlink/windlink/WiParse-R"
      }
    }
  }
}
```

也可复制 [`cursor.mcp.example.json`](cursor.mcp.example.json) 内容。

## MCP 工具列表

| 工具 | CLI 等价 |
|------|----------|
| `wiparse_cli_info` | 元信息 / CLI 路径 |
| `wiparse_version` | `wiparse version` |
| `wiparse_ports` | `wiparse ports` |
| `wiparse_serial_read` | `wiparse serial read ...` |
| `wiparse_serial_send` | `wiparse serial send ...` |
| `wiparse_parse_qi_line` | `wiparse parse line ...` |
| `wiparse_parse_metrics` | `wiparse parse metrics ...` |
| `wiparse_parse_file` | `wiparse parse file ...` |
| `wiparse_session_list` | `wiparse session list` |
| `wiparse_session_show` | `wiparse session show` |
| `wiparse_wave_live` | `wiparse wave live ...` |
| `wiparse_wave_session` | `wiparse wave session ...` |
| `wiparse_wave_export` | `wiparse wave export ...` |
| `wiparse_scope_list` | `wiparse scope list` |
| `wiparse_scope_shot` | `wiparse scope shot ...` |
| `wiparse_scope_waveform` | `wiparse scope wave ...` |

> `serial stream` 为无限 NDJSON 流，未暴露为 MCP 工具；请用 `wiparse_serial_read` 并设置 `duration_sec`。

完整 CLI 说明见 [`docs/CLI_REFERENCE.md`](../../docs/CLI_REFERENCE.md)。

## 本地测试

```powershell
# 构建后手动验证 CLI 解析
node -e "import('./dist/cli.js').then(m => m.runCli(['version']).then(console.log))"

# MCP stdio 需由客户端启动；可用 MCP Inspector：
npx @modelcontextprotocol/inspector node dist/index.js
```
