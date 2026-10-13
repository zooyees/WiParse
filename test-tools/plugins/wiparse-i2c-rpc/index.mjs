import { writeReportWithEvidence } from "../../lib/plugin-session.mjs";
/**
 * I2C master text RPC (WIPARSE-IF CH-I2C).
 */
import fs from "node:fs";
import path from "node:path";
import {
  CHANNELS,
  IF_SPEC,
  asCommandList,
  formatI2cCommand,
  overlayChannelArgs,
  parseI2cReply,
} from "../../lib/wiparse-if.mjs";
import {
  configureI2cBus,
  discoverUsb,
  openI2cChannel,
  runI2cRecipe,
} from "../../lib/wiparse-if-channel.mjs";
import { createClient } from "../../lib/wiparse-sdk.mjs";
import {
  acquireLock,
  attachArtifacts,
  clearStop,
  ensureArtifactDirs,
  loadStationConfig,
  normalizeResult,
  num,
  releaseLock,
  sessionStop,
  stopRequested,
  str,
  truthy,
  writeHud,
  writeJson,
} from "../../lib/plugin-session.mjs";

function mergeArgs(config, args) {
  return {
    transport: str(args.transport ?? config.channel?.transport, "auto"),
    host: str(args.host ?? config.channel?.host, IF_SPEC.mdns),
    tcp_port: num(args.tcp_port ?? config.channel?.tcp_port, CHANNELS.i2c.tcp_port),
    vid: str(args.vid ?? config.channel?.vid, "C251"),
    pid: str(args.pid ?? config.channel?.pid, "F01A"),
    ...overlayChannelArgs(config, args),
    connect_timeout_ms: num(args.connect_timeout_ms ?? config.channel?.connect_timeout_ms, 4000),
    port: str(args.port ?? config.serial?.port),
    speed_hz: num(args.speed_hz ?? config.i2c?.speed_hz, CHANNELS.i2c.default_speed),
    timeout_ms: num(args.timeout_ms ?? config.i2c?.timeout_ms, CHANNELS.i2c.default_timeout_ms),
    apply_bus: truthy(args.apply_bus ?? config.i2c?.apply_bus ?? true),
    op: str(args.op ?? config.run?.op, "scan").toLowerCase(),
    addr: str(args.addr ?? config.run?.addr, "50"),
    length: num(args.length ?? config.run?.length, 2),
    data_hex: str(args.data_hex ?? config.run?.data_hex),
    recipe: args.recipe ?? config.run?.recipe,
    script: args.script ?? config.run?.script ?? "",
  };
}

function opToItem(args) {
  const op = args.op;
  if (op === "help") return { cmd: "HELP" };
  if (op === "scan") return { cmd: "SCAN" };
  if (op === "ping") return { cmd: "PING", addr: args.addr };
  if (op === "recover") return { cmd: "RECOVER" };
  if (op === "speed") return { cmd: "SPEED", hz: args.speed_hz };
  if (op === "timeout") return { cmd: "TIMEOUT", ms: args.timeout_ms };
  if (op === "write") return { cmd: "WRITE", addr: args.addr, data: args.data_hex };
  if (op === "read") return { cmd: "READ", addr: args.addr, len: args.length };
  if (op === "writeread") {
    return { cmd: "WRITEREAD", addr: args.addr, data: args.data_hex, len: args.length };
  }
  return null;
}

function recipeFromArgs(args) {
  if (args.op === "recipe") return asCommandList(args.recipe);
  if (args.op === "script") return asCommandList(args.script);
  const one = opToItem(args);
  return one ? [one] : [];
}

async function discover(ctx, args) {
  const usb = await discoverUsb("i2c", args);
  const checks = [
    {
      id: "usb_i2c",
      ok: usb.match.length > 0,
      detail: usb.match.map((d) => d.name || d.instance_id).join("; ") || "no matching USB I2C interface",
    },
  ];
  let tcpOk = false;
  let tcpDetail = "";
  if (args.transport !== "usb" && args.transport !== "serial") {
    try {
      const ch = await openI2cChannel({ ...args, transport: "tcp" });
      tcpOk = true;
      tcpDetail = `${ch.peer?.host}:${ch.peer?.port}`;
      ch.close();
    } catch (e) {
      tcpDetail = String(e.message || e);
    }
  }
  checks.push({ id: "tcp_i2c", ok: tcpOk, detail: tcpDetail });
  return { usb, checks };
}

class GuiSerialRpc {
  constructor(client) {
    this.client = client;
    this.transport = "serial";
  }
  async rpc(line) {
    const hex = Buffer.from(line.endsWith("\n") ? line : `${line}\n`, "utf8").toString("hex");
    await this.client.api.invoke("serial.send", { hex });
    await new Promise((r) => setTimeout(r, 80));
    const rd = await this.client.api.invoke("serial.read", { max_logs: 50 }, 15_000, { allowFail: true });
    const logs = rd?.data?.logs || rd?.logs || [];
    const lines = logs.map((row) => String(row.line || row.text || row).replace(/\r$/, ""));
    return { lines, text: lines.join("\n") };
  }
  close() {}
}

async function openRpc(ctx, args) {
  if (args.transport === "serial") {
    if (!args.port) throw new Error("serial transport needs port");
    const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
    await w.api.invoke("serial.monitor.start", { port: args.port, baud: 115200 });
    return new GuiSerialRpc(w);
  }
  return openI2cChannel(args);
}

function renderReport({ stamp, args, results, transport }) {
  const rows = (results || [])
    .map((r) => {
      const mark = r.parsed?.ok ? "OK" : "ERR";
      return `| \`${r.line}\` | ${mark} | \`${(r.reply || "").replace(/\|/g, "\\|")}\` |`;
    })
    .join("\n");
  return [
    `# I2C RPC ${stamp}`,
    "",
    `- Transport: ${transport} ${args.host}:${args.tcp_port}`,
    `- Bus: ${args.speed_hz} Hz timeout ${args.timeout_ms} ms`,
    "",
    "| Command | Result | Reply |",
    "|---------|--------|-------|",
    rows || "| _(none)_ | | |",
    "",
  ].join("\n");
}

export async function preflight(ctx) {
  const { config, args: merged } = loadStationConfig(ctx);
  const args = mergeArgs(config, merged);
  writeHud(config.paths?.status_file, { step: "preflight", hint: "CH-I2C" });
  const d = await discover(ctx, args);
  return normalizeResult(
    {
      ok: d.checks.some((c) => c.ok),
      step: "preflight",
      checks: d.checks,
      error: d.checks.some((c) => c.ok) ? undefined : "no I2C USB or TCP",
    },
    "preflight"
  );
}

export async function stop(ctx) {
  return sessionStop(ctx);
}

export default async function run(ctx) {
  if (ctx.lifecycle === "stop") return stop(ctx);
  if (ctx.lifecycle === "preflight" || ctx.preflightOnly) return preflight(ctx);

  const { config, args: merged, layout } = loadStationConfig(ctx);
  const args = mergeArgs(config, merged);
  const stamp = ctx.stamp || layout.stamp;
  ensureArtifactDirs(config);
  clearStop(config.paths?.stop_file);
  const lock = acquireLock(config.paths?.lock_file, { plugin: ctx.plugin.id, stamp });
  if (!lock.ok) return normalizeResult({ ok: false, step: "lock", error: lock.error }, "run");
  const started = Date.now();
  const logsDir = config.paths.logs_dir || path.join(layout.artifacts_dir, "logs");
  fs.mkdirSync(logsDir, { recursive: true });
  writeHud(config.paths?.status_file, { step: args.op, hint: "CH-I2C", elapsed_s: 0 });
  let ch;
  try {
    if (args.op === "discover") {
      const d = await discover(ctx, args);
      writeJson(path.join(logsDir, `i2c_discover_${stamp}.json`), d);
      const md = config.paths.summary_md;
      writeReportWithEvidence(ctx.plugin, layout,
        md,
        `# I2C discover ${stamp}\n\n${d.checks.map((c) => `- ${c.ok ? "OK" : "X"} ${c.id}: ${c.detail}`).join("\n")}\n`
      );
      return normalizeResult(
        attachArtifacts({ ok: d.checks.some((c) => c.ok), step: "discover", summary_md: md }, ctx.plugin, layout),
        "run"
      );
    }
    ch = await openRpc(ctx, args);
    if (args.apply_bus && args.op !== "speed" && args.op !== "timeout") {
      try {
        await configureI2cBus(ch, { speed_hz: args.speed_hz, timeout_ms: args.timeout_ms });
      } catch (e) {
        ctx.log?.("stderr", `bus config: ${e.message || e}\n`);
      }
    }
    const items = recipeFromArgs(args);
    if (!items.length) throw new Error("no I2C commands");
    const jsonl = path.join(logsDir, `i2c_${stamp}.jsonl`);
    const stream = fs.createWriteStream(jsonl, { flags: "a" });
    const ran = await runI2cRecipe(ch, items, {
      timeoutMs: Math.max(2000, args.timeout_ms * 20),
      onLine: (line) => {
        if (stopRequested(config.paths.stop_file)) return;
        ctx.log?.("info", `> ${line}\n`);
        writeHud(config.paths.status_file, {
          step: args.op,
          hint: line.slice(0, 48),
          elapsed_s: Math.round((Date.now() - started) / 1000),
        });
      },
    });
    for (const r of ran.results || []) {
      stream.write(`${JSON.stringify({ line: r.line, reply: r.reply, ok: r.parsed?.ok })}\n`);
      ctx.log?.("info", `< ${r.reply}\n`);
    }
    stream.end();
    writeJson(path.join(logsDir, `i2c_${stamp}.json`), ran);
    const md = config.paths.summary_md;
    writeReportWithEvidence(ctx.plugin, layout,md, renderReport({ stamp, args, results: ran.results, transport: ch.transport }));
    const fail = (ran.results || []).find((r) => r.parsed?.ok === false);
    return normalizeResult(
      attachArtifacts(
        {
          ok: !fail,
          step: args.op,
          error: fail?.reply,
          summary_md: md,
        },
        ctx.plugin,
        layout
      ),
      "run"
    );
  } catch (e) {
    return normalizeResult({ ok: false, step: args.op, error: String(e.message || e) }, "run");
  } finally {
    try {
      if (args.transport === "serial") {
        const w = createClient({ cliPath: ctx.cliPath, url: ctx.url });
        await w.api.invoke("serial.monitor.stop", {}, 8_000, { allowFail: true });
      }
    } catch {
      /* ignore */
    }
    try {
      ch?.close?.();
    } catch {
      /* ignore */
    }
    writeHud(config.paths?.status_file, {
      step: "idle",
      elapsed_s: Math.round((Date.now() - started) / 1000),
    });
    releaseLock(config.paths?.lock_file);
  }
}

export { formatI2cCommand, parseI2cReply };
