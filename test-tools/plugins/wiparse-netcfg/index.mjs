/**
 * Dongle network config (WIPARSE-IF CH-NETCFG). USB MI_05 EP0 or TCP :1238.
 */
import fs from "node:fs";
import path from "node:path";
import {
  CHANNELS,
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

function trimStr(v, fallback = "") {
  if (v == null) return fallback;
  return String(v).trim();
}

function mergeArgs(config, args) {
  return {
    transport: str(args.transport ?? config.channel?.transport, "usb"),
    host: trimStr(args.host ?? config.channel?.host, ""),
    tcp_port: num(args.tcp_port ?? config.channel?.tcp_port, CHANNELS.netcfg.tcp_port),
    vid: str(args.vid ?? config.channel?.vid, "C251"),
    pid: str(args.pid ?? config.channel?.pid, "F01A"),
    ...overlayChannelArgs(config, args),
    fallback_host: trimStr(args.fallback_host ?? config.channel?.fallback_host, ""),
    usb_guid: str(args.usb_guid ?? config.channel?.usb_guid, CHANNELS.netcfg.guid),
    connect_timeout_ms: num(args.connect_timeout_ms ?? config.channel?.connect_timeout_ms, 4000),
    op: str(args.op ?? config.run?.op, "apply").toLowerCase(),
    ssid: trimStr(args.ssid ?? config.wifi?.ssid, ""),
    psk: String(args.psk ?? config.wifi?.psk ?? ""),
    dhcp: truthy(args.dhcp ?? config.wifi?.dhcp ?? true),
    ip: trimStr(args.ip ?? config.wifi?.ip, ""),
    gateway: trimStr(args.gateway ?? config.wifi?.gateway, ""),
    netmask: trimStr(args.netmask ?? config.wifi?.netmask, ""),
    hostname: trimStr(args.hostname ?? config.wifi?.hostname, ""),
    script: args.script ?? config.run?.script ?? "",
    wait_got_ip_ms: num(args.wait_got_ip_ms ?? config.run?.wait_got_ip_ms, 30000),
  };
}

function scriptLines(script) {
  return asCommandList(script);
}

function hasScript(script) {
  return scriptLines(script).length > 0;
}

function recipeFromArgs(args) {
  const script = scriptLines(args.script);
  if (script.length) return script;
  if (args.op === "status") return ["STATUS"];
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
    "apply"
  );
}

function recipeHasReconnect(items) {
  return (items || []).some((item) => {
    const line = typeof item === "string" ? item : String(item.cmd || item.command || "");
    return /^(APPLY|REVERT)\b/i.test(line.trim());
  });
}

function needsGotIpWait(args, items, results) {
  if (args.wait_got_ip_ms <= 0) return false;
  if (recipeHasReconnect(items)) return true;
  return (results || []).some((r) => /^(APPLY|REVERT)\b/i.test(String(r.line || "")));
}

function channelPeer(ch, args) {
  if (ch?.transport === "tcp") {
    const p = ch.peer;
    if (p?.host) return `${p.host}:${p.port || args.tcp_port}`;
    return `${args.host}:${args.tcp_port}`;
  }
  const name = ch?.peer?.name || args.usb_name || "WIPARSE NET";
  const mi = ch?.peer?.mi ?? args.usb_mi ?? 5;
  return `${name} MI_${String(mi).padStart(2, "0")}`;
}

async function discover(ctx, args) {
  const usb = await discoverUsb("netcfg", args);
  const checks = [
    {
      id: "usb_netcfg",
      ok: usb.match.some((d) => String(d.path || "").length > 0),
      detail:
        usb.match.map((d) => d.name || d.instance_id || d.path).join("; ") ||
        "no WIPARSE NET MI_05",
    },
  ];
  let tcpOk = false;
  let tcpDetail = "";
  const host = String(args.host || "").trim();
  const fallback = String(args.fallback_host || "").trim();
  if (args.transport === "usb") {
    tcpDetail = "skipped (USB-only)";
  } else if (!host && !fallback) {
    tcpDetail = "skipped (no Host)";
  } else {
    try {
      const ch = await openNetcfgChannel({ ...args, transport: "tcp" });
      tcpOk = true;
      tcpDetail = `${ch.peer?.host || host}:${args.tcp_port}`;
      ch.close();
    } catch (e) {
      tcpDetail = String(e.message || e);
    }
  }
  checks.push({ id: "tcp_netcfg", ok: tcpOk, detail: tcpDetail });
  return { usb, checks };
}

function renderReport({ stamp, args, results, transport, peer, status, usedScript }) {
  const rows = (results || [])
    .map((r) => {
      const mark = r.parsed?.ok ? "OK" : "ERR";
      return `| \`${redactNetcfgLine(r.line)}\` | ${mark} | \`${redactNetcfgLine(r.reply || "").replace(/\|/g, "\\|")}\` |`;
    })
    .join("\n");
  const st = status
    ? `- STATUS: wifi=${status.wifi ? 1 : 0} ip=${status.ip || "?"} ssid=${status.ssid || "?"} hostname=${status.hostname || "?"} source=${status.source || "?"}`
    : "";
  return [
    `# NETCFG ${stamp}`,
    "",
    `- Transport: ${transport} ${peer || ""}`.trimEnd(),
    `- Op: ${usedScript ? "script" : args.op}`,
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
  const usbOk = d.checks.some((c) => c.id === "usb_netcfg" && c.ok);
  const tcpOk = d.checks.some((c) => c.id === "tcp_netcfg" && c.ok);
  let ok = usbOk;
  if (args.transport === "tcp") ok = tcpOk;
  else if (args.transport === "auto") ok = usbOk || tcpOk;
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
  const usedScript = hasScript(args.script);
  const opLabel = usedScript ? "script" : args.op;
  writeHud(config.paths?.status_file, { step: opLabel, hint: "CH-NETCFG", elapsed_s: 0 });
  let ch;
  let stream;
  try {
    const items = recipeFromArgs(args);
    if (!items.length) throw new Error("no NETCFG commands");
    if (usedScript) ctx.log?.("info", "using advanced script (op ignored)\n");
    if (args.transport === "tcp" && recipeHasReconnect(items)) {
      ctx.log?.("stderr", "APPLY/REVERT over TCP may drop the socket; use USB while STA is down or about to reconnect.\n");
    }
    ch = await openNetcfgChannel(args);
    const jsonl = path.join(logsDir, `netcfg_${stamp}.jsonl`);
    stream = fs.createWriteStream(jsonl, { flags: "a" });
    const ran = await runNetcfgRecipe(ch, items, {
      timeoutMs: 8000,
      onLine: (line) => {
        if (stopRequested(config.paths.stop_file)) return;
        ctx.log?.("info", `> ${redactNetcfgLine(line)}\n`);
        writeHud(config.paths.status_file, {
          step: opLabel,
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
    if (needsGotIpWait(args, items, ran.results) && ran.ok !== false) {
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
    } else if (args.op === "status" && !usedScript) {
      const last = (ran.results || []).find((r) => /^STATUS\b/i.test(String(r.line || "")));
      if (last?.reply) status = parseNetcfgStatus(last.reply);
    }
    stream.end();
    const peer = channelPeer(ch, args);
    writeJson(path.join(logsDir, `netcfg_${stamp}.json`), {
      ok: ran.ok,
      transport: ch.transport,
      peer,
      used_script: usedScript,
      results: (ran.results || []).map((r) => ({
        line: redactNetcfgLine(r.line),
        reply: redactNetcfgLine(r.reply),
        ok: r.parsed?.ok,
      })),
      status: status
        ? {
            wifi: status.wifi,
            ip: status.ip,
            ssid: status.ssid,
            hostname: status.hostname,
            got_ip: status.got_ip,
            source: status.source,
          }
        : null,
    });
    const md = config.paths.summary_md;
    writeText(
      md,
      renderReport({
        stamp,
        args,
        results: ran.results,
        transport: ch.transport,
        peer,
        status,
        usedScript,
      })
    );
    const fail = (ran.results || []).find((r) => r.parsed?.ok === false);
    const waitFail = needsGotIpWait(args, items, ran.results) && status && !status.got_ip;
    return normalizeResult(
      attachArtifacts(
        {
          ok: !fail && !waitFail,
          step: opLabel,
          error: fail?.reply || (waitFail ? `wifi not up (${status.ip || "0.0.0.0"})` : undefined),
          summary_md: md,
        },
        ctx.plugin,
        layout
      ),
      "run"
    );
  } catch (e) {
    return normalizeResult({ ok: false, step: opLabel, error: String(e.message || e) }, "run");
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
