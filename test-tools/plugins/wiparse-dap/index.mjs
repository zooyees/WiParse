import { writeReportWithEvidence } from "../../lib/plugin-session.mjs";
/**
 * CMSIS-DAP v2 channel plugin (WIPARSE-IF CH-DAP).
 */
import path from "node:path";
import {
  CHANNELS,
  IF_SPEC,
  asCommandList,
  bytesToHex,
  overlayChannelArgs,
} from "../../lib/wiparse-if.mjs";
import {
  discoverUsb,
  guiCommand,
  guiConnect,
  listGuiInstruments,
  pickProbe,
  probeDapUsbInfo,
  probeSuggested,
  probeUsbip,
  spawnTool,
} from "../../lib/wiparse-if-channel.mjs";
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
  writeText,
} from "../../lib/plugin-session.mjs";

function mergeArgs(config, args) {
  return {
    transport: str(args.transport ?? config.channel?.transport, "auto"),
    host: str(args.host ?? config.channel?.host, IF_SPEC.mdns),
    tcp_port: num(args.tcp_port ?? config.channel?.tcp_port, CHANNELS.dap.tcp_port),
    vid: str(args.vid ?? config.channel?.vid, "C251"),
    pid: str(args.pid ?? config.channel?.pid, "F01A"),
    ...overlayChannelArgs(config, args),
    connect_timeout_ms: num(args.connect_timeout_ms ?? config.channel?.connect_timeout_ms, 4000),
    prefer_device_id: str(args.prefer_device_id ?? config.probe?.prefer_device_id),
    probe_resource: str(args.probe_resource ?? config.probe?.resource),
    probe_model: str(args.probe_model ?? config.probe?.model),
    chip: str(args.chip ?? config.probe?.chip),
    speed_khz: num(args.speed_khz ?? config.probe?.speed_khz, 1000),
    op: str(args.op ?? config.run?.op, "session").toLowerCase(),
    address: str(args.address ?? config.run?.address, "08000000"),
    length: num(args.length ?? config.run?.length, 64),
    data_hex: str(args.data_hex ?? config.run?.data_hex),
    firmware: str(args.firmware ?? config.run?.firmware),
    verify: truthy(args.verify ?? config.run?.verify ?? true),
    hardware_reset: truthy(args.hardware_reset ?? config.run?.hardware_reset),
    rtt_channel: num(args.rtt_channel ?? config.run?.rtt_channel, 0),
    tool_cmd: str(args.tool_cmd ?? config.run?.tool_cmd, "pyocd list"),
    recipe: args.recipe ?? config.run?.recipe,
  };
}

async function discoverAll(ctx, args) {
  const checks = [];
  const usb = await discoverUsb("dap", args);
  checks.push({
    id: "usb_pnp",
    ok: usb.match.length > 0 || usb.devices.length > 0,
    detail: usb.match.length
      ? usb.match.map((d) => d.name || d.instance_id).join("; ")
      : usb.devices.length
        ? `${usb.devices.length} VID/PID hits`
        : usb.error || "no USB composite",
  });
  let usbip = null;
  if (args.transport !== "usb" && args.transport !== "gui") {
    try {
      usbip = await probeUsbip(args.host, args.tcp_port, args.connect_timeout_ms, args.fallback_host);
      checks.push({
        id: "tcp_usbip",
        ok: Boolean(usbip?.ok),
        detail: usbip.proto
          ? `${usbip.proto} ${usbip.peer?.host}:${usbip.peer?.port} devices=${usbip.devices?.length ?? "?"}`
          : "",
      });
    } catch (e) {
      checks.push({ id: "tcp_usbip", ok: false, detail: String(e.message || e) });
    }
  }
  let gui = { devices: [] };
  let probe = null;
  if (args.transport !== "usb" && args.transport !== "tcp") {
    gui = await listGuiInstruments(ctx, "debug_probe");
    probe = pickProbe(gui.devices, args);
    checks.push({
      id: "gui_probe",
      ok: Boolean(probe),
      detail: probe
        ? `${probe.identity?.model || probe.kind} ${probe.resource || ""} id=${probe.device_id}`
        : gui.error || "no debug_probe in instrument.list",
    });
  }
  return { checks, usb, usbip, gui, probe };
}

function probeCommand(op, args) {
  switch (op) {
    case "halt":
      return "ProbeHalt";
    case "run":
      return "ProbeRun";
    case "reset":
      return { ProbeReset: { hardware: Boolean(args.hardware_reset) } };
    case "status":
      return "ProbeStatus";
    case "regs":
      return "ProbeRegs";
    case "erase":
      return "ProbeErase";
    case "speed":
      return { ProbeSpeed: { khz: args.speed_khz } };
    case "mem-read":
      return { ProbeMemRead: { address: args.address, len: args.length } };
    case "mem-write":
      return { ProbeMemWrite: { address: args.address, data_hex: args.data_hex } };
    case "flash":
      return {
        ProbeFlash: {
          path: args.firmware,
          verify: args.verify,
        },
      };
    case "rtt-start":
      return { ProbeRttStart: { up_channel: args.rtt_channel } };
    case "rtt-read":
      return "ProbeRttRead";
    case "rtt-stop":
      return "ProbeRttStop";
    case "attach":
      return { ProbeAttach: { target: args.chip } };
    default:
      return null;
  }
}

async function ensureGuiProbe(ctx, args, discovered) {
  let probe = discovered?.probe || pickProbe(discovered?.gui?.devices || [], args);
  if (!probe && args.probe_resource) {
    const conn = await guiConnect(ctx, args.probe_resource);
    const again = await listGuiInstruments(ctx, "debug_probe");
    probe = pickProbe(again.devices, args);
    return { probe, connect: conn };
  }
  return { probe, connect: null };
}

async function execOp(ctx, args, op, discovered, logLines) {
  const note = (s) => {
    logLines.push(s);
    ctx.log?.("info", `${s}\n`);
  };
  if (op === "discover" || op === "session") {
    return { ok: true, discover: discovered };
  }
  if (op === "usbip-list") {
    const usbip = await probeUsbip(args.host, args.tcp_port, args.connect_timeout_ms);
    note(`usbip ${JSON.stringify(usbip.peer || {})} proto=${usbip.proto}`);
    return { ok: Boolean(usbip.ok), usbip };
  }
  if (op === "dap-info") {
    const info = await probeDapUsbInfo(args);
    note(`dap-info ${info.ok ? info.dap_info?.text || info.packet_size : info.error}`);
    return info;
  }
  if (op === "tool") {
    const r = await spawnTool(args.tool_cmd, { log: ctx.log, timeoutMs: 180_000 });
    note(`tool exit ${r.code}`);
    return { ok: r.code === 0, tool: r };
  }
  if (op === "connect") {
    const resource = args.probe_resource || discovered?.probe?.resource;
    if (!resource) throw new Error("set probe_resource or pick a GUI debug_probe");
    const conn = await guiConnect(ctx, resource);
    if (args.chip && conn?.data?.device_id) {
      await guiCommand(ctx, conn.data.device_id, { ProbeAttach: { target: args.chip } });
    }
    return { ok: conn?.ok !== false, connect: conn };
  }
  const cmd = probeCommand(op, args);
  if (!cmd) throw new Error(`unknown op ${op}`);
  const { probe } = await ensureGuiProbe(ctx, args, discovered);
  if (!probe) throw new Error("no GUI debug_probe; connect in Instrument Control or set probe_resource");
  if (args.chip && (op === "mem-read" || op === "mem-write" || op === "flash" || op === "halt")) {
    try {
      await guiCommand(ctx, probe.device_id, { ProbeAttach: { target: args.chip } });
    } catch {
      /* already attached */
    }
  }
  const reply = await guiCommand(ctx, probe.device_id, cmd);
  note(`${op} → ${JSON.stringify(reply?.data || reply).slice(0, 400)}`);
  return { ok: reply?.ok !== false, reply };
}

function renderReport({ stamp, args, discovered, results }) {
  const usbRows = (discovered?.usb?.devices || [])
    .map((d) => `- \`${d.instance_id || d.name}\` ${d.name || ""}`)
    .join("\n");
  const probe = discovered?.probe;
  return [
    `# CMSIS-DAP session ${stamp || ""}`,
    "",
    `- Spec: ${IF_SPEC.id} ${IF_SPEC.version}`,
    `- Transport: \`${args.transport}\` host=\`${args.host}:${args.tcp_port}\` VID/PID=${args.vid}/${args.pid}`,
    `- Chip: \`${args.chip || "(none)"}\` SWJ ${args.speed_khz} kHz`,
    `- GUI probe: ${probe ? `\`${probe.resource}\` id=${probe.device_id}` : "_none_"}`,
    `- USB/IP: ${discovered?.usbip?.proto || "n/a"} devices=${discovered?.usbip?.devices?.length ?? 0}`,
    "",
    "## USB PnP",
    usbRows || "_none_",
    "",
    "## Results",
    "```json",
    JSON.stringify(results, null, 2).slice(0, 8000),
    "```",
    "",
  ].join("\n");
}

export async function preflight(ctx) {
  const { config, args: merged, layout } = loadStationConfig(ctx);
  const args = mergeArgs(config, merged);
  writeHud(config.paths?.status_file, { step: "preflight", hint: "CH-DAP discover" });
  const discovered = await discoverAll(ctx, args);
  const suggested = discovered.probe ? probeSuggested(discovered.probe) : {};
  const ok = discovered.checks.some((c) => c.ok);
  return normalizeResult(
    {
      ok,
      step: "preflight",
      checks: discovered.checks,
      suggested_params: suggested,
      suggested_params_policy: "untouched",
      error: ok ? undefined : "no DAP USB, USB/IP, or GUI probe found",
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
  if (!lock.ok) {
    return normalizeResult({ ok: false, step: "lock", error: lock.error }, "run");
  }
  const logLines = [];
  const started = Date.now();
  writeHud(config.paths?.status_file, { step: "discover", hint: "CH-DAP", elapsed_s: 0 });
  try {
    const discovered = await discoverAll(ctx, args);
    const ops =
      args.op === "recipe"
        ? asCommandList(args.recipe).map((item) =>
            typeof item === "string" ? { op: item } : { ...item, op: item.op || item.cmd }
          )
        : args.op === "session"
          ? [{ op: "discover" }].concat(
              discovered.probe ? [{ op: "status" }] : discovered.usbip?.ok ? [{ op: "usbip-list" }] : []
            )
          : [{ op: args.op }];
    const results = [];
    for (const item of ops) {
      if (stopRequested(config.paths?.stop_file)) break;
      const op = String(item.op || "discover").toLowerCase();
      writeHud(config.paths?.status_file, {
        step: op,
        hint: `CH-DAP ${op}`,
        elapsed_s: Math.round((Date.now() - started) / 1000),
      });
      const one = await execOp(ctx, { ...args, ...item }, op, discovered, logLines);
      results.push({ op, ok: one.ok !== false, ...one });
      if (one.ok === false && args.op !== "session") {
        break;
      }
    }
    const logsDir = config.paths.logs_dir || path.join(layout.artifacts_dir, "logs");
    const dumpsDir = config.paths.dumps_dir || path.join(layout.artifacts_dir, "dumps");
    writeJson(path.join(logsDir, `dap_${stamp}.json`), { args, discovered, results });
    writeText(path.join(logsDir, `dap_${stamp}.jsonl`), logLines.map((l) => JSON.stringify({ t: Date.now(), l })).join("\n"));
    const md = config.paths.summary_md || path.join(config.paths.report_dir, `WiparseDap_${stamp}.md`);
    writeReportWithEvidence(ctx.plugin, layout,md, renderReport({ stamp, args, discovered, results }));
    const fail = results.find((r) => r.ok === false);
    const raw = attachArtifacts(
      {
        ok: !fail,
        step: fail ? fail.op : args.op,
        error: fail?.error || fail?.reply?.error,
        summary_md: md,
        suggested_params: discovered.probe ? probeSuggested(discovered.probe) : undefined,
        suggested_params_policy: "untouched",
      },
      ctx.plugin,
      layout
    );
    return normalizeResult(raw, "run");
  } catch (e) {
    return normalizeResult({ ok: false, step: args.op, error: String(e.message || e) }, "run");
  } finally {
    writeHud(config.paths?.status_file, {
      step: "idle",
      hint: "",
      elapsed_s: Math.round((Date.now() - started) / 1000),
    });
    releaseLock(config.paths?.lock_file);
  }
}
