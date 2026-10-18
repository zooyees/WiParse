#!/usr/bin/env node
import {validateProject} from './lib/hub/project-config.mjs';
/**
 * Discover and run WiParse test-tool plugins (lifecycle: preflight | run | stop).
 *
 *   node runner.mjs --list
 *   node runner.mjs --plugin example-smoke --lifecycle preflight
 *   node runner.mjs --plugin scope-serial-monitor --lifecycle run -- --port COM7
 *   node runner.mjs --plugin scope-serial-monitor --lifecycle stop
 */

import fs from "node:fs";
import {workspace} from "./lib/hub/manifest.mjs";
import {ConfigStore} from "./lib/hub/config-store.mjs";
import {retryWorkspaceLock} from "./lib/hub/retry.mjs";
import {manifest as freezeManifest,completePreflight,beginRun,endRun,assertStationIdle} from "./lib/hub/run-binding.mjs";
import {readJson,jail,fail as hubFail} from "./lib/hub/common.mjs";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { createHttpClient, resolveCliPath, resolveApiUrl } from "./lib/wiparse-sdk.mjs";
import {
  LIFECYCLES,
  checkEngines,
  collectOutputFiles,
  ensureProjectScaffold,
  ensureTestInProject,
  loadOverlayFile,
  loadStationConfig,
  applyParamPaths,
  expandTemplates,
  makeStamp,
  mergeArgs,
  normalizeArtifacts,
  normalizeResult,
  requestStop,
  resolveConfigPath,
  resolveDataRoot,
  resolveLifecycle,
  resolveProjectLayout,
  sanitizeId,
  sanitizeTestId,
  resolveReportName,
  truthy,
  validatePluginManifest,
  writeRunJson,
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
    overlay: null,
    project: null,
    stamp: null,
    passthrough: [],
    approvedRoots: [],
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
    } else if (a === "--approved-config-root" && argv[i+1]) {
      if(out.approvedRoots.length>=16) throw new Error("CONFIG.ROOT_LIMIT");out.approvedRoots.push(argv[++i]);
    } else if (a === "--overlay" && argv[i + 1]) {
      out.overlay = argv[++i];
    } else if (a === "--project" && argv[i + 1]) {
      out.project = argv[++i];
    } else if (a === "--stamp" && argv[i + 1]) {
      out.stamp = argv[++i];
    } else if ((a === "--test-id" || a === "--test_id") && argv[i + 1]) {
      out.testId = argv[++i];
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
    report_name: raw.report_name ? String(raw.report_name) : undefined,
    report_name_zh: raw.report_name_zh ? String(raw.report_name_zh) : undefined,
    outputs: Array.isArray(raw.outputs) ? raw.outputs : [],
    publisher: raw.publisher ? String(raw.publisher) : undefined,
    sandbox: raw.sandbox && typeof raw.sandbox === "object" ? raw.sandbox : undefined,
    ui: raw.ui,
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
                  [--project id] [--test-id id] [--stamp yyyyMMdd_HHmmss]
                  [--overlay file.json]
                  [--marketplace-dir dir] [--plugins-root dir]
                  [-- --port COM3]

Lifecycle:
  preflight  Validate config / host / instruments (no stamp, no runs/)
  run        Start plugin (default; --preflight_only true → preflight)
  stop       Request graceful stop via paths.stop_file (no new run)

Writes (run only): {data_root}/projects/{project}/tests/{plugin}/runs/{stamp}/run.json
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
  const overlay = loadOverlayFile(opts.overlay);
  const args = mergeArgs(plugin.params, {
    ...parsePassthrough(opts.passthrough),
    ...overlay,
  });
  const dataRoot = resolveDataRoot(opts.dataRoot || args.data_root);
  const configPath = resolveConfigPath(plugin.dir, plugin);
  const url = resolveApiUrl(opts.url || (args.api ? String(args.api) : null));

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

  // configure is deliberately isolated from engine probing and project/run creation.
  if (lifecycle === "configure") {
    const {spawnSync} = await import('node:child_process');
    const request=opts.overlay ? readJson(opts.overlay,2*1048576) : {};
    const checked=spawnSync(process.execPath,[path.join(__dirname,'lib/hub/configure-worker.mjs')],{input:JSON.stringify({pluginDir:plugin.dir,entry:plugin.entry,values:[request.config ?? request.params?.config ?? {}]}),encoding:'utf8',timeout:10000,maxBuffer:4194304,windowsHide:true});
    let value; try{value=JSON.parse(checked.stdout);}catch{value={ok:false,errors:[{code:'CONFIG.CONFIGURE_FAILED'}]};}
    console.log(JSON.stringify({...value,lifecycle:'configure'}));if(checked.error||checked.status!==0||value.ok!==true)process.exitCode=1;
    return;
  }
  // Engines
  const wiparseVersion = await probeWiparseVersion(url);
  const eng = checkEngines(plugin.engines, { wiparseVersion });
  for (const w of eng.warnings) console.error(`[runner] warn: ${w}`);
  if (!eng.ok) {
    console.error(`[runner] engines check failed: ${eng.errors.join("; ")}`);
    process.exit(2);
  }

  const project = sanitizeId(opts.project || process.env.WIPARSE_PROJECT, "default");
  const stamp =
    lifecycle === "run"
      ? String(opts.stamp || process.env.WIPARSE_STAMP || makeStamp())
      : "";
  const reportName = resolveReportName(plugin, args);
  const testId = sanitizeTestId(
    opts.testId || process.env.WIPARSE_TEST_ID || reportName,
    plugin.id
  );
  const layout = resolveProjectLayout({
    dataRoot,
    project,
    testId,
    pluginId: plugin.id,
    stamp,
    lifecycle,
  });
  let hubStore=null, hubFrozen=null, hubBinding=null;
  let lockWait={};
  if(plugin.capabilities?.includes('configure') && lifecycle!=='stop') {
    hubStore=new ConfigStore(workspace(plugin,{pluginDir:plugin.dir,dataRoot,project,testId,params:args,approvedRoots:opts.approvedRoots,protectedRoots:[pluginsRoot,marketplaceRoot].filter(Boolean)}));
    let stopFile=path.join(layout.test_dir,'run.stop');
    if(fs.existsSync(configPath)) {
      const station=expandTemplates(applyParamPaths(readJson(configPath),plugin.params,args),{dataRoot,pluginDir:plugin.dir,layout});
      if(typeof station.paths?.stop_file==='string') stopFile=path.resolve(plugin.dir,station.paths.stop_file);
    }
    stopFile=jail(stopFile,hubStore.s.roots);
    // Preflight may publish validation metadata under a retained Stop, but may
    // not acknowledge/remove it. Only an explicit new start clears old Stop.
    lockWait={stopped:()=>lifecycle==='run'&&fs.existsSync(stopFile),onWait:()=>console.log('[runner] CONFIG.BUSY_WAIT: waiting briefly for the configuration repository')};
    const selection=opts.overlay ? readJson(opts.overlay,2*1048576).test_matrix : null;
    if(lifecycle==='preflight') hubFrozen=await retryWorkspaceLock(()=>hubStore.withLock(()=>{assertStationIdle(hubStore);fs.rmSync(path.join(hubStore.s.editorRoot,'preflight.json'),{force:true});const frozen=freezeManifest(hubStore,selection?.selected_item_ids);if(frozen.project_config_snapshot)validateProject(hubStore,frozen.project_config_snapshot.value);hubStore.validate(frozen.values.filter(v=>frozen.selected_item_ids.includes(v.id)));return frozen;}),lockWait);
    else {
      if(!selection?.config_manifest_sha256||!selection?.preflight_revision) throw new Error('PREFLIGHT.REQUIRED: run overlay must bind preflight');
      hubBinding=await retryWorkspaceLock(()=>beginRun(hubStore,selection.selected_item_ids,layout.run_dir,selection),lockWait);
      hubFrozen=hubBinding;
    }
  }
  try {
    if (lifecycle === "run") {
      ensureTestInProject(layout, plugin.id, {
        name: reportName,
      });
    } else {
      ensureProjectScaffold(layout);
    }
    if (lifecycle === "run" && layout.artifacts_dir) {
      fs.mkdirSync(layout.artifacts_dir, { recursive: true });
    }
  } catch (e) {
    console.error(`[runner] layout: ${e.message || e}`);
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
    project,
    stamp,
    layout,
    testMatrix: hubFrozen,
    projectConfig: hubFrozen?.project_config_snapshot,
    matrixFile: hubStore?.s.matrix,
    overlay: {params:args,test_matrix:hubFrozen?{config_manifest_sha256:hubFrozen.manifest_sha256,preflight_revision:hubBinding?.preflight_revision,selected_item_ids:hubFrozen.selected_item_ids}:undefined},
    configSnapshotDir: hubBinding?.snapshotDir,
  };

  console.log(
    `[runner] plugin=${plugin.id} lifecycle=${lifecycle} type=${plugin.type} source=${plugin.source || "bundled"} data_root=${dataRoot} project=${project} test_id=${testId}${stamp ? ` stamp=${stamp}` : ""}`
  );

  try {
    let mod = {};
    if (lifecycle !== "stop" || fs.existsSync(entryPath)) {
      mod = await import(pathToFileURL(entryPath).href);
    }
    if(hubStore&&lifecycle==='run'&&lockWait.stopped())hubFail('RUN.ABORTED','Stopped before plugin execution');
    const raw = await dispatchLifecycle(mod, ctx, lifecycle);
    const result = normalizeResult(raw, lifecycle);
    if(hubStore && lifecycle==='preflight'&&result.ok!==false) await retryWorkspaceLock(()=>completePreflight(hubStore,hubFrozen,result),lockWait);
    if(hubBinding) result.test_matrix={...result.test_matrix,manifest_sha256:hubBinding.manifest_sha256,preflight_revision:hubBinding.preflight_revision,schema_hash:hubStore.schemaHash,selected_item_ids:hubBinding.selected_item_ids,...(hubBinding.project_config?{project_config:hubBinding.project_config}:{})};
    if (lifecycle === "run" && layout.run_dir) {
      const collected =
        plugin.outputs && (!result.artifacts || !result.artifacts.items?.length)
          ? collectOutputFiles(plugin.outputs, {
              artifacts_dir: layout.artifacts_dir,
              run_dir: layout.run_dir,
            })
          : null;
      result.artifacts = normalizeArtifacts(collected || result.artifacts, {
        runDir: layout.run_dir,
        outputs: plugin.outputs,
      });
      if (!result.session) result.session = layout.stamp;
      const runJson = writeRunJson(layout, plugin, result);
      if (runJson) result.run_json = runJson;
    }
    const bits = [
      result.ok === false ? "FAIL" : "ok",
      `lifecycle=${result.lifecycle || lifecycle}`,
    ];
    if (result.step) bits.push(`step=${result.step}`);
    if (result.execution_status) bits.push(`execution=${result.execution_status}`);
    if (result.product_verdict) bits.push(`product=${result.product_verdict}`);
    if (result.validation_verdict) bits.push(`validation=${result.validation_verdict}`);
    if (result.error) bits.push(`error=${result.error}`);
    if (result.summary_md) bits.push(`md=${result.summary_md}`);
    if (result.run_json) bits.push(`run_json=${result.run_json}`);
    console.log(`[runner] ${bits.join(" ")}`);
    if (Array.isArray(result.checks)) {
      for (const c of result.checks) {
        const mark = c?.ok ? "OK" : "X ";
        const explanation=c?.detail||c?.message||'';
        const detail = explanation ? `  ${explanation}` : "";
        console.log(`  ${mark} ${c?.id || c?.code || "check"}${detail}`);
      }
    }
    const envelope = {
      type: "wiparse.plugin_result",
    };
    if (result.suggested_params && typeof result.suggested_params === "object") {
      envelope.suggested_params = result.suggested_params;
      envelope.suggested_params_policy = result.suggested_params_policy || "untouched";
    }
    if (result.test_matrix) envelope.test_matrix = result.test_matrix;
    if (result.session) envelope.session = result.session;
    if (result.artifacts) envelope.artifacts = result.artifacts;
    if (envelope.test_matrix || envelope.suggested_params || envelope.artifacts || envelope.session) {
      console.log(`[runner] result ${JSON.stringify(envelope)}`);
    }
    // Let pending HTTP handles and stdout drain; forced exit can abort libuv on Windows.
    if (result.ok === false) process.exitCode = 1;
  } catch (e) {
    const result = normalizeResult(
      { ok: false, error: String(e?.message || e) },
      lifecycle
    );
    console.error(`[runner] failed: ${result.error}`);
    console.log(`[runner] FAIL lifecycle=${result.lifecycle} error=${result.error}`);
    process.exitCode = 1;
  } finally { if(hubBinding) endRun(hubStore,hubBinding); }
}

const isDirect =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isDirect) {
  main().catch(e=>{console.error(`[runner] failed: ${e.message}`);process.exitCode=1;});
}
