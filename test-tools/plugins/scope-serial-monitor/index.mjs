/**
 * Scope & Serial Monitor — capture-loop plugin (id: scope-serial-monitor).
 * Lifecycles: preflight | run | stop
 */
import {
  collectOutputFiles,
  loadStationConfig,
  normalizeResult,
  requestStop,
  truthy,
} from "../../lib/plugin-contract.mjs";
import { runLoop } from "./loop.mjs";

function hasTriggerOverlay(args) {
  const v = args?.triggers;
  if (v == null || v === "") return false;
  if (typeof v === "object") return true;
  return String(v).trim() !== "";
}

function logBanner(ctx, config, layout) {
  const items = config.serial_triggers?.items;
  const n = Array.isArray(items) ? items.filter((t) => t && t.enabled !== false).length : 0;
  ctx.log?.(
    "info",
    `[scope-serial] product=${config.station?.product} prefix=${config.paths?.file_prefix}\n` +
      `[scope-serial] project=${layout?.project || ctx.project || "default"} test_dir=${config.paths?.test_dir}\n` +
      `[scope-serial] isf_dir=${config.paths?.isf_dir}\n` +
      `[scope-serial] report_dir=${config.paths?.report_dir}\n` +
      `[scope-serial] status_file=${config.paths?.status_file}\n` +
      `[scope-serial] port=${config.serial?.port}@${config.serial?.baud} api=${config.gui?.api}\n` +
      `[scope-serial] scope=${config.scope?.model || config.scope?.kind || "auto"} ${config.scope?.resource || ""}\n` +
      `[scope-serial] triggers=${n} rising_edge=${config.serial_triggers?.rising_edge !== false}\n`
  );
}

function runOpts(ctx, configPath, preflightOnly, stamp) {
  return {
    configPath,
    preflightOnly,
    log: ctx.log,
    pinTriggers: hasTriggerOverlay(ctx.args),
    stamp,
  };
}

function attachArtifacts(raw, config, plugin, stamp) {
  const vars = {
    artifacts_dir: config.paths?.artifacts_dir || "",
    run_dir: config.paths?.run_dir || "",
  };
  const collected = collectOutputFiles(plugin?.outputs, vars);
  return {
    ...raw,
    session: raw?.session || stamp,
    artifacts: Object.keys(collected).length ? collected : raw?.artifacts,
  };
}

export async function preflight(ctx) {
  const { config, configPath, layout } = loadStationConfig(ctx);
  logBanner(ctx, config, layout);
  const raw = await runLoop(config, runOpts(ctx, configPath, true, ctx.stamp || layout.stamp));
  return normalizeResult(raw, "preflight");
}

export async function stop(ctx) {
  const { config } = loadStationConfig(ctx);
  const r = requestStop(config, { reason: "plugin stop()" });
  ctx.log?.("info", `[scope-serial] stop → ${r.stop_file || r.error}\n`);
  return normalizeResult({ ok: r.ok, step: "stop", error: r.error }, "stop");
}

export default async function run(ctx) {
  const lifecycle = ctx.lifecycle || "run";
  if (lifecycle === "stop") return stop(ctx);
  if (lifecycle === "preflight" || ctx.preflightOnly || truthy(ctx.args?.preflight_only)) {
    return preflight(ctx);
  }

  const { config, configPath, layout } = loadStationConfig(ctx);
  const stamp = ctx.stamp || layout.stamp;
  logBanner(ctx, config, layout);
  const raw = await runLoop(config, runOpts(ctx, configPath, false, stamp));
  return normalizeResult(attachArtifacts(raw, config, ctx.plugin, stamp), "run");
}
