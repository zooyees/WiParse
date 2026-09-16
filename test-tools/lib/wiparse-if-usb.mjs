/**
 * Windows WinUSB host for WIPARSE composite functions (MI_00/01/03/05).
 * Compiles test-tools/lib/winusb-host.cs with .NET Framework csc on first use.
 * Linux/macOS: discovery is best-effort (lsusb / system_profiler); open is TCP/COM.
 */

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CHANNELS, IF_SPEC, resolveChannel, vidPidTokens } from "./wiparse-if.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CS_SRC = path.join(__dirname, "winusb-host.cs");

function cacheExe() {
  return path.join(os.tmpdir(), "wiparse-winusb-host.exe");
}

export function findCsc() {
  const roots = [
    path.join(process.env.WINDIR || "C:\\Windows", "Microsoft.NET", "Framework64", "v4.0.30319", "csc.exe"),
    path.join(process.env.WINDIR || "C:\\Windows", "Microsoft.NET", "Framework", "v4.0.30319", "csc.exe"),
  ];
  for (const p of roots) {
    if (fs.existsSync(p)) return p;
  }
  return "";
}

export function compileWinusbHost() {
  if (process.platform !== "win32") {
    return { ok: false, error: "WinUSB helper is Windows-only" };
  }
  if (!fs.existsSync(CS_SRC)) {
    return { ok: false, error: `missing ${CS_SRC}` };
  }
  const dest = cacheExe();
  try {
    const srcStat = fs.statSync(CS_SRC);
    if (fs.existsSync(dest) && fs.statSync(dest).mtimeMs >= srcStat.mtimeMs) {
      return { ok: true, exe: dest, cached: true };
    }
  } catch {
    /* rebuild */
  }
  const csc = findCsc();
  if (!csc) return { ok: false, error: "csc.exe not found (.NET Framework 4)" };
  const args = [
    "/nologo",
    "/optimize+",
    "/r:System.Web.Extensions.dll",
    `/out:${dest}`,
    CS_SRC,
  ];
  const r = spawnSync(csc, args, { encoding: "utf8", windowsHide: true });
  if (r.status !== 0 || !fs.existsSync(dest)) {
    return {
      ok: false,
      error: (r.stderr || r.stdout || `csc exit ${r.status}`).trim().slice(0, 400),
    };
  }
  return { ok: true, exe: dest, cached: false };
}

function parsePsJson(text) {
  const s = String(text || "").trim();
  if (!s) return [];
  try {
    const v = JSON.parse(s);
    return Array.isArray(v) ? v : [v];
  } catch {
    return [];
  }
}

export async function listPnpDevices({ vid, pid } = {}) {
  const { vid: v, pid: p, hardware } = vidPidTokens(vid, pid);
  if (process.platform === "win32") {
    const script = [
      "$ErrorActionPreference='SilentlyContinue'",
      `Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '${hardware.replace(/'/g, "")}' } |`,
      "Select-Object InstanceId, FriendlyName, Status, Class, Problem | ConvertTo-Json -Compress",
    ].join(" ");
    const r = spawnSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      { encoding: "utf8", windowsHide: true, timeout: 20_000 }
    );
    const rows = parsePsJson(r.stdout).map((row) => ({
      instance_id: row.InstanceId || row.instanceId || "",
      name: row.FriendlyName || row.friendlyName || "",
      status: row.Status || "",
      class: row.Class || "",
      problem: row.Problem || "",
      mi: parseMi(row.InstanceId || ""),
    }));
    return { ok: true, vid: v, pid: p, devices: rows, source: "pnp" };
  }
  if (process.platform === "linux") {
    const r = spawnSync("lsusb", ["-d", `${v}:${p}`], { encoding: "utf8" });
    const devices = String(r.stdout || "")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => ({ name: line, instance_id: line, status: "listed", mi: null }));
    return { ok: r.status === 0, vid: v, pid: p, devices, source: "lsusb" };
  }
  return { ok: true, vid: v, pid: p, devices: [], source: "unsupported" };
}

function parseMi(instanceId) {
  const m = String(instanceId || "").match(/MI_([0-9A-Fa-f]{2})/i);
  return m ? Number.parseInt(m[1], 16) : null;
}

export function classifyPnpRow(row) {
  const name = String(row.name || "").toUpperCase();
  const mi = row.mi;
  if (mi === CHANNELS.dap.mi || name.includes("CMSIS-DAP")) return "dap";
  if (mi === CHANNELS.netcfg.mi || name.includes("NETCFG") || name.includes("WIPARSE NET")) {
    return "netcfg";
  }
  if (mi === CHANNELS.uart.mi || name.includes("UART")) return "uart";
  if (mi === CHANNELS.i2c.mi || name.includes("I2C")) return "i2c";
  return "other";
}

export class WinusbSession {
  constructor(exe) {
    this.exe = exe;
    this.child = null;
    this.buf = "";
    this.pending = new Map();
    this.nextId = 1;
  }

  start() {
    if (this.child) return;
    this.child = spawn(this.exe, [], {
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (s) => this._onStdout(s));
    this.child.stderr?.on("data", () => {});
    this.child.on("close", () => {
      for (const [, rec] of this.pending) rec.reject(new Error("winusb-host exited"));
      this.pending.clear();
      this.child = null;
    });
  }

  _onStdout(s) {
    this.buf += s;
    let idx;
    while ((idx = this.buf.indexOf("\n")) >= 0) {
      const line = this.buf.slice(0, idx).trim();
      this.buf = this.buf.slice(idx + 1);
      if (!line) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      const rec = this.pending.get(msg.id);
      if (!rec) continue;
      this.pending.delete(msg.id);
      if (msg.ok === false) rec.reject(new Error(msg.error || "winusb error"));
      else rec.resolve(msg);
    }
  }

  request(op, extra = {}, timeoutMs = 8000) {
    this.start();
    const id = this.nextId++;
    const payload = { id, op, ...extra };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`winusb ${op} timeout`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
      this.child.stdin.write(`${JSON.stringify(payload)}\n`);
    });
  }

  list(opts = {}) {
    return this.request("list", {
      vid: opts.vid || IF_SPEC.vid.toString(16),
      pid: opts.pid || IF_SPEC.pid.toString(16),
      guid: opts.guid || "",
    });
  }

  open({ path: devicePath, mi, vid, pid, associate } = {}) {
    const extra = {
      path: devicePath || "",
      mi: mi ?? 0,
      vid: vid || IF_SPEC.vid.toString(16),
      pid: pid || IF_SPEC.pid.toString(16),
    };
    if (associate === 0 || associate === false) extra.associate = 0;
    else if (associate === 1 || associate === true) extra.associate = 1;
    return this.request("open", extra);
  }

  write(pipe, hex, timeoutMs = 3000) {
    return this.request("write", { pipe, hex, timeout_ms: timeoutMs }, timeoutMs + 1000);
  }

  read(pipe, max = 4096, timeoutMs = 1000) {
    return this.request(
      "read",
      { pipe, max, timeout_ms: timeoutMs },
      timeoutMs + 1000
    );
  }

  control({ type, request, value = 0, index = 0, hex = "", max = 64, timeoutMs = 2000 } = {}) {
    return this.request(
      "control",
      { type, request, value, index, hex, max, timeout_ms: timeoutMs },
      timeoutMs + 1000
    );
  }

  close() {
    const child = this.child;
    try {
      this.child?.stdin.write(`${JSON.stringify({ id: this.nextId++, op: "quit" })}\n`);
    } catch {
      /* ignore */
    }
    try {
      child?.kill();
    } catch {
      /* ignore */
    }
    this.child = null;
  }
}

export async function openWinusbChannel(channel, args = {}) {
  const built = compileWinusbHost();
  if (!built.ok) return { ok: false, error: built.error };
  const ch = resolveChannel(channel, args);
  const sess = new WinusbSession(built.exe);
  try {
    if (channel !== "dap") {
      const listed = await sess.list({ vid: args.vid, pid: args.pid, guid: ch.guid });
      const usable = (listed.devices || []).some(
        (d) => Number(d.mi) === ch.mi && String(d.path || "").length > 0
      );
      if (!usable) {
        sess.close();
        const bcd = `0x${IF_SPEC.bcd_device.toString(16).padStart(4, "0")}`;
        const fw = channel === "netcfg" ? `${bcd} (WIPARSE NET GUID)` : `${bcd} (UART/I2C GUID)`;
        return {
          ok: false,
          error: `${ch.usb_name || ch.name} MI_${ch.mi.toString(16).padStart(2, "0")} has no openable WinUSB path; flash bcdDevice ${fw} or use TCP :${ch.tcp_port}`,
        };
      }
    }
    await sess.open({
      mi: ch.mi,
      vid: args.vid,
      pid: args.pid,
      associate: channel === "netcfg" ? 0 : 1,
    });
    return { ok: true, session: sess, channel: ch };
  } catch (e) {
    sess.close();
    return { ok: false, error: String(e.message || e) };
  }
}

export default {
  compileWinusbHost,
  listPnpDevices,
  classifyPnpRow,
  WinusbSession,
  openWinusbChannel,
};
