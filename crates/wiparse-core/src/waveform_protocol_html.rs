//! Protocol analysis page opened in the system browser.
//! WiParse decodes the waveform. This page draws the samples, timing, and fields.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

#[derive(Clone, Debug)]
pub struct ProtocolReport {
    pub zh: bool,
    pub sources: Vec<String>,
    pub byte_limit: usize,
    /// Lane id and frame index to select on open.
    pub focus: Option<(u64, usize)>,
    pub waves: Vec<ProtocolReportWave>,
    pub lanes: Vec<ProtocolReportLane>,
}

#[derive(Clone, Debug)]
pub struct ProtocolReportWave {
    pub label: String,
    pub unit: String,
    /// Coarse `[x0, x1, y_min, y_max]` columns for the whole capture.
    pub overview: Vec<[f64; 4]>,
    /// Fine columns around decoded frames, so zoom can show edges.
    pub detail: Vec<[f64; 4]>,
}

impl ProtocolReportWave {
    pub fn new(label: String, unit: String, overview: Vec<[f64; 4]>, detail: Vec<[f64; 4]>) -> Self {
        Self { label, unit, overview, detail }
    }
}

#[derive(Clone, Debug)]
pub struct ProtocolReportLane {
    pub id: u64,
    pub title: String,
    pub channels: String,
    pub wave: Option<usize>,
    pub ready: bool,
    pub on_plot: bool,
    pub brief: String,
    pub info: String,
    pub error: Option<String>,
    pub truncated: bool,
    pub frames: Vec<ProtocolReportFrame>,
}

#[derive(Clone, Debug)]
pub struct ProtocolReportFrame {
    pub index: usize,
    pub t0: f64,
    pub t1: f64,
    pub summary: String,
    pub error: bool,
    pub fields: Vec<ProtocolReportField>,
}

#[derive(Clone, Debug)]
pub struct ProtocolReportField {
    pub name: String,
    pub value: String,
    pub code: String,
    pub raw: bool,
    pub note: String,
}

pub fn render_protocol_report(report: &ProtocolReport) -> String {
    let zh = report.zh;
    let title = if zh { "WiParse 协议分析" } else { "WiParse protocol analysis" };
    let mut html = String::with_capacity(128 * 1024);
    html.push_str("<!DOCTYPE html><html lang=\"");
    html.push_str(if zh { "zh-CN" } else { "en" });
    html.push_str("\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>");
    html.push_str(title);
    html.push_str("</title><style>");
    html.push_str(REPORT_CSS);
    html.push_str("</style></head><body><header><div class=\"brand\"><h1>");
    html.push_str(title);
    html.push_str("</h1><p class=\"meta\" id=\"source\"></p></div><div class=\"tools\"><input id=\"q\" type=\"search\" placeholder=\"");
    html.push_str(if zh { "筛选帧或字段" } else { "Filter frames or fields" });
    html.push_str("\"><div class=\"seg\"><button type=\"button\" id=\"zin\">");
    html.push_str(if zh { "放大" } else { "Zoom in" });
    html.push_str("</button><button type=\"button\" id=\"zout\">");
    html.push_str(if zh { "缩小" } else { "Zoom out" });
    html.push_str("</button><button type=\"button\" id=\"framezoom\">");
    html.push_str(if zh { "此帧" } else { "This frame" });
    html.push_str("</button><button type=\"button\" id=\"fit\">");
    html.push_str(if zh { "整段" } else { "Fit" });
    html.push_str("</button></div><button type=\"button\" id=\"clear\">");
    html.push_str(if zh { "清除光标" } else { "Clear cursors" });
    html.push_str("</button></div></header><main><section class=\"stage\"><canvas id=\"wave\"></canvas><p class=\"hint\">");
    html.push_str(if zh {
        "滚轮缩放，拖拽平移，按住 Shift 拖出范围放大。协议帧附近按采样点保留边沿；点一帧可看到时序、码值和换算后的字段。"
    } else {
        "Scroll to zoom, drag to pan, Shift-drag to zoom into a range. Samples are kept around frames; select a frame for timing, codes, and scaled fields."
    });
    html.push_str("</p><section id=\"readout\"></section></section><section class=\"work\"><div id=\"lanes\"></div><div class=\"frames\"><div class=\"listhead\"><span>#</span><span>");
    html.push_str(if zh { "时间" } else { "Time" });
    html.push_str("</span><span>");
    html.push_str(if zh { "帧" } else { "Frame" });
    html.push_str("</span></div><div id=\"frames\"><div id=\"spacer\"></div><div id=\"rows\"></div></div></div><section id=\"detail\"></section></section></main><script type=\"application/json\" id=\"data\">");
    html.push_str(&report_json(report));
    html.push_str("</script><script>");
    html.push_str(REPORT_SCRIPT);
    html.push_str("</script></body></html>\n");
    html
}

pub fn write_protocol_html(slot: &mut Option<PathBuf>, html: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("wiparse");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(old) = slot.take() {
        let _ = fs::remove_file(old);
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("protocol-{millis}.html"));
    fs::write(&path, html).map_err(|e| e.to_string())?;
    *slot = Some(path.clone());
    Ok(path)
}

fn series_json(columns: &[[f64; 4]]) -> Vec<Vec<f64>> {
    columns.iter().filter(|col| col.iter().all(|value| value.is_finite())).map(|col| col.to_vec()).collect()
}

fn report_json(report: &ProtocolReport) -> String {
    let waves = report.waves.iter().map(|wave| {
        json!({"label": wave.label, "unit": wave.unit, "overview": series_json(&wave.overview), "detail": series_json(&wave.detail)})
    }).collect::<Vec<_>>();
    let lanes = report.lanes.iter().map(|lane| {
        let frames = lane.frames.iter().map(|frame| {
            json!({
                "i": frame.index,
                "t0": frame.t0,
                "t1": frame.t1,
                "summary": frame.summary,
                "error": frame.error,
                "fields": frame.fields.iter().map(|field| json!({"n": field.name, "v": field.value, "c": field.code, "raw": field.raw, "note": field.note})).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>();
        json!({
            "id": lane.id,
            "title": lane.title,
            "channels": lane.channels,
            "wave": lane.wave,
            "ready": lane.ready,
            "active": lane.on_plot,
            "brief": lane.brief,
            "info": lane.info,
            "error": lane.error,
            "truncated": lane.truncated,
            "frames": frames,
        })
    }).collect::<Vec<_>>();
    let focus = report.focus.map(|(lane, frame)| json!([lane, frame]));
    let text = serde_json::to_string(&json!({
        "zh": report.zh,
        "sources": report.sources,
        "byteLimit": report.byte_limit,
        "focus": focus,
        "waves": waves,
        "lanes": lanes,
    })).unwrap_or_else(|_| "{}".into());
    text.replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

const REPORT_CSS: &str = r##"
:root { color-scheme: light; --bg:#eef1f4; --card:#fff; --ink:#1c2430; --muted:#5e6d7e; --line:#e3e8ee; --accent:#0b6bcb; --soft:#e7f2fc; --plot:#fbfcfd; --bad:#b42318; --bad-bg:#fdecec; --ok:#067647; --chip:#f4f7fa; }
* { box-sizing: border-box; }
html, body { height: 100%; }
body { margin:0; padding:12px; height:100%; display:grid; gap:12px; grid-template-rows:auto minmax(0,1fr); background:var(--bg); color:var(--ink); font:13px/1.45 "Segoe UI","Microsoft YaHei",sans-serif; }
main { min-height:0; display:grid; grid-template-columns:minmax(0,1.15fr) minmax(420px,1fr); gap:12px; }
header, .stage, .work { background:var(--card); border:1px solid var(--line); border-radius:10px; min-width:0; }
header { display:flex; justify-content:space-between; gap:12px 16px; align-items:center; flex-wrap:wrap; padding:10px 14px; }
h1 { margin:0; font-size:16px; font-weight:650; letter-spacing:0; }
.meta { margin:2px 0 0; color:var(--muted); font-size:12px; }
.tools, .seg { display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
.tools { justify-content:flex-end; }
.seg { gap:0; }
input, button { font:inherit; color:inherit; background:var(--card); border:1px solid var(--line); }
input { width:min(220px,100%); padding:6px 10px; border-radius:8px; }
button { padding:6px 12px; cursor:pointer; background:var(--chip); }
.seg button { border-radius:0; margin-left:-1px; }
.seg button:first-child { margin-left:0; border-radius:8px 0 0 8px; }
.seg button:last-child { border-radius:0 8px 8px 0; }
button:hover { background:#eaf0f6; }
#readout { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); border-top:1px solid var(--line); }
#readout div { min-width:0; padding:10px 12px; border-left:1px solid var(--line); border-top:1px solid var(--line); }
#readout div:nth-child(4n+1) { border-left:0; }
#readout div:nth-child(-n+4) { border-top:0; }
#readout span { display:block; color:var(--muted); font-size:11px; }
#readout b { display:block; margin-top:2px; font-size:15px; font-weight:640; font-variant-numeric:tabular-nums; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.stage, .work { min-height:0; display:flex; flex-direction:column; overflow:hidden; }
.work { display:grid; grid-template-rows:auto minmax(0,1fr) minmax(0,1.15fr); }
canvas { flex:1; width:100%; min-height:220px; display:block; background:var(--plot); cursor:crosshair; touch-action:none; border-radius:10px 10px 0 0; }
.hint { margin:0; padding:8px 12px; color:var(--muted); font-size:12px; border-top:1px solid var(--line); }
#lanes { display:flex; gap:8px; padding:10px; overflow:auto; border-bottom:1px solid var(--line); }
.lanebtn { min-width:148px; text-align:left; border-radius:8px; padding:6px 8px; background:#fff; }
.lanebtn.on { border-color:var(--accent); background:var(--soft); color:#0b4f96; }
.lanebtn small, .lanebtn b { display:block; }
.lanebtn b { font-weight:640; }
.lanebtn small { color:var(--muted); font-size:11px; white-space:nowrap; }
.listhead, .frow { display:grid; grid-template-columns:3rem 6.4rem minmax(0,1fr); gap:8px; align-items:center; }
.frames { min-height:0; display:flex; flex-direction:column; border-bottom:1px solid var(--line); }
.listhead { padding:8px 12px; color:var(--muted); font-size:11px; border-bottom:1px solid var(--line); }
#frames { position:relative; flex:1; min-height:0; overflow:auto; }
#rows { position:absolute; left:0; right:0; top:0; }
.frow { width:100%; height:32px; padding:0 12px; border:0; border-left:2px solid transparent; border-radius:0; background:transparent; text-align:left; }
.frow.on { background:var(--soft); border-left-color:var(--accent); }
.frow.bad .sum { color:var(--bad); }
.idx, .time { color:var(--muted); font-variant-numeric:tabular-nums; }
.sum { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
#detail { min-width:0; min-height:0; overflow:auto; padding:16px 20px 28px; }
.kicker { margin:0; color:var(--accent); font-size:12px; font-weight:650; }
#detail h2 { margin:2px 0 0; font-size:18px; font-weight:650; }
.brief { margin:8px 0 0; max-width:72ch; color:#31404f; }
.params { margin:4px 0 0; color:var(--muted); }
.when { display:grid; grid-template-columns:1fr; gap:10px; margin:14px 0 4px; padding:12px; background:var(--chip); border-radius:8px; }
.bar { display:flex; height:18px; border-radius:5px; overflow:hidden; background:#dbe3ec; }
.bar i { display:block; height:100%; }
.bar .gap { background:#b7c3d0; }
.bar .pulse { background:var(--accent); }
.bar .pulse.bad { background:var(--bad); }
.metrics { display:grid; grid-template-columns:repeat(5,minmax(0,1fr)); gap:8px; }
.metrics span { display:block; color:var(--muted); font-size:11px; }
.metrics b { font-weight:640; font-variant-numeric:tabular-nums; }
.field { display:grid; grid-template-columns:minmax(132px,200px) minmax(0,1fr); gap:2px 16px; padding:10px 0; border-top:1px solid var(--line); }
.field h3 { margin:0; font-size:13px; font-weight:640; color:#3d4c5c; }
.field .value { margin:0; font-size:15px; font-weight:640; overflow-wrap:anywhere; }
.field .note { grid-column:1 / -1; margin:0; color:var(--muted); font-size:12px; }
.field.raw .value { font-family:Consolas,"Cascadia Mono",monospace; font-size:13px; font-weight:500; }
.field.bad { background:var(--bad-bg); margin:0 -8px; padding:10px 8px; border-radius:6px; }
table.fields { width:100%; border-collapse:collapse; margin-top:8px; }
table.fields th { font-size:11px; color:var(--muted); font-weight:640; padding:8px; border-bottom:1px solid var(--line); text-align:left; }
table.fields th:nth-child(1), table.fields td:nth-child(1) { width:18%; }
table.fields th:nth-child(2), table.fields td:nth-child(2) { width:28%; }
table.fields th:nth-child(3), table.fields td:nth-child(3) { width:22%; }
table.fields th:nth-child(4), table.fields td:nth-child(4) { width:32%; }
table.fields td { padding:8px; border-top:1px solid var(--line); vertical-align:top; overflow-wrap:anywhere; }
table.fields td.code { font-family:Consolas,"Cascadia Mono",monospace; font-size:12px; color:#0b4f96; }
table.fields tr.bad td { background:var(--bad-bg); }
table.fields tr.ok td.actual { color:var(--ok); }
.wire { display:flex; flex-wrap:wrap; gap:6px; margin:12px 0 4px; }
.chip { border:1px solid var(--line); border-radius:8px; padding:6px 8px; background:#f7f9fb; min-width:92px; max-width:220px; }
.chip b, .chip em, .chip span { display:block; font-style:normal; }
.chip b { font-size:12px; }
.chip em { margin-top:2px; font-family:Consolas,"Cascadia Mono",monospace; font-size:12px; color:#0b4f96; }
.chip span { margin-top:2px; color:var(--muted); font-size:11px; }
.callout { margin:8px 0 0; color:var(--bad); }
@media (max-width:1100px) {
  body { height:auto; grid-template-rows:auto auto; }
  main { grid-template-columns:1fr; }
  .stage { min-height:520px; }
  .work { min-height:680px; }
  #readout, .metrics { grid-template-columns:repeat(2,minmax(0,1fr)); }
  #readout div:nth-child(4n+1) { border-left:1px solid var(--line); }
  #readout div:nth-child(odd) { border-left:0; }
  #readout div:nth-child(-n+2) { border-top:0; }
}
"##;

const REPORT_SCRIPT: &str = r###"
const data = JSON.parse(document.getElementById("data").textContent);
const zh = !!data.zh;
const canvas = document.getElementById("wave");
const ctx = canvas.getContext("2d");
const lanesEl = document.getElementById("lanes");
const framesEl = document.getElementById("frames");
const spacer = document.getElementById("spacer");
const rows = document.getElementById("rows");
const detail = document.getElementById("detail");
const readout = document.getElementById("readout");
const colors = ["#0b6bcb", "#b45309", "#0f766e", "#9f1239", "#6d28d9", "#0369a1", "#3f6212", "#9a3412"];
const full = extent();
let view = { x0: full[0], x1: full[1] };
let cursorA = null, cursorB = null, hover = null;
let laneFilter = 0;
let selected = null;
let items = [];
let drag = null;
document.getElementById("source").textContent = (data.sources || []).join("  ·  ");
data.lanes.forEach((lane, index) => {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "lanebtn" + (index === 0 ? " on" : "");
  btn.innerHTML = "<b></b><small></small>";
  btn.querySelector("b").textContent = lane.title || (zh ? "未命名" : "Untitled");
  const bits = [lane.channels, lane.frames.length + (zh ? " 帧" : " frames"), lane.active ? (zh ? "当前" : "current") : "", lane.truncated ? (zh ? "已截断" : "truncated") : ""].filter(Boolean);
  btn.querySelector("small").textContent = bits.join("  ·  ");
  btn.onclick = () => { laneFilter = index; selected = null; markLanes(); rebuild(); };
  lanesEl.appendChild(btn);
});
function markLanes() {
  document.querySelectorAll(".lanebtn").forEach((el, i) => el.classList.toggle("on", i === laneFilter));
}
if (data.focus) {
  const lane = data.lanes.findIndex((item) => item.id === data.focus[0]);
  if (lane >= 0) {
    laneFilter = lane;
    markLanes();
    const frame = data.lanes[lane].frames.findIndex((item) => item.i === data.focus[1]);
    if (frame >= 0) selected = { lane, frame };
  }
}
function rebuild() {
  const needle = document.getElementById("q").value.trim().toLowerCase();
  const lane = data.lanes[laneFilter];
  items = [];
  if (lane) lane.frames.forEach((frame, frameIndex) => {
    const hay = (frame.summary + " " + frame.fields.map((field) => field.n + " " + field.v + " " + (field.note || "")).join(" ")).toLowerCase();
    if (!needle || hay.includes(needle)) items.push(frameIndex);
  });
  spacer.style.height = (items.length * 32) + "px";
  paintList();
  paintDetail();
  draw();
}
function paintList() {
  const top = framesEl.scrollTop;
  const start = Math.max(0, Math.floor(top / 32) - 4);
  const end = Math.min(items.length, start + Math.ceil(framesEl.clientHeight / 32) + 8);
  rows.style.transform = "translateY(" + (start * 32) + "px)";
  rows.replaceChildren();
  const lane = data.lanes[laneFilter];
  for (let n = start; n < end; n++) {
    const frameIndex = items[n];
    const frame = lane.frames[frameIndex];
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "frow" + (frame.error ? " bad" : "") + (selected && selected.lane === laneFilter && selected.frame === frameIndex ? " on" : "");
    btn.innerHTML = '<span class="idx"></span><span class="time"></span><span class="sum"></span>';
    btn.querySelector(".idx").textContent = String(frame.i);
    btn.querySelector(".time").textContent = fmtTime(frame.t0);
    btn.querySelector(".sum").textContent = frame.summary;
    btn.onclick = () => choose(laneFilter, frameIndex, true);
    rows.appendChild(btn);
  }
}
framesEl.addEventListener("scroll", paintList);
function choose(lane, frame, zoom) {
  selected = { lane, frame };
  laneFilter = lane;
  markLanes();
  const item = data.lanes[lane].frames[frame];
  if (zoom && item) zoomFrame(item);
  const pos = items.indexOf(frame);
  if (pos >= 0) {
    const y = pos * 32;
    if (y < framesEl.scrollTop || y > framesEl.scrollTop + framesEl.clientHeight - 32) framesEl.scrollTop = Math.max(0, y - 96);
  }
  paintList();
  paintDetail();
  draw();
}
function zoomFrame(item) {
  const width = Math.max(item.t1 - item.t0, finest() * 32);
  const pad = Math.max(width * 0.75, finest() * 16);
  view = clampView(item.t0 - pad, item.t1 + pad);
}
function paintDetail() {
  const lane = selected ? data.lanes[selected.lane] : data.lanes[laneFilter];
  const frame = selected && lane ? lane.frames[selected.frame] : null;
  detail.replaceChildren();
  if (!lane) return;
  const kicker = document.createElement("p");
  kicker.className = "kicker";
  kicker.textContent = lane.title + (lane.channels ? "   " + lane.channels : "");
  detail.appendChild(kicker);
  const title = document.createElement("h2");
  title.textContent = frame ? ("#" + frame.i + "   " + frame.summary) : (zh ? "选择一帧" : "Select a frame");
  detail.appendChild(title);
  if (lane.brief) { const p = document.createElement("p"); p.className = "brief"; p.textContent = lane.brief; detail.appendChild(p); }
  if (lane.info) { const p = document.createElement("p"); p.className = "params"; p.textContent = (zh ? "解码参数  " : "Decode  ") + lane.info; detail.appendChild(p); }
  if (lane.error) { const p = document.createElement("p"); p.className = "callout"; p.textContent = lane.error; detail.appendChild(p); }
  if (lane.truncated) { const p = document.createElement("p"); p.className = "callout"; p.textContent = zh ? ("只解码了前 " + data.byteLimit + " 字节，后面的帧不在这页。") : ("Only the first " + data.byteLimit + " bytes were decoded."); detail.appendChild(p); }
  if (!lane.ready) { const p = document.createElement("p"); p.className = "params"; p.textContent = zh ? "通道还没选齐，这一路未解码。" : "Channels are incomplete, so this lane was not decoded."; detail.appendChild(p); }
  if (!frame) return;
  const prev = lane.frames[selected.frame - 1];
  const next = lane.frames[selected.frame + 1];
  const width = Math.max(frame.t1 - frame.t0, 0);
  const before = prev ? frame.t0 - prev.t1 : NaN;
  const after = next ? next.t0 - frame.t1 : NaN;
  const when = document.createElement("div");
  when.className = "when";
  const bar = document.createElement("div");
  bar.className = "bar";
  const cap = Math.max(width, finest()) * 3;
  const parts = [
    [Math.min(Math.max(before, 0) || 0, cap), "gap"],
    [Math.max(width, cap * 0.15), frame.error ? "pulse bad" : "pulse"],
    [Math.min(Math.max(after, 0) || 0, cap), "gap"],
  ];
  const sum = parts.reduce((total, part) => total + part[0], 0) || 1;
  parts.forEach(([weight, kind]) => {
    if (weight <= 0) return;
    const piece = document.createElement("i");
    piece.className = kind;
    piece.style.width = (100 * weight / sum) + "%";
    bar.appendChild(piece);
  });
  const metrics = document.createElement("div");
  metrics.className = "metrics";
  [
    [zh ? "起点" : "Start", fmtTime(frame.t0)],
    [zh ? "终点" : "End", fmtTime(frame.t1)],
    [zh ? "脉宽" : "Width", fmtTime(width)],
    [zh ? "帧前间隔" : "Gap before", isFinite(before) ? fmtTime(before) : "—"],
    [zh ? "帧后间隔" : "Gap after", isFinite(after) ? fmtTime(after) : "—"],
  ].forEach(([name, value]) => {
    const box = document.createElement("div");
    const span = document.createElement("span"); span.textContent = name;
    const strong = document.createElement("b"); strong.textContent = value;
    box.append(span, strong); metrics.appendChild(box);
  });
  when.append(bar, metrics);
  detail.appendChild(when);
  const wire = frame.fields.filter((field) => /起始|停止|同步|帧头|前导|结束|编码|包头|读\/写|地址字节|数据位|采样码|命令|寄存器|应答|奇校验|波形字节|Start|Stop|Sync|Header|Preamble|EOP/.test(field.n) || /^b\d+$/.test(field.n));
  if (wire.length) {
    const strip = document.createElement("div");
    strip.className = "wire";
    wire.forEach((field) => {
      const chip = document.createElement("div");
      chip.className = "chip";
      const name = document.createElement("b"); name.textContent = field.n;
      const code = document.createElement("em"); code.textContent = field.c || "—";
      const actual = document.createElement("span"); actual.textContent = field.v || "";
      chip.append(name, code, actual);
      strip.appendChild(chip);
    });
    detail.appendChild(strip);
  }
  const table = document.createElement("table");
  table.className = "fields";
  const head = document.createElement("tr");
  [zh ? "字段" : "Field", zh ? "码值" : "Code", zh ? "实际值" : "Value", zh ? "说明" : "Meaning"].forEach((label) => {
    const th = document.createElement("th"); th.textContent = label; head.appendChild(th);
  });
  const thead = document.createElement("thead"); thead.appendChild(head); table.appendChild(thead);
  const body = document.createElement("tbody");
  frame.fields.forEach((field) => {
    const tr = document.createElement("tr");
    const tone = fieldClass(field.v, field.raw);
    if (tone) tr.className = tone;
    const cells = [field.n, field.c || "—", field.v, field.note || ""];
    cells.forEach((text, index) => {
      const td = document.createElement("td");
      if (index === 1 || field.raw && index === 2) td.className = "code";
      if (index === 2) td.classList.add("actual");
      td.textContent = text;
      tr.appendChild(td);
    });
    body.appendChild(tr);
  });
  table.appendChild(body);
  detail.appendChild(table);
}
function fieldClass(value, raw) {
  if (raw) return "";
  if (value.includes("不一致") || value.includes("mismatch")) return "bad";
  if (value.includes("一致") || value.includes("match")) return "ok";
  return "";
}
function extent() {
  let a = Infinity, b = -Infinity;
  data.waves.forEach((wave) => (wave.overview || []).concat(wave.detail || []).forEach((col) => { a = Math.min(a, col[0]); b = Math.max(b, col[1]); }));
  data.lanes.forEach((lane) => lane.frames.forEach((frame) => { a = Math.min(a, frame.t0); b = Math.max(b, frame.t1); }));
  if (!isFinite(a) || !isFinite(b) || a === b) { a = isFinite(a) ? a : 0; b = a + 1; }
  const pad = (b - a) * 0.02;
  return [a - pad, b + pad];
}
function finest() {
  let dt = Infinity;
  data.waves.forEach((wave) => {
    const cols = wave.detail && wave.detail.length ? wave.detail : wave.overview;
    if (cols && cols.length >= 2) dt = Math.min(dt, Math.max((cols[cols.length - 1][1] - cols[0][0]) / cols.length, 1e-15));
  });
  return isFinite(dt) ? dt : Math.max((full[1] - full[0]) / 4000, 1e-12);
}
function clampView(x0, x1) {
  let span = x1 - x0;
  const minSpan = Math.max(finest() * 4, 1e-12);
  const maxSpan = (full[1] - full[0]) * 1.05;
  span = Math.min(maxSpan, Math.max(minSpan, span));
  const mid = (x0 + x1) / 2;
  return { x0: mid - span / 2, x1: mid + span / 2 };
}
function zoomAt(factor, t) {
  const span0 = view.x1 - view.x0;
  const next = clampView(t - span0 * factor * ((t - view.x0) / span0), t + span0 * factor * ((view.x1 - t) / span0));
  view = next;
  draw();
}
function fmtTime(seconds) {
  if (!isFinite(seconds)) return "—";
  const sign = seconds < 0 ? "-" : "";
  const abs = Math.abs(seconds);
  if (abs === 0) return "0 s";
  if (abs < 1e-6) return sign + (abs * 1e9).toFixed(1) + " ns";
  if (abs < 1e-3) return sign + (abs * 1e6).toFixed(2) + " µs";
  if (abs < 1) return sign + (abs * 1e3).toFixed(3) + " ms";
  return sign + abs.toFixed(6) + " s";
}
function fmtHz(hz) {
  const abs = Math.abs(hz);
  if (abs >= 1e6) return (hz / 1e6).toFixed(3) + " MHz";
  if (abs >= 1e3) return (hz / 1e3).toFixed(3) + " kHz";
  return hz.toFixed(3) + " Hz";
}
function xOf(t, box) { return box.gutter + (t - view.x0) / (view.x1 - view.x0) * box.plotW; }
function tOf(x, box) { return view.x0 + (x - box.gutter) / box.plotW * (view.x1 - view.x0); }
function scene(width, height) {
  const lanes = Math.max(data.lanes.length, 1);
  const trackH = Math.min(24 * lanes + 10, height * 0.32);
  const gutter = 96;
  return { ruler: 28, gutter, trackH, plotTop: 28, plotBottom: Math.max(64, height - trackH), plotW: Math.max(1, width - gutter - 8) };
}
function raster(columns, x0, x1, pixels) {
  const buckets = new Array(pixels).fill(null);
  if (!columns || !columns.length || pixels < 1) return buckets;
  let lo = 0, hi = columns.length - 1, start = columns.length;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (columns[mid][1] < x0) lo = mid + 1;
    else { start = mid; hi = mid - 1; }
  }
  const span = x1 - x0 || 1;
  for (let i = Math.max(0, start - 1); i < columns.length; i++) {
    const col = columns[i];
    if (col[0] > x1) break;
    let p0 = Math.floor((col[0] - x0) / span * pixels);
    let p1 = Math.floor((col[1] - x0) / span * pixels);
    if (p1 < p0) p1 = p0;
    p0 = Math.max(0, p0); p1 = Math.min(pixels - 1, p1);
    for (let p = p0; p <= p1; p++) {
      const slot = buckets[p];
      if (!slot) buckets[p] = [col[2], col[3]];
      else { slot[0] = Math.min(slot[0], col[2]); slot[1] = Math.max(slot[1], col[3]); }
    }
  }
  return buckets;
}
function levelAt(wave, t) {
  const cols = (wave.detail && wave.detail.length) ? wave.detail : wave.overview;
  let lo = 0, hi = cols.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cols[mid][1] < t) lo = mid + 1;
    else if (cols[mid][0] > t) hi = mid - 1;
    else return (cols[mid][2] + cols[mid][3]) / 2;
  }
  return null;
}
function ticks(x0, x1) {
  const span = Math.max(x1 - x0, 1e-15);
  const rough = span / 8;
  const pow = Math.pow(10, Math.floor(Math.log10(rough)));
  const mantissa = rough / pow;
  const step = pow * (mantissa < 1.5 ? 1 : mantissa < 3.5 ? 2 : mantissa < 7.5 ? 5 : 10);
  const out = [];
  let t = Math.ceil(x0 / step) * step;
  for (let n = 0; n < 16 && t <= x1 + step * 0.01; n++, t += step) out.push(t);
  return out;
}
function columnsInView(columns) {
  if (!columns || !columns.length) return [];
  let lo = 0, hi = columns.length - 1, start = columns.length;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (columns[mid][1] < view.x0) lo = mid + 1;
    else { start = mid; hi = mid - 1; }
  }
  const out = [];
  for (let i = Math.max(0, start - 1); i < columns.length; i++) {
    if (columns[i][0] > view.x1) break;
    out.push(columns[i]);
  }
  return out;
}
function overlaps(columns, start, end) {
  if (!columns || !columns.length) return false;
  let lo = 0, hi = columns.length - 1, i = columns.length;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (columns[mid][1] < start) lo = mid + 1;
    else { i = mid; hi = mid - 1; }
  }
  return i < columns.length && columns[i][0] <= end;
}
function draw() {
  const dpr = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || 800;
  const height = canvas.clientHeight || 360;
  canvas.width = Math.max(1, Math.floor(width * dpr));
  canvas.height = Math.max(1, Math.floor(height * dpr));
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const box = scene(width, height);
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = "#fbfcfd";
  ctx.fillRect(0, 0, width, height);
  ctx.font = "11px Segoe UI, Microsoft YaHei, sans-serif";
  ctx.textBaseline = "middle";
  ctx.strokeStyle = "#e6ebf1";
  ctx.lineWidth = 1;
  ticks(view.x0, view.x1).forEach((t) => {
    const x = xOf(t, box);
    if (x < box.gutter || x > width - 4) return;
    ctx.beginPath(); ctx.moveTo(x, box.plotTop); ctx.lineTo(x, box.plotBottom); ctx.stroke();
    ctx.fillStyle = "#5e6d7e"; ctx.textAlign = "center";
    ctx.fillText(fmtTime(t), Math.min(Math.max(x, box.gutter + 28), width - 36), 14);
  });
  const frame = selected ? data.lanes[selected.lane].frames[selected.frame] : null;
  if (frame) {
    const x0 = xOf(frame.t0, box), x1 = xOf(frame.t1, box);
    ctx.fillStyle = frame.error ? "rgba(180,35,24,0.08)" : "rgba(11,107,203,0.10)";
    ctx.fillRect(Math.min(x0, x1), box.plotTop, Math.max(2, Math.abs(x1 - x0)), box.plotBottom - box.plotTop);
  }
  const rowsN = Math.max(data.waves.length, 1);
  const rowH = (box.plotBottom - box.plotTop) / rowsN;
  data.waves.forEach((wave, index) => {
    const top = box.plotTop + index * rowH;
    const color = colors[index % colors.length];
    ctx.strokeStyle = "#eef2f6";
    ctx.beginPath(); ctx.moveTo(box.gutter, top + rowH); ctx.lineTo(width, top + rowH); ctx.stroke();
    ctx.fillStyle = color; ctx.fillRect(12, top + 10, 8, 8);
    ctx.fillStyle = "#1c2430"; ctx.textAlign = "left";
    ctx.fillText(wave.label, 26, top + 14);
    ctx.fillStyle = "#5e6d7e";
    ctx.fillText(wave.unit || "", 26, top + 28);
    const coarse = columnsInView(wave.overview).filter((col) => !overlaps(wave.detail, col[0], col[1]));
    const fine = columnsInView(wave.detail);
    let ymin = Infinity, ymax = -Infinity;
    coarse.concat(fine).forEach((col) => { ymin = Math.min(ymin, col[2]); ymax = Math.max(ymax, col[3]); });
    if (!isFinite(ymin) || ymin === ymax) { ymin = (isFinite(ymin) ? ymin : 0) - 1; ymax = ymin + 2; }
    const pad = (ymax - ymin) * 0.14;
    ymin -= pad; ymax += pad;
    const yOf = (value) => top + 36 + (ymax - value) / (ymax - ymin) * Math.max(8, rowH - 48);
    const paintSeries = (columns) => {
      ctx.fillStyle = color; ctx.globalAlpha = 0.38;
      columns.forEach((col) => {
        if (col[2] === col[3]) return;
        const x0 = xOf(col[0], box);
        const x1 = Math.max(xOf(col[1], box), x0 + 1);
        const high = yOf(col[3]), low = yOf(col[2]);
        ctx.fillRect(x0, Math.min(high, low), Math.max(1, x1 - x0), Math.max(1, Math.abs(low - high)));
      });
      ctx.globalAlpha = 1; ctx.strokeStyle = color; ctx.lineWidth = 1.4; ctx.lineJoin = "round"; ctx.beginPath();
      let drawing = false, prev = null;
      columns.forEach((col) => {
        if (col[2] !== col[3]) { drawing = false; prev = col; return; }
        const x = (xOf(col[0], box) + xOf(col[1], box)) / 2;
        const y = yOf(col[2]);
        const gap = prev ? col[0] - prev[1] : 0;
        const step = Math.max(col[1] - col[0], prev ? prev[1] - prev[0] : 0, 0);
        if (!drawing || gap > Math.max(step * 4, (view.x1 - view.x0) / box.plotW)) { ctx.moveTo(x, y); drawing = true; }
        else ctx.lineTo(x, y);
        prev = col;
      });
      ctx.stroke();
    };
    paintSeries(coarse);
    paintSeries(fine);
    ctx.fillStyle = "#5e6d7e"; ctx.textAlign = "right";
    ctx.fillText(ymax.toFixed(2), width - 8, top + 36);
    ctx.fillText(ymin.toFixed(2), width - 8, top + rowH - 8);
  });
  if (!data.waves.length) {
    ctx.fillStyle = "#5e6d7e"; ctx.textAlign = "left";
    ctx.fillText(zh ? "没有波形数据。帧时间仍可在下方查看。" : "No waveform samples. Frame times are still listed below.", box.gutter, box.plotTop + 24);
  }
  const laneH = (height - box.plotBottom) / Math.max(data.lanes.length, 1);
  ctx.strokeStyle = "#e3e8ee"; ctx.beginPath(); ctx.moveTo(0, box.plotBottom); ctx.lineTo(width, box.plotBottom); ctx.stroke();
  data.lanes.forEach((lane, index) => {
    const y = box.plotBottom + index * laneH;
    const color = colors[(lane.wave == null ? index : lane.wave) % colors.length];
    ctx.fillStyle = index === laneFilter ? "#0b4f96" : "#5e6d7e"; ctx.textAlign = "left";
    ctx.fillText(lane.title, 12, y + laneH / 2);
    lane.frames.forEach((item, frameIndex) => {
      const x0 = xOf(item.t0, box);
      const x1 = Math.max(xOf(item.t1, box), x0 + 2);
      if (x1 < box.gutter || x0 > width) return;
      const left = Math.max(box.gutter, x0);
      const right = Math.min(width - 4, x1);
      const on = selected && selected.lane === index && selected.frame === frameIndex;
      ctx.globalAlpha = on ? 1 : 0.72;
      ctx.fillStyle = item.error ? "#b42318" : color;
      ctx.fillRect(left, y + 6, Math.max(2, right - left), Math.max(8, laneH - 12));
      ctx.globalAlpha = 1;
      if (right - left > 52) {
        ctx.fillStyle = "#fff";
        ctx.save(); ctx.beginPath(); ctx.rect(left, y, right - left, laneH); ctx.clip();
        ctx.fillText(item.summary, left + 4, y + laneH / 2);
        ctx.restore();
      }
    });
  });
  drawCursor(cursorA, "#0b6bcb", "A", box, width, height);
  drawCursor(cursorB, "#b45309", "B", box, width, height);
  if (hover != null && hover >= view.x0 && hover <= view.x1) {
    const x = xOf(hover, box);
    ctx.strokeStyle = "#91a0b0"; ctx.setLineDash([3, 3]);
    ctx.beginPath(); ctx.moveTo(x, box.plotTop); ctx.lineTo(x, height); ctx.stroke();
    ctx.setLineDash([]);
  }
  if (drag && drag.shift && drag.moved) {
    const left = Math.min(drag.x, drag.x2), right = Math.max(drag.x, drag.x2);
    ctx.fillStyle = "rgba(11,107,203,0.12)"; ctx.strokeStyle = "#0b6bcb";
    ctx.fillRect(left, box.plotTop, right - left, height - box.plotTop);
    ctx.strokeRect(left + 0.5, box.plotTop + 0.5, right - left, height - box.plotTop);
  }
  paintReadout();
}
function drawCursor(t, color, label, box, width, height) {
  if (t == null || t < view.x0 || t > view.x1) return;
  const x = xOf(t, box);
  ctx.strokeStyle = color; ctx.fillStyle = color;
  ctx.beginPath(); ctx.moveTo(x, box.plotTop); ctx.lineTo(x, height); ctx.stroke();
  const text = label + " " + fmtTime(t);
  const w = ctx.measureText(text).width + 8;
  ctx.fillRect(Math.min(x + 4, width - w - 4), box.plotTop + 4, w, 16);
  ctx.fillStyle = "#fff"; ctx.textAlign = "left";
  ctx.fillText(text, Math.min(x + 8, width - w), box.plotTop + 12);
}
function paintReadout() {
  const dt = cursorA != null && cursorB != null ? Math.abs(cursorB - cursorA) : NaN;
  const lane = selected ? data.lanes[selected.lane] : null;
  const frame = lane && selected ? lane.frames[selected.frame] : null;
  const prev = lane && selected ? lane.frames[selected.frame - 1] : null;
  const cells = [
    ["A", cursorA == null ? "—" : fmtTime(cursorA)],
    ["B", cursorB == null ? "—" : fmtTime(cursorB)],
    ["Δt", fmtTime(dt)],
    [zh ? "频率" : "Frequency", isFinite(dt) && dt > 0 ? fmtHz(1 / dt) : "—"],
    [zh ? "脉宽" : "Width", frame ? fmtTime(frame.t1 - frame.t0) : "—"],
    [zh ? "帧间隔" : "Gap", prev && frame ? fmtTime(frame.t0 - prev.t1) : "—"],
    [zh ? "指针" : "Pointer", hover == null ? "—" : fmtTime(hover)],
  ];
  readout.replaceChildren();
  cells.forEach(([name, value]) => {
    const box = document.createElement("div");
    const span = document.createElement("span"); span.textContent = name;
    const strong = document.createElement("b"); strong.textContent = value;
    box.append(span, strong); readout.appendChild(box);
  });
  if (cursorA != null && data.waves.length) {
    const line = document.createElement("div");
    line.style.gridColumn = "1 / -1";
    line.style.borderLeft = "0";
    line.style.borderTop = "1px solid #e3e8ee";
    const span = document.createElement("span"); span.textContent = zh ? "光标 A 的电压" : "Voltage at cursor A";
    const strong = document.createElement("b");
    strong.textContent = data.waves.map((wave) => {
      const value = levelAt(wave, cursorA);
      return wave.label + "  " + (value == null ? "—" : value.toFixed(4) + " " + (wave.unit || ""));
    }).join("      ");
    line.append(span, strong); readout.appendChild(line);
  }
}
canvas.addEventListener("pointerdown", (event) => {
  drag = { x: event.offsetX, y: event.offsetY, x2: event.offsetX, x0: view.x0, x1: view.x1, moved: false, shift: event.shiftKey };
  canvas.setPointerCapture(event.pointerId);
});
canvas.addEventListener("pointermove", (event) => {
  const box = scene(canvas.clientWidth, canvas.clientHeight);
  hover = event.offsetX < box.gutter ? null : tOf(event.offsetX, box);
  if (!drag) { draw(); return; }
  drag.x2 = event.offsetX;
  if (Math.abs(event.offsetX - drag.x) > 3) drag.moved = true;
  if (!drag.shift) {
    const dt = (drag.x - event.offsetX) / box.plotW * (drag.x1 - drag.x0);
    view = { x0: drag.x0 + dt, x1: drag.x1 + dt };
  }
  draw();
});
canvas.addEventListener("pointerup", (event) => {
  if (!drag) return;
  const box = scene(canvas.clientWidth, canvas.clientHeight);
  const height = canvas.clientHeight;
  if (drag.shift && drag.moved) {
    const t0 = tOf(Math.min(drag.x, drag.x2), box);
    const t1 = tOf(Math.max(drag.x, drag.x2), box);
    if (t1 > t0) view = clampView(t0, t1);
    drag = null; draw(); return;
  }
  if (drag.moved) { drag = null; return; }
  const t = tOf(event.offsetX, box);
  if (event.offsetY >= box.plotBottom && data.lanes.length) {
    const laneH = (height - box.plotBottom) / data.lanes.length;
    const lane = Math.min(data.lanes.length - 1, Math.floor((event.offsetY - box.plotBottom) / laneH));
    const frames = data.lanes[lane].frames;
    let found = -1, lo = 0, hi = frames.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (frames[mid].t1 < t) lo = mid + 1;
      else if (frames[mid].t0 > t) hi = mid - 1;
      else { found = mid; break; }
    }
    if (found >= 0) { drag = null; rebuild(); choose(lane, found, false); return; }
  } else if (event.offsetX >= box.gutter) {
    if (event.shiftKey || (cursorA != null && cursorB == null)) { if (cursorA == null) cursorA = t; else cursorB = t; }
    else if (cursorA == null) cursorA = t;
    else if (cursorB == null) cursorB = t;
    else if (Math.abs(t - cursorA) <= Math.abs(t - cursorB)) cursorA = t;
    else cursorB = t;
  }
  drag = null;
  draw();
});
canvas.addEventListener("wheel", (event) => {
  event.preventDefault();
  const box = scene(canvas.clientWidth, canvas.clientHeight);
  const t = event.offsetX < box.gutter ? (view.x0 + view.x1) / 2 : tOf(event.offsetX, box);
  zoomAt(event.deltaY > 0 ? 1.18 : 1 / 1.18, t);
}, { passive: false });
document.getElementById("zin").onclick = () => zoomAt(0.5, (view.x0 + view.x1) / 2);
document.getElementById("zout").onclick = () => zoomAt(2, (view.x0 + view.x1) / 2);
document.getElementById("framezoom").onclick = () => { if (selected) choose(selected.lane, selected.frame, true); };
document.getElementById("fit").onclick = () => { view = { x0: full[0], x1: full[1] }; draw(); };
document.getElementById("clear").onclick = () => { cursorA = null; cursorB = null; draw(); };
document.getElementById("q").addEventListener("input", rebuild);
window.addEventListener("resize", draw);
window.addEventListener("keydown", (event) => {
  if (event.target.tagName === "INPUT") return;
  if (event.key === "+" || event.key === "=") { zoomAt(0.5, (view.x0 + view.x1) / 2); return; }
  if (event.key === "-" || event.key === "_") { zoomAt(2, (view.x0 + view.x1) / 2); return; }
  if (!selected) return;
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  const lane = data.lanes[selected.lane];
  const next = selected.frame + (event.key === "ArrowRight" ? 1 : -1);
  if (next >= 0 && next < lane.frames.length) { event.preventDefault(); choose(selected.lane, next, true); }
});
rebuild();
if (selected) choose(selected.lane, selected.frame, true);
"###;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ProtocolReport {
        ProtocolReport {
            zh: true,
            sources: vec!["CH1.wfm".into(), "CH2.wfm".into()],
            byte_limit: 5000,
            focus: Some((2, 0)),
            waves: vec![ProtocolReportWave::new(
                "CH1".into(),
                "V".into(),
                vec![[0.0, 0.001, 0.0, 3.3]],
                vec![[0.0, 0.0005, 0.0, 0.2], [0.0005, 0.001, 3.3, 3.3]],
            )],
            lanes: vec![
                ProtocolReportLane {
                    id: 1,
                    title: "UART · CH1".into(),
                    channels: "CH1".into(),
                    wave: Some(0),
                    ready: true,
                    on_plot: true,
                    brief: "异步串口。一帧是一个字符。".into(),
                    info: "115200 8N1".into(),
                    error: None,
                    truncated: false,
                    frames: vec![ProtocolReportFrame {
                        index: 0,
                        t0: 0.0,
                        t1: 8.7e-6,
                        summary: "A <tag>".into(),
                        error: false,
                        fields: vec![ProtocolReportField {
                            name: "数据".into(),
                            value: "65（十进制）  0x41".into(),
                            raw: false,
                            note: "这一帧的数据。十进制便于读，十六进制便于对照规格。".into(),
                            code: "0x41".into(),
                        }],
                    }],
                },
                ProtocolReportLane {
                    id: 2,
                    title: "SCP · CH2".into(),
                    channels: "CH2".into(),
                    wave: Some(0),
                    ready: true,
                    on_plot: false,
                    brief: "单线超级充电。".into(),
                    info: String::new(),
                    error: None,
                    truncated: true,
                    frames: vec![ProtocolReportFrame {
                        index: 0,
                        t0: 0.001,
                        t1: 0.0012,
                        summary: "SCP SBRRD".into(),
                        error: true,
                        fields: vec![
                            ProtocolReportField { name: "CRC-8".into(), value: "不一致".into(), code: "收到 11 / 计算 22".into(), raw: false, note: "按协议多项式重算。一致表示这帧字节没有传错。".into() },
                            ProtocolReportField { name: "原始字节".into(), value: "0C 2B 11".into(), code: String::new(), raw: true, note: "线上字节，按时间顺序排列。".into() },
                        ],
                    }],
                },
            ],
        }
    }

    #[test]
    fn report_lists_every_lane_and_escapes_text() {
        let html = render_protocol_report(&sample());
        assert!(html.contains("UART · CH1"));
        assert!(html.contains("SCP · CH2"));
        assert!(html.contains("CH1.wfm"));
        assert!(html.contains("\\u003ctag\\u003e"));
        assert!(!html.contains("<tag>"));
        assert!(html.contains("id=\"wave\""));
        assert!(html.contains("此帧"));
        assert!(html.contains("color-scheme: light"));
        assert!(!html.contains("prefers-color-scheme"));
        assert!(html.contains("不一致"));
        assert!(html.contains("5000"));
        assert!(html.contains("码值"));
        let json = html.split("id=\"data\">").nth(1).unwrap().split("</script>").next().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json.replace("\\u003c", "<").replace("\\u003e", ">").replace("\\u0026", "&")).unwrap();
        assert_eq!(value["waves"][0]["overview"][0][3], 3.3);
        assert_eq!(value["waves"][0]["detail"].as_array().unwrap().len(), 2);
        assert_eq!(value["lanes"][1]["frames"][0]["t0"], 0.001);
        assert!(value["lanes"][1]["frames"][0]["fields"][0]["note"].as_str().unwrap().contains("多项式"));
        assert_eq!(value["focus"][0], 2);
    }

    #[test]
    fn protocol_html_file_replaces_the_previous_one() {
        let mut slot = None;
        let first = write_protocol_html(&mut slot, "<p>one</p>").unwrap();
        assert!(first.is_file());
        let second = write_protocol_html(&mut slot, "<p>two</p>").unwrap();
        assert_ne!(first, second);
        assert!(!Path::new(&first).exists());
        assert_eq!(fs::read_to_string(&second).unwrap(), "<p>two</p>");
        let _ = fs::remove_file(second);
    }
}

pub fn decimate_series(x: &[f64], y: &[f64], max_cols: usize) -> Vec<[f64; 4]> {
    let n = x.len().min(y.len());
    if n == 0 || max_cols == 0 {
        return Vec::new();
    }
    if n <= max_cols {
        return (0..n).map(|index| {
            let end = if index + 1 < n { x[index + 1] } else { x[index] };
            [x[index], end.max(x[index]), y[index], y[index]]
        }).collect();
    }
    let mut out = Vec::with_capacity(max_cols);
    for bucket in 0..max_cols {
        let start = bucket * n / max_cols;
        let end = ((bucket + 1) * n / max_cols).max(start + 1).min(n);
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for index in start..end {
            ymin = ymin.min(y[index]);
            ymax = ymax.max(y[index]);
        }
        out.push([x[start], x[end - 1].max(x[start]), ymin, ymax]);
    }
    out
}

pub fn thin_columns(columns: &[[f64; 4]], max_cols: usize) -> Vec<[f64; 4]> {
    if columns.len() <= max_cols || max_cols == 0 {
        return columns.iter().copied().filter(|col| col.iter().all(|value| value.is_finite())).collect();
    }
    let mut out = Vec::with_capacity(max_cols);
    for bucket in 0..max_cols {
        let start = bucket * columns.len() / max_cols;
        let end = ((bucket + 1) * columns.len() / max_cols).max(start + 1).min(columns.len());
        let mut x0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for col in &columns[start..end] {
            if !col.iter().all(|value| value.is_finite()) {
                continue;
            }
            x0 = x0.min(col[0]);
            x1 = x1.max(col[1]);
            ymin = ymin.min(col[2]);
            ymax = ymax.max(col[3]);
        }
        if x0.is_finite() {
            out.push([x0, x1.max(x0), ymin, ymax]);
        }
    }
    out
}

pub fn detail_windows(lanes: &[ProtocolReportLane]) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = lanes.iter().flat_map(|lane| lane.frames.iter()).filter_map(|frame| {
        if frame.t0.is_finite() && frame.t1.is_finite() {
            Some((frame.t0.min(frame.t1), frame.t0.max(frame.t1)))
        } else {
            None
        }
    }).collect();
    if spans.is_empty() {
        return spans;
    }
    spans.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut widths: Vec<f64> = spans.iter().map(|(start, end)| end - start).filter(|width| *width > 0.0).collect();
    widths.sort_by(|left, right| left.total_cmp(right));
    let typical = widths.get(widths.len() / 2).copied().unwrap_or(0.0);
    let pad = (typical * 2.0).max(1e-12);
    let mut merged = Vec::<(f64, f64)>::new();
    for (start, end) in spans {
        let start = start - pad;
        let end = end + pad;
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// High-resolution min/max columns around decoded frames. The overview stays coarse.
pub fn detail_columns(x: &[f64], y: &[f64], windows: &[(f64, f64)], budget: usize) -> Vec<[f64; 4]> {
    let n = x.len().min(y.len());
    if n == 0 || budget == 0 {
        return Vec::new();
    }
    if windows.is_empty() || n <= budget {
        return decimate_series(&x[..n], &y[..n], budget.min(n));
    }
    let mut ranges = Vec::<(usize, usize)>::new();
    let mut covered = 0usize;
    for (start_t, end_t) in windows {
        let start = x[..n].partition_point(|time| *time < *start_t);
        let end = x[..n].partition_point(|time| *time <= *end_t).max(start).min(n);
        if start < end {
            covered += end - start;
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        return decimate_series(&x[..n], &y[..n], budget.min(n));
    }
    if covered <= budget {
        let mut out = Vec::with_capacity(covered);
        for (start, end) in ranges {
            out.extend(decimate_series(&x[start..end], &y[start..end], end - start));
        }
        return out;
    }
    let lengths: Vec<usize> = ranges.iter().map(|(start, end)| end - start).collect();
    let shares = share_columns(&lengths, budget);
    let mut out = Vec::new();
    for ((start, end), share) in ranges.iter().zip(shares) {
        if share == 0 {
            continue;
        }
        out.extend(decimate_series(&x[*start..*end], &y[*start..*end], share));
    }
    if out.is_empty() { decimate_series(&x[..n], &y[..n], budget.min(n)) } else { out }
}

/// Split a sample budget across frame windows without dropping a later frame to zero.
pub fn share_columns(lengths: &[usize], budget: usize) -> Vec<usize> {
    let covered: usize = lengths.iter().sum();
    if covered == 0 || budget == 0 {
        return vec![0; lengths.len()];
    }
    let mut shares: Vec<usize> = lengths.iter().map(|len| {
        let raw = ((budget as f64) * (*len as f64) / (covered as f64)).round() as usize;
        raw.clamp(1, *len)
    }).collect();
    let mut sum: usize = shares.iter().sum();
    while sum > budget {
        let Some((index, _)) = shares.iter().enumerate().filter(|(_, share)| **share > 1).max_by_key(|(_, share)| *share) else {
            break;
        };
        shares[index] -= 1;
        sum -= 1;
    }
    shares
}

