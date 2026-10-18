import { writeReportWithEvidence } from "../../lib/plugin-session.mjs";
/**
 * UART transparent bridge (WIPARSE-IF CH-UART).
 */
import fs from "node:fs";
import path from "node:path";
import {
  CHANNELS,
  IF_SPEC,
  asCommandList,
  bytesToHex,
  overlayChannelArgs,
  parseHexBytes,
} from "../../lib/wiparse-if.mjs";
import {
  discoverUsb,
  listSerialPorts,
  openUartChannel,
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
    tcp_port: num(args.tcp_port ?? config.channel?.tcp_port, CHANNELS.uart.tcp_port),
    vid: str(args.vid ?? config.channel?.vid, "C251"),
    pid: str(args.pid ?? config.channel?.pid, "F01A"),
    ...overlayChannelArgs(config, args),
    send_baud_cmd: args.send_baud_cmd ?? config.channel?.send_baud_cmd ?? true,
    connect_timeout_ms: num(args.connect_timeout_ms ?? config.channel?.connect_timeout_ms, 4000),
    port: str(args.port ?? config.serial?.port),
    baud: num(args.baud ?? config.serial?.baud, CHANNELS.uart.default_baud),
    data_bits: str(args.data_bits ?? config.serial?.data_bits, "8"),
    parity: str(args.parity ?? config.serial?.parity, "none"),
    stop_bits: str(args.stop_bits ?? config.serial?.stop_bits, "1"),
    dtr: args.dtr ?? config.serial?.dtr ?? true,
    rts: args.rts ?? config.serial?.rts ?? false,
    op: str(args.op ?? config.run?.op, "capture").toLowerCase(),
    duration_s: num(args.duration_s ?? config.run?.duration_s, 10),
    send_text: args.send_text ?? config.run?.send_text ?? "",
    send_hex: str(args.send_hex ?? config.run?.send_hex),
    expect_contains: str(args.expect_contains ?? config.run?.expect_contains),
    timestamp: truthy(args.timestamp ?? config.run?.timestamp ?? true),
    recipe: args.recipe ?? config.run?.recipe,
  };
}

async function discover(ctx, args) {
  const usb = await discoverUsb("uart", args);
  const serial = await listSerialPorts(ctx);
  const checks = [
    {
      id: "usb_uart",
      ok: usb.match.length > 0,
      detail: usb.match.map((d) => d.name || d.instance_id).join("; ") || "no matching USB UART interface",
    },
  ];
  let tcpOk = false;
  let tcpDetail = "";
  if (args.transport !== "usb" && args.transport !== "serial") {
    try {
      const ch = await openUartChannel({ ...args, transport: "tcp" });
      tcpOk = true;
      tcpDetail = `${ch.peer?.host}:${ch.peer?.port}`;
      ch.close();
    } catch (e) {
      tcpDetail = String(e.message || e);
    }
  }
  checks.push({ id: "tcp_uart", ok: tcpOk, detail: tcpDetail });
  const ports = serial.ports || [];
  const portNames = ports.map((p) => p.name || p.port || p.device || JSON.stringify(p));
  checks.push({
    id: "gui_serial",
    ok: Boolean(args.port) || ports.length > 0,
    detail: args.port || portNames.slice(0, 8).join(", ") || serial.error || "no COM list",
  });
  return { usb, serial, checks };
}

function payloadFrom(args) {
  if (args.send_hex) return parseHexBytes(args.send_hex);
  if (args.send_text) return Buffer.from(String(args.send_text), "utf8");
  return Buffer.alloc(0);
}

async function runGuiSerial(ctx, args, { logsDir, stamp, stopFile, hudFile, started }) {
  const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
  await w.api.invoke("serial.monitor.start", { port: args.port, baud: args.baud });
  const toSend = payloadFrom(args);
  if (toSend.length) {
    await w.api.invoke("serial.send", { hex: toSend.toString("hex") });
  }
  const jsonl = path.join(logsDir, `uart_${stamp}.jsonl`);
  const binPath = path.join(logsDir, `uart_${stamp}.bin`);
  const stream = fs.createWriteStream(jsonl, { flags: "a" });
  const chunks = [];
  const deadline = args.duration_s > 0 ? Date.now() + args.duration_s * 1000 : 0;
  try {
    while (!stopRequested(stopFile) && (deadline === 0 || Date.now() < deadline)) {
      const rd = await w.api.invoke("serial.read", { max_logs: 200 }, 15_000, { allowFail: true });
      const logs = rd?.data?.logs || rd?.logs || [];
      for (const row of logs) {
        const line = row.line || row.text || String(row);
        stream.write(`${JSON.stringify({ t: Date.now(), line })}\n`);
        chunks.push(Buffer.from(line, "utf8"));
      }
      writeHud(hudFile, {
        step: "capture",
        hint: `COM ${args.port}`,
        elapsed_s: Math.round((Date.now() - started) / 1000),
      });
      await new Promise((r) => setTimeout(r, 300));
    }
  } finally {
    stream.end();
    try {
      await w.api.invoke("serial.monitor.stop", {}, 10_000, { allowFail: true });
    } catch {
      /* ignore */
    }
  }
  const bin = Buffer.concat(chunks);
  fs.writeFileSync(binPath, bin);
  return { bytes: bin.length, jsonl, binPath, transport: "serial" };
}

async function runByteChannel(ctx, args, { logsDir, stamp, stopFile, hudFile, started }) {
  const ch = await openUartChannel(args);
  const jsonl = path.join(logsDir, `uart_${stamp}.jsonl`);
  const binPath = path.join(logsDir, `uart_${stamp}.bin`);
  const hexPath = path.join(logsDir, `uart_${stamp}.hex`);
  const stream = fs.createWriteStream(jsonl, { flags: "a" });
  const chunks = [];
  const toSend = payloadFrom(args);
  try {
    if (toSend.length) await ch.write(toSend);
    await ch.pump({
      durationMs: args.duration_s > 0 ? args.duration_s * 1000 : 0,
      stop: () => stopRequested(stopFile),
      onData: (c) => {
        chunks.push(c);
        const rec = {
          t: Date.now(),
          n: c.length,
          hex: bytesToHex(c.subarray(0, 128)),
        };
        if (args.timestamp) rec.text = c.toString("utf8");
        stream.write(`${JSON.stringify(rec)}\n`);
      },
      onTick: ({ elapsed_ms, bytes }) => {
        writeHud(hudFile, {
          step: "capture",
          hint: `${ch.transport} ${bytes} B`,
          elapsed_s: Math.round(elapsed_ms / 1000),
        });
      },
    });
  } finally {
    try {
      if (ch.cdcDtr) await ch.cdcDtr(false, false);
    } catch {
      /* ignore */
    }
    ch.close();
    stream.end();
  }
  const bin = Buffer.concat(chunks);
  fs.writeFileSync(binPath, bin);
  fs.writeFileSync(hexPath, bytesToHex(bin, ""));
  let expectOk = true;
  if (args.expect_contains) {
    expectOk = bin.toString("utf8").includes(args.expect_contains);
  }
  return {
    ok: expectOk,
    bytes: bin.length,
    jsonl,
    binPath,
    transport: ch.transport,
    error: expectOk ? undefined : `expect_contains missed: ${args.expect_contains}`,
  };
}

async function runRecipe(ctx, args, session) {
  const ch = await openUartChannel(args);
  const results = [];
  try {
    for (const item of asCommandList(args.recipe)) {
      if (stopRequested(session.stopFile)) break;
      const step = {
        send_text: item.send_text,
        send_hex: item.send_hex,
        wait_ms: item.wait_ms || 200,
        expect_contains: item.expect_contains || "",
      };
      const buf = item.send_hex
        ? parseHexBytes(item.send_hex)
        : Buffer.from(String(item.send_text || ""), "utf8");
      if (buf.length) await ch.write(buf);
      const chunk = await ch.read({ max: 8192, timeoutMs: Number(step.wait_ms) || 200 }).catch(() => Buffer.alloc(0));
      const text = chunk.toString("utf8");
      const ok = step.expect_contains ? text.includes(step.expect_contains) : true;
      results.push({ ...step, ok, got: text.slice(0, 400), hex: bytesToHex(chunk.subarray(0, 64)) });
      if (!ok) break;
    }
  } finally {
    try {
      if (ch.cdcDtr) await ch.cdcDtr(false, false);
    } catch {
      /* ignore */
    }
    ch.close();
  }
  writeJson(path.join(session.logsDir, `uart_recipe_${session.stamp}.json`), results);
  return { ok: results.every((r) => r.ok), results, transport: ch.transport };
}

export async function preflight(ctx) {
  const { config, args: merged } = loadStationConfig(ctx);
  const args = mergeArgs(config, merged);
  writeHud(config.paths?.status_file, { step: "preflight", hint: "CH-UART" });
  const d = await discover(ctx, args);
  const ok = d.checks.some((c) => c.ok);
  const suggested = {};
  if (args.port) suggested.port = args.port;
  return normalizeResult(
    {
      ok,
      step: "preflight",
      checks: d.checks,
      suggested_params: suggested,
      error: ok ? undefined : "no UART USB, TCP, or COM",
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
  writeHud(config.paths?.status_file, { step: args.op, hint: "CH-UART", elapsed_s: 0 });
  try {
    if (args.op === "discover") {
      const d = await discover(ctx, args);
      writeJson(path.join(logsDir, `uart_discover_${stamp}.json`), d);
      const md = config.paths.summary_md;
      writeReportWithEvidence(ctx.plugin, layout,
        md,
        `# UART discover ${stamp}\n\n${d.checks.map((c) => `- ${c.ok ? "OK" : "X"} ${c.id}: ${c.detail}`).join("\n")}\n`
      );
      return normalizeResult(
        attachArtifacts({ ok: d.checks.some((c) => c.ok), step: "discover", summary_md: md }, ctx.plugin, layout),
        "run"
      );
    }
    const session = {
      logsDir,
      stamp,
      stopFile: config.paths.stop_file,
      hudFile: config.paths.status_file,
      started,
    };
    let result;
    if (args.op === "recipe") result = await runRecipe(ctx, args, session);
    else if (args.op === "ping") {
      if (args.transport === "serial") {
        const w = createClient({ cliPath: ctx.cliPath, url: ctx.url, log: ctx.log });
        await w.api.invoke("serial.monitor.start", { port: args.port, baud: args.baud });
        await w.api.invoke("serial.monitor.stop", {}, 10_000, { allowFail: true });
        result = { ok: true, transport: "serial" };
      } else {
        const ch = await openUartChannel(args);
        try {
          if (ch.cdcDtr) await ch.cdcDtr(false, false);
        } finally {
          ch.close();
        }
        result = { ok: true, transport: ch.transport, peer: ch.peer };
      }
    } else if (args.transport === "serial") {
      result = await runGuiSerial(ctx, args, session);
      result.ok = result.ok !== false;
    } else {
      result = await runByteChannel(ctx, { ...args, duration_s: args.op === "send" ? Math.max(args.duration_s, 0.5) : args.duration_s }, session);
    }
    writeJson(path.join(logsDir, `uart_${stamp}.json`), { args, result });
    const md = config.paths.summary_md;
    writeReportWithEvidence(ctx.plugin, layout,
      md,
      [
        `# UART ${args.op} ${stamp}`,
        "",
        `- Transport: ${result.transport || args.transport} ${args.host}:${args.tcp_port}`,
        `- Baud: ${args.baud} ${args.data_bits}${args.parity[0] || "N"}${args.stop_bits} DTR=${args.dtr}`,
        `- Bytes: ${result.bytes ?? 0}`,
        result.error ? `- Error: ${result.error}` : "",
        "",
      ].join("\n")
    );
    return normalizeResult(
      attachArtifacts(
        { ok: result.ok !== false, step: args.op, error: result.error, summary_md: md },
        ctx.plugin,
        layout
      ),
      "run"
    );
  } catch (e) {
    return normalizeResult({ ok: false, step: args.op, error: String(e.message || e) }, "run");
  } finally {
    writeHud(config.paths?.status_file, {
      step: "idle",
      elapsed_s: Math.round((Date.now() - started) / 1000),
    });
    releaseLock(config.paths?.lock_file);
  }
}
