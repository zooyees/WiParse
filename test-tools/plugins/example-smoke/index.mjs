import { createClient } from "../../lib/wiparse-sdk.mjs";
import { normalizeResult, truthy } from "../../lib/plugin-contract.mjs";

async function checkCli(ctx) {
  const w = createClient({
    cliPath: ctx.cliPath,
    url: ctx.url,
    log: ctx.log,
  });
  ctx.log("info", "[example-smoke] checking CLI...\n");
  await w.cli(["version"]);
  return w;
}

export async function preflight(ctx) {
  const w = await checkCli(ctx);
  const checks = [{ id: "cli_version", ok: true }];
  if (truthy(ctx.args?.check_api ?? "true")) {
    try {
      const h = await w.api.health();
      ctx.log("info", `${JSON.stringify(h)}\n`);
      checks.push({ id: "api_health", ok: true });
    } catch (e) {
      checks.push({ id: "api_health", ok: false, detail: String(e.message || e) });
      return normalizeResult(
        { ok: false, step: "preflight", checks, error: String(e.message || e) },
        "preflight"
      );
    }
  }
  return normalizeResult({ ok: true, step: "preflight", checks }, "preflight");
}

export async function stop() {
  // Stateless smoke plugin — nothing to stop.
  return normalizeResult({ ok: true, step: "stop" }, "stop");
}

export default async function run(ctx) {
  if (ctx.lifecycle === "preflight" || ctx.preflightOnly) {
    return preflight(ctx);
  }
  if (ctx.lifecycle === "stop") {
    return stop(ctx);
  }

  const w = await checkCli(ctx);
  const wantApi = truthy(ctx.args?.check_api ?? "true");
  if (wantApi) {
    ctx.log("info", "[example-smoke] api health...\n");
    try {
      const h = await w.api.health();
      ctx.log("info", `${JSON.stringify(h)}\n`);
    } catch (e) {
      return normalizeResult(
        {
          ok: false,
          step: "api.health",
          error: String(e.message || e),
        },
        "run"
      );
    }
  }
  return normalizeResult(
    { ok: true, plugin: ctx.plugin.id, type: ctx.plugin.type },
    "run"
  );
}
