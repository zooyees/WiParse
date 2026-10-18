/**
 * Shared run-session helpers for Testing Hub plugins: HUD, lock, stop, artifacts.
 */

import fs from "node:fs";
import path from "node:path";
import {
  collectOutputFiles,
  loadStationConfig,
  normalizeResult,
  requestStop,
  truthy,
} from "./plugin-contract.mjs";

export function writeHud(file, fields = {}) {
  if (!file) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  let prev = {};
  try {
    prev = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    prev = {};
  }
  const doc = {
    step: fields.step ?? prev.step ?? "",
    hint: fields.hint ?? prev.hint ?? "",
    cycle: fields.cycle ?? prev.cycle ?? 0,
    elapsed_s: fields.elapsed_s ?? prev.elapsed_s ?? 0,
  };
  fs.writeFileSync(file, `${JSON.stringify(doc)}\n`, "utf8");
}

export function pidAlive(pid) {
  const n = Number(pid);
  if (!Number.isInteger(n) || n <= 0) return false;
  try {
    process.kill(n, 0);
    return true;
  } catch {
    return false;
  }
}

export function acquireLock(lockFile, meta = {}) {
  if (!lockFile) return { ok: true, skipped: true };
  fs.mkdirSync(path.dirname(lockFile), { recursive: true });
  if (fs.existsSync(lockFile)) {
    try {
      const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
      if (prev?.pid && pidAlive(prev.pid) && Number(prev.pid) !== process.pid) {
        return { ok: false, error: `locked by pid ${prev.pid}` };
      }
    } catch {
      /* stale */
    }
  }
  const doc = {
    pid: process.pid,
    started: new Date().toISOString(),
    ...meta,
  };
  fs.writeFileSync(lockFile, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  return { ok: true, lock_file: lockFile };
}

export function releaseLock(lockFile) {
  if (!lockFile) return;
  try {
    if (!fs.existsSync(lockFile)) return;
    const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
    if (prev?.pid && Number(prev.pid) !== process.pid && pidAlive(prev.pid)) return;
  } catch {
    /* remove anyway */
  }
  try {
    fs.unlinkSync(lockFile);
  } catch {
    /* ignore */
  }
}

export function stopRequested(stopFile) {
  return Boolean(stopFile && fs.existsSync(stopFile));
}

export function clearStop(stopFile) {
  if (!stopFile) return;
  try {
    if (fs.existsSync(stopFile)) fs.unlinkSync(stopFile);
  } catch {
    /* ignore */
  }
}

export function ensureArtifactDirs(config) {
  const p = config?.paths || {};
  for (const key of ["run_dir", "artifacts_dir", "logs_dir", "report_dir", "docs_dir"]) {
    const d = p[key];
    if (typeof d === "string" && d && !d.includes("{")) {
      fs.mkdirSync(d, { recursive: true });
    }
  }
}

export function writeText(file, text) {
  if (!file) return "";
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text, "utf8");
  return file;
}

/** Write a Markdown report with portable links to the collected evidence. */
export function writeReportWithEvidence(plugin, layout, file, text) {
  if (!file) return "";
  const outputs = collectOutputFiles(plugin?.outputs, {
    artifacts_dir: layout?.artifacts_dir || "", run_dir: layout?.run_dir || "",
  });
  const files = [...new Set(Object.values(outputs).flat())].filter(p=>
    path.resolve(p)!==path.resolve(file) && !/\.(md|markdown)$/i.test(p));
  const sections = new Map();
  for (const p of files) {
    const ext = path.extname(p).toLowerCase();
    const section = /\.(isf|wfm|trc|vcd|csv)$/.test(ext) ? '波形与采样数据 / Waveforms'
      : /\.(png|jpg|jpeg|gif|bmp|webp)$/.test(ext) ? '图片 / Images'
      : /\.(json|jsonl|log|txt)$/.test(ext) ? '记录与日志 / Records'
      : '其他文件 / Other files';
    const label=path.basename(p).replaceAll('|','\\|').replaceAll('[','\\[').replaceAll(']','\\]');
    const url=path.relative(path.dirname(file),p).split(path.sep).map(encodeURIComponent).join('/');
    if (!sections.has(section)) sections.set(section, []);
    sections.get(section).push(`- [${label}](${url})`);
  }
  const evidence = [...sections].map(([title, links])=>`### ${title}\n\n${links.join('\n')}`).join('\n\n');
  return writeText(file, text + (evidence ? `\n\n## 数据与附件 / Evidence\n\n${evidence}\n` : ''));
}

export function writeJson(file, obj) {
  return writeText(file, `${JSON.stringify(obj, null, 2)}\n`);
}

export function attachArtifacts(raw, plugin, layout, extraFiles = {}) {
  const vars = {
    artifacts_dir: layout?.artifacts_dir || "",
    run_dir: layout?.run_dir || "",
  };
  const collected = collectOutputFiles(plugin?.outputs, vars);
  const merged = { ...collected };
  for (const [k, v] of Object.entries(extraFiles)) {
    if (!v) continue;
    merged[k] = merged[k] ? [...(Array.isArray(merged[k]) ? merged[k] : [merged[k]]), v] : v;
  }
  return {
    ...raw,
    session: raw?.session || layout?.stamp,
    artifacts: Object.keys(merged).length ? merged : raw?.artifacts,
  };
}

export function sessionStop(ctx) {
  const { config } = loadStationConfig(ctx);
  const r = requestStop(config, { reason: "plugin stop()" });
  ctx.log?.("info", `[${ctx.plugin?.id || "plugin"}] stop → ${r.stop_file || r.error}\n`);
  return normalizeResult({ ok: r.ok, step: "stop", error: r.error }, "stop");
}

export function num(v, dflt) {
  const n = Number(v);
  return Number.isFinite(n) ? n : dflt;
}

export function str(v, dflt = "") {
  if (v == null) return dflt;
  const s = String(v).trim();
  return s === "" ? dflt : s;
}

export { loadStationConfig, normalizeResult, truthy };
