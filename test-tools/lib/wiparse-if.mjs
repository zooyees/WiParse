/**
 * WIPARSE-IF-001 host helpers (ICD 2026-09-15).
 * Protocol only — no Hub plugin.id branches. Defaults match the dongle;
 * every identifier is overridable so the same plugins talk to any peer
 * that speaks CMSIS-DAP v2 / UART bridge / I2C text RPC.
 */

export const IF_SPEC = Object.freeze({
  id: "WIPARSE-IF-001",
  version: "1.2",
  date: "2026-09-15",
  manufacturer: "WIPARSE",
  product: "WIPARSE CMSIS-DAP",
  vid: 0xc251,
  pid: 0xf01a,
  bcd_device: 0x0206,
  mdns: "dap.local",
  default_ipv4: "192.168.137.123",
  sta_ssids: Object.freeze(["OTA", "DAP"]),
  sta_password: "12345678",
  softap_ssid: "DAP-Dongle",
  cmsis_guid: "{CDB3B5AD-293B-4663-AA36-1AAE46463776}",
  uart_guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0101}",
  i2c_guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0103}",
  netcfg_guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105}",
  usb_device_guid: "{A5DCBF10-6530-11D2-901F-00C04FB951ED}",
  winusb_guid: "{88BAE032-5A81-49F0-BC3D-A4FF138216D6}",
});

export const CHANNELS = Object.freeze({
  dap: Object.freeze({
    id: "CH-DAP",
    name: "WIPARSE CMSIS-DAP v2",
    mi: 0,
    out_ep: 0x01,
    in_ep: 0x81,
    packet: 512,
    mps: 64,
    tcp_port: 3240,
    default_swj_hz: 1_000_000,
    pins: Object.freeze({
      SWDIO: 11,
      SWCLK: 12,
      nRESET: 13,
      nTRST: 14,
      TDI: 10,
      TDO: 9,
    }),
  }),
  uart: Object.freeze({
    id: "CH-UART",
    name: "WIPARSE UART",
    mi: 1,
    out_ep: 0x02,
    in_ep: 0x83,
    notify_ep: 0x82,
    tcp_port: 1234,
    baud_min: 1200,
    baud_max: 2_000_000,
    default_baud: 115200,
    cdc_if: 1,
    guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0101}",
  }),
  i2c: Object.freeze({
    id: "CH-I2C",
    name: "WIPARSE I2C",
    mi: 3,
    out_ep: 0x03,
    in_ep: 0x84,
    tcp_port: 1236,
    speed_min: 10_000,
    speed_max: 1_000_000,
    default_speed: 400_000,
    timeout_min: 1,
    timeout_max: 1000,
    default_timeout_ms: 50,
    scan_lo: 0x08,
    scan_hi: 0x77,
    max_frame: 256,
    max_line: 1024,
    guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0103}",
  }),
  netcfg: Object.freeze({
    id: "CH-NETCFG",
    name: "WIPARSE NET",
    mi: 5,
    tcp_port: 1238,
    tx_breq: 0x20,
    rx_breq: 0x21,
    bm_out: 0x41,
    bm_in: 0xc1,
    mps: 64,
    max_line: 128,
    guid: "{7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105}",
  }),
});

export const USBIP = Object.freeze({
  version: 0x0111,
  op_req_devlist: 0x8005,
  op_rep_devlist: 0x0005,
  elaphure_magic: 0x8a656c70,
});

export const DAP_CMD = Object.freeze({
  info: 0x00,
  host_status: 0x01,
  connect: 0x02,
  disconnect: 0x03,
  transfer_configure: 0x04,
  transfer: 0x05,
  swj_clock: 0x11,
  swj_sequence: 0x12,
  swd_configure: 0x13,
  reset_target: 0x0a,
});

export const DAP_INFO_ID = Object.freeze({
  vendor: 0x01,
  product: 0x02,
  serial: 0x03,
  fw_version: 0x04,
  caps: 0xf0,
  packet_count: 0xfe,
  packet_size: 0xff,
});

export function parseHexInt(v, fallback = NaN) {
  if (v == null || v === "") return fallback;
  if (typeof v === "number" && Number.isFinite(v)) return v >>> 0;
  const s = String(v).trim();
  if (!s) return fallback;
  const n = Number.parseInt(s, s.startsWith("0x") || s.startsWith("0X") ? 16 : 10);
  return Number.isFinite(n) ? n >>> 0 : fallback;
}

export function parseHexWord(token) {
  if (token == null || token === "") return null;
  const s = String(token).trim();
  if (!s) return null;
  const n = Number.parseInt(s.replace(/^0x/i, ""), 16);
  if (!Number.isFinite(n) || n < 0 || n > 255) return null;
  return n;
}

export function hexByte(n) {
  return (n & 0xff).toString(16).toUpperCase().padStart(2, "0");
}

export function bytesToHex(buf, sep = " ") {
  const u8 = Buffer.isBuffer(buf) ? buf : Buffer.from(buf || []);
  const parts = [];
  for (const b of u8) parts.push(hexByte(b));
  return parts.join(sep);
}

export function parseHexBytes(input) {
  if (input == null || input === "") return Buffer.alloc(0);
  if (Buffer.isBuffer(input)) return input;
  if (Array.isArray(input)) {
    return Buffer.from(input.map((x) => parseHexWord(x) ?? Number(x) & 0xff));
  }
  const s = String(input).trim();
  if (!s) return Buffer.alloc(0);
  if (/^[0-9a-fA-F]+$/.test(s) && s.length % 2 === 0 && !s.includes(" ")) {
    return Buffer.from(s, "hex");
  }
  const out = [];
  for (const tok of s.split(/[\s,;:]+/).filter(Boolean)) {
    const b = parseHexWord(tok);
    if (b == null) throw new Error(`invalid hex byte: ${tok}`);
    out.push(b);
  }
  return Buffer.from(out);
}

export function vidPidTokens(vid, pid) {
  const v = (parseHexInt(vid, IF_SPEC.vid) & 0xffff).toString(16).toUpperCase().padStart(4, "0");
  const p = (parseHexInt(pid, IF_SPEC.pid) & 0xffff).toString(16).toUpperCase().padStart(4, "0");
  return { vid: v, pid: p, hardware: `VID_${v}&PID_${p}` };
}

/** ICD §7.3 first-payload baud command (TCP :1234). */
export function uartBaudFirstPacket(baud) {
  const n = Number(baud);
  if (!Number.isInteger(n) || n < CHANNELS.uart.baud_min || n > CHANNELS.uart.baud_max) {
    return null;
  }
  const s = String(n);
  if (s.length < 2 || s.length > 7) return null;
  if (!/^[1-9][0-9]*$/.test(s)) return null;
  return Buffer.from(s, "ascii");
}

export function isUartBaudFirstPacket(buf) {
  const raw = Buffer.isBuffer(buf) ? buf : Buffer.from(String(buf ?? ""), "utf8");
  if (raw.length < 2 || raw.length > 7) return false;
  const s = raw.toString("ascii");
  if (!/^[0-9]+$/.test(s)) return false;
  const n = Number.parseInt(s, 10);
  if (n < CHANNELS.uart.baud_min || n > CHANNELS.uart.baud_max) return false;
  if (String(n).length !== raw.length) return false;
  return true;
}

export function encodeCdcLineCoding({
  baud = CHANNELS.uart.default_baud,
  stop = 0,
  parity = 0,
  data_bits = 8,
} = {}) {
  const b = Buffer.alloc(7);
  b.writeUInt32LE(Number(baud) >>> 0, 0);
  b.writeUInt8(Number(stop) & 0xff, 4);
  b.writeUInt8(Number(parity) & 0xff, 5);
  b.writeUInt8(Number(data_bits) & 0xff, 6);
  return b;
}

export function decodeCdcLineCoding(buf) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf || []);
  if (b.length < 7) return null;
  return {
    baud: b.readUInt32LE(0),
    stop: b[4],
    parity: b[5],
    data_bits: b[6],
  };
}

export function cdcControlLineState(dtr, rts) {
  return (dtr ? 1 : 0) | (rts ? 2 : 0);
}

export function stopBitsToCdc(v) {
  const s = String(v ?? "1").trim();
  if (s === "1.5" || s === "1,5") return 1;
  if (s === "2") return 2;
  return 0;
}

export function parityToCdc(v) {
  const s = String(v ?? "none").trim().toLowerCase();
  if (s === "odd" || s === "1") return 1;
  if (s === "even" || s === "2") return 2;
  return 0;
}

const I2C_CMDS = new Set([
  "HELP",
  "?",
  "PING",
  "SCAN",
  "SPEED",
  "TIMEOUT",
  "RECOVER",
  "BUSRESET",
  "WRITE",
  "READ",
  "WRITEREAD",
]);

function i2cAddrToken(addr) {
  if (addr == null || addr === "") throw new Error("I2C address required");
  const s = String(addr).trim();
  const n = /^0x/i.test(s) || /^[0-9a-fA-F]{1,2}$/.test(s)
    ? Number.parseInt(s.replace(/^0x/i, ""), 16)
    : Number.parseInt(s, 10);
  if (!Number.isInteger(n) || n < 0 || n > 0x7f) {
    throw new Error(`I2C address out of 7-bit range: ${addr}`);
  }
  return hexByte(n);
}

function i2cDataTokens(data) {
  if (data == null || data === "") return [];
  const buf = parseHexBytes(data);
  if (buf.length > CHANNELS.i2c.max_frame) {
    throw new Error(`I2C payload exceeds ${CHANNELS.i2c.max_frame} bytes`);
  }
  return [...buf].map(hexByte);
}

/**
 * Build one I2C RPC line (no trailing newline). Accepts a string, or
 * `{ cmd, addr, data, len, hz, ms, line }`.
 */
export function formatI2cCommand(item) {
  if (item == null) throw new Error("empty I2C command");
  if (typeof item === "string") {
    const line = item.trim();
    if (!line) throw new Error("empty I2C command");
    if (Buffer.byteLength(line, "utf8") > CHANNELS.i2c.max_line - 1) {
      throw new Error("I2C line exceeds 1023 bytes");
    }
    return line;
  }
  if (item.line) return formatI2cCommand(String(item.line));
  const cmd = String(item.cmd || item.command || item.op || "")
    .trim()
    .toUpperCase();
  if (!cmd) throw new Error("I2C cmd required");
  if (cmd === "HELP" || cmd === "?") return "HELP";
  if (cmd === "SCAN") return "SCAN";
  if (cmd === "RECOVER" || cmd === "BUSRESET") return "RECOVER";
  if (cmd === "SPEED") {
    if (item.hz == null && item.speed == null && item.value == null) return "SPEED";
    const hz = Number(item.hz ?? item.speed ?? item.value);
    if (!Number.isFinite(hz)) throw new Error("SPEED hz invalid");
    return `SPEED ${hz}`;
  }
  if (cmd === "TIMEOUT") {
    if (item.ms == null && item.timeout == null && item.value == null) return "TIMEOUT";
    const ms = Number(item.ms ?? item.timeout ?? item.value);
    if (!Number.isFinite(ms)) throw new Error("TIMEOUT ms invalid");
    return `TIMEOUT ${ms}`;
  }
  if (cmd === "PING") return `PING ${i2cAddrToken(item.addr ?? item.address)}`;
  if (cmd === "WRITE") {
    const toks = [i2cAddrToken(item.addr ?? item.address), ...i2cDataTokens(item.data ?? item.data_hex ?? item.bytes)];
    return `WRITE ${toks.join(" ")}`.trim();
  }
  if (cmd === "READ") {
    const n = Number(item.len ?? item.n ?? item.length);
    if (!Number.isInteger(n) || n < 1 || n > CHANNELS.i2c.max_frame) {
      throw new Error("READ len must be 1–256");
    }
    return `READ ${i2cAddrToken(item.addr ?? item.address)} ${n}`;
  }
  if (cmd === "WRITEREAD") {
    const n = Number(item.len ?? item.n ?? item.length);
    if (!Number.isInteger(n) || n < 1 || n > CHANNELS.i2c.max_frame) {
      throw new Error("WRITEREAD len must be 1–256");
    }
    const data = i2cDataTokens(item.data ?? item.data_hex ?? item.bytes);
    return `WRITEREAD ${[i2cAddrToken(item.addr ?? item.address), ...data, String(n)].join(" ")}`;
  }
  throw new Error(`unknown I2C command: ${cmd}`);
}

export function parseI2cReply(line) {
  const raw = String(line ?? "").replace(/\r$/, "");
  const parts = raw.trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return { ok: false, kind: "empty", raw };
  const head = parts[0].toUpperCase();
  if (head === "OK") {
    const tag = (parts[1] || "").toUpperCase();
    const rest = parts.slice(2);
    const hex = [];
    for (const t of rest) {
      const b = parseHexWord(t);
      if (b == null) break;
      hex.push(b);
    }
    return {
      ok: true,
      kind: "ok",
      tag,
      args: rest,
      hex: Buffer.from(hex),
      raw,
    };
  }
  if (head === "ERR") {
    return {
      ok: false,
      kind: "err",
      reason: parts[1] || "unknown",
      detail: parts.slice(2).join(" "),
      raw,
    };
  }
  return { ok: false, kind: "unknown", raw };
}

export function isI2cCommandName(name) {
  return I2C_CMDS.has(String(name || "").trim().toUpperCase());
}

export function encodeUsbipDevlistRequest() {
  const b = Buffer.alloc(8);
  b.writeUInt16BE(USBIP.version, 0);
  b.writeUInt16BE(USBIP.op_req_devlist, 2);
  b.writeUInt32BE(0, 4);
  return b;
}

export function encodeElaphureMagic() {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(USBIP.elaphure_magic, 0);
  return b;
}

/**
 * Parse USB/IP OP_REP_DEVLIST enough to list vendor/product/busid.
 */
export function parseUsbipDevlistReply(buf) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf || []);
  if (b.length < 8) return { ok: false, error: "short usbip header", devices: [] };
  const version = b.readUInt16BE(0);
  const code = b.readUInt16BE(2);
  const status = b.readUInt32BE(4);
  if (code !== USBIP.op_rep_devlist && code !== USBIP.op_req_devlist) {
    return {
      ok: false,
      error: `unexpected usbip code 0x${code.toString(16)}`,
      version,
      code,
      status,
      devices: [],
    };
  }
  if (b.length < 12) {
    return { ok: true, version, code, status, devices: [], truncated: true };
  }
  const n = b.readUInt32BE(8);
  const devices = [];
  let off = 12;
  for (let i = 0; i < n && off + 312 <= b.length; i++) {
    const path = b.slice(off, off + 256).toString("ascii").replace(/\0+$/, "");
    const busid = b.slice(off + 256, off + 288).toString("ascii").replace(/\0+$/, "");
    const busnum = b.readUInt32BE(off + 288);
    const devnum = b.readUInt32BE(off + 292);
    const speed = b.readUInt32BE(off + 296);
    const idVendor = b.readUInt16BE(off + 300);
    const idProduct = b.readUInt16BE(off + 302);
    const bcdDevice = b.readUInt16BE(off + 304);
    const bDeviceClass = b[off + 306];
    const bNumInterfaces = b[off + 310];
    devices.push({
      path,
      busid,
      busnum,
      devnum,
      speed,
      vid: idVendor,
      pid: idProduct,
      bcd_device: bcdDevice,
      device_class: bDeviceClass,
      interfaces: bNumInterfaces,
    });
    off += 312 + Math.max(0, bNumInterfaces) * 4;
  }
  return { ok: status === 0, version, code, status, count: n, devices };
}

export function encodeDapInfo(id = DAP_INFO_ID.packet_size) {
  return Buffer.from([DAP_CMD.info, id & 0xff]);
}

export function decodeDapInfo(buf) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf || []);
  if (!b.length || b[0] !== DAP_CMD.info) return { ok: false, raw: b };
  const len = b.length > 1 ? b[1] : 0;
  const payload = b.slice(2, 2 + len);
  let text = "";
  let u16 = null;
  if (len >= 1 && payload[len - 1] === 0) {
    text = payload.slice(0, -1).toString("utf8");
  } else {
    text = payload.toString("utf8");
  }
  if (len === 2) u16 = payload.readUInt16LE(0);
  if (len === 1) u16 = payload[0];
  return { ok: true, len, payload, text: text.replace(/\0+$/, ""), u16, raw: b };
}

export function padDapPacket(cmd, packetSize = CHANNELS.dap.packet) {
  const src = Buffer.isBuffer(cmd) ? cmd : Buffer.from(cmd);
  const n = Math.max(Number(packetSize) || CHANNELS.dap.packet, src.length);
  const out = Buffer.alloc(n, 0);
  src.copy(out);
  return out;
}

/**
 * Overlay ICD channel defaults with plugin params (MI / endpoints / TCP).
 * Dongle layout is the default, not a hard lock.
 */
export function resolveChannel(kind, args = {}) {
  const base = CHANNELS[kind];
  if (!base) throw new Error(`unknown channel ${kind}`);
  const numOr = (v, dflt) => {
    if (v == null || v === "") return dflt;
    const n = Number(v);
    return Number.isFinite(n) ? n : dflt;
  };
  const epOr = (v, dflt) => {
    if (v == null || v === "") return dflt;
    const s = String(v).trim();
    if (/^0x/i.test(s) || /^[0-9a-fA-F]{1,2}$/.test(s)) {
      const n = Number.parseInt(s.replace(/^0x/i, ""), 16);
      return Number.isFinite(n) ? n : dflt;
    }
    const n = Number(s);
    return Number.isFinite(n) ? n : dflt;
  };
  const fb = args.fallback_host;
  return {
    ...base,
    kind,
    mi: numOr(args.usb_mi, base.mi),
    out_ep: epOr(args.out_ep, base.out_ep),
    in_ep: epOr(args.in_ep, base.in_ep),
    notify_ep: epOr(args.notify_ep, base.notify_ep),
    cdc_if: numOr(args.cdc_if, base.cdc_if ?? 0),
    tcp_port: numOr(args.tcp_port, base.tcp_port),
    packet: numOr(args.packet_size, base.packet || 512),
    usb_name: String(args.usb_name ?? "").trim(),
    fallback_host:
      fb === undefined || fb === null ? IF_SPEC.default_ipv4 : String(fb).trim(),
  };
}

export function hostMatchesDevice(row, spec) {
  if (!spec) return false;
  const mi = row?.mi;
  if (mi != null && Number(mi) === Number(spec.mi)) return true;
  const name = String(row?.name || "").toUpperCase();
  if (spec.usb_name && name.includes(String(spec.usb_name).toUpperCase())) return true;
  if (spec.kind === "dap" && name.includes("CMSIS-DAP")) return true;
  if (spec.kind === "uart" && !spec.usb_name && name.includes("UART")) return true;
  if (spec.kind === "i2c" && !spec.usb_name && name.includes("I2C")) return true;
  if (spec.kind === "netcfg" && !spec.usb_name && (name.includes("NET") || name.includes("NETCFG"))) {
    return true;
  }
  return false;
}

export function hostCandidates(host, fallbackHost) {
  const raw = String(host || "").trim();
  const fb =
    fallbackHost === undefined || fallbackHost === null
      ? IF_SPEC.default_ipv4
      : String(fallbackHost).trim();
  const list = [];
  const add = (h) => {
    const s = String(h || "").trim();
    if (s && !list.includes(s)) list.push(s);
  };
  add(raw);
  if (!raw) {
    add(IF_SPEC.mdns);
    add(fb);
  } else if (raw.toLowerCase() === IF_SPEC.mdns && fb) {
    add(fb);
  }
  return list.length ? list : fb ? [fb] : [];
}

export function overlayChannelArgs(config, args = {}) {
  const ch = config?.channel || {};
  return {
    usb_mi: args.usb_mi ?? ch.usb_mi,
    usb_name: args.usb_name ?? ch.usb_name,
    out_ep: args.out_ep ?? ch.out_ep,
    in_ep: args.in_ep ?? ch.in_ep,
    cdc_if: args.cdc_if ?? ch.cdc_if,
    packet_size: args.packet_size ?? ch.packet_size,
    fallback_host: args.fallback_host ?? ch.fallback_host,
  };
}

export function parseJsonParam(value, fallback) {
  if (value == null || value === "") return fallback;
  if (typeof value === "object") return value;
  const s = String(value).trim();
  if (!s) return fallback;
  try {
    return JSON.parse(s);
  } catch {
    return fallback;
  }
}

export function asCommandList(value) {
  const parsed = parseJsonParam(value, value);
  if (parsed == null || parsed === "") return [];
  if (Array.isArray(parsed)) return parsed.filter((x) => x != null && x !== "");
  if (typeof parsed === "string") {
    return parsed
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#"));
  }
  return [parsed];
}

export default {
  IF_SPEC,
  CHANNELS,
  USBIP,
  DAP_CMD,
  DAP_INFO_ID,
};
