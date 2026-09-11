/**
 * Scope & Serial Monitor — capture-loop plugin (id: scope-serial-monitor).
 * Lifecycles: preflight | run | stop
 */
import {
  loadStationConfig,
  normalizeResult,
  requestStop,
  truthy,
} from "../../lib/plugin-contract.mjs";
import { runLoop } from "./loop.mjs";

function logBanner(ctx, config) {
  ctx.log?.(
    "info",
    `[scope-serial] product=${config.station?.product} prefix=${config.paths?.file_prefix}\n` +
      `[scope-serial] isf_dir=${config.paths?.isf_dir}\n` +
      `[scope-serial] report_dir=${config.paths?.report_dir}\n` +
      `[scope-serial] port=${config.serial?.port}@${config.serial?.baud} api=${config.gui?.api}\n` +
      `[scope-serial] scope=${config.scope?.model || config.scope?.kind || "auto"} ${config.scope?.resource || ""}\n`
  );
}

export async function preflight(ctx) {
  const { config, configPath } = loadStationConfig(ctx);
  logBanner(ctx, config);
  const raw = await runLoop(config, {
    configPath,
    preflightOnly: true,
    log: ctx.log,
  });
  return normalizeResult(raw, "preflight");
}

export async function stop(ctx) {
  const { config } = loadStationConfig(ctx);
  const r = requestStop(config, { reason: "plugin stop()" });
  ctx.log?.("info", `[scope-serial] stop → ${r.stop_file || r.error}\n`);
  return normalizeResult(
    { ok: r.ok, step: "stop", artifacts: r.stop_file ? { stop_file: r.stop_file } : undefined, error: r.error },
    "stop"
  );
}

export default async function run(ctx) {
  const lifecycle = ctx.lifecycle || "run";
  if (lifecycle === "stop") return stop(ctx);
  if (lifecycle === "preflight" || ctx.preflightOnly || truthy(ctx.args?.preflight_only)) {
    return preflight(ctx);
  }

  const { config, configPath } = loadStationConfig(ctx);
  logBanner(ctx, config);
  const raw = await runLoop(config, {
    configPath,
    preflightOnly: false,
    log: ctx.log,
  });
  return normalizeResult(raw, "run");
}
