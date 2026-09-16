# Dongle Network Config (`wiparse-netcfg`)

WIPARSE-IF **CH-NETCFG** (ICD **v1.4**): write STA SSID / PSK / IP into NVS and **APPLY** without flashing. USB works while Wi-Fi is down. TCP `:1238` is only reachable after GOT_IP.

GUI **1.1.16+**. Plugin **v1.1.0**. Firmware `bcdDevice = 0x0208`. After a 0x0207→0x0208 upgrade, uninstall `VID_C251&PID_F01A` in Device Manager if `WIPARSE NET` is missing.

## Transports

| `transport` | Path |
|-------------|------|
| `usb` (default) | WinUSB `MI_05` / GUID `{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105}` / `WIPARSE NET`. **No bulk endpoints** — EP0 vendor `0x41/0x20` (LAST bit) and `0xC1/0x21`. |
| `tcp` | `dap.local:1238` (`TCP_NODELAY`, `_netcfg._tcp`). Same UTF-8 line protocol. |
| `auto` | USB then TCP |

Do not open this GUID as CMSIS-DAP. Do not `GetAssociatedInterface` on the composite parent.

## Commands

`HELP` `STATUS` `SSID` `PSK` `DHCP` `STATIC` `HOSTNAME` `SAVE` `APPLY` `REVERT`

- `STATUS` never returns the password.
- `OK APPLY` means NVS saved and reconnect started — **not** GOT_IP. The plugin polls `STATUS` (`wifi=` / `ip=`) every 500 ms for `wait_got_ip_ms` (default 15 s).
- Logs and JSONL redact `PSK` / `PASS` / `PASSWORD`.

Default run (static IP, factory SSID; fill PSK in the Hub form):

```text
SSID WINDLINK
PSK <from form>
STATIC 192.168.137.123 192.168.137.1 255.255.255.0
HOSTNAME dap
APPLY
STATUS
```

```powershell
wiparse ui test-tool run --id wiparse-netcfg --preflight true
wiparse ui test-tool run --id wiparse-netcfg --params "{\"op\":\"status\"}"
wiparse ui test-tool run --id wiparse-netcfg --params "{\"op\":\"apply\",\"ssid\":\"WINDLINK\",\"psk\":\"********\"}"
```

Artifacts: `projects/{project}/tests/{report_name}/runs/{stamp}/artifacts/{logs,reports}/` (`file_prefix` may override the case dir).
