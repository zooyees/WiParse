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
} from "./wiparse-if.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));

test("ICD defaults match WIPARSE-IF-001", () => {
  assert.equal(IF_SPEC.vid, 0xc251);
  assert.equal(IF_SPEC.pid, 0xf01a);
  assert.equal(CHANNELS.dap.tcp_port, 3240);
  assert.equal(CHANNELS.uart.tcp_port, 1234);
  assert.equal(CHANNELS.i2c.tcp_port, 1236);
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

for (const id of ["wiparse-dap", "wiparse-uart-bridge", "wiparse-i2c-rpc"]) {
  test(`${id} plugin.json + station.json validate`, () => {
    const dir = path.join(here, "..", "plugins", id);
    const plugin = JSON.parse(fs.readFileSync(path.join(dir, "plugin.json"), "utf8"));
    const station = JSON.parse(fs.readFileSync(path.join(dir, "station.json"), "utf8"));
    const p = validatePluginManifest(plugin);
    assert.equal(p.ok, true, p.errors?.join("; "));
    assert.equal(plugin.id, id);
    assert.ok(Array.isArray(plugin.outputs) && plugin.outputs.length >= 2);
    assert.ok(plugin.params.some((x) => x.name === "transport"));
    const s = validateStationConfig(station);
    assert.equal(s.ok, true, s.errors?.join("; "));
    assert.match(station.paths.lock_file, /test_dir/);
    assert.doesNotMatch(station.paths.lock_file, /plugin_dir/);
  });
}
