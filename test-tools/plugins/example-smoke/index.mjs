import { writeReportWithEvidence } from "../../lib/plugin-session.mjs";
import fs from "node:fs";
import path from "node:path";
import { createClient } from "../../lib/wiparse-sdk.mjs";
import { normalizeResult, resolveReportName, truthy } from "../../lib/plugin-contract.mjs";
import { attachArtifacts, writeJson } from "../../lib/plugin-session.mjs";

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
  let health = null;
  if (wantApi) {
    ctx.log("info", "[example-smoke] api health...\n");
    try {
      health = await w.api.health();
      ctx.log("info", `${JSON.stringify(health)}\n`);
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

  const layout = ctx.layout;
  const prefix = resolveReportName(ctx.plugin, ctx.args);
  const stamp = layout?.stamp || ctx.stamp || "run";
  let summaryMd;
  if (layout?.artifacts_dir) {
    const logsDir = path.join(layout.artifacts_dir, "logs");
    const reportsDir = path.join(layout.artifacts_dir, "reports");
    fs.mkdirSync(logsDir, { recursive: true });
    fs.mkdirSync(reportsDir, { recursive: true });
    writeJson(path.join(logsDir, `smoke_${stamp}.json`), {
      ok: true,
      plugin: ctx.plugin.id,
      report_name: prefix,
      health,
    });
    summaryMd = path.join(reportsDir, `${prefix}_${stamp}.md`);
    writeReportWithEvidence(ctx.plugin, layout,
      summaryMd,
      [`# ${prefix} ${stamp}`, "", `- Plugin: \`${ctx.plugin.id}\``, `- API: ${wantApi ? "checked" : "skipped"}`, ""].join(
        "\n"
      )
    );
  }

  return normalizeResult(
    attachArtifacts(
      { ok: true, plugin: ctx.plugin.id, type: ctx.plugin.type, summary_md: summaryMd },
      ctx.plugin,
      layout
    ),
    "run"
  );
}
