/**
 * Dongle network config (WIPARSE-IF CH-NETCFG). USB MI_05 EP0 or TCP :1238.
 */
import fs from "node:fs";
import path from "node:path";
import {
  CHANNELS,
  IF_SPEC,
  asCommandList,
  buildNetcfgRecipe,
  overlayChannelArgs,
  parseNetcfgStatus,
  redactNetcfgLine,
} from "../../lib/wiparse-if.mjs";
import {
  discoverUsb,
  openNetcfgChannel,
  runNetcfgRecipe,
  waitNetcfgGotIp,
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
    transport: str(args.transport ?? config.channel?.transport, "usb"),
    host: str(args.host ?? config.channel?.host, IF_SPEC.mdns),
    tcp_port: num(args.tcp_port ?? config.channel?.tcp_port, CHANNELS.netcfg.tcp_port),
    vid: str(args.vid ?? config.channel?.vid, "C251"),
    pid: str(args.pid ?? config.channel?.pid, "F01A"),
    ...overlayChannelArgs(config, args),
    usb_guid: str(args.usb_guid ?? config.channel?.usb_guid, CHANNELS.netcfg.guid),
    connect_timeout_ms: num(args.connect_timeout_ms ?? config.channel?.connect_timeout_ms, 4000),
    op: str(args.op ?? config.run?.op, "apply").toLowerCase(),
    ssid: str(args.ssid ?? config.wifi?.ssid, "WINDLINK"),
    psk: String(args.psk ?? config.wifi?.psk ?? ""),
    dhcp: truthy(args.dhcp ?? config.wifi?.dhcp ?? false),
    ip: str(args.ip ?? config.wifi?.ip, "192.168.137.123"),
    gateway: str(args.gateway ?? config.wifi?.gateway, "192.168.137.1"),
    netmask: str(args.netmask ?? config.wifi?.netmask, "255.255.255.0"),
    hostname: str(args.hostname ?? config.wifi?.hostname, "dap"),
    script: args.script ?? config.run?.script ?? "",
    wait_got_ip_ms: num(args.wait_got_ip_ms ?? config.run?.wait_got_ip_ms, 15000),
  };
}

function recipeFromArgs(args) {
  if (args.op === "script") return asCommandList(args.script);
  return buildNetcfgRecipe(
    {
      ssid: args.ssid,
      psk: args.psk,
      dhcp: args.dhcp,
      ip: args.ip,
      gateway: args.gateway,
      netmask: args.netmask,
      hostname: args.hostname,
    },
    args.op
  );
}

function needsGotIpWait(args, results) {
  if (args.wait_got_ip_ms <= 0) return false;
  if (args.op === "apply" || args.op === "revert") return true;
  return (results || []).some((r) => /^(APPLY|REVERT)\b/i.test(String(r.line || "")));
}

async function discover(ctx, args) {
  const usb = await discoverUsb("netcfg", args);
  const checks = [
    {
      id: "usb_netcfg",
      ok: usb.match.some((d) => String(d.path || "").length > 0),
      detail:
        usb.match.map((d) => d.name || d.instance_id || d.path).join("; ") ||
        "no WIPARSE NET MI_05 (need bcdDevice 0x" +
        IF_SPEC.bcd_device.toString(16).padStart(4, "0") +
        ")",
    },
  ];
  let tcpOk = false;
  let tcpDetail = "";
  if (args.transport !== "usb") {
    try {
      const ch = await openNetcfgChannel({ ...args, transport: "tcp" });
      tcpOk = true;
      tcpDetail = `${ch.peer?.host || args.host}:${args.tcp_port}`;
      ch.close();
    } catch (e) {
      tcpDetail = String(e.message || e);
    }
  }
  checks.push({ id: "tcp_netcfg", ok: tcpOk, detail: tcpDetail || "skipped (USB-only)" });
  return { usb, checks };
}

function renderReport({ stamp, args, results, transport, status }) {
  const rows = (results || [])
    .map((r) => {
      const mark = r.parsed?.ok ? "OK" : "ERR";
      return `| \`${redactNetcfgLine(r.line)}\` | ${mark} | \`${redactNetcfgLine(r.reply || "").replace(/\|/g, "\\|")}\` |`;
    })
    .join("\n");
  const st = status
    ? `- STATUS: wifi=${status.wifi ? 1 : 0} ip=${status.ip || "?"} ssid=${status.ssid || "?"} source=${status.source || "?"}`
    : "";
  return [
    `# NETCFG ${stamp}`,
    "",
    `- Transport: ${transport} ${args.host}:${args.tcp_port}`,
    `- Op: ${args.op}`,
    st,
    "",
    "| Command | Result | Reply |",
    "|---------|--------|-------|",
    rows || "| _(none)_ | | |",
    "",
    "PSK is never written to this report.",
    "",
  ]
    .filter((l) => l !== "")
    .join("\n")
    .concat("\n");
}

export async function preflight(ctx) {
  const { config, args: merged } = loadStationConfig(ctx);
  const args = mergeArgs(config, merged);
  writeHud(config.paths?.status_file, { step: "preflight", hint: "CH-NETCFG" });
  const d = await discover(ctx, args);
  const ok = args.transport === "tcp" ? d.checks.some((c) => c.id === "tcp_netcfg" && c.ok) : d.checks.some((c) => c.ok);
  return normalizeResult(
    {
      ok,
      step: "preflight",
      checks: d.checks,
      error: ok ? undefined : "no NETCFG USB or TCP",
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
  writeHud(config.paths?.status_file, { step: args.op, hint: "CH-NETCFG", elapsed_s: 0 });
  let ch;
  let stream;
  try {
    if (args.op === "discover") {
      const d = await discover(ctx, args);
      writeJson(path.join(logsDir, `netcfg_discover_${stamp}.json`), d);
      const md = config.paths.summary_md;
      writeText(
        md,
        `# NETCFG discover ${stamp}\n\n${d.checks.map((c) => `- ${c.ok ? "OK" : "X"} ${c.id}: ${c.detail}`).join("\n")}\n`
      );
      return normalizeResult(
        attachArtifacts({ ok: d.checks.some((c) => c.ok), step: "discover", summary_md: md }, ctx.plugin, layout),
        "run"
      );
    }
    if (args.transport === "tcp" && (args.op === "apply" || args.op === "revert")) {
      ctx.log?.("stderr", "APPLY/REVERT over TCP may drop the socket; USB MI_05 is required while STA is down.\n");
    }
    ch = await openNetcfgChannel(args);
    const items = recipeFromArgs(args);
    if (!items.length) throw new Error("no NETCFG commands");
    const jsonl = path.join(logsDir, `netcfg_${stamp}.jsonl`);
    stream = fs.createWriteStream(jsonl, { flags: "a" });
    const ran = await runNetcfgRecipe(ch, items, {
      timeoutMs: 8000,
      onLine: (line) => {
        if (stopRequested(config.paths.stop_file)) return;
        ctx.log?.("info", `> ${redactNetcfgLine(line)}\n`);
        writeHud(config.paths.status_file, {
          step: args.op,
          hint: redactNetcfgLine(line).slice(0, 48),
          elapsed_s: Math.round((Date.now() - started) / 1000),
        });
      },
    });
    for (const r of ran.results || []) {
      stream.write(
        `${JSON.stringify({ line: redactNetcfgLine(r.line), reply: redactNetcfgLine(r.reply), ok: r.parsed?.ok })}\n`
      );
      ctx.log?.("info", `< ${redactNetcfgLine(r.reply)}\n`);
    }
    let status = null;
    if (needsGotIpWait(args, ran.results) && ran.ok !== false) {
      writeHud(config.paths.status_file, {
        step: "wait_ip",
        hint: "STATUS until wifi=1",
        elapsed_s: Math.round((Date.now() - started) / 1000),
      });
      status = await waitNetcfgGotIp(ch, {
        timeoutMs: args.wait_got_ip_ms,
        intervalMs: 500,
        stop: () => stopRequested(config.paths.stop_file),
        onTick: (st) => {
          writeHud(config.paths.status_file, {
            step: "wait_ip",
            hint: `wifi=${st.wifi ? 1 : 0} ip=${st.ip || "?"}`,
            elapsed_s: Math.round((Date.now() - started) / 1000),
          });
        },
      });
      if (status) {
        stream.write(`${JSON.stringify({ line: "STATUS", reply: redactNetcfgLine(status.raw), ok: status.ok })}\n`);
        ctx.log?.("info", `< ${redactNetcfgLine(status.raw)}\n`);
      }
    }
    stream.end();
    writeJson(path.join(logsDir, `netcfg_${stamp}.json`), {
      ok: ran.ok,
      transport: ch.transport,
      results: (ran.results || []).map((r) => ({
        line: redactNetcfgLine(r.line),
        reply: redactNetcfgLine(r.reply),
        ok: r.parsed?.ok,
      })),
      status: status
        ? { wifi: status.wifi, ip: status.ip, ssid: status.ssid, got_ip: status.got_ip, source: status.source }
        : null,
    });
    const md = config.paths.summary_md;
    writeText(md, renderReport({ stamp, args, results: ran.results, transport: ch.transport, status }));
    const fail = (ran.results || []).find((r) => r.parsed?.ok === false);
    const waitFail = needsGotIpWait(args, ran.results) && status && !status.got_ip;
    return normalizeResult(
      attachArtifacts(
        {
          ok: !fail && !waitFail,
          step: args.op,
          error: fail?.reply || (waitFail ? `wifi not up (${status.ip || "0.0.0.0"})` : undefined),
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
      stream?.end();
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
