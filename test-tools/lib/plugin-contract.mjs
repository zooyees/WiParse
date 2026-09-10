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
  if (t === "number") {
    const n = Number(raw);
    return Number.isFinite(n) ? n : raw;
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
    if (key === "stamp") return m;
    if (Object.prototype.hasOwnProperty.call(vars, key) && vars[key] != null) {
      return String(vars[key]);
    }
    return m;
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

export function expandTemplates(config, { dataRoot, pluginDir } = {}) {
  const product = config?.station?.product || "";
  const vars = {
    data_root: dataRoot || resolveDataRoot(),
    product,
    plugin_dir: pluginDir || "",
  };
  return walkExpand(config, vars);
}

/**
 * Keep HUD/status next to waveform ISF dir so overrides of isf_dir remain operable.
 * Call after template expansion / param merge.
 */
export function colocateStatusWithIsf(config) {
  if (!config?.paths || typeof config.paths !== "object") return config;
  const isf = config.paths.isf_dir;
  if (typeof isf !== "string" || !isf.trim()) return config;
  config.paths.status_file = path.join(isf, "_loop_status.json");
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
        if (p.type != null && !["string", "number", "boolean", "path"].includes(p.type)) {
          errors.push(`params[${i}].type: invalid`);
        }
      });
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
    for (const key of ["file_prefix", "isf_dir", "report_dir"]) {
      if (!raw.paths[key] || typeof raw.paths[key] !== "string") {
        errors.push(`paths.${key}: required string`);
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
  if (raw.artifacts != null && typeof raw.artifacts === "object") {
    out.artifacts = raw.artifacts;
  }
  // Pass through useful extras without breaking contract
  for (const key of ["summary_md", "session", "preflight", "plugin", "type"]) {
    if (raw[key] !== undefined) out[key] = raw[key];
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
  config = expandTemplates(config, { dataRoot, pluginDir });
  colocateStatusWithIsf(config);

  if (config.paths) {
    for (const key of ["lock_file", "stop_file", "status_file", "isf_dir", "report_dir"]) {
      const v = config.paths[key];
      if (typeof v === "string" && v && !path.isAbsolute(v)) {
        config.paths[key] = path.resolve(pluginDir, v);
      }
    }
    if (typeof config.paths.summary_md === "string" && config.paths.summary_md) {
      const sm = config.paths.summary_md;
      if (!path.isAbsolute(sm) && !sm.includes("{stamp}")) {
        config.paths.summary_md = path.resolve(pluginDir, sm);
      } else if (!path.isAbsolute(sm) && sm.includes("{stamp}")) {
        const base = config.paths.report_dir || pluginDir;
        config.paths.summary_md = path.isAbsolute(sm)
          ? sm
          : path.join(base, path.basename(sm));
      }
    }
  }

  return { config, args, configPath, dataRoot };
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
  projectRoot,
  resolveDataRoot,
  truthy,
  setByPath,
  getByPath,
  mergeArgs,
  applyParamPaths,
  expandTemplates,
  colocateStatusWithIsf,
  resolveConfigPath,
  parseSemver,
  cmpSemver,
  satisfiesMinVersion,
  nodeMajor,
  validatePluginManifest,
  validateStationConfig,
  checkEngines,
  normalizeResult,
  requestStop,
  loadStationConfig,
  resolveLifecycle,
};
