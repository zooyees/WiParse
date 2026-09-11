/**
 * Production capture loop: serial trigger rising-edge → delay →
 * ScopeStop + serial stop → screenshot + ISF + PDF → ScopeRun + serial start.
 *
 * Export runLoop(cfg, opts) for the standard test-tool plugin contract.
 * Do not call test.start in the capture window: it reopens the serial monitor.
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHttpClient } from "../../lib/wiparse-sdk.mjs";
import {
  colocateStatusWithIsf,
  colocateSummaryWithReport,
  pickInstrument,
  suggestedParamsFromInstrument,
} from "../../lib/plugin-contract.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));

/** @type {any} */
let cfg;
/** @type {string} */
let cfgPath = path.join(here, "station.json");
let api = "";
let outDir = "";
let reportDir = "";
let stopFile = "";
let statusFile = "";
let hintFile = "";
let lockFile = "";
let prefix = "";
let sessionStamp = "";
let summaryMd = "";
let port = "";
let baud = 0;
let delayS = 0.3;
let settleMs = 1500;
let pollMs = 400;
/** @type {string[]} */
let browsers = [];
let preflightOnly = false;
/** @type {((stream: string, text: string) => void) | undefined} */
let externalLog;
/** Rising-edge latch: fire only on false→true per trigger id. */
let risingEdge = true;
/** @type {Map<string, boolean>} */
const edgeHigh = new Map();
/** Honor station.interlocks.hold_serial_off_during_capture (default true). */
let holdSerialOffEnabled = true;
/** @type {{ url: string, health: Function, invoke: Function } | null} */
let http = null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const CONTEXT_KEEP = 80;

function log(stream, text) {
  const raw = typeof text === "string" ? text : JSON.stringify(text);
  const line = raw.endsWith("\n") ? raw : `${raw}\n`;
  if (typeof externalLog === "function") {
    try {
      externalLog(stream === "err" ? "stderr" : "stdout", line);
    } catch {
      /* ignore host log failures */
    }
  }
  if (stream === "err") console.error(raw);
  else console.log(raw);
}


function sanitizeTriggerId(id) {
  const s = String(id || "").trim();
  if (!s) throw new Error("trigger id 不能为空");
  return s.replace(/[^\w.-]+/g, "_");
}

function rawTriggerItems(block) {
  if (!block) return [];
  if (Array.isArray(block)) return block;
  if (Array.isArray(block.items)) return block.items;
  return Object.entries(block)
    .filter(([, v]) => v && typeof v === "object" && (v.pattern || v.regex || v.text))
    .map(([key, v]) => ({ id: v.id || key, ...v, pattern: v.pattern || v.regex || v.text }));
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

function compileTrigger(raw, index) {
  if (raw.enabled === false) return null;
  const id = sanitizeTriggerId(raw.id || `T${index + 1}`);
  const label = String(raw.label || id);
  const type = String(raw.type || "regex").toLowerCase();
  const pattern = raw.pattern ?? raw.regex ?? raw.text;
  if (pattern == null || String(pattern) === "") throw new Error(`trigger ${id}: 需要 pattern`);
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

function triggerSpecPath(data = cfg) {
  const block = data.serial_triggers || data.triggers;
  if (block?.file) return path.resolve(here, block.file);
  return cfgPath;
}

let compiledTriggers = [];
let triggerStamp = "";

function loadTriggersFromDisk() {
  const specPath = triggerSpecPath();
  const data = JSON.parse(fs.readFileSync(specPath, "utf8"));
  const block = specPath === cfgPath ? (data.serial_triggers || data.triggers) : data;
  const items = rawTriggerItems(block);
  const compiled = [];
  for (let i = 0; i < items.length; i++) {
    const one = compileTrigger(items[i], i);
    if (one) compiled.push(one);
  }
  if (!compiled.length) throw new Error("serial_triggers.items 没有已启用的条件");
  const st = fs.statSync(specPath);
  compiledTriggers = compiled;
  triggerStamp = `${specPath}|${st.mtimeMs}|${compiled.map((t) => t.id).join(",")}`;
  return compiled;
}

function refreshTriggers(force = false) {
  try {
    const specPath = triggerSpecPath();
    const st = fs.statSync(specPath);
    const stamp = `${specPath}|${st.mtimeMs}`;
    if (!force && compiledTriggers.length && triggerStamp.startsWith(stamp)) return compiledTriggers;
    const prev = compiledTriggers.slice();
    const next = loadTriggersFromDisk();
    if (prev.length && prev.map((t) => t.id).join() !== next.map((t) => t.id).join()) {
      log("info", `[triggers] reloaded ${next.map((t) => t.id).join(", ")}`);
    }
    return next;
  } catch (e) {
    if (compiledTriggers.length) {
      log("err", `[triggers] reload failed: ${e.message || e}`);
      return compiledTriggers;
    }
    throw e;
  }
}

function triggerLabels() {
  return compiledTriggers.map((t) => t.label).join(" / ") || "(无)";
}

function classify(line) {
  if (!risingEdge) {
    for (const t of compiledTriggers) {
      if (t.match(line)) return t.id;
    }
    return null;
  }
  // Industrial rising-edge: fire once when a trigger goes inactive→active.
  let fired = null;
  for (const t of compiledTriggers) {
    const on = t.match(line);
    const was = edgeHigh.get(t.id) === true;
    if (on && !was && fired == null) fired = t.id;
    edgeHigh.set(t.id, on);
  }
  return fired;
}

function stamp(d = new Date()) {
  const p = (n, w = 2) => String(n).padStart(w, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

function stampNice(d = new Date()) {
  const p = (n, w = 2) => String(n).padStart(w, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function esc(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function cmpVer(a, b) {
  const pa = String(a || "0").split(".").map((x) => parseInt(x, 10) || 0);
  const pb = String(b || "0").split(".").map((x) => parseInt(x, 10) || 0);
  const n = Math.max(pa.length, pb.length);
  for (let i = 0; i < n; i++) {
    if ((pa[i] || 0) > (pb[i] || 0)) return 1;
    if ((pa[i] || 0) < (pb[i] || 0)) return -1;
  }
  return 0;
}

function pidAlive(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function invoke(method, params = {}, ms = 120_000, opts = {}) {
  if (!http) {
    http = createHttpClient({ url: api, log: externalLog });
  }
  return http.invoke(method, params, ms, opts);
}

function writeStatus(extra) {
  fs.mkdirSync(path.dirname(statusFile), { recursive: true });
  const payload = {
    ts: new Date().toISOString(),
    sop: cfg.sop,
    station: cfg.station,
    ...extra,
  };
  fs.writeFileSync(statusFile, JSON.stringify(payload, null, 2), "utf8");
  if (payload.hint) fs.writeFileSync(hintFile, payload.hint, "utf8");
}

function setHint(step, hint, extra = {}) {
  writeStatus({ step, hint, ...extra });
  const n = extra.cycle != null ? ` #${extra.cycle}` : "";
  log("info", `[${step}${n}] ${hint}`);
}

async function serialStatus() {
  const st = await invoke("serial.status", {}, 30_000, { allowFail: true });
  return Boolean(st.data?.monitoring);
}

async function serialStop() {
  let r = await invoke("serial.monitor.stop", {}, 30_000, { allowFail: true });
  if (r.ok === false) r = await invoke("serial.stop", {}, 30_000, { allowFail: true });
  return r;
}

async function serialStopUntilOff(ms = cfg.timing.serial_stop_timeout_ms) {
  const t0 = Date.now();
  let last = true;
  while (Date.now() - t0 < ms) {
    await serialStop();
    last = await serialStatus();
    if (!last) return false;
    await sleep(120);
  }
  return last;
}

async function serialStart() {
  let r = await invoke("serial.monitor.start", { port, baud }, 30_000, { allowFail: true });
  if (r.ok === false) r = await invoke("serial.start", { port, baud }, 30_000, { allowFail: true });
  const t0 = Date.now();
  while (Date.now() - t0 < 4000) {
    if (await serialStatus()) return r;
    await sleep(120);
  }
  return r;
}

function holdSerialOff() {
  if (!holdSerialOffEnabled) {
    return async () => {};
  }
  serialStop().catch(() => {});
  // Keep serial off during capture without hammering the API every 250ms.
  const keeper = setInterval(() => {
    serialStop().catch(() => {});
  }, 2000);
  return async () => {
    clearInterval(keeper);
    await serialStopUntilOff();
  };
}

async function scopeCommand(deviceId, command) {
  return invoke("instrument.command", { device_id: deviceId, command, timeout_s: 20 });
}

function newestIsf(dir, stem, sinceMs) {
  if (!fs.existsSync(dir)) return null;
  let best = null;
  for (const name of fs.readdirSync(dir)) {
    if (!name.startsWith(stem) || !/\.isf$/i.test(name)) continue;
    const p = path.join(dir, name);
    try {
      const st = fs.statSync(p);
      if (st.mtimeMs < sinceMs - 2000 || st.size < 1024) continue;
      if (!best || st.mtimeMs > best.mtimeMs) best = { path: p, size: st.size, mtimeMs: st.mtimeMs };
    } catch {}
  }
  return best;
}

async function waitNewIsf(dir, filename, sinceMs, timeoutMs) {
  const stem = filename.replace(/\.isf$/i, "");
  const t0 = Date.now();
  let lastSize = -1;
  let stable = 0;
  // Serial already stopped by caller; one extra stop, then FS-only poll.
  await serialStop().catch(() => {});
  while (Date.now() - t0 < timeoutMs) {
    if (fs.existsSync(stopFile)) return null;
    const hit = newestIsf(dir, stem, sinceMs);
    if (hit) {
      if (hit.size === lastSize) {
        stable += 1;
        if (stable >= 3) return hit.path;
      } else {
        lastSize = hit.size;
        stable = 0;
      }
    }
    await sleep(pollMs);
  }
  return newestIsf(dir, stem, sinceMs)?.path ?? null;
}

async function resolveScope(depth = 0) {
  const list = await invoke("instrument.list");
  const devices = list.data?.devices || [];
  const spec = scopeSpec();
  let scope = pickInstrument(devices, spec);
  if (!scope) {
    const resource = String(spec.resource || "").trim();
    if (!resource || depth >= 3) {
      throw new Error(
        resource
          ? `resolveScope: connect failed for ${resource}`
          : "resolveScope: 未连接仪器。请在「仪器」页连接，或填写 VISA resource 后预检"
      );
    }
    await invoke("instrument.connect", {
      kind: spec.kind || "oscilloscope",
      resource,
    });
    await sleep(1500 * (depth + 1));
    return resolveScope(depth + 1);
  }
  await invoke("ui.instrument.select", { device_id: scope.device_id });
  return scope;
}

function scopeSpec() {
  const s = cfg.scope || {};
  return {
    deviceId: s.prefer_device_id,
    resource: s.resource,
    model: s.model,
    kind: s.kind || "oscilloscope",
  };
}

/** Incremental live log cursor — avoids re-fetching thousands of lines each poll. */
async function liveLinesSince(fromRow, limit = 200) {
  const chunk = await invoke("log.lines.get", {
    tab_id: 0,
    from_row: fromRow,
    limit,
  });
  const lines = (chunk.data?.lines || []).map(String);
  const total = Number(chunk.data?.total ?? fromRow + lines.length);
  return { lines, total, next: fromRow + lines.length };
}

async function waitTrigger(cycle, scopeId) {
  const probe = await liveLinesSince(0, 1);
  let fromRow = Number(probe.total) || 0;
  const t0 = Date.now();
  let lastBeat = -1;
  /** @type {string[]} rolling context across chunks */
  const ring = [];
  while (!fs.existsSync(stopFile)) {
    refreshTriggers();
    const elapsed = Math.floor((Date.now() - t0) / 1000);
    if (elapsed !== lastBeat && elapsed % 2 === 0) {
      lastBeat = elapsed;
      setHint("wait", `监控中：第 ${cycle} 轮，等待串口 ${triggerLabels()}（已等 ${elapsed}s）`, {
        cycle,
        scope_id: scopeId,
        elapsed_s: elapsed,
        triggers: compiledTriggers.map((t) => t.label),
      });
    }
    const { lines, total, next } = await liveLinesSince(fromRow, 200);
    if (total < fromRow) {
      // Log buffer reset — keep recent ring for continuity, reset cursor.
      fromRow = total;
      continue;
    }
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      ring.push(line);
      if (ring.length > CONTEXT_KEEP) ring.shift();
      const trigger = classify(line);
      if (trigger) {
        const before = ring.slice(0, -1).slice(-25);
        const after = lines.slice(i + 1, i + 26);
        return {
          trigger,
          at: new Date(),
          line,
          context: [...before, line, ...after],
        };
      }
    }
    fromRow = next > fromRow ? next : total;
    await sleep(pollMs);
  }
  return { trigger: "stop" };
}

async function stopScopeAndSerial(deviceId) {
  const [serial, scope] = await Promise.all([
    serialStopUntilOff(),
    (async () => {
      const cmd = await scopeCommand(deviceId, "ScopeStop");
      await sleep(settleMs);
      return cmd;
    })(),
  ]);
  await serialStopUntilOff();
  return { scope, serial_result: serial, monitoring: await serialStatus() };
}

async function startScopeAndSerial(deviceId) {
  const [scope, serial] = await Promise.all([
    (async () => {
      const cmd = await scopeCommand(deviceId, "ScopeRun");
      await sleep(settleMs);
      return cmd;
    })(),
    serialStart(),
  ]);
  return { scope, serial, monitoring: await serialStatus() };
}

async function captureShotAndIsf(deviceId, shotPath, filename) {
  const shot = await invoke("scope.shot", { index: 0, out: shotPath }, cfg.timing.shot_timeout_ms);
  const shotFile = shot.data?.path || (fs.existsSync(shotPath) ? shotPath : null);
  const sinceMs = Date.now();
  const src = await invoke("instrument.waveform_source", {
    device_id: deviceId,
    dir: outDir,
    filename,
    overwrite: false,
    timeout_s: Math.round(cfg.timing.isf_timeout_ms / 1000),
  });
  const isfPath = await waitNewIsf(outDir, filename, sinceMs, cfg.timing.isf_timeout_ms);
  return {
    shot,
    shotFile: shotFile ? String(shotFile).replace(/^\\\\\?\\/, "") : null,
    waveform: src,
    isfPath: isfPath || path.join(outDir, filename),
  };
}

function findBrowser() {
  return browsers.find((p) => fs.existsSync(p));
}

function htmlToPdf(htmlPath, pdfPath) {
  const browser = findBrowser();
  if (!browser) throw new Error("Edge/Chrome not found for PDF");
  const PDF_TIMEOUT_MS = 120_000;
  return new Promise((resolve, reject) => {
    const child = spawn(
      browser,
      ["--headless=new", "--disable-gpu", `--print-to-pdf=${pdfPath}`, "--no-pdf-header-footer", pathToFileURL(htmlPath).href],
      { windowsHide: true },
    );
    let err = "";
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      try {
        child.kill();
      } catch {
        /* ignore */
      }
      reject(new Error(`pdf timeout after ${PDF_TIMEOUT_MS}ms`));
    }, PDF_TIMEOUT_MS);
    child.stderr.on("data", (d) => {
      if (err.length < 4000) err += d.toString();
    });
    child.on("error", (e) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(e);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (code === 0 && fs.existsSync(pdfPath)) resolve(pdfPath);
      else reject(new Error(`pdf failed code=${code} ${err.slice(0, 300)}`));
    });
  });
}

function copyIfExists(src, dest) {
  if (!src || !fs.existsSync(src)) return null;
  const from = String(src).replace(/^\\\\\?\\/, "");
  if (path.resolve(from) === path.resolve(dest)) return dest;
  fs.copyFileSync(from, dest);
  return dest;
}

function mdCell(s) {
  return String(s ?? "")
    .replaceAll("\\", "\\\\")
    .replaceAll("|", "\\|")
    .replaceAll("\r", "")
    .replaceAll("\n", "<br>");
}

function mdFence(s) {
  return String(s ?? "").replaceAll("~~~", "--");
}

function ensureSummaryHeader() {
  fs.mkdirSync(path.dirname(summaryMd), { recursive: true });
  if (fs.existsSync(summaryMd)) return;
  const head = `# ${cfg.sop.title} · 总报告

| 项 | 值 |
|---|---|
| 作业指导 | ${mdCell(cfg.sop.id)} Rev ${mdCell(cfg.sop.rev)} |
| 工位 | ${mdCell(cfg.station.id)} / ${mdCell(cfg.station.name)} |
| 产品 | ${mdCell(cfg.station.product)} |
| 串口 | ${mdCell(port)} @ ${mdCell(baud)} |
| 报告目录 | ${mdCell(reportDir)} |

每次触发追加一节；节名含轮次、触发条件和时间，记录 ID 与单份 PDF/ISF 文件名一致。

`;
  fs.writeFileSync(summaryMd, head, "utf8");
}

function appendSessionBanner(scopeInfo) {
  ensureSummaryHeader();
  const trig = compiledTriggers.map((t) => t.label).join(" / ");
  const block = `
---

## 会话 SESSION-${sessionStamp}

| 项 | 值 |
|---|---|
| 启动时间 | ${mdCell(stampNice())} |
| GUI | ${mdCell(scopeInfo.guiVersion)} |
| 示波器 | ${mdCell(scopeInfo.scopeModel)} id=${mdCell(scopeInfo.scopeId)} |
| 触发规格 | ${mdCell(trig)} |

`;
  fs.appendFileSync(summaryMd, block, "utf8");
}

function mdRelPath(fromFile, target) {
  if (!target) return "";
  const rel = path.relative(path.dirname(fromFile), target);
  if (!rel || rel.startsWith("..")) return "";
  return rel.split(path.sep).join("/");
}

function materializeScreenshot(src, trigger, recStamp) {
  if (!src) return null;
  fs.mkdirSync(reportDir, { recursive: true });
  const dest = path.join(reportDir, `${prefix}_${trigger}_${recStamp}.png`);
  return copyIfExists(src, dest);
}

function mdImageBlock(absShot) {
  const rel = mdRelPath(summaryMd, absShot);
  if (!rel) return "_无截图_";
  if (/[\s()]/.test(rel)) return `![波形](<${rel}>)`;
  return `![波形](${rel})`;
}

function appendSummaryRecord(rec) {
  ensureSummaryHeader();
  const recId = `${prefix}_${rec.trigger}_${rec.stamp}`;
  const shotName = rec.screenshot ? path.basename(String(rec.screenshot)) : "";
  const pdfName = rec.pdfPath ? path.basename(String(rec.pdfPath)) : "";
  const ctx = (rec.context || []).map((l) => {
    const mark = l === rec.line ? " <<HIT>>" : "";
    return mdFence(l) + mark;
  }).join("\n");
  const shotMd = rec.screenshot ? mdImageBlock(String(rec.screenshot)) : "_无截图_";
  const block = `
---

## 第 ${rec.cycle} 轮 · ${rec.trigger} · ${rec.timeText}

**记录 ID：** \`${recId}\`

| 项 | 值 |
|---|---|
| 轮次 | ${mdCell(rec.cycle)} |
| 触发条件 | ${mdCell(rec.trigger)} |
| 触发时间 | ${mdCell(rec.timeText)} |
| 命中行 | ${mdCell(rec.line)} |
| 处理时串口 | monitoring=${mdCell(rec.serialMonitoring)} |
| 示波器 | ${mdCell(rec.scopeModel)} id=${mdCell(rec.scopeId)} |
| ISF | \`${mdCell(rec.isfPath)}\` |
| 截图 | ${shotName ? `\`${mdCell(rec.screenshot)}\`` : "无"} |
| PDF | ${pdfName ? `\`${mdCell(rec.pdfPath)}\`` : "无"} |

### 波形截图

${shotMd}

### Log 上下文

~~~
${ctx}
~~~

`;
  fs.appendFileSync(summaryMd, block, "utf8");
  return summaryMd;
}

function writePdfReport(rec) {
  fs.mkdirSync(reportDir, { recursive: true });
  const base = `${prefix}_${rec.trigger}_${rec.stamp}`;
  const htmlPath = path.join(reportDir, `${base}.html`);
  const pdfPath = path.join(reportDir, `${base}.pdf`);
  const jsonPath = path.join(reportDir, `${base}.json`);
  const shotDest = rec.screenshotSrc
    ? copyIfExists(rec.screenshotSrc, path.join(reportDir, `${base}.png`))
    : null;
  const shotUri = shotDest ? pathToFileURL(shotDest).href : "";
  const ctx = (rec.context || []).map((l) => {
    const mark = l === rec.line ? " class=\"hit\"" : "";
    return `<div${mark}>${esc(l)}</div>`;
  }).join("\n");
  const html = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>${esc(base)}</title>
<style>
@page{size:A4;margin:12mm}
html,body{font-family:"Microsoft YaHei UI","Segoe UI",sans-serif;font-size:10px;line-height:1.35;color:#222;margin:0}
body{padding:10px 12px}
h1{font-size:13px;font-weight:600;margin:0 0 8px;letter-spacing:0}
h2{font-size:11px;font-weight:600;margin:12px 0 6px}
table{border-collapse:collapse;width:100%;margin:0 0 8px;font-size:9.5px}
th{width:18%;background:#f3f3f3;font-weight:600;color:#444}
td,th{border:1px solid #d5d5d5;padding:3px 6px;text-align:left;vertical-align:top}
.hit{background:#fff3cd;font-weight:600}
.ctx{font-family:Consolas,"Courier New",monospace;font-size:8px;line-height:1.3;white-space:pre-wrap;background:#f7f7f7;padding:5px 6px;border:1px solid #ddd}
img{max-width:100%;max-height:118mm;object-fit:contain;border:1px solid #ccc}
p{margin:4px 0;font-size:10px}
</style></head><body>
<h1>${esc(cfg.sop.title)}</h1>
<table>
<tr><th>作业指导</th><td>${esc(cfg.sop.id)} Rev ${esc(cfg.sop.rev)}</td></tr>
<tr><th>工位</th><td>${esc(cfg.station.id)} / ${esc(cfg.station.name)}</td></tr>
<tr><th>产品</th><td>${esc(cfg.station.product)}</td></tr>
<tr><th>GUI</th><td>${esc(rec.guiVersion)}</td></tr>
<tr><th>示波器</th><td>${esc(rec.scopeModel)} id=${esc(rec.scopeId)}</td></tr>
<tr><th>串口</th><td>${esc(port)} @ ${esc(baud)}</td></tr>
<tr><th>触发时间</th><td>${esc(rec.timeText)}</td></tr>
<tr><th>触发条件</th><td>${esc(rec.trigger)}</td></tr>
<tr><th>命中行</th><td>${esc(rec.line)}</td></tr>
<tr><th>处理时串口</th><td>monitoring=${esc(rec.serialMonitoring)}</td></tr>
<tr><th>波形源文件</th><td>${esc(rec.isfPath)}</td></tr>
<tr><th>波形截图</th><td>${esc(shotDest || rec.screenshotSrc || "无")}</td></tr>
<tr><th>轮次</th><td>${esc(rec.cycle)}</td></tr>
</table>
<h2>波形截图</h2>
${shotUri ? `<img src="${shotUri}" alt="scope">` : "<p>无截图</p>"}
<h2>Log 上下文</h2>
<div class="ctx">${ctx}</div>
</body></html>`;
  fs.writeFileSync(htmlPath, html, "utf8");
  fs.writeFileSync(jsonPath, JSON.stringify({
    sop: cfg.sop,
    station: cfg.station,
    ...rec,
    screenshot: shotDest,
    htmlPath,
    pdfPath,
  }, null, 2), "utf8");
  const indexPath = path.join(reportDir, "index.jsonl");
  fs.appendFileSync(indexPath, JSON.stringify({
    sop: cfg.sop.id,
    rev: cfg.sop.rev,
    station: cfg.station.id,
    time: rec.timeText,
    trigger: rec.trigger,
    cycle: rec.cycle,
    pdf: pdfPath,
    isf: rec.isfPath,
    screenshot: shotDest,
    serial_monitoring: rec.serialMonitoring,
  }) + "\n", "utf8");
  return { htmlPath, pdfPath, jsonPath, screenshot: shotDest };
}

function acquireLock() {
  fs.mkdirSync(path.dirname(lockFile), { recursive: true });
  if (fs.existsSync(lockFile)) {
    try {
      const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
      if (prev.pid && prev.pid !== process.pid && pidAlive(prev.pid)) {
        throw new Error(`已有循环在运行 pid=${prev.pid}，请先在 Testing Hub 点 Stop 或执行 stop.ps1`);
      }
    } catch (e) {
      if (String(e.message || e).includes("已有循环")) throw e;
    }
  }
  fs.writeFileSync(lockFile, JSON.stringify({
    pid: process.pid,
    started: new Date().toISOString(),
    config: cfgPath,
    sop: cfg.sop,
  }, null, 2), "utf8");
}

function releaseLock() {
  try {
    if (!fs.existsSync(lockFile)) return;
    const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
    if (prev.pid === process.pid) fs.unlinkSync(lockFile);
  } catch {}
}

async function preflight() {
  const checks = [];
  const push = (id, ok, detail) => checks.push({ id, ok, detail });
  let health;
  try {
    health = await http.health(15_000);
    push("gui_api", true, health.data?.listening || api);
  } catch (e) {
    push("gui_api", false, String(e.message || e));
    return { ok: false, health: null, checks };
  }
  const ver = health.data?.version;
  push("gui_version", cmpVer(ver, cfg.gui.min_version) >= 0, `${ver} (min ${cfg.gui.min_version})`);
  push("browser", Boolean(findBrowser()), findBrowser() || "Edge/Chrome missing");
  try {
    fs.mkdirSync(outDir, { recursive: true });
    fs.mkdirSync(reportDir, { recursive: true });
    const probe = path.join(outDir, ".write_probe");
    fs.writeFileSync(probe, "ok");
    fs.unlinkSync(probe);
    push("isf_dir", true, outDir);
    push("report_dir", true, reportDir);
    push("status_file", true, statusFile);
  } catch (e) {
    push("dirs", false, String(e.message || e));
  }

  // Single-instance lock
  if (cfg.interlocks?.single_instance !== false) {
    if (fs.existsSync(lockFile)) {
      try {
        const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
        if (prev.pid && prev.pid !== process.pid && pidAlive(prev.pid)) {
          push("single_instance", false, `lock held by pid=${prev.pid}`);
        } else {
          push("single_instance", true, "stale lock (will reclaim)");
        }
      } catch {
        push("single_instance", true, "lock unreadable (will reclaim)");
      }
    } else {
      push("single_instance", true, "free");
    }
  }

  // Serial API reachability (does not require monitoring already on)
  try {
    const st = await invoke("serial.status", {}, 15_000, { allowFail: true });
    if (st && st.ok === false) {
      push("serial_api", false, st.error || "serial.status failed");
    } else {
      const mon = Boolean(st?.data?.monitoring);
      push("serial_api", true, mon ? `monitoring on (${st?.data?.port || port})` : `ready (${port}@${baud})`);
    }
  } catch (e) {
    push("serial_api", false, String(e.message || e));
  }

  const list = await invoke("instrument.list", {}, 30_000, { allowFail: true });
  const devices = list.data?.devices || [];
  const spec = scopeSpec();
  const scope = pickInstrument(devices, spec);
  const kindLabel = spec.kind || "oscilloscope";
  push(
    "scope",
    Boolean(scope),
    scope
      ? `${scope.identity?.model || kindLabel} id=${scope.device_id} ${scope.resource || ""}`.trim()
      : devices.length
        ? `未匹配 ${kindLabel}（已连接 ${devices.length} 台）`
        : `未连接 ${kindLabel}：请在「仪器」页连接，或填写 VISA resource`
  );
  const expectModel = String(spec.model || "").trim();
  if (scope && expectModel && String(scope.identity?.model || "") !== expectModel) {
    push(
      "scope_model",
      true,
      `connected ${scope.identity?.model || "?"} (form had ${expectModel}; Hub will refresh)`
    );
  } else if (scope) {
    push("scope_model", true, scope.identity?.model || "any");
  }
  const suggested_params = suggestedParamsFromInstrument(scope, {
    prefer_device_id: "device_id",
    scope_resource: "resource",
    scope_model: "model",
    scope_kind: "kind",
  });
  try {
    const list = refreshTriggers(true);
    push("serial_triggers", list.length > 0, list.map((t) => {
      const skip = (t.unless || []).map((u) => u.label || u.pattern).join(",");
      return skip ? `${t.id}:${t.pattern} unless(${skip})` : `${t.id}:${t.type}:${t.pattern}`;
    }).join(" | "));
  } catch (e) {
    push("serial_triggers", false, String(e.message || e));
  }
  const ok = checks.every((c) => c.ok);
  return { ok, health, checks, scope, suggested_params };
}


function bindConfig(cfgInput, opts = {}) {
  cfg = cfgInput;
  if (opts.configPath) cfgPath = opts.configPath;
  colocateStatusWithIsf(cfg);
  colocateSummaryWithReport(cfg);
  api = String(cfg.gui?.api || "http://127.0.0.1:7878").replace(/\/$/, "");
  outDir = cfg.paths.isf_dir;
  reportDir = cfg.paths.report_dir;
  stopFile = cfg.paths.stop_file;
  statusFile = cfg.paths.status_file;
  hintFile = path.join(path.dirname(statusFile), "_loop_hint.txt");
  lockFile = cfg.paths.lock_file;
  prefix = cfg.paths.file_prefix;
  sessionStamp = stamp();
  summaryMd = (cfg.paths.summary_md || path.join(reportDir, `${prefix}_summary_{stamp}.md`)).replaceAll(
    "{stamp}",
    sessionStamp
  );
  // After {stamp} expand, park the md in report_dir again (basename only).
  summaryMd = path.join(reportDir, path.basename(summaryMd));
  port = cfg.serial.port;
  baud = cfg.serial.baud;
  delayS = cfg.timing.post_trigger_delay_s;
  settleMs = Math.round((cfg.timing.scope_settle_s ?? 1.5) * 1000);
  pollMs = cfg.timing.poll_ms ?? 400;
  browsers = cfg.pdf?.browsers || [];
  preflightOnly = Boolean(opts.preflightOnly);
  externalLog = typeof opts.log === "function" ? opts.log : undefined;
  risingEdge = cfg.serial_triggers?.rising_edge !== false;
  edgeHigh.clear();
  holdSerialOffEnabled = cfg.interlocks?.hold_serial_off_during_capture !== false;
  http = createHttpClient({ url: api, log: externalLog });
}

/**
 * @param {object} cfgInput - station config (already merged / templates expanded)
 * @param {{ preflightOnly?: boolean, configPath?: string, log?: Function }} [opts]
 */
export async function runLoop(cfgInput, opts = {}) {
  bindConfig(cfgInput, opts);
  const pf = await preflight();
  if (pf.ok) {
    log("info", "preflight OK");
  } else {
    log("err", "preflight FAIL");
  }
  for (const c of pf.checks || []) {
    const mark = c.ok ? "OK" : "X ";
    const detail = c.detail ? `  ${c.detail}` : "";
    log(c.ok ? "info" : "err", `  ${mark} ${c.id}${detail}`);
  }
  if (!pf.ok) {
    setHint("stopped", "预检失败，未启动监控", { checks: pf.checks });
    return { ok: false, step: "preflight", checks: pf.checks, suggested_params: pf.suggested_params };
  }
  if (preflightOnly) {
    return {
      ok: true,
      preflight: true,
      checks: pf.checks,
      suggested_params: pf.suggested_params,
    };
  }

  acquireLock();
  process.on("exit", releaseLock);
  process.on("SIGINT", () => process.exit(130));
  process.on("SIGTERM", () => process.exit(143));

  let cycle = 0;
  let scope = null;
  try {
    if (fs.existsSync(stopFile)) fs.unlinkSync(stopFile);
    fs.mkdirSync(outDir, { recursive: true });
    fs.mkdirSync(reportDir, { recursive: true });

    if (cfg.interlocks?.never_test_start_during_capture !== false) {
      await invoke(
        "test.abort",
        { reason: `${cfg.sop.id} start: do not leave a test plan holding serial` },
        30_000,
        { allowFail: true },
      );
    }
    scope = await resolveScope();
    const guiVersion = pf.health?.data?.version;
    setHint("armed", "准备中：启动示波器与串口", {
      cycle: 0,
      scope_id: scope.device_id,
      rising_edge: risingEdge,
    });
    await startScopeAndSerial(scope.device_id);

    setHint("armed", "准备完成：示波器 RUN 已完成，进入监控", {
      cycle: 0,
      version: guiVersion,
      port,
      baud,
      monitoring: await serialStatus(),
      scope_id: scope.device_id,
      scope_model: scope.identity?.model ?? null,
      rising_edge: risingEdge,
      triggers: compiledTriggers.map((t) => t.label),
      trigger_spec: compiledTriggers.map((t) => ({
        id: t.id,
        label: t.label,
        type: t.type,
        pattern: t.pattern,
        unless: t.unless || [],
      })),
      delay_s: delayS,
      dir: outDir,
      reportDir,
      summary_md: summaryMd,
      sop: cfg.sop.id,
      rev: cfg.sop.rev,
    });
    appendSessionBanner({
      guiVersion,
      scopeId: scope.device_id,
      scopeModel: scope.identity?.model ?? cfg.scope.model,
    });

    while (!fs.existsSync(stopFile)) {
      cycle += 1;
      const live = await resolveScope();
      setHint("wait", `监控中：第 ${cycle} 轮，等待串口 ${triggerLabels()}`, {
        cycle,
        scope_id: live.device_id,
        triggers: compiledTriggers.map((t) => t.label),
      });
      const hit = await waitTrigger(cycle, live.device_id);
      if (hit.trigger === "stop" || fs.existsSync(stopFile)) break;

      const at = hit.at;
      const trigger = hit.trigger;
      const ts = stamp(at);
      const filename = `${prefix}_${trigger}_${ts}.isf`;
      setHint(
        "processing",
        `已触发 ${trigger}：数据处理中（延时 ${delayS * 1000}ms / 同时停示波器与串口 / 截图 / 存源文件 / 出报告）`,
        { cycle, trigger, filename },
      );

      await sleep(delayS * 1000);
      if (fs.existsSync(stopFile)) break;
      setHint("processing", `已触发 ${trigger}：数据处理中（同时暂停示波器与串口）`, {
        cycle,
        trigger,
        filename,
      });
      const releaseSerial = holdSerialOff();
      let saved = path.join(outDir, filename);
      let shot = null;
      let monitoring = true;
      try {
        const stopped = await stopScopeAndSerial(live.device_id);
        monitoring = stopped.monitoring;
        setHint("processing", `已触发 ${trigger}：串口已停 monitoring=${monitoring}，截图并读源文件`, {
          cycle,
          trigger,
          filename,
          serial_monitoring: monitoring,
        });
        const shotPath = path.join(reportDir, `${prefix}_${trigger}_${ts}.png`);
        const cap = await captureShotAndIsf(live.device_id, shotPath, filename);
        saved = cap.isfPath;
        shot = cap.shotFile;
        monitoring = await serialStatus();
        setHint(
          "processing",
          `已触发 ${trigger}：数据处理中（生成 PDF 报告；串口 monitoring=${monitoring}）`,
          { cycle, trigger, filename, serial_monitoring: monitoring },
        );
      } finally {
        await releaseSerial();
        monitoring = await serialStatus();
      }
      let pdfPath = null;
      let mdPath = null;
      const shotInReport = materializeScreenshot(
        shot && fs.existsSync(String(shot)) ? String(shot) : null,
        trigger,
        ts,
      );
      try {
        const written = writePdfReport({
          cycle,
          trigger,
          stamp: ts,
          timeText: stampNice(at),
          line: hit.line,
          context: hit.context || [],
          isfPath: String(saved),
          screenshotSrc: shotInReport || (shot && fs.existsSync(String(shot)) ? String(shot) : null),
          guiVersion,
          scopeId: live.device_id,
          scopeModel: live.identity?.model ?? cfg.scope.model,
          serialMonitoring: monitoring,
        });
        await htmlToPdf(written.htmlPath, written.pdfPath);
        pdfPath = written.pdfPath;
        mdPath = appendSummaryRecord({
          cycle,
          trigger,
          stamp: ts,
          timeText: stampNice(at),
          line: hit.line,
          context: hit.context || [],
          isfPath: String(saved),
          screenshot: written.screenshot || shotInReport,
          pdfPath,
          guiVersion,
          scopeId: live.device_id,
          scopeModel: live.identity?.model ?? cfg.scope.model,
          serialMonitoring: monitoring,
        });
      } catch (e) {
        log("err", `[pdf] ${e?.message || e}`);
        try {
          mdPath = appendSummaryRecord({
            cycle,
            trigger,
            stamp: ts,
            timeText: stampNice(at),
            line: hit.line,
            context: hit.context || [],
            isfPath: String(saved),
            screenshot: shotInReport,
            pdfPath: null,
            guiVersion,
            scopeId: live.device_id,
            scopeModel: live.identity?.model ?? cfg.scope.model,
            serialMonitoring: monitoring,
          });
        } catch (e2) {
          log("err", `[md] ${e2?.message || e2}`);
        }
      }

      if (fs.existsSync(stopFile)) break;
      setHint("processing", `已触发 ${trigger}：数据处理中（同时启动示波器与串口）`, {
        cycle,
        trigger,
        filename,
      });
      await startScopeAndSerial(live.device_id);

      setHint(
        "captured",
        `本轮完成：已保存 ${path.basename(String(saved))}${pdfPath ? " / PDF" : ""}${mdPath ? " / 总报告MD" : ""}，进入下一轮`,
        {
          cycle,
          trigger,
          filename: path.basename(String(saved)),
          pdf: pdfPath,
          summary_md: mdPath,
          serial_monitoring: await serialStatus(),
        },
      );
    }

    setHint("stopped", "监控已停止", { cycle });
    return { ok: true, summary_md: summaryMd, session: sessionStamp };
  } catch (e) {
    const msg = String(e?.message || e);
    log("err", `[fatal] ${msg}`);
    try {
      setHint("stopped", `异常停止：${msg}`, { cycle, error: msg });
    } catch {
      /* ignore */
    }
    return { ok: false, step: "fatal", error: msg, summary_md: summaryMd, session: sessionStamp };
  } finally {
    releaseLock();
    if (scope?.device_id != null) {
      try {
        await startScopeAndSerial(scope.device_id);
      } catch {
        /* best-effort restore */
      }
    }
  }
}

// Standalone: prefer runner.mjs; this path still expands templates for parity.
const isDirect =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isDirect) {
  const argConfig = process.argv.find((a) => a.startsWith("--config="))?.slice(9);
  const pth = path.resolve(
    argConfig || process.env.WIPARSE_STATION_CONFIG || path.join(here, "station.json"),
  );
  import("../../lib/plugin-contract.mjs")
    .then(({ expandTemplates, resolveDataRoot, projectRoot, colocateStatusWithIsf, colocateSummaryWithReport }) => {
      const loaded = JSON.parse(fs.readFileSync(pth, "utf8"));
      const dataRoot = resolveDataRoot(process.env.WIPARSE_DATA_ROOT || projectRoot());
      const config = expandTemplates(loaded, { dataRoot, pluginDir: here });
      colocateStatusWithIsf(config);
      for (const key of ["lock_file", "stop_file", "status_file", "isf_dir", "report_dir"]) {
        const v = config.paths?.[key];
        if (typeof v === "string" && v && !path.isAbsolute(v)) {
          config.paths[key] = path.resolve(here, v);
        }
      }
      colocateSummaryWithReport(config);
      return runLoop(config, {
        configPath: pth,
        preflightOnly: process.argv.includes("--preflight"),
      });
    })
    .then((r) => {
      if (r && r.ok === false) process.exit(1);
    })
    .catch((e) => {
      console.error(e);
      process.exit(1);
    });
}
