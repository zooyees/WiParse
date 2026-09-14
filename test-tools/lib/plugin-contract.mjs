/**
 * Standard plugin contract: validate manifests, merge params, expand templates,
 * normalize results, and shared lifecycle helpers.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export const LIFECYCLES = Object.freeze(["preflight", "run", "stop"]);
export const PLUGIN_TYPES = Object.freeze([
  "smoke",
  "serial",
  "instrument",
  "capture_loop",
  "custom",
]);

/** Declared sandbox / isolation hooks for marketplace plugins. */
export const SANDBOX_PERMISSIONS = Object.freeze([
  "gui.api",
  "cli",
  "serial",
  "fs.data_root",
  "fs.plugin_dir",
  "network.outbound",
]);

export const MARKETPLACE_CHANNELS = Object.freeze(["stable", "beta", "internal"]);

export const PARAM_TYPES = Object.freeze([
  "string",
  "number",
  "boolean",
  "path",
  "device",
  "enum",
  "serial_port",
  "json",
  "text",
]);

export const OUTPUT_VIEWS = Object.freeze(["wave", "log", "report", "external"]);

const ARTIFACT_SKIP_KEYS = new Set([
  "stop_file",
  "lock_file",
  "status_file",
]);

const LAYOUT_PATH_KEYS = Object.freeze([
  "lock_file",
  "stop_file",
  "status_file",
  "isf_dir",
  "report_dir",
  "test_dir",
  "run_dir",
  "artifacts_dir",
  "summary_md",
  "shot_dir",
  "docs_dir",
]);

export function projectRoot() {
  return path.resolve(__dirname, "..", "..");
}

export function resolveDataRoot(explicit) {
  if (explicit && String(explicit).trim()) {
    return path.resolve(String(explicit).trim());
  }
  if (process.env.WIPARSE_DATA_ROOT) {
    return path.resolve(process.env.WIPARSE_DATA_ROOT);
  }
  return projectRoot();
}

export function truthy(v) {
  if (v === true || v === 1) return true;
  const s = String(v ?? "").trim().toLowerCase();
  return s === "true" || s === "1" || s === "yes" || s === "on";
}

export function setByPath(obj, dotted, value) {
  const parts = String(dotted).split(".").filter(Boolean);
  if (!parts.length) return obj;
  let cur = obj;
  for (let i = 0; i < parts.length - 1; i++) {
    const k = parts[i];
    if (cur[k] == null || typeof cur[k] !== "object") cur[k] = {};
    cur = cur[k];
  }
  cur[parts[parts.length - 1]] = value;
  return obj;
}

export function getByPath(obj, dotted) {
  const parts = String(dotted).split(".").filter(Boolean);
  let cur = obj;
  for (const k of parts) {
    if (cur == null) return undefined;
    cur = cur[k];
  }
  return cur;
}

function coerceParam(type, raw) {
  if (raw === undefined || raw === null) return raw;
  const t = String(type || "string").toLowerCase();
  if (t === "boolean") return truthy(raw);
  if (t === "number" || t === "device") {
    if (raw === "" || raw === null) return raw;
    const n = Number(raw);
    return Number.isFinite(n) ? n : raw;
  }
  if (t === "json") {
    if (typeof raw === "object") return raw;
    const s = String(raw).trim();
    if (!s) return raw;
    try {
      return JSON.parse(s);
    } catch {
      return raw;
    }
  }
  return String(raw);
}

export function mergeArgs(paramDefs, cliArgs = {}) {
  const out = { ...cliArgs };
  for (const p of paramDefs || []) {
    if (!p || !p.name) continue;
    if (out[p.name] === undefined && p.default !== undefined) {
      out[p.name] = p.default;
    }
    if (out[p.name] !== undefined) {
      out[p.name] = coerceParam(p.type, out[p.name]);
    }
  }
  return out;
}

export function applyParamPaths(config, paramDefs, args) {
  const cfg = structuredClone(config);
  for (const p of paramDefs || []) {
    if (!p?.name || !p.path) continue;
    if (args[p.name] === undefined || args[p.name] === null || args[p.name] === "") {
      continue;
    }
    setByPath(cfg, p.path, coerceParam(p.type, args[p.name]));
  }
  return cfg;
}

function expandString(s, vars) {
  if (typeof s !== "string") return s;
  let out = s.replace(/\{(\w+)\}/g, (m, key) => {
    if (!Object.prototype.hasOwnProperty.call(vars, key)) return m;
    const val = vars[key];
    if (val == null || val === "") return m;
    return String(val);
  });
  if (/^[A-Za-z]:[\\/]/.test(out) || out.startsWith("\\\\") || path.isAbsolute(out)) {
    out = path.normalize(out);
  }
  return out;
}

function walkExpand(node, vars) {
  if (typeof node === "string") return expandString(node, vars);
  if (Array.isArray(node)) return node.map((x) => walkExpand(x, vars));
  if (node && typeof node === "object") {
    const out = {};
    for (const [k, v] of Object.entries(node)) {
      out[k] = walkExpand(v, vars);
    }
    return out;
  }
  return node;
}

export function makeStamp(d = new Date()) {
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

export function sanitizeId(raw, fallback = "default") {
  const s = String(raw ?? "").trim();
  if (/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(s)) return s;
  return fallback;
}

/**
 * Canonical project layout. `stamp` / `run_dir` / `artifacts_dir` exist only for lifecycle=run.
 */
export function resolveProjectLayout({
  dataRoot,
  project,
  testId,
  stamp,
  lifecycle,
} = {}) {
  const root = dataRoot || resolveDataRoot();
  const projectId = sanitizeId(project || process.env.WIPARSE_PROJECT, "default");
  const test_id = String(testId || "").trim();
  const test_dir = path.join(root, "projects", projectId, "tests", test_id);
  const stampVal = lifecycle === "run" && stamp ? String(stamp) : "";
  const run_dir = stampVal ? path.join(test_dir, "runs", stampVal) : "";
  const artifacts_dir = run_dir ? path.join(run_dir, "artifacts") : "";
  return {
    project: projectId,
    project_id: projectId,
    test_id,
    plugin_id: test_id,
    stamp: stampVal,
    test_dir,
    run_dir,
    artifacts_dir,
  };
}

export function injectDefaultPathTemplates(config, { hasRunDir } = {}) {
  if (!config.paths || typeof config.paths !== "object") config.paths = {};
  const p = config.paths;
  const set = (k, v) => {
    if (p[k] == null || String(p[k]).trim() === "") p[k] = v;
  };
  set("test_dir", "{data_root}/projects/{project}/tests/{test_id}");
  if (hasRunDir) {
    set("run_dir", "{test_dir}/runs/{stamp}");
    set("artifacts_dir", "{run_dir}/artifacts");
  }
  set("lock_file", "{test_dir}/run.lock");
  set("stop_file", "{test_dir}/run.stop");
  set("status_file", "{test_dir}/status.json");
  return config;
}

export function ensureProjectScaffold(layout) {
  if (!layout?.test_dir) return null;
  const projectDir = path.resolve(layout.test_dir, "..", "..");
  fs.mkdirSync(layout.test_dir, { recursive: true });
  const pj = path.join(projectDir, "project.json");
  if (!fs.existsSync(pj)) {
    fs.mkdirSync(projectDir, { recursive: true });
    fs.writeFileSync(
      pj,
      `${JSON.stringify(
        {
          schema: "wiparse.project/v1",
          id: layout.project,
          name: layout.project,
          tests: [],
        },
        null,
        2
      )}\n`,
      "utf8"
    );
  }
  return pj;
}

function layoutTemplateVars(layout) {
  if (!layout) return {};
  const vars = {};
  for (const [k, v] of Object.entries(layout)) {
    if (v == null || v === "") continue;
    vars[k] = v;
  }
  return vars;
}

export function expandTemplates(config, { dataRoot, pluginDir, layout } = {}) {
  const product = config?.station?.product || "";
  const file_prefix = config?.paths?.file_prefix || "";
  const vars = {
    data_root: dataRoot || resolveDataRoot(),
    product,
    file_prefix,
    plugin_dir: pluginDir || "",
    ...layoutTemplateVars(layout),
  };
  let out = walkExpand(config, vars);
  const pathVars = { ...vars };
  if (out.paths && typeof out.paths === "object") {
    for (const [k, v] of Object.entries(out.paths)) {
      if (typeof v === "string" && v && !v.includes("{")) pathVars[k] = v;
    }
  }
  out = walkExpand(out, pathVars);
  return out;
}

/**
 * HUD lives in test_dir. Never overwrite an explicit status_file.
 * Legacy fallback: if neither status_file nor test_dir is set, park next to isf_dir.
 */
export function colocateStatusWithIsf(config) {
  if (!config?.paths || typeof config.paths !== "object") return config;
  const existing = config.paths.status_file;
  if (typeof existing === "string" && existing.trim()) return config;
  const testDir = config.paths.test_dir;
  if (typeof testDir === "string" && testDir.trim() && !testDir.includes("{")) {
    config.paths.status_file = path.join(testDir, "status.json");
    return config;
  }
  const isf = config.paths.isf_dir;
  if (typeof isf === "string" && isf.trim() && !isf.includes("{")) {
    config.paths.status_file = path.join(isf, "_loop_status.json");
  }
  return config;
}

export function loadOverlayFile(file) {
  if (!file) return {};
  const p = String(file).trim();
  if (!p || !fs.existsSync(p)) return {};
  const raw = JSON.parse(fs.readFileSync(p, "utf8"));
  if (raw && typeof raw === "object" && !Array.isArray(raw) && raw.params && typeof raw.params === "object") {
    return raw.params;
  }
  if (raw && typeof raw === "object" && !Array.isArray(raw)) return raw;
  return {};
}

export function isPathInside(child, parent) {
  if (!child || !parent) return false;
  const rel = path.relative(path.resolve(parent), path.resolve(child));
  return rel === "" || (!rel.startsWith("..") && !path.isAbsolute(rel));
}

function matchSimpleGlob(name, pattern) {
  const n = String(name || "");
  const g = String(pattern || "*");
  if (g === "*") return true;
  if (g.startsWith("*.") && !g.slice(2).includes("*")) {
    return n.toLowerCase().endsWith(g.slice(1).toLowerCase());
  }
  return n === g;
}

function viewForOutput(outputs, id) {
  const hit = (outputs || []).find((o) => o && o.id === id);
  return hit?.view || undefined;
}

/**
 * Normalize plugin artifacts (map or items[]) → `{ items: [{ output, path, view? }] }`.
 * Paths outside `runDir` are dropped. `stop_file` / lock / status are never artifacts.
 */
export function normalizeArtifacts(raw, { runDir, outputs } = {}) {
  const items = [];
  const push = (output, pathStr, view) => {
    if (!pathStr || ARTIFACT_SKIP_KEYS.has(String(output))) return;
    const abs = path.resolve(String(pathStr));
    if (runDir && !isPathInside(abs, runDir)) return;
    items.push({
      output: output ? String(output) : undefined,
      path: abs,
      view: view || viewForOutput(outputs, output),
    });
  };
  if (raw == null) return { items };
  if (Array.isArray(raw)) {
    for (const x of raw) {
      if (typeof x === "string") push(undefined, x);
      else if (x && typeof x === "object") push(x.output, x.path, x.view);
    }
    return { items };
  }
  if (typeof raw !== "object") return { items };
  if (Array.isArray(raw.items)) {
    return normalizeArtifacts(raw.items, { runDir, outputs });
  }
  for (const [output, val] of Object.entries(raw)) {
    const list = Array.isArray(val) ? val : [val];
    for (const x of list) {
      if (typeof x === "string") push(output, x);
      else if (x && typeof x === "object") push(output, x.path, x.view);
    }
  }
  return { items };
}

export function collectOutputFiles(outputs, vars = {}) {
  const map = {};
  for (const o of outputs || []) {
    if (!o?.id) continue;
    const dir = expandString(o.dir || `{artifacts_dir}/${o.id}`, vars);
    if (!dir || dir.includes("{") || !fs.existsSync(dir)) continue;
    const include = Array.isArray(o.include) && o.include.length ? o.include : ["*"];
    const files = [];
    for (const name of fs.readdirSync(dir)) {
      if (!include.some((g) => matchSimpleGlob(name, g))) continue;
      const abs = path.join(dir, name);
      try {
        if (fs.statSync(abs).isFile()) files.push(abs);
      } catch {
        /* skip */
      }
    }
    if (files.length) map[o.id] = files;
  }
  return map;
}

export function writeRunJson(layout, plugin, result) {
  if (!layout?.run_dir) return null;
  fs.mkdirSync(layout.run_dir, { recursive: true });
  const artifacts = normalizeArtifacts(result?.artifacts, {
    runDir: layout.run_dir,
    outputs: plugin?.outputs,
  });
  const doc = {
    schema: "wiparse.run/v1",
    plugin: plugin?.id,
    project: layout.project,
    stamp: layout.stamp,
    ok: result?.ok !== false,
    lifecycle: result?.lifecycle || "run",
    session: result?.session || layout.stamp,
    ended: new Date().toISOString(),
    artifacts,
  };
  const dest = path.join(layout.run_dir, "run.json");
  fs.writeFileSync(dest, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  return dest;
}

export function sanitizeFilePrefix(raw) {
  const s = String(raw ?? "").trim() || "report";
  const cleaned = s.replace(/[<>:"/\\|?*\x00-\x1f]/g, "_").replace(/[. ]+$/g, "");
  return cleaned || "report";
}

/**
 * Keep the summary Markdown beside report_dir so waveform screenshots
 * resolve next to the .md even when the Hub overrides report_dir
 * independently of the summary_md template.
 *
 * Filename always follows `paths.file_prefix` (Hub "报告名称/前缀"),
 * not a hardcoded ScopeSerial_summary basename in station.json.
 */
export function colocateSummaryWithReport(config) {
  if (!config?.paths || typeof config.paths !== "object") return config;
  const reportDir = config.paths.report_dir;
  if (typeof reportDir !== "string" || !reportDir.trim()) return config;
  const prefix = sanitizeFilePrefix(config.paths.file_prefix);
  config.paths.file_prefix = prefix;
  config.paths.summary_md = path.join(reportDir, `${prefix}_summary_{stamp}.md`);
  return config;
}

export function resolveConfigPath(pluginDir, manifest) {
  const name = manifest?.config || "station.json";
  const p = path.isAbsolute(name) ? name : path.join(pluginDir, name);
  return p;
}

/** Parse "1.2.3" → [1,2,3] */
export function parseSemver(v) {
  const m = String(v || "0")
    .trim()
    .replace(/^v/i, "")
    .match(/^(\d+)(?:\.(\d+))?(?:\.(\d+))?/);
  if (!m) return [0, 0, 0];
  return [parseInt(m[1], 10) || 0, parseInt(m[2], 10) || 0, parseInt(m[3], 10) || 0];
}

export function cmpSemver(a, b) {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  for (let i = 0; i < 3; i++) {
    if (pa[i] > pb[i]) return 1;
    if (pa[i] < pb[i]) return -1;
  }
  return 0;
}

/** Support ">=1.1.6" or "1.1.6" (treated as >=). */
export function satisfiesMinVersion(actual, requirement) {
  if (!requirement) return true;
  const req = String(requirement).trim();
  const m = req.match(/^>=?\s*(.+)$/) || req.match(/^(.+)$/);
  const min = (m && m[1] ? m[1] : req).trim();
  return cmpSemver(actual, min) >= 0;
}

export function nodeMajor() {
  const m = String(process.versions.node || "0").match(/^(\d+)/);
  return parseInt(m?.[1] || "0", 10);
}

/**
 * Lightweight plugin.json validation (aligned with schemas/plugin.schema.json).
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validatePluginManifest(raw) {
  const errors = [];
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { ok: false, errors: ["manifest must be an object"] };
  }
  if (!raw.id || typeof raw.id !== "string") errors.push("id: required string");
  else if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(raw.id)) {
    errors.push("id: must match [a-zA-Z0-9][a-zA-Z0-9._-]*");
  }
  if (!raw.name || typeof raw.name !== "string") errors.push("name: required string");
  if (!raw.type || typeof raw.type !== "string") errors.push("type: required string");
  else if (!PLUGIN_TYPES.includes(raw.type)) {
    errors.push(`type: must be one of ${PLUGIN_TYPES.join("|")}`);
  }
  if (raw.capabilities != null) {
    if (!Array.isArray(raw.capabilities)) errors.push("capabilities: must be array");
    else {
      for (const c of raw.capabilities) {
        if (!LIFECYCLES.includes(c)) {
          errors.push(`capabilities: unknown '${c}' (allowed: ${LIFECYCLES.join("|")})`);
        }
      }
    }
  }
  if (raw.engines != null) {
    if (typeof raw.engines !== "object" || Array.isArray(raw.engines)) {
      errors.push("engines: must be object");
    } else {
      for (const k of Object.keys(raw.engines)) {
        if (k !== "node" && k !== "wiparse") {
          errors.push(`engines: unknown key '${k}'`);
        }
      }
    }
  }
  if (raw.params != null) {
    if (!Array.isArray(raw.params)) errors.push("params: must be array");
    else {
      raw.params.forEach((p, i) => {
        if (!p || typeof p !== "object") {
          errors.push(`params[${i}]: must be object`);
          return;
        }
        if (!p.name || typeof p.name !== "string") {
          errors.push(`params[${i}].name: required`);
        }
        if (p.type != null && !PARAM_TYPES.includes(p.type)) {
          errors.push(`params[${i}].type: invalid`);
        }
      });
    }
  }
  if (raw.outputs != null) {
    if (!Array.isArray(raw.outputs)) errors.push("outputs: must be array");
    else {
      raw.outputs.forEach((o, i) => {
        if (!o || typeof o !== "object") {
          errors.push(`outputs[${i}]: must be object`);
          return;
        }
        if (!o.id || typeof o.id !== "string") {
          errors.push(`outputs[${i}].id: required`);
        } else if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(o.id)) {
          errors.push(`outputs[${i}].id: must match [a-zA-Z0-9][a-zA-Z0-9._-]*`);
        }
        if (o.view != null && !OUTPUT_VIEWS.includes(o.view)) {
          errors.push(`outputs[${i}].view: must be ${OUTPUT_VIEWS.join("|")}`);
        }
      });
    }
  }
  if (raw.marketplace != null) {
    if (typeof raw.marketplace !== "object" || Array.isArray(raw.marketplace)) {
      errors.push("marketplace: must be object");
    } else if (
      raw.marketplace.channel != null &&
      !MARKETPLACE_CHANNELS.includes(raw.marketplace.channel)
    ) {
      errors.push(`marketplace.channel: must be one of ${MARKETPLACE_CHANNELS.join("|")}`);
    }
  }
  if (raw.sandbox != null) {
    if (typeof raw.sandbox !== "object" || Array.isArray(raw.sandbox)) {
      errors.push("sandbox: must be object");
    } else if (raw.sandbox.permissions != null) {
      if (!Array.isArray(raw.sandbox.permissions)) {
        errors.push("sandbox.permissions: must be array");
      } else {
        for (const p of raw.sandbox.permissions) {
          if (!SANDBOX_PERMISSIONS.includes(p)) {
            errors.push(`sandbox.permissions: unknown '${p}'`);
          }
        }
      }
    }
  }
  return { ok: errors.length === 0, errors };
}

/**
 * Lightweight station.json validation (aligned with schemas/station.schema.json).
 */
export function validateStationConfig(raw) {
  const errors = [];
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { ok: false, errors: ["station must be an object"] };
  }
  if (!raw.gui || typeof raw.gui !== "object") errors.push("gui: required object");
  else if (!raw.gui.api || typeof raw.gui.api !== "string") {
    errors.push("gui.api: required string");
  }
  if (!raw.paths || typeof raw.paths !== "object") errors.push("paths: required object");
  else {
    for (const [key, val] of Object.entries(raw.paths)) {
      if (val != null && typeof val !== "string") {
        errors.push(`paths.${key}: must be string`);
      }
    }
  }
  return { ok: errors.length === 0, errors };
}

/**
 * Check engines.node / engines.wiparse against runtime.
 * @returns {{ ok: boolean, errors: string[], warnings: string[] }}
 */
export function checkEngines(engines, { wiparseVersion } = {}) {
  const errors = [];
  const warnings = [];
  if (!engines || typeof engines !== "object") {
    return { ok: true, errors, warnings };
  }
  if (engines.node) {
    const req = String(engines.node).trim();
    // Support ">=18" style for major-only
    const majorReq = req.match(/^>=?\s*(\d+)/);
    if (majorReq) {
      const need = parseInt(majorReq[1], 10);
      if (nodeMajor() < need) {
        errors.push(`engines.node: need ${req}, running ${process.versions.node}`);
      }
    } else if (!satisfiesMinVersion(process.versions.node, req)) {
      errors.push(`engines.node: need ${req}, running ${process.versions.node}`);
    }
  }
  if (engines.wiparse) {
    if (!wiparseVersion) {
      warnings.push(
        `engines.wiparse: ${engines.wiparse} (GUI version unknown; start WiParse.exe for a full check)`
      );
    } else if (!satisfiesMinVersion(wiparseVersion, engines.wiparse)) {
      errors.push(`engines.wiparse: need ${engines.wiparse}, got ${wiparseVersion}`);
    }
  }
  return { ok: errors.length === 0, errors, warnings };
}

/**
 * Normalize plugin return value to the standard result contract.
 * { ok, lifecycle, step?, checks?, artifacts?, error?, ... }
 */
export function normalizeResult(raw, lifecycle = "run") {
  const life = LIFECYCLES.includes(lifecycle) ? lifecycle : "run";
  if (raw == null) {
    return { ok: true, lifecycle: life };
  }
  if (typeof raw !== "object" || Array.isArray(raw)) {
    return { ok: false, lifecycle: life, error: String(raw) };
  }
  const out = {
    ok: raw.ok !== false,
    lifecycle: raw.lifecycle || life,
  };
  if (raw.step != null) out.step = String(raw.step);
  if (raw.error != null) out.error = String(raw.error);
  if (Array.isArray(raw.checks)) out.checks = raw.checks;
  if (raw.artifacts != null) {
    out.artifacts = normalizeArtifacts(raw.artifacts);
  }
  // Pass through useful extras without breaking contract
  for (const key of [
    "summary_md",
    "session",
    "preflight",
    "plugin",
    "type",
    "suggested_params",
    "suggested_params_policy",
    "scope",
  ]) {
    if (raw[key] !== undefined) out[key] = raw[key];
  }
  return out;
}

function normalizeKind(s) {
  const k = String(s || "")
    .trim()
    .toLowerCase()
    .replace(/-/g, "_");
  if (["scope", "osc", "oscilloscope"].includes(k)) return "oscilloscope";
  if (["psu", "dcsource", "dc_source", "source", "power"].includes(k)) return "dc_source";
  if (["load", "electronic_load", "eload"].includes(k)) return "electronic_load";
  if (["dmm", "multimeter", "meter"].includes(k)) return "multimeter";
  return k;
}

/**
 * Pick a connected instrument from `instrument.list` devices.
 * Match order: VISA resource → device_id (kind-safe) → kind+model → first of kind.
 * Hub stays instrument-agnostic; plugins pass `{ kind, resource, deviceId, model }`.
 */
export function pickInstrument(devices, spec = {}) {
  const list = Array.isArray(devices) ? devices : [];
  const kinds = String(spec.kind || "")
    .split(/[,|]/)
    .map((s) => normalizeKind(s))
    .filter(Boolean);
  const inKind = (d) => !kinds.length || kinds.includes(normalizeKind(d.kind));

  const resource = String(spec.resource || "").trim();
  if (resource) {
    const hit = list.find((d) => String(d.resource || "") === resource);
    if (hit) return hit;
  }

  const idRaw = spec.deviceId ?? spec.device_id ?? spec.prefer_device_id;
  const idNum = idRaw === "" || idRaw == null ? NaN : Number(idRaw);
  if (Number.isFinite(idNum)) {
    const hit = list.find((d) => Number(d.device_id) === idNum && inKind(d));
    if (hit) return hit;
  }

  const pool = list.filter(inKind);
  const model = String(spec.model || "").trim().toLowerCase();
  if (model) {
    const hit = pool.find(
      (d) => String(d.identity?.model || "").toLowerCase() === model
    );
    if (hit) return hit;
  }
  return pool[0] || null;
}

/**
 * Build Hub `suggested_params` from a live device.
 * `binds` maps plugin param name → device field
 * (`device_id` | `resource` | `kind` | `model` | `manufacturer` | `serial`).
 */
export function suggestedParamsFromInstrument(device, binds = {}) {
  if (!device || !binds || typeof binds !== "object") return {};
  const fields = {
    device_id: device.device_id,
    id: device.device_id,
    resource: device.resource,
    kind: device.kind,
    model: device.identity?.model,
    manufacturer: device.identity?.manufacturer,
    serial: device.identity?.serial,
  };
  const out = {};
  for (const [param, field] of Object.entries(binds)) {
    if (!param) continue;
    const key = String(field || "").replace(/^identity\./, "");
    const val = fields[key];
    if (val == null || String(val).trim() === "") continue;
    out[param] = String(val);
  }
  return out;
}

/**
 * Write station stop_file so a running loop exits gracefully.
 */
export function requestStop(config, { reason = "host stop" } = {}) {
  const stopFile = config?.paths?.stop_file;
  if (!stopFile) {
    return { ok: false, error: "paths.stop_file not configured" };
  }
  fs.mkdirSync(path.dirname(stopFile), { recursive: true });
  fs.writeFileSync(stopFile, `${reason}\n`, "utf8");
  return { ok: true, stop_file: stopFile, reason };
}

export function loadStationConfig(ctx) {
  const pluginDir = ctx.pluginDir || ctx.plugin?.dir;
  if (!pluginDir) throw new Error("loadStationConfig: missing pluginDir");
  const configPath =
    ctx.configPath || resolveConfigPath(pluginDir, ctx.plugin);
  if (!fs.existsSync(configPath)) {
    throw new Error(`station config not found: ${configPath}`);
  }
  const raw = JSON.parse(fs.readFileSync(configPath, "utf8"));
  const stationCheck = validateStationConfig(raw);
  if (!stationCheck.ok) {
    throw new Error(`invalid station.json: ${stationCheck.errors.join("; ")}`);
  }
  const args = mergeArgs(ctx.plugin?.params, ctx.args || {});
  let config = applyParamPaths(raw, ctx.plugin?.params, args);
  const dataRoot = resolveDataRoot(ctx.dataRoot || args.data_root);
  const layout =
    ctx.layout ||
    resolveProjectLayout({
      dataRoot,
      project: ctx.project || args.project,
      testId: ctx.plugin?.id,
      stamp: ctx.stamp,
      lifecycle: ctx.lifecycle || "run",
    });
  injectDefaultPathTemplates(config, { hasRunDir: Boolean(layout.run_dir) });
  config = expandTemplates(config, { dataRoot, pluginDir, layout });
  colocateStatusWithIsf(config);

  if (config.paths) {
    for (const key of LAYOUT_PATH_KEYS) {
      const v = config.paths[key];
      if (typeof v === "string" && v && !v.includes("{") && !path.isAbsolute(v)) {
        config.paths[key] = path.resolve(pluginDir, v);
      }
    }
    colocateSummaryWithReport(config);
  }

  return { config, args, configPath, dataRoot, layout };
}

/**
 * Resolve lifecycle from CLI flags.
 */
export function resolveLifecycle({ lifecycle, args } = {}) {
  if (lifecycle && LIFECYCLES.includes(lifecycle)) return lifecycle;
  if (truthy(args?.preflight_only)) return "preflight";
  return "run";
}

export default {
  LIFECYCLES,
  PLUGIN_TYPES,
  SANDBOX_PERMISSIONS,
  MARKETPLACE_CHANNELS,
  PARAM_TYPES,
  OUTPUT_VIEWS,
  projectRoot,
  resolveDataRoot,
  truthy,
  setByPath,
  getByPath,
  mergeArgs,
  applyParamPaths,
  makeStamp,
  sanitizeId,
  resolveProjectLayout,
  injectDefaultPathTemplates,
  ensureProjectScaffold,
  expandTemplates,
  colocateStatusWithIsf,
  colocateSummaryWithReport,
  loadOverlayFile,
  isPathInside,
  normalizeArtifacts,
  collectOutputFiles,
  writeRunJson,
  sanitizeFilePrefix,
  resolveConfigPath,
  parseSemver,
  cmpSemver,
  satisfiesMinVersion,
  nodeMajor,
  validatePluginManifest,
  validateStationConfig,
  checkEngines,
  normalizeResult,
  pickInstrument,
  suggestedParamsFromInstrument,
  requestStop,
  loadStationConfig,
  resolveLifecycle,
};
