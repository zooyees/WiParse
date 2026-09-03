//! Incremental live-log briefing: counts, phase, alerts, sparse notables.
//! Intended for Agent consumption — never dumps raw lines.

use crate::metrics::{parse_metric_frame, MetricSample};
use crate::protocol::decode_qi_message;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Result of ingesting one serial line into the live brief.
#[derive(Debug)]
pub enum IngestResult {
    None,
    Event(BriefEvent),
    Metric { t: f64, sample: MetricSample },
}

const MAX_NOTABLES: usize = 12;
const MAX_METRICS_RING: usize = 256;
const TOP_N: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QiPhase {
    Idle,
    Ping,
    Id,
    Cfg,
    Pt,
    Ept,
}

impl QiPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Ping => "ping",
            Self::Id => "id",
            Self::Cfg => "cfg",
            Self::Pt => "pt",
            Self::Ept => "ept",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BriefEvent {
    pub r: u64,
    pub t: f64,
    pub k: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub s: String,
    pub n: u32,
    pub t: String,
}

#[derive(Debug, Clone)]
pub struct MetricPoint {
    pub t: f64,
    pub sample: MetricSample,
}

/// Rolling session analyser. Cheap to update per serial line.
pub struct LiveBrief {
    t0: Instant,
    next_row: u64,
    n_lines: u64,
    ask: u32,
    fsk: u32,
    unk: u32,
    csum: u32,
    nak: u32,
    other: u32,
    metric_n: u32,
    packet_counts: HashMap<String, u32>,
    header_counts: HashMap<u8, u32>,
    ept_reasons: HashMap<String, u32>,
    phase: QiPhase,
    vin: Extrema,
    iin: Extrema,
    vout: Extrema,
    p: Extrema,
    eff_now: f64,
    ce: Extrema,
    ce_n: u32,
    notables: VecDeque<BriefEvent>,
    alerts: HashMap<String, Alert>,
    recent_metrics: VecDeque<MetricPoint>,
    last_unk: Option<String>,
    last_hex: Option<String>,
    last_qi_t: Option<f64>,
    dt_last: f64,
    dt_max: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Extrema {
    min: f64,
    max: f64,
    now: f64,
    set: bool,
}

impl Extrema {
    fn push(&mut self, v: f64) {
        if !self.set {
            self.min = v;
            self.max = v;
            self.now = v;
            self.set = true;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
            self.now = v;
        }
    }

    fn triple(self) -> [f64; 3] {
        if self.set {
            [round3(self.min), round3(self.max), round3(self.now)]
        } else {
            [0.0, 0.0, 0.0]
        }
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

impl Default for LiveBrief {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveBrief {
    pub fn new() -> Self {
        Self {
            t0: Instant::now(),
            next_row: 0,
            n_lines: 0,
            ask: 0,
            fsk: 0,
            unk: 0,
            csum: 0,
            nak: 0,
            other: 0,
            metric_n: 0,
            packet_counts: HashMap::new(),
            header_counts: HashMap::new(),
            ept_reasons: HashMap::new(),
            phase: QiPhase::Idle,
            vin: Extrema::default(),
            iin: Extrema::default(),
            vout: Extrema::default(),
            p: Extrema::default(),
            eff_now: 0.0,
            ce: Extrema::default(),
            ce_n: 0,
            notables: VecDeque::new(),
            alerts: HashMap::new(),
            recent_metrics: VecDeque::new(),
            last_unk: None,
            last_hex: None,
            last_qi_t: None,
            dt_last: 0.0,
            dt_max: 0.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn elapsed_s(&self) -> f64 {
        self.t0.elapsed().as_secs_f64()
    }

    pub fn next_row(&self) -> u64 {
        self.next_row
    }

    pub fn n_lines(&self) -> u64 {
        self.n_lines
    }

    pub fn phase(&self) -> QiPhase {
        self.phase
    }

    pub fn csum(&self) -> u32 {
        self.csum
    }

    pub fn vin_now(&self) -> Option<f64> {
        self.vin.set.then_some(self.vin.now)
    }

    pub fn p_now(&self) -> Option<f64> {
        self.p.set.then_some(self.p.now)
    }

    pub fn seen_ept(&self) -> bool {
        self.phase == QiPhase::Ept || self.packet_counts.contains_key("EPT")
    }

    /// True if a Qi packet type (e.g. `CE`, `EPT`) was counted this session.
    pub fn seen_packet(&self, name: &str) -> bool {
        self.packet_count(name) > 0
    }

    pub fn packet_count(&self, name: &str) -> u32 {
        let want = name.trim().to_ascii_uppercase();
        if want.is_empty() {
            return 0;
        }
        self.packet_counts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&want))
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    pub fn header_count(&self, header: u8) -> u32 {
        self.header_counts.get(&header).copied().unwrap_or(0)
    }

    pub fn last_hex(&self) -> Option<&str> {
        self.last_hex.as_deref()
    }

    pub fn note_send(&mut self, hex: &str) {
        self.last_hex = Some(hex.to_ascii_uppercase());
    }

    pub fn recent_metrics(&self) -> &VecDeque<MetricPoint> {
        &self.recent_metrics
    }

    pub fn metrics_window(&self, t0: f64, t1: f64) -> MetricWindow {
        let mut w = MetricWindow::default();
        for pt in &self.recent_metrics {
            if pt.t >= t0 && pt.t <= t1 {
                w.push(&pt.sample);
            }
        }
        w
    }

    /// Ingest one live line. `row` should be the 0-based index in the live tab.
    pub fn ingest_line(&mut self, row: u64, line: &str) -> IngestResult {
        self.next_row = self.next_row.max(row + 1);
        self.n_lines += 1;
        let t = self.elapsed_s();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return IngestResult::None;
        }

        if trimmed.starts_with("[ERR]") || trimmed.starts_with("[err]") {
            self.bump_alert("err", "serial");
            return IngestResult::Event(self.push_event(row, t, "ERR", Some(truncate(trimmed, 80))));
        }

        if let Some(m) = parse_metric_frame(trimmed) {
            self.metric_n += 1;
            self.vin.push(m.v_in);
            self.iin.push(m.i_in);
            self.vout.push(m.v_out);
            self.p.push(m.p);
            self.eff_now = m.eff;
            if let Some(limit) = self.vin.set.then_some(self.vin.now) {
                if limit > 20.0 {
                    self.bump_alert("err", "vin");
                }
            }
            self.recent_metrics.push_back(MetricPoint {
                t,
                sample: m.clone(),
            });
            while self.recent_metrics.len() > MAX_METRICS_RING {
                self.recent_metrics.pop_front();
            }
            return IngestResult::Metric { t, sample: m };
        }

        if let Some(d) = decode_qi_message(trimmed) {
            let is_ask = d.direction.eq_ignore_ascii_case("ASK");
            if is_ask {
                self.ask += 1;
            } else {
                self.fsk += 1;
            }
            let name = d.name.to_ascii_uppercase();
            *self.packet_counts.entry(name.clone()).or_insert(0) += 1;
            *self.header_counts.entry(d.header).or_insert(0) += 1;
            if !d.known {
                self.unk += 1;
                self.last_unk = Some(format!("{} 0x{:02X}", d.direction, d.header));
                self.bump_alert("warn", "unk");
            }
            if d.checksum_ok == Some(false) {
                self.csum += 1;
                self.bump_alert("err", "csum");
            }
            if name.contains("NAK") || name == "DSR" && field_contains(&d.fields, "nak") {
                self.nak += 1;
                self.bump_alert("warn", "nak");
            }
            if let Some(prev) = self.last_qi_t {
                let dt = (t - prev).max(0.0);
                self.dt_last = dt;
                self.dt_max = self.dt_max.max(dt);
            }
            self.last_qi_t = Some(t);
            if name == "CE" {
                if let Some(v) = field_f64(&d.fields, "control_error") {
                    self.ce.push(v);
                    self.ce_n += 1;
                }
            }
            if name == "EPT" {
                let reason = field_str(&d.fields, "reason")
                    .or_else(|| field_str(&d.fields, "reason_code"))
                    .unwrap_or_else(|| "EPT".into());
                *self.ept_reasons.entry(reason.clone()).or_insert(0) += 1;
            }
            self.update_phase(&name);
            if notable(&name, !d.known, d.checksum_ok == Some(false)) {
                let extra = if name == "EPT" {
                    field_str(&d.fields, "reason")
                } else if name == "CE" {
                    field_f64(&d.fields, "control_error").map(|v| format!("{v}"))
                } else if !d.known {
                    Some(format!("0x{:02X}", d.header))
                } else {
                    None
                };
                let k = format!("{} {name}", d.direction);
                return IngestResult::Event(self.push_event(row, t, &k, extra));
            }
            return IngestResult::None;
        }

        self.other += 1;
        IngestResult::None
    }

    fn update_phase(&mut self, name: &str) {
        match name {
            "SS" if matches!(self.phase, QiPhase::Idle | QiPhase::Ept) => {
                self.phase = QiPhase::Ping;
            }
            "ID" | "XID" => {
                if self.phase != QiPhase::Ept {
                    self.phase = QiPhase::Id;
                }
            }
            "CFG" => {
                if self.phase != QiPhase::Ept {
                    self.phase = QiPhase::Cfg;
                }
            }
            "CE" | "RP" | "RP8" | "CHS" => {
                if !matches!(self.phase, QiPhase::Ept) {
                    self.phase = QiPhase::Pt;
                }
            }
            "EPT" => self.phase = QiPhase::Ept,
            "ACK" => {
                if matches!(self.phase, QiPhase::Id | QiPhase::Cfg | QiPhase::Ping) {
                    self.phase = QiPhase::Pt;
                }
            }
            _ => {}
        }
    }

    fn push_event(&mut self, r: u64, t: f64, k: &str, x: Option<String>) -> BriefEvent {
        let ev = BriefEvent {
            r,
            t: round3(t),
            k: k.to_string(),
            x,
        };
        self.notables.push_back(ev.clone());
        while self.notables.len() > MAX_NOTABLES {
            self.notables.pop_front();
        }
        ev
    }

    fn bump_alert(&mut self, sev: &str, key: &str) {
        self.alerts
            .entry(key.to_string())
            .and_modify(|a| a.n += 1)
            .or_insert(Alert {
                s: sev.into(),
                n: 1,
                t: key.into(),
            });
    }

    /// Compact JSON for MCP / CLI. `since_row` only affects `n` (lines since cursor).
    pub fn snapshot_json(&self, since_row: u64, detail: Option<&str>) -> Value {
        let n = self.next_row.saturating_sub(since_row);
        let mut top: Vec<(&String, &u32)> = self.packet_counts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        top.truncate(TOP_N);
        let top_v: Vec<Value> = top
            .into_iter()
            .map(|(k, n)| json!([k, n]))
            .collect();
        let mut al: Vec<&Alert> = self.alerts.values().collect();
        al.sort_by(|a, b| a.t.cmp(&b.t));
        let mut body = json!({
            "next_row": self.next_row,
            "dt_s": round3(self.elapsed_s()),
            "n": n,
            "phase": self.phase.as_str(),
            "m": {
                "n": self.metric_n,
                "vin": self.vin.triple(),
                "iin": self.iin.triple(),
                "p": self.p.triple(),
                "eff": round3(self.eff_now),
            },
            "qi": {
                "ask": self.ask,
                "fsk": self.fsk,
                "unk": self.unk,
                "csum": self.csum,
                "nak": self.nak,
                "other": self.other,
                "top": top_v,
                "ept": self.ept_reasons,
                "dt_ms": [round3(self.dt_last * 1000.0), round3(self.dt_max * 1000.0)],
            },
            "al": al,
            "ev": self.notables,
        });
        if let Some(hex) = &self.last_hex {
            body["last_hex"] = json!(hex);
        }
        if self.ce_n > 0 {
            body["qi"]["ce"] = json!({
                "n": self.ce_n,
                "v": self.ce.triple(),
            });
        }
        match detail {
            Some("qi") => {
                body["unk"] = json!(self.last_unk);
            }
            Some("alerts") => {
                // alerts already in `al`; keep payload small
            }
            Some("m") => {}
            _ => {}
        }
        body
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MetricWindow {
    pub n: u32,
    pub vin: [f64; 3],
    pub p: [f64; 3],
    pub dp: f64,
}

impl MetricWindow {
    fn push(&mut self, m: &MetricSample) {
        if self.n == 0 {
            self.vin = [m.v_in, m.v_in, m.v_in];
            self.p = [m.p, m.p, m.p];
        } else {
            self.vin[0] = self.vin[0].min(m.v_in);
            self.vin[1] = self.vin[1].max(m.v_in);
            self.vin[2] = m.v_in;
            self.p[0] = self.p[0].min(m.p);
            self.p[1] = self.p[1].max(m.p);
            self.p[2] = m.p;
        }
        self.n += 1;
        self.dp = round3(self.p[2] - self.p[0]);
        self.vin = [
            round3(self.vin[0]),
            round3(self.vin[1]),
            round3(self.vin[2]),
        ];
        self.p = [round3(self.p[0]), round3(self.p[1]), round3(self.p[2])];
    }
}

fn notable(name: &str, unk: bool, csum_fail: bool) -> bool {
    unk || csum_fail
        || matches!(
            name,
            "SS" | "ID"
                | "XID"
                | "CFG"
                | "ACK"
                | "NAK"
                | "EPT"
                | "DSR"
                | "SRQ"
                | "NEGO"
                | "CLOAK"
        )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

fn field_str(fields: &[crate::protocol::QiField], name: &str) -> Option<String> {
    fields.iter().find(|f| f.name == name).map(|f| match &f.value {
        Value::String(s) => s.clone(),
        other => other.to_string().trim_matches('"').to_string(),
    })
}

fn field_f64(fields: &[crate::protocol::QiField], name: &str) -> Option<f64> {
    fields.iter().find(|f| f.name == name).and_then(|f| match &f.value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn field_contains(fields: &[crate::protocol::QiField], needle: &str) -> bool {
    let n = needle.to_ascii_lowercase();
    fields.iter().any(|f| {
        f.name.to_ascii_lowercase().contains(&n)
            || f.value
                .as_str()
                .is_some_and(|s| s.to_ascii_lowercase().contains(&n))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_phase_and_compact_snapshot() {
        let mut b = LiveBrief::new();
        b.ingest_line(0, "TX0:[12:00:00.000] ASK 01 80 81 F ");
        assert_eq!(b.phase(), QiPhase::Ping);
        b.ingest_line(1, "TX0:[12:00:00.010] ASK 71 01 02 03 70 F ");
        b.ingest_line(2, "TX0:[12:00:00.020] ASK 51 00 00 00 00 00 51 F ");
        b.ingest_line(3, "TX0:[12:00:00.100] ASK 03 00 03 F ");
        assert_eq!(b.phase(), QiPhase::Pt);
        b.ingest_line(4, "AA55:9000:1500:8500:1400:4000:3000:45:80:EDED");
        assert!(b.p_now().is_some());
        let snap = b.snapshot_json(0, None);
        assert_eq!(snap["phase"], "pt");
        assert_eq!(snap["qi"]["ask"].as_u64().unwrap(), 4);
        assert!(snap["ev"].as_array().unwrap().len() >= 1);
        assert!(snap["m"]["n"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn ept_and_csum_alerts() {
        let mut b = LiveBrief::new();
        b.ingest_line(0, "ASK 02 08 0A F ");
        assert_eq!(b.phase(), QiPhase::Ept);
        assert!(b.seen_ept());
        let snap = b.snapshot_json(0, None);
        assert!(!snap["qi"]["ept"].as_object().unwrap().is_empty());
    }

    #[test]
    fn jitter_ascii_is_other_not_qi() {
        let mut b = LiveBrief::new();
        b.ingest_line(0, "hello world noise");
        let snap = b.snapshot_json(0, None);
        assert_eq!(snap["qi"]["other"], 1);
        assert_eq!(snap["qi"]["ask"], 0);
        assert_eq!(snap["phase"], "idle");
    }
}
