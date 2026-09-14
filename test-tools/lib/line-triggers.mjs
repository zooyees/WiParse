/**
 * Generic line-match triggers for Testing Hub plugins.
 *
 * Hub stays instrument-agnostic: plugins pass a trigger *block* (JSON object /
 * items array / optional external file). Match types are regex | contains.
 * Rising-edge latch is per trigger id (inactive → active).
 */

import fs from "node:fs";
import path from "node:path";

export function sanitizeTriggerId(id) {
  const s = String(id || "").trim();
  if (!s) throw new Error("trigger id 不能为空");
  return s.replace(/[^\w.-]+/g, "_");
}

export function parseJsonValue(raw, { label = "JSON" } = {}) {
  if (raw == null || raw === "") return raw;
  if (typeof raw === "object") return raw;
  const s = String(raw).trim();
  if (!s) return raw;
  try {
    return JSON.parse(s);
  } catch (e) {
    throw new Error(`${label}: invalid JSON (${e.message})`);
  }
}

export function rawTriggerItems(block) {
  if (!block) return [];
  if (Array.isArray(block)) return block;
  if (Array.isArray(block.items)) return block.items;
  if (typeof block === "string") {
    const parsed = parseJsonValue(block, { label: "triggers" });
    return rawTriggerItems(parsed);
  }
  return Object.entries(block)
    .filter(([, v]) => v && typeof v === "object" && (v.pattern || v.regex || v.text))
    .map(([key, v]) => ({
      id: v.id || key,
      ...v,
      pattern: v.pattern || v.regex || v.text,
    }));
}

function compilePredicate(spec, fallbackType = "regex", fallbackFlags = "") {
  const type = String(spec.type || fallbackType || "regex").toLowerCase();
  const pattern = spec.pattern ?? spec.regex ?? spec.text;
  if (pattern == null || String(pattern) === "") throw new Error("需要 pattern");
  if (type === "regex") {
    const re = new RegExp(String(pattern), spec.flags ?? fallbackFlags ?? "");
    return (line) => re.test(line);
  }
  if (type === "contains") {
    const cs = spec.case_sensitive === true;
    const needle = cs ? String(pattern) : String(pattern).toLowerCase();
    return (line) => {
      const hay = cs ? line : line.toLowerCase();
      return hay.includes(needle);
    };
  }
  throw new Error(`type 只能是 regex 或 contains，收到 ${type}`);
}

function listUnless(raw) {
  const out = [];
  const add = (item, flags) => {
    if (item == null || item === "") return;
    if (typeof item === "string") out.push({ type: "regex", pattern: item, flags: flags || "" });
    else if (typeof item === "object") out.push(item);
  };
  if (Array.isArray(raw.unless)) raw.unless.forEach((x) => add(x, raw.exclude_flags));
  if (Array.isArray(raw.except)) raw.except.forEach((x) => add(x, raw.exclude_flags));
  if (raw.exclude != null && raw.exclude !== "") {
    if (Array.isArray(raw.exclude)) raw.exclude.forEach((x) => add(x, raw.exclude_flags));
    else add(raw.exclude, raw.exclude_flags);
  }
  return out;
}

export function compileTrigger(raw, index) {
  if (raw.enabled === false) return null;
  const id = sanitizeTriggerId(raw.id || `T${index + 1}`);
  const label = String(raw.label || id);
  const type = String(raw.type || "regex").toLowerCase();
  const pattern = raw.pattern ?? raw.regex ?? raw.text;
  if (pattern == null || String(pattern) === "") {
    throw new Error(`trigger ${id}: 需要 pattern`);
  }
  const include = compilePredicate({ ...raw, type, pattern }, type, raw.flags || "");
  const unlessSpecs = listUnless(raw);
  const unless = unlessSpecs.map((u, i) => {
    try {
      return {
        id: u.id || `unless_${i + 1}`,
        label: u.label || u.pattern || u.text || `unless_${i + 1}`,
        type: u.type || "regex",
        pattern: String(u.pattern ?? u.regex ?? u.text ?? ""),
        test: compilePredicate(u, "regex", u.flags || raw.exclude_flags || ""),
      };
    } catch (e) {
      throw new Error(`trigger ${id} unless[${i}]: ${e.message || e}`);
    }
  });
  const match = (line) => include(line) && !unless.some((u) => u.test(line));
  return {
    id,
    label,
    type,
    pattern: String(pattern),
    flags: raw.flags || "",
    unless: unless.map((u) => ({ id: u.id, label: u.label, type: u.type, pattern: u.pattern })),
    note: raw.note || "",
    match,
  };
}

/**
 * Compile enabled rules from a station `serial_triggers` / `triggers` block.
 * @throws if none are enabled
 */
export function compileTriggers(block) {
  const items = rawTriggerItems(block);
  const compiled = [];
  for (let i = 0; i < items.length; i++) {
    const one = compileTrigger(items[i], i);
    if (one) compiled.push(one);
  }
  if (!compiled.length) {
    throw new Error("serial_triggers.items 没有已启用的条件");
  }
  return compiled;
}

export function classifyLine(compiled, line, { risingEdge = true, edgeHigh } = {}) {
  const list = Array.isArray(compiled) ? compiled : [];
  if (!risingEdge) {
    for (const t of list) {
      if (t.match(line)) return t.id;
    }
    return null;
  }
  const latch = edgeHigh instanceof Map ? edgeHigh : new Map();
  let fired = null;
  for (const t of list) {
    const on = t.match(line);
    const was = latch.get(t.id) === true;
    if (on && !was && fired == null) fired = t.id;
    latch.set(t.id, on);
  }
  return fired;
}

export function triggerSpecPath(block, { configPath, pluginDir } = {}) {
  if (block?.file) {
    const f = String(block.file);
    if (path.isAbsolute(f)) return f;
    const base = pluginDir || (configPath ? path.dirname(configPath) : process.cwd());
    return path.resolve(base, f);
  }
  return configPath || null;
}

/**
 * Resolve the live trigger block: Hub/CLI overlay (`items`) wins; otherwise
 * optional `file`, otherwise the in-memory station block.
 */
export function resolveTriggerBlock(config, { configPath, pluginDir } = {}) {
  const block = config?.serial_triggers || config?.triggers || {};
  const items = rawTriggerItems(block);
  if (items.length) return { ...block, items };
  const specPath = triggerSpecPath(block, { configPath, pluginDir });
  if (specPath && specPath !== configPath && fs.existsSync(specPath)) {
    const disk = JSON.parse(fs.readFileSync(specPath, "utf8"));
    const fileItems = rawTriggerItems(disk);
    return { ...block, ...disk, items: fileItems, _specPath: specPath };
  }
  return { ...block, items };
}

export function createLineClassifier(block, { risingEdge = true } = {}) {
  let compiled = compileTriggers(block);
  const edgeHigh = new Map();
  let edge = risingEdge !== false;
  return {
    get compiled() {
      return compiled;
    },
    get risingEdge() {
      return edge;
    },
    setRisingEdge(v) {
      edge = v !== false;
    },
    reload(nextBlock) {
      compiled = compileTriggers(nextBlock);
      edgeHigh.clear();
      return compiled;
    },
    labels() {
      return compiled.map((t) => t.label).join(" / ") || "(无)";
    },
    spec() {
      return compiled.map((t) => ({
        id: t.id,
        label: t.label,
        type: t.type,
        pattern: t.pattern,
        unless: t.unless || [],
      }));
    },
    classify(line) {
      return classifyLine(compiled, line, { risingEdge: edge, edgeHigh });
    },
  };
}

export default {
  sanitizeTriggerId,
  parseJsonValue,
  rawTriggerItems,
  compileTrigger,
  compileTriggers,
  classifyLine,
  triggerSpecPath,
  resolveTriggerBlock,
  createLineClassifier,
};
