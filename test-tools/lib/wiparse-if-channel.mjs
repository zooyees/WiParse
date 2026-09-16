/**
 * Open a WIPARSE-IF channel by transport: auto | tcp | usb | serial | gui.
 */

import { spawn } from "node:child_process";
import {
  CHANNELS,
  IF_SPEC,
  asCommandList,
  bytesToHex,
  cdcControlLineState,
  encodeCdcLineCoding,
  encodeDapInfo,
  encodeElaphureMagic,
  encodeUsbipDevlistRequest,
  formatI2cCommand,
  padDapPacket,
  decodeDapInfo,
  parseHexBytes,
  parseI2cReply,
  parseUsbipDevlistReply,
  parityToCdc,
  resolveChannel,
  hostMatchesDevice,
  splitNetcfgControlChunks,
  parseNetcfgStatus,
  formatNetcfgCommand,
  stopBitsToCdc,
  uartBaudFirstPacket,
  vidPidTokens,
} from "./wiparse-if.mjs";
import {
  closeSocket,
  connectTcp,
  i2cRpc,
  pumpUntil,
  readLine,
  readSome,
  writeAll,
} from "./wiparse-if-net.mjs";
import {
  classifyPnpRow,
  compileWinusbHost,
  listPnpDevices,
  openWinusbChannel,
  WinusbSession,
} from "./wiparse-if-usb.mjs";
import { createClient } from "./wiparse-sdk.mjs";
import { pickInstrument, suggestedParamsFromInstrument } from "./plugin-contract.mjs";

export { decodeDapInfo };

export async function discoverUsb(channel, args = {}) {
  const spec = resolveChannel(channel, args);
  const pnp = await listPnpDevices({ vid: args.vid, pid: args.pid });
  const devices = (pnp.devices || []).map((row) => ({
    ...row,
    channel: classifyPnpRow(row),
  }));
  const built = compileWinusbHost();
  if (built.ok) {
    const sess = new WinusbSession(built.exe);
    try {
      const listed = await sess.list({ vid: args.vid, pid: args.pid });
      for (const d of listed.devices || []) {
        const row = {
          instance_id: d.instance_id || d.path || "",
          name: d.name || d.path || "",
          mi: d.mi,
          status: "listed",
          path: d.path || "",
        };
        row.channel = classifyPnpRow(row);
        devices.push(row);
      }
    } catch {
      /* helper list optional */
    } finally {
      sess.close();
    }
  }
  const match = devices.filter((d) => hostMatchesDevice(d, spec));
  return { ...pnp, channel, spec, match, devices };
}

export async function probeUsbip(host, port, timeoutMs = 4000, fallbackHost) {
  const sock = await connectTcp({ host, port, timeoutMs, fallbackHost });
  try {
    await writeAll(sock, encodeUsbipDevlistRequest());
    const buf = await readSome(sock, { max: 4096, timeoutMs: Math.max(800, timeoutMs), min: 8 });
    const parsed = parseUsbipDevlistReply(buf);
    if (parsed.ok || parsed.devices?.length) {
      return { ok: true, proto: "usbip", ...parsed, peer: sock.peer };
    }
    closeSocket(sock);
  } catch {
    try {
      closeSocket(sock);
    } catch {
      /* ignore */
    }
  }
  const sock2 = await connectTcp({ host, port, timeoutMs, fallbackHost });
  try {
    await writeAll(sock2, encodeElaphureMagic());
    const buf = await readSome(sock2, { max: 64, timeoutMs: 800, min: 0 }).catch(() => Buffer.alloc(0));
    return {
      ok: true,
      proto: "elaphure",
      magic: "0x8A656C70",
      peer: sock2.peer,
      preview_hex: bytesToHex(buf),
    };
  } finally {
    closeSocket(sock2);
  }
}

export async function probeDapUsbInfo(args = {}) {
  const opened = await openWinusbChannel("dap", args);
  if (!opened.ok) return opened;
  const { session, channel } = opened;
  try {
    const pkt = padDapPacket(encodeDapInfo(0xff), channel.packet);
    await session.write(channel.out_ep, pkt.toString("hex"), 2000);
    const rd = await session.read(channel.in_ep, channel.packet, 2000);
    const info = decodeDapInfo(Buffer.from(String(rd.hex || ""), "hex"));
    return { ok: true, dap_info: info, packet_size: info.u16, path: rd.path };
  } catch (e) {
    return { ok: false, error: String(e.message || e) };
  } finally {
    session.close();
  }
}

export async function listGuiInstruments(ctx, kind) {
  try {
    const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
    const res = await w.api.invoke(
      "instrument.list",
      kind ? { kind } : {},
      20_000,
      { allowFail: true }
    );
    const devices = res?.devices || res?.result?.devices || res?.data?.devices || [];
    return { ok: res?.ok !== false, devices: Array.isArray(devices) ? devices : [], raw: res };
  } catch (e) {
    return { ok: false, devices: [], error: String(e.message || e) };
  }
}

export async function listSerialPorts(ctx) {
  try {
    const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
    const res = await w.api.invoke("serial.ports", {}, 15_000, { allowFail: true });
    const ports = res?.ports || res?.result?.ports || res?.data || [];
    return { ok: res?.ok !== false, ports: Array.isArray(ports) ? ports : [], raw: res };
  } catch (e) {
    return { ok: false, ports: [], error: String(e.message || e) };
  }
}

export function pickProbe(devices, args) {
  return pickInstrument(devices, {
    kind: "debug_probe",
    resource: args.probe_resource || args.resource,
    deviceId: args.prefer_device_id,
    model: args.probe_model,
  });
}

export function probeSuggested(device) {
  return suggestedParamsFromInstrument(device, {
    prefer_device_id: "device_id",
    probe_resource: "resource",
    probe_model: "model",
  });
}

export async function guiCommand(ctx, deviceId, command, timeoutMs = 60_000) {
  const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
  return w.api.invoke(
    "instrument.command",
    { device_id: Number(deviceId), command },
    timeoutMs
  );
}

export async function guiConnect(ctx, resource, chip) {
  const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
  const params = { resource: String(resource) };
  if (chip) params.chip = String(chip);
  return w.api.invoke("instrument.connect", params, 60_000, { allowFail: true });
}

export function spawnTool(cmdline, { timeoutMs = 120_000, cwd, log } = {}) {
  const line = String(cmdline || "").trim();
  if (!line) return Promise.reject(new Error("tool command is empty"));
  return new Promise((resolve, reject) => {
    const child =
      process.platform === "win32"
        ? spawn("cmd.exe", ["/d", "/s", "/c", line], { cwd, windowsHide: true })
        : spawn("sh", ["-lc", line], { cwd });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {
        /* ignore */
      }
      reject(new Error(`tool timeout after ${timeoutMs}ms`));
    }, timeoutMs);
    child.stdout?.on("data", (b) => {
      const s = b.toString();
      stdout += s;
      log?.("stdout", s);
    });
    child.stderr?.on("data", (b) => {
      const s = b.toString();
      stderr += s;
      log?.("stderr", s);
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code: code ?? 1, stdout, stderr });
    });
  });
}

class TcpByteChannel {
  constructor(sock, kind) {
    this.kind = kind;
    this.transport = "tcp";
    this.sock = sock;
    this.peer = sock.peer;
  }
  write(buf) {
    return writeAll(this.sock, buf);
  }
  read(opts) {
    return readSome(this.sock, opts);
  }
  readLine(opts) {
    return readLine(this.sock, opts);
  }
  rpc(line, opts) {
    return i2cRpc(this.sock, line, opts);
  }
  pump(opts) {
    return pumpUntil({ sock: this.sock, ...opts });
  }
  close() {
    closeSocket(this.sock);
  }
}

class WinusbByteChannel {
  constructor(session, spec, kind) {
    this.kind = kind;
    this.transport = "usb";
    this.session = session;
    this.spec = spec;
    this._rx = Buffer.alloc(0);
  }
  async write(buf) {
    const hex = Buffer.isBuffer(buf) ? buf.toString("hex") : Buffer.from(buf).toString("hex");
    await this.session.write(this.spec.out_ep, hex, 3000);
  }
  async read({ max = 4096, timeoutMs = 1000 } = {}) {
    const rd = await this.session.read(this.spec.in_ep, max, timeoutMs);
    return Buffer.from(String(rd.hex || ""), "hex");
  }
  async readLine({ timeoutMs = 8000, max = 1024 } = {}) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const s = this._rx.toString("utf8");
      const m = s.match(/^([^\r\n]*)(\r\n|\n|\r)/);
      if (m) {
        this._rx = Buffer.from(s.slice(m[0].length), "utf8");
        if (!m[1].trim()) continue;
        return m[1];
      }
      const chunk = await this.read({
        max: 512,
        timeoutMs: Math.max(50, deadline - Date.now()),
      }).catch(() => Buffer.alloc(0));
      if (chunk.length) this._rx = Buffer.concat([this._rx, chunk]);
      if (this._rx.length > max) throw new Error("line overflow");
    }
    throw new Error("line timeout");
  }
  async rpc(line, { timeoutMs = 8000 } = {}) {
    const body = line.endsWith("\n") ? line : `${line}\n`;
    await this.write(body);
    const cmd = String(line).trim().split(/\s+/)[0]?.toUpperCase();
    if (cmd === "HELP" || cmd === "?") {
      const lines = [];
      const deadline = Date.now() + Math.min(timeoutMs, 2000);
      while (Date.now() < deadline) {
        try {
          lines.push(await this.readLine({ timeoutMs: Math.max(80, deadline - Date.now()) }));
        } catch {
          break;
        }
      }
      return { lines, text: lines.join("\n") };
    }
    const reply = await this.readLine({ timeoutMs });
    return { lines: [reply], text: reply };
  }
  async cdcLineCoding(opts) {
    const hex = encodeCdcLineCoding(opts).toString("hex");
    await this.session.control({
      type: 0x21,
      request: 0x20,
      value: 0,
      index: this.spec.cdc_if ?? 0,
      hex,
    });
  }
  async cdcDtr(dtr, rts) {
    await this.session.control({
      type: 0x21,
      request: 0x22,
      value: cdcControlLineState(dtr, rts),
      index: this.spec.cdc_if ?? 0,
      hex: "",
    });
  }
  async pump({ stop, durationMs = 0, onData, onTick } = {}) {
    const started = Date.now();
    let bytes = 0;
    while (true) {
      if (stop?.()) break;
      if (durationMs > 0 && Date.now() - started >= durationMs) break;
      const chunk = await this.read({ max: 4096, timeoutMs: 250 }).catch(() => Buffer.alloc(0));
      if (chunk.length) {
        bytes += chunk.length;
        onData?.(chunk);
      }
      onTick?.({ elapsed_ms: Date.now() - started, bytes });
    }
    return { bytes, elapsed_ms: Date.now() - started };
  }
  close() {
    this.session.close();
  }
}

export async function openUartChannel(args) {
  const spec = resolveChannel("uart", args);
  const transport = String(args.transport || "auto").toLowerCase();
  const host = args.host || IF_SPEC.mdns;
  const port = spec.tcp_port;
  const baud = Number(args.baud || CHANNELS.uart.default_baud);
  const timeoutMs = Number(args.connect_timeout_ms || 4000);
  const tcpOpts = { host, port, timeoutMs, fallbackHost: spec.fallback_host };
  const tryTcp = async () => {
    const sock = await connectTcp(tcpOpts);
    const first = uartBaudFirstPacket(baud);
    if (first && args.send_baud_cmd !== false && args.send_baud_cmd !== "false") {
      await writeAll(sock, first);
    }
    return new TcpByteChannel(sock, "uart");
  };
  const tryUsb = async () => {
    const opened = await openWinusbChannel("uart", args);
    if (!opened.ok) throw new Error(opened.error || "uart usb open failed");
    const ch = new WinusbByteChannel(opened.session, opened.channel, "uart");
    try {
      await ch.cdcLineCoding({
        baud,
        stop: stopBitsToCdc(args.stop_bits),
        parity: parityToCdc(args.parity),
        data_bits: Number(args.data_bits || 8),
      });
    } catch {
      /* line coding optional */
    }
    const dtr = args.dtr === false || args.dtr === "false" ? false : true;
    try {
      await ch.cdcDtr(dtr, Boolean(args.rts === true || args.rts === "true"));
    } catch {
      /* DTR optional */
    }
    return ch;
  };
  if (transport === "tcp") return tryTcp();
  if (transport === "usb") return tryUsb();
  if (transport === "auto") {
    try {
      return await tryUsb();
    } catch {
      return tryTcp();
    }
  }
  throw new Error(`UART transport ${transport} is not a byte channel (use serial via GUI)`);
}

export async function openI2cChannel(args) {
  const spec = resolveChannel("i2c", args);
  const transport = String(args.transport || "auto").toLowerCase();
  const host = args.host || IF_SPEC.mdns;
  const port = spec.tcp_port;
  const timeoutMs = Number(args.connect_timeout_ms || 4000);
  const tryTcp = async () => {
    const sock = await connectTcp({
      host,
      port,
      timeoutMs,
      fallbackHost: spec.fallback_host,
    });
    return new TcpByteChannel(sock, "i2c");
  };
  const tryUsb = async () => {
    const opened = await openWinusbChannel("i2c", args);
    if (!opened.ok) throw new Error(opened.error || "i2c usb open failed");
    return new WinusbByteChannel(opened.session, opened.channel, "i2c");
  };
  if (transport === "tcp") return tryTcp();
  if (transport === "usb") return tryUsb();
  if (transport === "auto") {
    try {
      return await tryUsb();
    } catch {
      return tryTcp();
    }
  }
  throw new Error(`I2C transport ${transport} is not an RPC channel`);
}

export async function runI2cRecipe(ch, items, { onLine, timeoutMs = 8000 } = {}) {
  const results = [];
  for (const item of asCommandList(items)) {
    const line = formatI2cCommand(item);
    onLine?.(line);
    const { text, lines } = await ch.rpc(line, { timeoutMs });
    const parsed = parseI2cReply(lines?.[0] || text);
    results.push({ line, reply: text, parsed, lines });
    if (parsed.ok === false && parsed.kind === "err") {
      const cont =
        item && typeof item === "object" && (item.continue_on_error === true || item.continue_on_error === "true");
      if (!cont) return { ok: false, results, error: parsed.raw };
    }
  }
  return { ok: results.every((r) => r.parsed?.ok === true), results };
}

class WinusbNetcfgChannel {
  constructor(session, spec) {
    this.kind = "netcfg";
    this.transport = "usb";
    this.session = session;
    this.spec = spec;
  }
  async rpc(line, { timeoutMs = 8000 } = {}) {
    const body = String(line || "").replace(/[\r\n]+$/g, "");
    if (!body) return { lines: [], text: "" };
    const spec = this.spec;
    const chunks = splitNetcfgControlChunks(body, spec.mps);
    for (const chunk of chunks) {
      await this.session.control({
        type: spec.bm_out,
        request: spec.tx_breq,
        value: chunk.last ? 1 : 0,
        index: spec.mi,
        hex: chunk.hex,
        timeoutMs,
      });
    }
    let reply = Buffer.alloc(0);
    let off = 0;
    let empty = 0;
    const deadline = Date.now() + timeoutMs;
    const cmd = body.trim().split(/\s+/)[0]?.toUpperCase();
    const help = cmd === "HELP" || cmd === "?";
    while (Date.now() < deadline) {
      const rd = await this.session.control({
        type: spec.bm_in,
        request: spec.rx_breq,
        value: off,
        index: spec.mi,
        hex: "",
        max: spec.mps,
        timeoutMs: Math.max(50, deadline - Date.now()),
      });
      const n = Number(rd.transferred ?? 0);
      const data = Buffer.from(String(rd.hex || ""), "hex").subarray(0, n > 0 ? n : 0);
      if (!data.length) {
        if (off === 0 && empty < 10) {
          empty += 1;
          await new Promise((r) => setTimeout(r, 20));
          continue;
        }
        break;
      }
      empty = 0;
      reply = Buffer.concat([reply, data]);
      if (!help && (reply.includes(0x0a) || data.length < spec.mps)) break;
      if (help && data.length < spec.mps) break;
      off += data.length;
      if (off > 2048) break;
    }
    const text = reply.toString("utf8").replace(/\r/g, "").trimEnd();
    const lines = text.split("\n").map((l) => l.trimEnd()).filter((l) => l.length);
    return { lines, text: lines.join("\n") };
  }
  close() {
    this.session.close();
  }
}

export async function openNetcfgChannel(args) {
  const spec = resolveChannel("netcfg", args);
  const transport = String(args.transport || "usb").toLowerCase();
  const host = args.host || IF_SPEC.mdns;
  const port = spec.tcp_port;
  const timeoutMs = Number(args.connect_timeout_ms || 4000);
  const tryTcp = async () => {
    const sock = await connectTcp({
      host,
      port,
      timeoutMs,
      fallbackHost: spec.fallback_host,
    });
    return new TcpByteChannel(sock, "netcfg");
  };
  const tryUsb = async () => {
    const opened = await openWinusbChannel("netcfg", args);
    if (!opened.ok) throw new Error(opened.error || "netcfg usb open failed");
    return new WinusbNetcfgChannel(opened.session, opened.channel);
  };
  if (transport === "tcp") return tryTcp();
  if (transport === "usb") return tryUsb();
  if (transport === "auto") {
    try {
      return await tryUsb();
    } catch {
      return tryTcp();
    }
  }
  throw new Error(`NETCFG transport ${transport} is not supported (use usb, tcp, or auto)`);
}

export async function runNetcfgRecipe(ch, items, { onLine, timeoutMs = 8000 } = {}) {
  const results = [];
  for (const item of asCommandList(items)) {
    const line = typeof item === "string" ? item.trim() : formatNetcfgCommand(item);
    if (!line) continue;
    onLine?.(line);
    const { text, lines } = await ch.rpc(line, { timeoutMs });
    const parsed = parseI2cReply(lines?.[0] || text);
    results.push({ line, reply: text, parsed, lines });
    if (parsed.ok === false && parsed.kind === "err") {
      const cont =
        item && typeof item === "object" && (item.continue_on_error === true || item.continue_on_error === "true");
      if (!cont) return { ok: false, results, error: parsed.raw };
    }
  }
  return { ok: results.every((r) => r.parsed?.ok !== false), results };
}

export async function waitNetcfgGotIp(ch, { timeoutMs = 15000, intervalMs = 500, stop, onTick } = {}) {
  const deadline = Date.now() + Math.max(0, Number(timeoutMs) || 0);
  let last = null;
  while (Date.now() <= deadline) {
    if (stop?.()) break;
    const { text } = await ch.rpc("STATUS");
    last = parseNetcfgStatus(text);
    onTick?.(last);
    if (last.got_ip) return last;
    if (Date.now() >= deadline) break;
    await new Promise((r) => setTimeout(r, Math.max(100, Number(intervalMs) || 500)));
  }
  return last;
}

export async function configureI2cBus(ch, { speed_hz, timeout_ms } = {}) {
  const out = {};
  if (speed_hz != null && String(speed_hz) !== "") {
    out.speed = await runI2cRecipe(ch, [{ cmd: "SPEED", hz: Number(speed_hz) }]);
  }
  if (timeout_ms != null && String(timeout_ms) !== "") {
    out.timeout = await runI2cRecipe(ch, [{ cmd: "TIMEOUT", ms: Number(timeout_ms) }]);
  }
  return out;
}

export function hexPreview(buf, limit = 64) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf || []);
  const slice = b.subarray(0, limit);
  return {
    bytes: b.length,
    hex: bytesToHex(slice),
    truncated: b.length > limit,
  };
}

export { parseHexBytes, vidPidTokens, compileWinusbHost, CHANNELS, IF_SPEC };
