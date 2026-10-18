import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validatePluginManifest, validateStationConfig } from "./plugin-contract.mjs";
import {
  CHANNELS,
  IF_SPEC,
  decodeCdcLineCoding,
  decodeDapInfo,
  encodeCdcLineCoding,
  encodeDapInfo,
  encodeUsbipDevlistRequest,
  formatI2cCommand,
  hostCandidates,
  hostMatchesDevice,
  isUartBaudFirstPacket,
  padDapPacket,
  parseHexBytes,
  parseI2cReply,
  parseUsbipDevlistReply,
  resolveChannel,
  uartBaudFirstPacket,
  splitNetcfgControlChunks,
  redactNetcfgLine,
  parseNetcfgStatus,
  formatNetcfgCommand,
  buildNetcfgRecipe,
} from "./wiparse-if.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));

test("ICD defaults match WIPARSE-IF-001", () => {
  assert.equal(IF_SPEC.vid, 0xc251);
  assert.equal(IF_SPEC.pid, 0xf01a);
  assert.equal(IF_SPEC.bcd_device, 0x0208);
  assert.equal(IF_SPEC.version, "1.4");
  assert.equal(CHANNELS.dap.tcp_port, 3240);
  assert.equal(CHANNELS.uart.tcp_port, 1234);
  assert.equal(CHANNELS.i2c.tcp_port, 1236);
  assert.equal(CHANNELS.netcfg.tcp_port, 1238);
  assert.equal(CHANNELS.netcfg.mi, 5);
  assert.equal(CHANNELS.dap.packet, 512);
  assert.equal(CHANNELS.uart.out_ep, 0x02);
  assert.equal(CHANNELS.i2c.in_ep, 0x84);
});

test("channel overlays are generic (MI/EP/host not locked to the dongle)", () => {
  const uart = resolveChannel("uart", { usb_mi: 4, out_ep: "0x05", in_ep: "86", tcp_port: 9000 });
  assert.equal(uart.mi, 4);
  assert.equal(uart.out_ep, 5);
  assert.equal(uart.in_ep, 0x86);
  assert.equal(uart.tcp_port, 9000);
  assert.deepEqual(hostCandidates("10.1.2.3"), ["10.1.2.3"]);
  assert.ok(hostCandidates("dap.local").includes("192.168.137.123"));
  assert.deepEqual(hostCandidates("dap.local", ""), ["dap.local"]);
  assert.equal(
    hostMatchesDevice({ mi: 4, name: "Acme UART" }, uart),
    true
  );
  assert.equal(
    hostMatchesDevice({ mi: 1, name: "other" }, uart),
    false
  );
});

test("UART first-packet baud follows ICD §7.3", () => {
  assert.deepEqual(uartBaudFirstPacket(115200), Buffer.from("115200"));
  assert.equal(uartBaudFirstPacket(1199), null);
  assert.equal(uartBaudFirstPacket(2000001), null);
  assert.equal(isUartBaudFirstPacket(Buffer.from("115200")), true);
  assert.equal(isUartBaudFirstPacket(Buffer.from("0115200")), false);
  assert.equal(isUartBaudFirstPacket(Buffer.from("9600\n")), false);
});

test("CDC line coding roundtrip", () => {
  const b = encodeCdcLineCoding({ baud: 921600, stop: 0, parity: 2, data_bits: 8 });
  assert.equal(b.length, 7);
  const d = decodeCdcLineCoding(b);
  assert.equal(d.baud, 921600);
  assert.equal(d.parity, 2);
  assert.equal(d.data_bits, 8);
});

test("I2C RPC formatter and parser", () => {
  assert.equal(formatI2cCommand({ cmd: "SCAN" }), "SCAN");
  assert.equal(formatI2cCommand({ cmd: "PING", addr: "0x50" }), "PING 50");
  assert.equal(formatI2cCommand({ cmd: "READ", addr: "50", len: 2 }), "READ 50 2");
  assert.equal(
    formatI2cCommand({ cmd: "WRITE", addr: "50", data: "00 12 34" }),
    "WRITE 50 00 12 34"
  );
  assert.equal(
    formatI2cCommand({ cmd: "WRITEREAD", addr: "50", data: "00", len: 2 }),
    "WRITEREAD 50 00 2"
  );
  assert.equal(formatI2cCommand({ cmd: "SPEED", hz: 100000 }), "SPEED 100000");
  const ok = parseI2cReply("OK READ 12 34");
  assert.equal(ok.ok, true);
  assert.equal(ok.tag, "READ");
  assert.deepEqual([...ok.hex], [0x12, 0x34]);
  const err = parseI2cReply("ERR nack ESP_ERR_TIMEOUT");
  assert.equal(err.ok, false);
  assert.equal(err.reason, "nack");
  assert.throws(() => formatI2cCommand({ cmd: "READ", addr: "50", len: 0 }));
  assert.throws(() => formatI2cCommand({ cmd: "WRITE", addr: "50", data: "zz" }));
});

test("NETCFG EP0 chunks, STATUS parse, PSK redact, apply recipe", () => {
  const chunks = splitNetcfgControlChunks("SSID WINDLINK", 8);
  assert.equal(chunks.length, 2);
  assert.equal(chunks[0].last, false);
  assert.equal(chunks[1].last, true);
  assert.equal(Buffer.concat(chunks.map((c) => c.bytes)).toString("utf8"), "SSID WINDLINK");
  const st = parseNetcfgStatus(
    "OK STATUS ssid=WINDLINK ip=192.168.137.123 gw=192.168.137.1 mask=255.255.255.0 dhcp=0 source=nvs wifi=1 rssi=-40 hostname=dap ap=0"
  );
  assert.equal(st.ok, true);
  assert.equal(st.got_ip, true);
  assert.equal(st.ssid, "WINDLINK");
  assert.equal(st.dhcp, false);
  const down = parseNetcfgStatus("OK STATUS ssid=WINDLINK ip=0.0.0.0 wifi=0");
  assert.equal(down.got_ip, false);
  assert.equal(redactNetcfgLine("PSK secret-pass"), "PSK ***");
  assert.equal(redactNetcfgLine("STATUS"), "STATUS");
  assert.equal(formatNetcfgCommand({ cmd: "STATIC", ip: "1.2.3.4", gw: "1.2.3.1", mask: "255.255.255.0" }), "STATIC 1.2.3.4 1.2.3.1 255.255.255.0");
  assert.deepEqual(buildNetcfgRecipe({ dhcp: true }, "apply"), ["DHCP", "APPLY"]);
  assert.deepEqual(buildNetcfgRecipe({}, "apply"), ["DHCP", "APPLY"]);
  assert.deepEqual(
    buildNetcfgRecipe({ ssid: "WINDLINK", psk: "abcdefgh", dhcp: true }, "apply"),
    ["SSID WINDLINK", "PSK abcdefgh", "DHCP", "APPLY"]
  );
  assert.deepEqual(buildNetcfgRecipe({ ssid: "WINDLINK", dhcp: true }, "apply"), [
    "SSID WINDLINK",
    "DHCP",
    "APPLY",
  ]);
  const recipe = buildNetcfgRecipe(
    {
      ssid: "WINDLINK",
      psk: "abcdefgh",
      dhcp: false,
      ip: "192.168.137.123",
      gateway: "192.168.137.1",
      netmask: "255.255.255.0",
      hostname: "lab-ap",
    },
    "apply"
  );
  assert.deepEqual(recipe, [
    "SSID WINDLINK",
    "PSK abcdefgh",
    "STATIC 192.168.137.123 192.168.137.1 255.255.255.0",
    "HOSTNAME lab-ap",
    "APPLY",
  ]);
  assert.throws(
    () => buildNetcfgRecipe({ dhcp: false, ssid: "WINDLINK" }, "apply"),
    /STATIC needs ip, gateway and netmask/
  );
  assert.throws(
    () => buildNetcfgRecipe({ dhcp: false, ip: "10.0.0.8", gateway: "10.0.0.1" }, "apply"),
    /STATIC needs ip, gateway and netmask/
  );
  const net = resolveChannel("netcfg", {});
  assert.equal(net.tcp_port, 1238);
  assert.equal(net.guid, "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105}");
  assert.equal(hostMatchesDevice({ mi: 5, name: "WIPARSE NET" }, net), true);
  assert.equal(hostMatchesDevice({ mi: 0, name: "WIPARSE CMSIS-DAP v2" }, net), false);
  const custom = resolveChannel("netcfg", { usb_guid: "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}" });
  assert.equal(custom.guid, "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}");
});

test("USB/IP DEVLIST header and DAP_Info packet", () => {
  const req = encodeUsbipDevlistRequest();
  assert.equal(req.length, 8);
  assert.equal(req.readUInt16BE(0), 0x0111);
  assert.equal(req.readUInt16BE(2), 0x8005);
  const fake = Buffer.alloc(12 + 312);
  fake.writeUInt16BE(0x0111, 0);
  fake.writeUInt16BE(0x0005, 2);
  fake.writeUInt32BE(1, 8);
  fake.write("1-1\0", 12 + 256, "ascii");
  fake.writeUInt16BE(0xc251, 12 + 300);
  fake.writeUInt16BE(0xf01a, 12 + 302);
  const parsed = parseUsbipDevlistReply(fake);
  assert.equal(parsed.count, 1);
  assert.equal(parsed.devices[0].vid, 0xc251);
  assert.equal(parsed.devices[0].pid, 0xf01a);
  const pkt = padDapPacket(encodeDapInfo(0xff));
  assert.equal(pkt.length, 512);
  assert.equal(pkt[0], 0x00);
  const info = decodeDapInfo(Buffer.from([0x00, 0x02, 0x00, 0x02]));
  assert.equal(info.ok, true);
  assert.equal(info.u16, 512);
  assert.deepEqual([...parseHexBytes("0A FF")], [0x0a, 0xff]);
});

for (const id of ["wiparse-dap", "wiparse-uart-bridge", "wiparse-i2c-rpc", "wiparse-netcfg"]) {
  test(`${id} plugin.json + station.json validate`, () => {
    const dir = path.join(here, "..", "plugins", id);
    const plugin = JSON.parse(fs.readFileSync(path.join(dir, "plugin.json"), "utf8"));
    const station = JSON.parse(fs.readFileSync(path.join(dir, "station.json"), "utf8"));
    const p = validatePluginManifest(plugin);
    assert.equal(p.ok, true, p.errors?.join("; "));
    assert.equal(plugin.id, id);
    assert.ok(plugin.report_name && plugin.report_name.trim().length > 0);
    assert.ok(plugin.report_name_zh && plugin.report_name_zh.trim().length > 0);
    assert.equal(plugin.engines?.wiparse, ">=1.1.16");
    assert.ok(Array.isArray(plugin.outputs) && plugin.outputs.length >= 2);
    assert.ok(plugin.params.some((x) => x.name === "file_prefix"));
    assert.ok(plugin.params.some((x) => x.name === "transport"));
    const s = validateStationConfig(station);
    assert.equal(s.ok, true, s.errors?.join("; "));
    assert.equal(station.gui?.min_version, "1.1.16");
    assert.match(station.paths.lock_file, /test_dir/);
    assert.doesNotMatch(station.paths.lock_file, /plugin_dir/);
    if (id === "wiparse-netcfg") {
      assert.equal(plugin.version, "1.2.2");
      const byName = Object.fromEntries(plugin.params.map((x) => [x.name, x]));
      assert.equal(byName.ssid.default, "");
      assert.equal(byName.host.default, "");
      assert.equal(byName.hostname.default, "");
      assert.equal(byName.ip.default, "");
      assert.equal(byName.dhcp.default, "true");
      assert.equal(byName.wait_got_ip_ms.default, 30000);
      assert.equal(byName.file_prefix.advanced, true);
      assert.equal(byName.transport.advanced, true);
      assert.deepEqual(
        byName.op.options.map((o) => o.value),
        ["apply", "status"]
      );
      assert.equal(
        plugin.params.some((x) => x.name === "fallback_host"),
        false
      );
      assert.equal(station.wifi.dhcp, true);
      assert.equal(station.wifi.ssid, "");
      assert.equal(station.channel.host, "");
      assert.equal(station.channel.fallback_host, "");
      assert.equal(station.run.wait_got_ip_ms, 30000);
    }
  });
}
