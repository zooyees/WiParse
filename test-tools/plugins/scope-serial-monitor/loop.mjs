/**
 * Production capture loop: serial trigger rising-edge → delay →
 * ScopeStop + serial stop → screenshot + ISF + PDF → ScopeRun + serial start.
 *
 * Export runLoop(cfg, opts) for the standard test-tool plugin contract.
 * Do not call test.start in the capture window: it reopens the serial monitor.
 */
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
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
import {
  createLineClassifier,
  resolveTriggerBlock,
  triggerSpecPath,
} from "../../lib/line-triggers.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));

export function probeWritableDirectory(dir) {
  fs.mkdirSync(dir, { recursive: true });
  const probe = path.join(dir, `.wiparse-write-probe-${randomUUID()}`);
  const fd = fs.openSync(probe, "wx");
  try { fs.writeFileSync(fd, "ok"); } finally { try { fs.closeSync(fd); } finally { fs.unlinkSync(probe); } }
}

export async function ensureNoActiveTest(invokeFn) {
  const response = await invokeFn("test.status", {}, 15_000, {allowFail: true});
  if (response?.ok !== true || typeof response.data?.running !== "boolean") throw new Error("无法确认其他测试状态");
  if (response.data.running) throw new Error("其他测试正在运行，请先停止后再启动监控");
}

export function targetSerialSession(status, target) {
  if (status?.ok !== true || !Array.isArray(status.data?.sessions)) throw new Error("无法确认目标串口状态");
  return status.data.sessions.find(s => String(s?.port || "").toLowerCase() === String(target).toLowerCase()) || null;
}

export async function awaitInstrumentJob(invokeFn,deviceId,accepted,timeoutMs=30_000) {
  const job=accepted?.data?.job_id;
  if(accepted?.ok!==true || !Number.isSafeInteger(job) || Number(accepted.data.device_id)!==Number(deviceId)) {
    const error=new Error("仪器任务未确认提交");error.code="INSTRUMENT.UNKNOWN";throw error;
  }
  const deadline=performance.now()+timeoutMs;
  while(performance.now()<deadline) {
    let response;
    try {response=await invokeFn("instrument.job.get",{job_id:job},Math.min(5000,Math.max(1,deadline-performance.now())),{allowFail:true});}
    catch {const error=new Error("无法确认仪器任务状态");error.code="INSTRUMENT.UNKNOWN";throw error;}
    if(response?.ok!==true || !response.data || response.data.job_id!==job){const error=new Error("仪器任务状态无效");error.code="INSTRUMENT.UNKNOWN";throw error;}
    if(response.data.state==="completed") {
      if(Number(response.data.device_id)!==Number(deviceId)){const error=new Error("仪器任务对应设备已变化");error.code="INSTRUMENT.UNKNOWN";throw error;}
      if(response.data.ok!==true){const error=new Error(`仪器任务失败：${response.data.detail?.error || response.data.detail?.message || '未知错误'}`);error.code="INSTRUMENT.FAILED";throw error;}
      return response.data;
    }
    if(response.data.state!=="pending_or_unknown"){const error=new Error("仪器任务状态无效");error.code="INSTRUMENT.UNKNOWN";throw error;}
    await sleep(Math.min(200,Math.max(1,deadline-performance.now())));
  }
  const error=new Error("仪器任务超时，设备状态待确认");error.code="INSTRUMENT.UNKNOWN";throw error;
}

export function selectSourceFile(dir,files) {
  const root=fs.realpathSync(dir);
  return (Array.isArray(files)?files:[]).map(file=>{try{return fs.realpathSync(String(file));}catch{return null;}}).find(file=>{
    if(!file)return false;const rel=path.relative(root,file);
    if(!rel||rel.startsWith(`..${path.sep}`)||rel==='..'||path.isAbsolute(rel))return false;
    try{const stat=fs.statSync(file);return stat.isFile()&&stat.size>0;}catch{return false;}
  }) || null;
}

/** @type {any} */
let cfg;
/** @type {string} */
let cfgPath = path.join(here, "station.json");
let api = "";
let outDir = "";
let reportDir = "";
let shotDir = "";
let docsDir = "";
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
/** When Hub/CLI overlays `triggers`, do not reload items from disk. */
let pinTriggers = false;
/** @type {ReturnType<typeof createLineClassifier> | null} */
let classifier = null;
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


let compiledTriggers = [];
let triggerStamp = '';

function currentTriggerBlock() {
  return resolveTriggerBlock(cfg, { configPath: cfgPath, pluginDir: here });
}

function rebuildClassifier(block) {
  risingEdge = cfg?.serial_triggers?.rising_edge !== false;
  classifier = createLineClassifier(block, { risingEdge });
  compiledTriggers = classifier.compiled;
  return compiledTriggers;
}

function loadTriggersFromDisk() {
  if (!pinTriggers && cfgPath && fs.existsSync(cfgPath)) {
    try {
      const data = JSON.parse(fs.readFileSync(cfgPath, "utf8"));
      const diskBlock = data.serial_triggers || data.triggers;
      if (diskBlock && typeof diskBlock === "object") {
        cfg.serial_triggers = { ...(cfg.serial_triggers || {}), ...diskBlock };
      }
    } catch {
      /* keep in-memory block */
    }
  }
  const block = currentTriggerBlock();
  const specPath = triggerSpecPath(block, { configPath: cfgPath, pluginDir: here }) || cfgPath;
  const compiled = rebuildClassifier(block);
  const st = fs.existsSync(specPath) ? fs.statSync(specPath) : { mtimeMs: 0 };
  triggerStamp = `${specPath}|${st.mtimeMs}|${compiled.map((t) => t.id).join(",")}`;
  return compiled;
}

function refreshTriggers(force = false) {
  if (pinTriggers && !force) return compiledTriggers;
  try {
    const block = currentTriggerBlock();
    const specPath = triggerSpecPath(block, { configPath: cfgPath, pluginDir: here }) || cfgPath;
    const st = fs.existsSync(specPath) ? fs.statSync(specPath) : { mtimeMs: 0 };
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
  return classifier?.labels() || compiledTriggers.map((t) => t.label).join(' / ') || '(无)';
}

function classify(line) {
  return classifier ? classifier.classify(line) : null;
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

async function serialSession() {
  const st = await invoke("serial.monitor.status", {}, 30_000, { allowFail: true });
  return targetSerialSession(st,port);
}
async function serialStatus() {
  const session = await serialSession();
  return Boolean(session?.confirmed && Number(session.baud) === Number(baud));
}

async function serialStop() {
  const result = await invoke("serial.monitor.stop", {port}, 30_000, { allowFail: true });
  if (result.ok !== true) throw new Error("无法停止目标串口");
  return result;
}

async function serialStopUntilOff(ms = cfg.timing.serial_stop_timeout_ms) {
  const t0 = Date.now();
  let last = true;
  try {
    while (Date.now() - t0 < ms) {
      await serialStop();
      last = Boolean(await serialSession());
      if (!last) return false;
      await sleep(120);
    }
  } catch(error) {retainLock=true;throw error;}
  if(last)retainLock=true;
  return last;
}

async function serialStart() {
  try {
    let r = await invoke("serial.monitor.start", { port, baud }, 30_000, { allowFail: true });
    if (r.ok === false) r = await invoke("serial.start", { port, baud }, 30_000, { allowFail: true });
    const t0 = Date.now();
    while (Date.now() - t0 < 4000) {
      if (await serialStatus()) return r;
      await sleep(120);
    }
    throw new Error(`目标串口未确认启动 (${port}@${baud})`);
  } catch(error) {retainLock=true;throw error;}
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
    if (await serialStopUntilOff()) throw new Error("目标串口未能停止，设备状态待确认");
  };
}

async function scopeCommand(deviceId, command) {
  return confirmedInstrument("instrument.command", { device_id: deviceId, command, timeout_s: 20 }, 30_000);
}

async function confirmedInstrument(method,params,timeoutMs) {
  try {
    const accepted=await invoke(method,params,30_000,{allowFail:true});
    return await awaitInstrumentJob(invoke,params.device_id,accepted,timeoutMs);
  } catch(error) {
    if(error?.code!=="INSTRUMENT.FAILED") retainLock=true;
    throw error;
  }
}

/** Wait until `instrument.capture` finishes writing PNG (HTTP returns accepted immediately). */
async function waitFile(file, timeoutMs, sinceMs = 0) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    if (fs.existsSync(stopFile)) return null;
    try {
      const st = fs.statSync(file);
      if (st.size > 32 && st.mtimeMs >= sinceMs - 2000) return file;
    } catch {
      /* not yet */
    }
    await sleep(pollMs);
  }
  return null;
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

/** Incremental live log cursor — avoids re-fetching thousands of lines each poll.
 *  `from_row` is an absolute session sequence. After the 5000-line FIFO wraps,
 *  WiParse still pages by that seq (`total` / `next_row` keep growing; `dropped`
 *  is how many lines left the window).
 */
export async function readTargetLines(invokeFn,target,fromRow,limit = 200) {
  const chunk = await invokeFn("log.lines.get", {
    port:target,
    from_row: fromRow,
    limit,
  });
  if (chunk?.ok !== true || !Array.isArray(chunk.data?.lines)) throw new Error("无法读取目标串口日志");
  const lines = chunk.data.lines.map(String);
  const total = Number(chunk.data?.total ?? fromRow + lines.length);
  const next = Number(chunk.data?.next_row ?? fromRow + lines.length);
  const dropped = Number(chunk.data?.dropped ?? 0);
  if (![total,next,dropped].every(Number.isSafeInteger) || total<0 || next<0 || dropped<0 || next>total) throw new Error("串口日志游标无效");
  return { lines, total, next, dropped };
}
async function liveLinesSince(fromRow, limit = 200) {
  return readTargetLines(invoke,port,fromRow,limit);
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
    const { lines, total, next, dropped } = await liveLinesSince(fromRow, 200);
    if (total < fromRow || dropped > fromRow || next < fromRow || (next === fromRow && total > fromRow)) throw new Error("串口日志发生清空、丢失或游标异常，监控已停止，请检查数据后重试");
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
  if (serial || await serialStopUntilOff()) throw new Error("目标串口未能停止，取消波形抓取");
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

async function captureShotAndIsf(deviceId, shotPath) {
  fs.mkdirSync(path.dirname(shotPath), { recursive: true });
  if (fs.existsSync(shotPath)) throw new Error("截图目标已存在，取消抓取以避免覆盖");
  const sinceMs = Date.now();
  // Prefer instrument.capture (HARDCopy PNG). scope.shot is deprecated local-VISA.
  const shot = await confirmedInstrument(
    "instrument.capture",
    { device_id: deviceId, path: shotPath, wait: true },
    cfg.timing.shot_timeout_ms,
  );
  const shotFile = await waitFile(shotPath, cfg.timing.shot_timeout_ms, sinceMs);
  if (!shotFile) throw new Error("截图任务结束但未找到本次截图");
  const src = await confirmedInstrument("instrument.waveform_source", {
    device_id: deviceId,
    dir: outDir,
    overwrite: false,
    timeout_s: Math.round(cfg.timing.isf_timeout_ms / 1000),
  }, cfg.timing.isf_timeout_ms);
  const isfPath=selectSourceFile(outDir,src.detail?.files);
  if (!isfPath) throw new Error("波形源任务结束但未找到本次波形文件");
  return {
    shot,
    shotFile: shotFile ? String(shotFile).replace(/^\\\\\?\\/, "") : null,
    waveform: src,
    isfPath,
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
  if (!rel) return "";
  // Reports live in artifacts/reports/; shots are siblings. Keep POSIX
  // separators so the Test Report preview can resolve ../shots/*.png.
  return rel.split(path.sep).join("/");
}

function mdFileLink(target) {
  if (!target) return "无";
  const rel = mdRelPath(summaryMd, String(target));
  const url = rel.split("/").map(encodeURIComponent).join("/");
  return `[${mdCell(path.basename(String(target)))}](${url})`;
}

function mdScreenshot(target) {
  if (!target) return "未生成波形截图";
  const rel = mdRelPath(summaryMd, String(target));
  const url = rel.split("/").map(encodeURIComponent).join("/");
  return `![波形截图](${url})`;
}

function materializeScreenshot(src, trigger, recStamp) {
  if (!src) return null;
  fs.mkdirSync(shotDir || reportDir, { recursive: true });
  const dest = path.join(shotDir || reportDir, `${prefix}_${trigger}_${recStamp}.png`);
  return copyIfExists(src, dest);
}

function appendSummaryRecord(rec) {
  ensureSummaryHeader();
  const recId = `${prefix}_${rec.trigger}_${rec.stamp}`;
  const ctx = (rec.context || []).map((l) => {
    const mark = l === rec.line ? " <<HIT>>" : "";
    return mdFence(l) + mark;
  }).join("\n");
  const dataPath = path.join(docsDir || reportDir, `${recId}.json`);
  const indexPath = path.join(docsDir || reportDir, "index.jsonl");
  const dataLinks = [
    ["记录数据", dataPath], ["采集索引", indexPath],
  ].filter(([, file]) => fs.existsSync(file))
    .map(([label, file]) => `| ${label} | ${mdFileLink(file)} |`).join("\n");
  const block = `
---

### 第 ${rec.cycle} 轮 · ${rec.trigger} · ${rec.timeText}

**记录 ID：** \`${recId}\`

| 项 | 值 |
|---|---|
| 轮次 | ${mdCell(rec.cycle)} |
| 触发条件 | ${mdCell(rec.trigger)} |
| 触发时间 | ${mdCell(rec.timeText)} |
| 命中行 | ${mdCell(rec.line)} |
| 处理时串口 | monitoring=${mdCell(rec.serialMonitoring)} |
| 示波器 | ${mdCell(rec.scopeModel)} id=${mdCell(rec.scopeId)} |
| 波形源 | ${mdFileLink(rec.isfPath)} |
| PDF | ${mdFileLink(rec.pdfPath)} |
${dataLinks}

#### 波形截图

${mdScreenshot(rec.screenshot)}

#### Log 上下文

~~~
${ctx}
~~~

`;
  fs.appendFileSync(summaryMd, block, "utf8");
  return summaryMd;
}

function writePdfReport(rec) {
  fs.mkdirSync(docsDir || reportDir, { recursive: true });
  const base = `${prefix}_${rec.trigger}_${rec.stamp}`;
  const htmlPath = path.join(docsDir || reportDir, `${base}.html`);
  const pdfPath = path.join(docsDir || reportDir, `${base}.pdf`);
  const jsonPath = path.join(docsDir || reportDir, `${base}.json`);
  const shotDest = rec.screenshotSrc
    ? copyIfExists(rec.screenshotSrc, path.join(shotDir || reportDir, `${base}.png`))
    : rec.screenshot || null;
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
  const indexPath = path.join(docsDir || reportDir, "index.jsonl");
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

let lockToken = null;
let retainLock = false;
export function acquireMonitorLock(file, details) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const token=randomUUID();
  let fd;
  try {fd=fs.openSync(file,"wx");}
  catch(error) {if(error.code==="EEXIST")throw new Error("监控锁已存在，请确认任务已结束并检查锁文件后再恢复");throw error;}
  try {fs.writeFileSync(fd,JSON.stringify({...details,token},null,2),"utf8");fs.fsyncSync(fd);}
  finally {fs.closeSync(fd);}
  return token;
}
function acquireLock() {
  lockToken=acquireMonitorLock(lockFile,{pid:process.pid,started:new Date().toISOString(),config:cfgPath,sop:cfg.sop});
}

function releaseLock() {
  if(retainLock) return;
  try {
    if (!fs.existsSync(lockFile)) return;
    const prev = JSON.parse(fs.readFileSync(lockFile, "utf8"));
    if (lockToken && prev.pid === process.pid && prev.token === lockToken) fs.unlinkSync(lockFile);
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
    const canWrite = (dir, id) => {
      if (typeof dir !== "string" || !dir.trim() || dir.includes("{")) {
        push(id, true, "deferred until run (no stamp)");
        return;
      }
      probeWritableDirectory(dir);
      push(id, true, dir);
    };
    canWrite(cfg.paths?.test_dir, "test_dir");
    if (!preflightOnly) {
      canWrite(outDir, "isf_dir");
      canWrite(reportDir, "report_dir");
    }
    push("status_file", Boolean(statusFile && !String(statusFile).includes("{")), statusFile);
  } catch (e) {
    push("dirs", false, String(e.message || e));
  }

  // Single-instance lock
  if (cfg.interlocks?.single_instance !== false) {
    if (fs.existsSync(lockFile)) {
      push("single_instance", false, "监控锁已存在；请确认任务已结束并检查锁文件");
    } else {
      push("single_instance", true, "free");
    }
  }

  // Serial API reachability (does not require monitoring already on)
  try {
    const st = await invoke("serial.monitor.status", {}, 15_000, { allowFail: true });
    const session=targetSerialSession(st,port);
    if(session && (!session.confirmed || Number(session.baud)!==Number(baud))) push("serial_api",false,`目标串口已打开但尚未确认或波特率不一致 (${port}@${baud})`);
    else push("serial_api",true,session?`目标串口已就绪 (${port}@${baud})`:`待启动目标串口 (${port}@${baud})`);
  } catch (e) {
    push("serial_api", false, String(e.message || e));
  }

  try { await ensureNoActiveTest(invoke); push("test_idle", true, "没有其他活动测试"); }
  catch (e) { push("test_idle", false, String(e.message || e)); }
  try {
    const result=await invoke("instrument.job.get",{job_id:0},5000,{allowFail:true});
    const supported=result?.ok===true && result.data?.state==="pending_or_unknown" && result.data?.job_id===0;
    push("instrument_job_api",supported,supported?"支持任务完成确认":"请更新 WiParse：缺少仪器任务确认接口");
  } catch(e) {push("instrument_job_api",false,`无法确认仪器任务接口：${e.message || e}`);}

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
  retainLock=false;lockToken=null;
  if (opts.configPath) cfgPath = opts.configPath;
  colocateStatusWithIsf(cfg);
  colocateSummaryWithReport(cfg);
  api = String(cfg.gui?.api || "http://127.0.0.1:7878").replace(/\/$/, "");
  outDir = cfg.paths.isf_dir;
  reportDir = cfg.paths.report_dir;
  shotDir =
    cfg.paths.shot_dir ||
    (cfg.paths.artifacts_dir && !String(cfg.paths.artifacts_dir).includes("{")
      ? path.join(cfg.paths.artifacts_dir, "shots")
      : reportDir);
  docsDir =
    cfg.paths.docs_dir ||
    (cfg.paths.artifacts_dir && !String(cfg.paths.artifacts_dir).includes("{")
      ? path.join(cfg.paths.artifacts_dir, "docs")
      : reportDir);
  stopFile = cfg.paths.stop_file;
  statusFile = cfg.paths.status_file;
  hintFile = statusFile
    ? path.join(path.dirname(statusFile), "_loop_hint.txt")
    : "";
  lockFile = cfg.paths.lock_file;
  prefix = cfg.paths.file_prefix;
  sessionStamp = String(opts.stamp || "").trim() || stamp();
  summaryMd = (cfg.paths.summary_md || path.join(reportDir, `${prefix}_summary_{stamp}.md`)).replaceAll(
    "{stamp}",
    sessionStamp
  );
  if (reportDir && !String(reportDir).includes("{")) {
    summaryMd = path.join(reportDir, path.basename(summaryMd));
  }
  port = cfg.serial.port;
  baud = cfg.serial.baud;
  delayS = cfg.timing.post_trigger_delay_s;
  settleMs = Math.round((cfg.timing.scope_settle_s ?? 1.5) * 1000);
  pollMs = cfg.timing.poll_ms ?? 400;
  browsers = cfg.pdf?.browsers || [];
  preflightOnly = Boolean(opts.preflightOnly);
  externalLog = typeof opts.log === "function" ? opts.log : undefined;
  risingEdge = cfg.serial_triggers?.rising_edge !== false;
  pinTriggers = Boolean(opts.pinTriggers);
  holdSerialOffEnabled = cfg.interlocks?.hold_serial_off_during_capture !== false;
  http = createHttpClient({ url: api, log: externalLog });
  rebuildClassifier(currentTriggerBlock());
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
  const reportErrors = [];
  try {
    if (fs.existsSync(stopFile)) fs.unlinkSync(stopFile);
    fs.mkdirSync(outDir, { recursive: true });
    fs.mkdirSync(reportDir, { recursive: true });
    if (shotDir) fs.mkdirSync(shotDir, { recursive: true });
    if (docsDir) fs.mkdirSync(docsDir, { recursive: true });

    if (cfg.interlocks?.never_test_start_during_capture !== false) {
      await ensureNoActiveTest(invoke);
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
      trigger_spec: classifier?.spec() || compiledTriggers.map((t) => ({
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
      const ts = `${stamp(at)}_${randomUUID().slice(0,8)}`;
      setHint(
        "processing",
        `已触发 ${trigger}：数据处理中（延时 ${delayS * 1000}ms / 同时停示波器与串口 / 截图 / 存源文件 / 出报告）`,
        { cycle, trigger },
      );

      await sleep(delayS * 1000);
      if (fs.existsSync(stopFile)) break;
      setHint("processing", `已触发 ${trigger}：数据处理中（同时暂停示波器与串口）`, {
        cycle,
        trigger,
      });
      const releaseSerial = holdSerialOff();
      let saved = null;
      let shot = null;
      let monitoring = true;
      try {
        const stopped = await stopScopeAndSerial(live.device_id);
        monitoring = stopped.monitoring;
        setHint("processing", `已触发 ${trigger}：串口已停 monitoring=${monitoring}，截图并读源文件`, {
          cycle,
          trigger,
          serial_monitoring: monitoring,
        });
        const shotPath = path.join(shotDir || reportDir, `${prefix}_${trigger}_${ts}.png`);
        const cap = await captureShotAndIsf(live.device_id, shotPath);
        saved = cap.isfPath;
        shot = cap.shotFile;
        monitoring = await serialStatus();
        setHint(
          "processing",
          `已触发 ${trigger}：数据处理中（生成 PDF 报告；串口 monitoring=${monitoring}）`,
          { cycle, trigger, serial_monitoring: monitoring },
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
        reportErrors.push(String(e?.message || e));
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
    return { ok: reportErrors.length===0, summary_md: summaryMd, session: sessionStamp, ...(reportErrors.length?{error:`报告生成失败：${reportErrors.join('; ')}`}:{}) };
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
    if (scope?.device_id != null && !retainLock) {
      try {
        await startScopeAndSerial(scope.device_id);
      } catch (error) {
        retainLock=true;
        log("err",`恢复设备失败，保留监控锁供检查：${error?.message || error}`);
        throw error;
      }
    }
    releaseLock();
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
    .then(({ loadStationConfig, makeStamp }) => {
      const lifecycle = process.argv.includes("--preflight") ? "preflight" : "run";
      const stamp = process.env.WIPARSE_STAMP || (lifecycle === "run" ? makeStamp() : "");
      const { config, configPath, layout } = loadStationConfig({
        plugin: { id: "scope-serial-monitor", dir: here, params: [] },
        pluginDir: here,
        args: {},
        dataRoot: process.env.WIPARSE_DATA_ROOT,
        project: process.env.WIPARSE_PROJECT,
        testId: process.env.WIPARSE_TEST_ID,
        configPath: pth,
        lifecycle,
        stamp,
      });
      return runLoop(config, {
        configPath,
        preflightOnly: lifecycle === "preflight",
        stamp: layout.stamp || stamp,
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
