#!/usr/bin/env node
/**
 * Discover and run WiParse test-tool plugins (lifecycle: preflight | run | stop).
 *
 *   node runner.mjs --list
 *   node runner.mjs --plugin example-smoke --lifecycle preflight
 *   node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --port COM7
 *   node runner.mjs --plugin scope-serial-monitor --lifecycle stop
 */

import fs from "node:fs";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { createHttpClient, resolveCliPath } from "./lib/wiparse-sdk.mjs";
import {
  LIFECYCLES,
  checkEngines,
  loadStationConfig,
  mergeArgs,
  normalizeResult,
  requestStop,
  resolveConfigPath,
  resolveDataRoot,
  resolveLifecycle,
  truthy,
  validatePluginManifest,
} from "./lib/plugin-contract.mjs";
import {
  resolveMarketplaceRoot,
  resolveActivePluginDirs,
} from "./lib/marketplace-registry.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PLUGINS_ROOT = path.join(__dirname, "plugins");

function parseArgs(argv) {
  const out = {
    list: false,
    plugin: null,
    cli: null,
    url: null,
    type: null,
    dataRoot: null,
    lifecycle: null,
    json: false,
    skipValidate: false,
    marketplaceDir: null,
    pluginsRoot: null,
    passthrough: [],
  };
  let i = 0;
  while (i < argv.length) {
    const a = argv[i];
    if (a === "--") {
      out.passthrough = argv.slice(i + 1);
      break;
    }
    if (a === "--list") out.list = true;
    else if (a === "--json") out.json = true;
    else if (a === "--skip-validate") out.skipValidate = true;
    else if (a === "--plugin" && argv[i + 1]) {
      out.plugin = argv[++i];
    } else if (a === "--cli" && argv[i + 1]) {
      out.cli = argv[++i];
    } else if (a === "--url" && argv[i + 1]) {
      out.url = argv[++i];
    } else if (a === "--type" && argv[i + 1]) {
      out.type = argv[++i];
    } else if ((a === "--data-root" || a === "--data_root") && argv[i + 1]) {
      out.dataRoot = argv[++i];
    } else if ((a === "--lifecycle" || a === "--life") && argv[i + 1]) {
      out.lifecycle = argv[++i];
    } else if (
      (a === "--marketplace-dir" || a === "--marketplace_dir") &&
      argv[i + 1]
    ) {
      out.marketplaceDir = argv[++i];
    } else if ((a === "--plugins-root" || a === "--plugins_root") && argv[i + 1]) {
      out.pluginsRoot = argv[++i];
    } else if (a === "--help" || a === "-h") {
      out.help = true;
    } else {
      console.error(`unknown arg: ${a}`);
      process.exit(2);
    }
    i += 1;
  }
  return out;
}

function parsePassthrough(args) {
  const params = {};
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a.startsWith("--") && a.length > 2) {
      const key = a.slice(2);
      const next = args[i + 1];
      if (next != null && !next.startsWith("--")) {
        params[key] = next;
        i += 1;
      } else {
        params[key] = true;
      }
    }
  }
  return params;
}

function readManifest(dir, { validate = true } = {}) {
  const manifestPath = path.join(dir, "plugin.json");
  if (!fs.existsSync(manifestPath)) return null;
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch (e) {
    return {
      id: path.basename(dir),
      invalid: true,
      errors: [`JSON parse error: ${e.message}`],
      dir,
      manifestPath,
    };
  }
  if (validate) {
    const check = validatePluginManifest(raw);
    if (!check.ok) {
      return {
        id: String(raw.id || path.basename(dir)),
        invalid: true,
        errors: check.errors,
        dir,
        manifestPath,
      };
    }
  }
  const id = String(raw.id || path.basename(dir)).trim();
  if (!id) return null;
  return {
    id,
    name: String(raw.name || id),
    name_zh: raw.name_zh ? String(raw.name_zh) : undefined,
    type: String(raw.type || "custom"),
    version: String(raw.version || "0.0.0"),
    entry: String(raw.entry || "index.mjs"),
    config: raw.config ? String(raw.config) : undefined,
    description: raw.description ? String(raw.description) : "",
    capabilities: Array.isArray(raw.capabilities) ? raw.capabilities : ["run"],
    engines: raw.engines && typeof raw.engines === "object" ? raw.engines : undefined,
    params: Array.isArray(raw.params) ? raw.params : [],
    publisher: raw.publisher ? String(raw.publisher) : undefined,
    sandbox: raw.sandbox && typeof raw.sandbox === "object" ? raw.sandbox : undefined,
    source: "bundled",
    dir,
    manifestPath,
  };
}

export function discoverPlugins(root = PLUGINS_ROOT, { includeInvalid = false } = {}) {
  if (!fs.existsSync(root)) return [];
  const out = [];
  for (const name of fs.readdirSync(root)) {
    const dir = path.join(root, name);
    let st;
    try {
      st = fs.statSync(dir);
    } catch {
      continue;
    }
    if (!st.isDirectory()) continue;
    const m = readManifest(dir);
    if (!m) continue;
    if (m.invalid && !includeInvalid) {
      console.error(`[runner] skip invalid plugin ${dir}: ${m.errors?.join("; ")}`);
      continue;
    }
    out.push(m);
  }
  out.sort((a, b) => a.id.localeCompare(b.id));
  return out;
}

/**
 * Merge bundled plugins with marketplace active installs.
 * Marketplace active wins on id conflict (with warning).
 */
export function discoverPluginsMerged({
  pluginsRoot = PLUGINS_ROOT,
  marketplaceRoot = null,
  includeInvalid = false,
} = {}) {
  const bundled = discoverPlugins(pluginsRoot, { includeInvalid });
  const byId = new Map();
  for (const p of bundled) {
    byId.set(p.id, p);
  }
  if (marketplaceRoot) {
    for (const active of resolveActivePluginDirs(marketplaceRoot)) {
      const m = readManifest(active.dir);
      if (!m || m.invalid) {
        if (m?.invalid) {
          console.error(
            `[runner] skip invalid marketplace plugin ${active.dir}: ${m.errors?.join("; ")}`
          );
        }
        continue;
      }
      m.source = "marketplace";
      m.marketplace_version = active.version;
      if (byId.has(m.id) && byId.get(m.id).source !== "marketplace") {
        console.error(
          `[runner] warn: marketplace ${m.id}@${active.version} overrides bundled ${byId.get(m.id).dir}`
        );
      }
      byId.set(m.id, m);
    }
  }
  return [...byId.values()].sort((a, b) => a.id.localeCompare(b.id));
}

function printHelp() {
  console.log(`Usage:
  node runner.mjs --list [--type smoke] [--json]
  node runner.mjs --plugin <id> [--lifecycle preflight|run|stop]
                  [--cli path] [--url url] [--data-root dir]
                  [--marketplace-dir dir] [--plugins-root dir]
                  [-- --port COM3]

Lifecycle:
  preflight  Validate config / host / instruments (no long loop)
  run        Start plugin (default; --preflight_only true → preflight)
  stop       Request graceful stop via paths.stop_file

Plugins root: ${PLUGINS_ROOT}
Marketplace active installs override bundled ids when present.
See PLUGIN_SPEC.md
`);
}

async function probeWiparseVersion(url) {
  try {
    const http = createHttpClient({ url });
    const h = await http.health(5000);
    return h?.data?.version || h?.version || null;
  } catch {
    return null;
  }
}

async function dispatchLifecycle(mod, ctx, lifecycle) {
  if (lifecycle === "stop") {
    if (typeof mod.stop === "function") {
      return mod.stop(ctx);
    }
    // Default stop: write station stop_file when config exists
    if (ctx.configPath && fs.existsSync(ctx.configPath)) {
      const { config } = loadStationConfig(ctx);
      return requestStop(config, { reason: "runner --lifecycle stop" });
    }
    return {
      ok: false,
      error: "stop: plugin has no stop() and no station config",
    };
  }

  if (lifecycle === "preflight") {
    if (typeof mod.preflight === "function") {
      return mod.preflight({ ...ctx, lifecycle, preflightOnly: true });
    }
    const run = mod.default || mod.run;
    if (typeof run !== "function") {
      throw new Error("plugin entry must export default run(ctx) or preflight(ctx)");
    }
    return run({ ...ctx, lifecycle, preflightOnly: true });
  }

  // run
  const run = mod.default || mod.run;
  if (typeof run !== "function") {
    throw new Error("plugin entry must export default async function run(ctx)");
  }
  return run({ ...ctx, lifecycle, preflightOnly: false });
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const dataRootEarly = resolveDataRoot(opts.dataRoot);
  const pluginsRoot = opts.pluginsRoot
    ? path.resolve(opts.pluginsRoot)
    : PLUGINS_ROOT;
  const marketplaceRoot = resolveMarketplaceRoot({
    installDir:
      opts.marketplaceDir ||
      process.env.WIPARSE_MARKETPLACE_INSTALL_DIR ||
      "",
    dataRoot: dataRootEarly,
  });

  let plugins = discoverPluginsMerged({
    pluginsRoot,
    marketplaceRoot,
  });
  if (opts.type) {
    const t = opts.type.toLowerCase();
    plugins = plugins.filter((p) => !p.invalid && p.type.toLowerCase() === t);
  }

  if (opts.list) {
    if (opts.json) {
      console.log(JSON.stringify(plugins, null, 2));
    } else if (!plugins.length) {
      console.log("(no plugins)");
    } else {
      for (const p of plugins) {
        const caps = (p.capabilities || []).join(",");
        const src = p.source === "marketplace" ? "marketplace" : "bundled";
        console.log(
          `${p.id}\t${p.type}\tv${p.version}\t${p.name_zh || p.name}\t[${caps}]\t${src}`
        );
      }
    }
    return;
  }

  if (!opts.plugin) {
    printHelp();
    process.exit(2);
  }

  const plugin = plugins.find((p) => p.id === opts.plugin);
  if (!plugin) {
    const all = discoverPluginsMerged({
      pluginsRoot,
      marketplaceRoot,
      includeInvalid: true,
    });
    const hit = all.find((p) => p.id === opts.plugin);
    if (!hit) {
      console.error(`plugin not found: ${opts.plugin}`);
      process.exit(1);
    }
    if (hit.invalid) {
      console.error(`plugin invalid: ${hit.errors?.join("; ")}`);
      process.exit(1);
    }
    console.error(`plugin '${opts.plugin}' excluded by --type`);
    process.exit(1);
  }

  if (!opts.skipValidate) {
    const raw = JSON.parse(fs.readFileSync(plugin.manifestPath, "utf8"));
    const check = validatePluginManifest(raw);
    if (!check.ok) {
      console.error(`[runner] invalid plugin.json: ${check.errors.join("; ")}`);
      process.exit(2);
    }
  }

  const entryPath = path.join(plugin.dir, plugin.entry);
  if (!fs.existsSync(entryPath) && opts.lifecycle !== "stop") {
    console.error(`entry not found: ${entryPath}`);
    process.exit(1);
  }

  const cliPath = resolveCliPath(opts.cli);
  const args = mergeArgs(plugin.params, parsePassthrough(opts.passthrough));
  const dataRoot = resolveDataRoot(opts.dataRoot || args.data_root);
  const configPath = resolveConfigPath(plugin.dir, plugin);
  const url =
    opts.url ||
    (args.api ? String(args.api) : null) ||
    process.env.WIPARSE_URL ||
    "http://127.0.0.1:7878";

  let lifecycle = resolveLifecycle({
    lifecycle: opts.lifecycle,
    args,
  });
  if (opts.lifecycle && !LIFECYCLES.includes(opts.lifecycle)) {
    console.error(`invalid --lifecycle ${opts.lifecycle}; use ${LIFECYCLES.join("|")}`);
    process.exit(2);
  }
  if (opts.lifecycle) lifecycle = opts.lifecycle;

  // Capability gate
  const caps = plugin.capabilities?.length ? plugin.capabilities : ["run"];
  if (!caps.includes(lifecycle)) {
    console.error(
      `plugin '${plugin.id}' does not declare capability '${lifecycle}' (has: ${caps.join(",")})`
    );
    process.exit(2);
  }

  // Engines
  const wiparseVersion = await probeWiparseVersion(url);
  const eng = checkEngines(plugin.engines, { wiparseVersion });
  for (const w of eng.warnings) console.error(`[runner] warn: ${w}`);
  if (!eng.ok) {
    console.error(`[runner] engines check failed: ${eng.errors.join("; ")}`);
    process.exit(2);
  }

  const log = (stream, text) => {
    if (stream === "info") process.stdout.write(text);
    else if (stream === "stderr") process.stderr.write(text);
    else process.stdout.write(text);
  };

  const ctx = {
    plugin,
    args,
    cliPath,
    url,
    log,
    pluginsRoot,
    marketplaceRoot,
    pluginDir: plugin.dir,
    dataRoot,
    configPath: fs.existsSync(configPath) ? configPath : undefined,
    lifecycle,
    preflightOnly: lifecycle === "preflight" || truthy(args.preflight_only),
  };

  console.log(
    `[runner] plugin=${plugin.id} lifecycle=${lifecycle} type=${plugin.type} source=${plugin.source || "bundled"} data_root=${dataRoot}`
  );

  try {
    let mod = {};
    if (lifecycle !== "stop" || fs.existsSync(entryPath)) {
      mod = await import(pathToFileURL(entryPath).href);
    }
    const raw = await dispatchLifecycle(mod, ctx, lifecycle);
    const result = normalizeResult(raw, lifecycle);
    const bits = [
      result.ok === false ? "FAIL" : "ok",
      `lifecycle=${result.lifecycle || lifecycle}`,
    ];
    if (result.step) bits.push(`step=${result.step}`);
    if (result.error) bits.push(`error=${result.error}`);
    if (result.summary_md) bits.push(`md=${result.summary_md}`);
    console.log(`[runner] ${bits.join(" ")}`);
    if (Array.isArray(result.checks)) {
      for (const c of result.checks) {
        const mark = c?.ok ? "OK" : "X ";
        const detail = c?.detail ? `  ${c.detail}` : "";
        console.log(`  ${mark} ${c?.id || "check"}${detail}`);
      }
    }
    if (result.suggested_params && typeof result.suggested_params === "object") {
      console.log(
        `[runner] result ${JSON.stringify({
          type: "wiparse.plugin_result",
          suggested_params: result.suggested_params,
          suggested_params_policy: result.suggested_params_policy || "untouched",
        })}`
      );
    }
    if (result.ok === false) process.exit(1);
  } catch (e) {
    const result = normalizeResult(
      { ok: false, error: String(e?.message || e) },
      lifecycle
    );
    console.error(`[runner] failed: ${result.error}`);
    console.log(`[runner] FAIL lifecycle=${result.lifecycle} error=${result.error}`);
    process.exit(1);
  }
}

const isDirect =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isDirect) {
  main();
}
