# Dongle Network Config (`wiparse-netcfg`)

WIPARSE-IF **CH-NETCFG**：把 STA 的 SSID / PSK / 寻址写入 NVS 并 **APPLY**，不烧录。USB 在 Wi-Fi 断开时仍可用。TCP `:1238` 只在 GOT_IP 之后可达。

GUI **1.1.16+**。插件 **v1.2.0**。EP0 配网不要求 `bcdDevice = 0x0210`（该版本才把路由器显示名改成 `WiParseDongle_XXXX`）。

主界面只填 **2.4 GHz** 名称和密码，保持「自动获取 IP」打开，点「下发并生效」。SSID / PSK 留空表示不改设备里已保存的凭据。不要填 `WINDLINK-5G`。

## Transports

| `transport` | Path |
|-------------|------|
| `usb` (default) | WinUSB `MI_05` / GUID `{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105}` / `WIPARSE NET`。无 bulk 端点，只用 EP0 vendor `0x41/0x20`（LAST bit）和 `0xC1/0x21`。 |
| `tcp` | 已入网后备。Host 填 `STATUS` 的 `ip=` 或 `wiparsedongle-xxxx.local`，端口 `1238`。不要用 `dap.local`。未入网或即将重连时请用 USB。 |
| `auto` | 先 USB；打不开再 TCP。Host 为空且 USB 打不开时直接失败，不回退 `dap.local`。 |

不要把这个 GUID 当成 CMSIS-DAP。不要对复合设备父节点做 `GetAssociatedInterface`。

## Commands

`HELP` `STATUS` `SSID` `PSK` `DHCP` `STATIC` `HOSTNAME` `SAVE` `APPLY` `REVERT`

- 表单 `op` 只有 `apply`（组配方 + `APPLY` + 轮询）和 `status`（只发 `STATUS`）。
- `SAVE` / `REVERT` / `HELP` 写在高级脚本里；脚本非空时按原文逐行发送，忽略 `op`。
- `STATUS` 不回密码。
- `OK APPLY` 表示已写入 NVS 并开始重连，**不是** GOT_IP。之后每 500 ms 轮询 `STATUS`（`wifi=` / `ip=`），默认等 30 s。
- 日志、jsonl、报告脱敏 `PSK` / `PASS` / `PASSWORD`。
- 默认配方不下发 `HOSTNAME`，也不把出厂地址 `192.168.137.123` 当默认值。

DHCP 勾选、名称和密码都填了：

```text
SSID <名称>
PSK <密码>
DHCP
APPLY
```

名称和密码都空、DHCP 打开（有意行为：只改寻址并重连已有凭据）：

```text
DHCP
APPLY
```

```powershell
wiparse ui test-tool run --id wiparse-netcfg --preflight true
wiparse ui test-tool run --id wiparse-netcfg --params "{\"op\":\"status\"}"
wiparse ui test-tool run --id wiparse-netcfg --params "{\"op\":\"apply\",\"ssid\":\"WINDLINK\",\"psk\":\"********\"}"
```

Artifacts: `projects/{project}/tests/{report_name}/runs/{stamp}/artifacts/{logs,reports}/`（`file_prefix` 可覆盖用例目录）。
