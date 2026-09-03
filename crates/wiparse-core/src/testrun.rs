//! Deterministic closed-loop test plan + driver (no LLM on the hot path).

use crate::brief::{LiveBrief, QiPhase};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AbortSpec {
    /// Abort when an EPT packet is seen.
    #[serde(default)]
    pub on_ept: bool,
    #[serde(default)]
    pub csum_gt: Option<u32>,
    /// Whole-run timeout (seconds).
    #[serde(default)]
    pub timeout_s: Option<f64>,
    #[serde(default)]
    pub vin_gt: Option<f64>,
    #[serde(default)]
    pub p_lt: Option<f64>,
    /// Trigger a scope capture when aborting.
    #[serde(default)]
    pub capture_scope: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaitSpec {
    #[serde(default)]
    pub phase: Option<String>,
    /// Decoded Qi packet name (e.g. `ID` for header 0x71). Not a hardcoded header.
    #[serde(default)]
    pub packet: Option<String>,
    /// Header byte as number (`113`) or string (`"0x71"` / `"71"`).
    #[serde(default)]
    pub header: Option<Value>,
    /// When true, only packets/headers counted *after* this step starts count.
    #[serde(default)]
    pub rising: bool,
    #[serde(default = "default_timeout")]
    pub timeout_s: f64,
}

fn default_timeout() -> f64 {
    8.0
}

fn default_long_timeout() -> f64 {
    600.0
}

fn default_capture_timeout() -> f64 {
    60.0
}

/// Match a new live log line (rising-edge: lines present when the step starts are ignored).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaitLineSpec {
    pub regex: String,
    #[serde(default)]
    pub exclude: Option<String>,
    #[serde(default = "default_long_timeout")]
    pub timeout_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentCommandSpec {
    #[serde(default)]
    pub device_id: Option<u64>,
    pub command: Value,
    #[serde(default = "default_long_timeout")]
    pub timeout_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaveformSourceSpec {
    #[serde(default)]
    pub device_id: Option<u64>,
    /// Destination directory. Empty/omitted → GUI Save As dialog.
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default = "default_long_timeout")]
    pub timeout_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExpectSpec {
    #[serde(default)]
    pub ept: Option<bool>,
    #[serde(default)]
    pub p_min: Option<f64>,
    #[serde(default)]
    pub p_max: Option<f64>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SleepSpec {
    pub s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CaptureSpec {
    #[serde(default)]
    pub tag: Option<String>,
    /// When true, block until the PNG is written to evidence (or `timeout_s`).
    /// Default false keeps the old fire-and-forget queue used by `qi_pt_smoke`.
    #[serde(default)]
    pub save: bool,
    #[serde(default)]
    pub device_id: Option<u64>,
    #[serde(default = "default_capture_timeout")]
    pub timeout_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TestStep {
    WaitLine { wait_line: WaitLineSpec },
    InstrumentCommand {
        #[serde(rename = "instrument.command")]
        command: InstrumentCommandSpec,
    },
    WaveformSource {
        #[serde(rename = "instrument.waveform_source")]
        waveform_source: WaveformSourceSpec,
    },
    Wait { wait: WaitSpec },
    Action { action: String },
    Sleep { sleep: SleepSpec },
    Expect { expect: ExpectSpec },
    CaptureScope { capture_scope: CaptureSpec },
}

impl TestStep {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Wait { .. } => "wait",
            Self::WaitLine { .. } => "wait_line",
            Self::Action { .. } => "action",
            Self::Sleep { .. } => "sleep",
            Self::Expect { .. } => "expect",
            Self::CaptureScope { .. } => "capture_scope",
            Self::InstrumentCommand { .. } => "instrument.command",
            Self::WaveformSource { .. } => "instrument.waveform_source",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Wait { wait } => {
                let hdr = wait
                    .header
                    .as_ref()
                    .and_then(parse_header)
                    .map(|h| format!("0x{h:02X}"))
                    .unwrap_or_default();
                format!(
                    "wait {} {} {}",
                    wait.phase.as_deref().unwrap_or("-"),
                    wait.packet.as_deref().unwrap_or(""),
                    hdr
                )
            }
            Self::WaitLine { wait_line } => format!("wait_line /{}/", wait_line.regex),
            Self::Action { action } => format!("action {action}"),
            Self::Sleep { sleep } => format!("sleep {}s", sleep.s),
            Self::Expect { expect } => format!(
                "expect phase={} ept={:?}",
                expect.phase.as_deref().unwrap_or("-"),
                expect.ept
            ),
            Self::CaptureScope { capture_scope } => {
                format!("capture {}", capture_scope.tag.as_deref().unwrap_or("scope"))
            }
            Self::InstrumentCommand { command } => {
                format!("instrument.command {}", command_label(&command.command))
            }
            Self::WaveformSource { waveform_source } => format!(
                "waveform_source {}",
                waveform_source.dir.as_deref().unwrap_or("(dialog)")
            ),
        }
    }
}

fn command_label(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(obj) = v.as_object() {
        if let Some(k) = obj.keys().next() {
            return k.clone();
        }
    }
    "command".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPlan {
    pub id: String,
    #[serde(default)]
    pub macros: HashMap<String, String>,
    #[serde(default)]
    pub abort: AbortSpec,
    /// When true, `action` may be raw hex instead of a `macros` name.
    #[serde(default)]
    pub allow_raw_hex: bool,
    #[serde(default)]
    pub steps: Vec<TestStep>,
}

impl TestPlan {
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid test plan: {e}"))
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(s).map_err(|e| format!("invalid plan JSON: {e}"))?;
        Self::from_json(&v)
    }

    pub fn resolve_macro(&self, name: &str) -> Result<Option<String>, String> {
        let key = name.trim();
        if key.is_empty() || key.eq_ignore_ascii_case("NOP") || key.eq_ignore_ascii_case("NONE") {
            return Ok(None);
        }
        if let Some(hex) = self.macros.get(key) {
            return Ok(Some(clean_hex(hex)?));
        }
        if looks_like_hex(key) {
            if self.allow_raw_hex {
                return Ok(Some(clean_hex(key)?));
            }
            return Err(format!(
                "raw hex action '{key}' disabled; add plan.macros.{key} or set allow_raw_hex"
            ));
        }
        // case-insensitive macro lookup
        for (k, v) in &self.macros {
            if k.eq_ignore_ascii_case(key) {
                return Ok(Some(clean_hex(v)?));
            }
        }
        Err(format!(
            "unknown action '{key}' (add it to plan.macros or use hex / NOP)"
        ))
    }
}

fn looks_like_hex(s: &str) -> bool {
    let c: String = s.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    c.len() >= 2 && c.len() % 2 == 0 && c.len() == s.chars().filter(|ch| !ch.is_whitespace()).count()
}

pub fn clean_hex(s: &str) -> Result<String, String> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.is_empty() {
        return Ok(String::new());
    }
    if cleaned.len() % 2 != 0 {
        return Err("hex length must be even".into());
    }
    Ok(cleaned.to_ascii_uppercase())
}

/// Replace `{instruments.waveform_source_dir}` in plan JSON strings.
pub fn resolve_plan_placeholders(v: &mut Value, waveform_source_dir: &str) {
    match v {
        Value::String(s) => {
            if s.contains("{instruments.waveform_source_dir}") {
                *s = s.replace("{instruments.waveform_source_dir}", waveform_source_dir);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_plan_placeholders(item, waveform_source_dir);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                resolve_plan_placeholders(item, waveform_source_dir);
            }
        }
        _ => {}
    }
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned = clean_hex(s)?;
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let h = std::str::from_utf8(&bytes[i..i + 2]).unwrap_or("00");
        out.push(u8::from_str_radix(h, 16).map_err(|e| e.to_string())?);
        i += 2;
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunVerdict {
    Running,
    Passed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepRecord {
    pub i: usize,
    pub kind: String,
    pub label: String,
    pub t_s: f64,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TickAction {
    None,
    Send { name: String, hex: String },
    Capture {
        tag: String,
        save: bool,
        device_id: Option<u64>,
    },
    WaveformSource {
        device_id: Option<u64>,
        dir: String,
        filename: Option<String>,
        overwrite: bool,
    },
    Command {
        device_id: Option<u64>,
        command: Value,
    },
    Done,
}

pub struct TestDriver {
    pub plan: TestPlan,
    pub verdict: RunVerdict,
    pub reason: String,
    pub step: usize,
    pub step_log: Vec<StepRecord>,
    /// Last live line that satisfied `wait_line` (for the evidence pack).
    pub last_hit_line: Option<String>,
    run_t0: Instant,
    step_t0: Instant,
    action_sent: bool,
    wait_seeded: bool,
    wait_pkt_base: u32,
    wait_hdr_base: u32,
    wait_line_seed_n: u64,
    instrument_block: bool,
}

impl TestDriver {
    pub fn new(plan: TestPlan) -> Self {
        let now = Instant::now();
        Self {
            plan,
            verdict: RunVerdict::Running,
            reason: String::new(),
            step: 0,
            step_log: Vec::new(),
            last_hit_line: None,
            run_t0: now,
            step_t0: now,
            action_sent: false,
            wait_seeded: false,
            wait_pkt_base: 0,
            wait_hdr_base: 0,
            wait_line_seed_n: 0,
            instrument_block: false,
        }
    }

    pub fn elapsed_s(&self) -> f64 {
        self.run_t0.elapsed().as_secs_f64()
    }

    pub fn is_running(&self) -> bool {
        self.verdict == RunVerdict::Running
    }

    pub fn current_label(&self) -> String {
        self.plan
            .steps
            .get(self.step)
            .map(|s| s.label())
            .unwrap_or_else(|| "done".into())
    }

    pub fn abort(&mut self, reason: &str) -> TickAction {
        if !self.is_running() {
            return TickAction::None;
        }
        self.instrument_block = false;
        self.verdict = RunVerdict::Aborted;
        self.reason = reason.to_string();
        self.record("abort", reason);
        if self.plan.abort.capture_scope {
            TickAction::Capture {
                tag: "abort".into(),
                save: false,
                device_id: None,
            }
        } else {
            TickAction::Done
        }
    }

    /// Complete a blocking instrument step (`save: true` capture, command, waveform_source).
    pub fn notify_job(&mut self, ok: bool, detail: &str) -> TickAction {
        if !self.is_running() || !self.instrument_block {
            return TickAction::None;
        }
        self.instrument_block = false;
        if ok {
            self.advance(if detail.is_empty() { "ok" } else { detail });
            TickAction::None
        } else {
            self.fail(detail)
        }
    }

    pub fn tick(&mut self, brief: &LiveBrief, live_lines: &[String]) -> TickAction {
        if !self.is_running() {
            return TickAction::None;
        }
        if let Some(act) = self.check_abort(brief) {
            return act;
        }
        if self.plan.steps.is_empty() {
            return TickAction::None;
        }
        if self.step >= self.plan.steps.len() {
            return self.finish(RunVerdict::Passed, "all steps ok");
        }
        let step_elapsed = self.step_t0.elapsed().as_secs_f64();
        match self.plan.steps[self.step].clone() {
            TestStep::Wait { wait } => {
                if step_elapsed >= wait.timeout_s {
                    return self.fail(&format!("wait timeout ({:.1}s)", wait.timeout_s));
                }
                if wait.rising && !self.wait_seeded {
                    self.seed_wait(&wait, brief);
                    return TickAction::None;
                }
                match wait_met(&wait, brief, self.wait_pkt_base, self.wait_hdr_base) {
                    Ok(true) => self.advance("ok"),
                    Ok(false) => {}
                    Err(e) => return self.fail(&e),
                }
                TickAction::None
            }
            TestStep::WaitLine { wait_line } => {
                if step_elapsed >= wait_line.timeout_s {
                    return self.fail(&format!(
                        "wait_line timeout ({:.1}s)",
                        wait_line.timeout_s
                    ));
                }
                if !self.wait_seeded {
                    self.wait_line_seed_n = brief.n_lines();
                    self.wait_seeded = true;
                    return TickAction::None;
                }
                match wait_line_hit(&wait_line, brief, live_lines, self.wait_line_seed_n) {
                    Ok(Some(hit)) => {
                        self.last_hit_line = Some(hit);
                        self.advance("ok");
                    }
                    Ok(None) => {}
                    Err(e) => return self.fail(&e),
                }
                TickAction::None
            }
            TestStep::Action { action } => {
                if self.action_sent {
                    self.advance("sent");
                    return TickAction::None;
                }
                match self.plan.resolve_macro(&action) {
                    Ok(None) => {
                        self.advance("nop");
                        TickAction::None
                    }
                    Ok(Some(hex)) => {
                        self.action_sent = true;
                        TickAction::Send {
                            name: action,
                            hex,
                        }
                    }
                    Err(e) => self.fail(&e),
                }
            }
            TestStep::Sleep { sleep } => {
                if step_elapsed >= sleep.s {
                    self.advance("ok");
                }
                TickAction::None
            }
            TestStep::Expect { expect } => {
                match expect_status(&expect, brief) {
                    ExpectStatus::Pass => {
                        self.advance("ok");
                        TickAction::None
                    }
                    ExpectStatus::Fail(msg) => self.fail(&msg),
                    ExpectStatus::Wait => {
                        if step_elapsed >= expect.timeout_s {
                            if expect.ept == Some(false) && !brief.seen_ept() {
                                self.advance("ok");
                                TickAction::None
                            } else {
                                self.fail(&format!("expect timeout ({:.1}s)", expect.timeout_s))
                            }
                        } else {
                            TickAction::None
                        }
                    }
                }
            }
            TestStep::CaptureScope { capture_scope } => {
                if self.instrument_block {
                    if step_elapsed >= capture_scope.timeout_s {
                        return self.fail(&format!(
                            "capture timeout ({:.1}s)",
                            capture_scope.timeout_s
                        ));
                    }
                    return TickAction::None;
                }
                let tag = capture_scope
                    .tag
                    .clone()
                    .unwrap_or_else(|| format!("s{}", self.step));
                if capture_scope.save {
                    self.instrument_block = true;
                    TickAction::Capture {
                        tag,
                        save: true,
                        device_id: capture_scope.device_id,
                    }
                } else {
                    self.advance("queued");
                    TickAction::Capture {
                        tag,
                        save: false,
                        device_id: capture_scope.device_id,
                    }
                }
            }
            TestStep::InstrumentCommand { command } => {
                if self.instrument_block {
                    if step_elapsed >= command.timeout_s {
                        return self.fail(&format!(
                            "instrument.command timeout ({:.1}s)",
                            command.timeout_s
                        ));
                    }
                    return TickAction::None;
                }
                self.instrument_block = true;
                TickAction::Command {
                    device_id: command.device_id,
                    command: command.command,
                }
            }
            TestStep::WaveformSource { waveform_source } => {
                if self.instrument_block {
                    if step_elapsed >= waveform_source.timeout_s {
                        return self.fail(&format!(
                            "waveform_source timeout ({:.1}s)",
                            waveform_source.timeout_s
                        ));
                    }
                    return TickAction::None;
                }
                self.instrument_block = true;
                TickAction::WaveformSource {
                    device_id: waveform_source.device_id,
                    dir: waveform_source.dir.unwrap_or_default(),
                    filename: waveform_source.filename,
                    overwrite: waveform_source.overwrite,
                }
            }
        }
    }

    fn seed_wait(&mut self, wait: &WaitSpec, brief: &LiveBrief) {
        self.wait_seeded = true;
        if let Some(pkt) = wait.packet.as_deref() {
            self.wait_pkt_base = brief.packet_count(pkt);
        }
        if let Some(h) = wait.header.as_ref().and_then(parse_header) {
            self.wait_hdr_base = brief.header_count(h);
        }
    }

    fn check_abort(&mut self, brief: &LiveBrief) -> Option<TickAction> {
        let a = &self.plan.abort;
        if let Some(lim) = a.timeout_s {
            if self.elapsed_s() >= lim {
                return Some(self.fail(&format!("run timeout_s {lim}")));
            }
        }
        if a.on_ept && brief.seen_ept() {
            return Some(self.abort("ept"));
        }
        if let Some(n) = a.csum_gt {
            if brief.csum() > n {
                return Some(self.abort(&format!("csum {}>{}", brief.csum(), n)));
            }
        }
        if let Some(v) = a.vin_gt {
            if brief.vin_now().is_some_and(|x| x > v) {
                return Some(self.abort(&format!("vin {}", brief.vin_now().unwrap_or(0.0))));
            }
        }
        if let Some(v) = a.p_lt {
            if brief.p_now().is_some_and(|x| x < v) {
                return Some(self.abort(&format!("p {}", brief.p_now().unwrap_or(0.0))));
            }
        }
        None
    }

    fn advance(&mut self, result: &str) {
        self.record(result, "");
        self.step += 1;
        self.step_t0 = Instant::now();
        self.action_sent = false;
        self.wait_seeded = false;
        self.wait_pkt_base = 0;
        self.wait_hdr_base = 0;
        self.wait_line_seed_n = 0;
        self.instrument_block = false;
        if self.step >= self.plan.steps.len() {
            self.verdict = RunVerdict::Passed;
            self.reason = "all steps ok".into();
        }
    }

    pub fn fail(&mut self, reason: &str) -> TickAction {
        self.instrument_block = false;
        self.verdict = RunVerdict::Failed;
        self.reason = reason.to_string();
        self.record("fail", reason);
        if self.plan.abort.capture_scope {
            TickAction::Capture {
                tag: "fail".into(),
                save: false,
                device_id: None,
            }
        } else {
            TickAction::Done
        }
    }

    fn finish(&mut self, verdict: RunVerdict, reason: &str) -> TickAction {
        self.verdict = verdict;
        self.reason = reason.to_string();
        self.record(
            match verdict {
                RunVerdict::Passed => "pass",
                RunVerdict::Failed => "fail",
                RunVerdict::Aborted => "abort",
                RunVerdict::Running => "run",
            },
            reason,
        );
        TickAction::Done
    }

    fn record(&mut self, result: &str, extra: &str) {
        let label = if extra.is_empty() {
            self.current_label()
        } else {
            format!("{} ({extra})", self.current_label())
        };
        self.step_log.push(StepRecord {
            i: self.step,
            kind: self
                .plan
                .steps
                .get(self.step)
                .map(|s| s.kind_name().to_string())
                .unwrap_or_else(|| result.to_string()),
            label,
            t_s: round3(self.elapsed_s()),
            result: result.into(),
        });
    }
}

enum ExpectStatus {
    Pass,
    Fail(String),
    Wait,
}

fn wait_met(wait: &WaitSpec, brief: &LiveBrief, pkt_base: u32, hdr_base: u32) -> Result<bool, String> {
    if let Some(phase) = wait.phase.as_deref() {
        if !phase_matches(brief.phase(), phase) {
            return Ok(false);
        }
    }
    if let Some(pkt) = wait.packet.as_deref() {
        let n = brief.packet_count(pkt);
        if wait.rising {
            if n <= pkt_base {
                return Ok(false);
            }
        } else if n == 0 {
            return Ok(false);
        }
    }
    if let Some(raw) = wait.header.as_ref() {
        let h = parse_header(raw).ok_or_else(|| format!("invalid wait.header {raw}"))?;
        let n = brief.header_count(h);
        if wait.rising {
            if n <= hdr_base {
                return Ok(false);
            }
        } else if n == 0 {
            return Ok(false);
        }
    }
    Ok(wait.phase.is_some() || wait.packet.is_some() || wait.header.is_some())
}

fn wait_line_hit(
    spec: &WaitLineSpec,
    brief: &LiveBrief,
    live_lines: &[String],
    seed_n: u64,
) -> Result<Option<String>, String> {
    let re = Regex::new(&spec.regex).map_err(|e| format!("wait_line regex: {e}"))?;
    let excl = match spec.exclude.as_deref() {
        Some(s) if !s.is_empty() => {
            Some(Regex::new(s).map_err(|e| format!("wait_line exclude: {e}"))?)
        }
        _ => None,
    };
    let new_n = brief.n_lines().saturating_sub(seed_n) as usize;
    if new_n == 0 {
        return Ok(None);
    }
    let start = live_lines.len().saturating_sub(new_n);
    for line in &live_lines[start..] {
        if re.is_match(line) && !excl.as_ref().is_some_and(|e| e.is_match(line)) {
            return Ok(Some(line.clone()));
        }
    }
    Ok(None)
}

pub fn parse_header(v: &Value) -> Option<u8> {
    if let Some(n) = v.as_u64() {
        return u8::try_from(n).ok();
    }
    if let Some(n) = v.as_i64() {
        return u8::try_from(n).ok();
    }
    let s = v.as_str()?.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).ok();
    }
    if !s.is_empty() && s.len() <= 2 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u8::from_str_radix(s, 16).ok();
    }
    s.parse::<u8>().ok()
}

fn expect_status(ex: &ExpectSpec, brief: &LiveBrief) -> ExpectStatus {
    if let Some(want) = ex.ept {
        if want && !brief.seen_ept() {
            return ExpectStatus::Wait;
        }
        if !want && brief.seen_ept() {
            return ExpectStatus::Fail("unexpected EPT".into());
        }
    }
    if let Some(phase) = ex.phase.as_deref() {
        if !phase_matches(brief.phase(), phase) {
            return ExpectStatus::Wait;
        }
    }
    if let Some(min) = ex.p_min {
        match brief.p_now() {
            None => return ExpectStatus::Wait,
            Some(p) if p < min => return ExpectStatus::Wait,
            Some(_) => {}
        }
    }
    if let Some(max) = ex.p_max {
        if brief.p_now().is_some_and(|p| p > max) {
            return ExpectStatus::Fail(format!("p {} > max {max}", brief.p_now().unwrap_or(0.0)));
        }
    }
    if ex.ept.is_none() && ex.phase.is_none() && ex.p_min.is_none() && ex.p_max.is_none() {
        return ExpectStatus::Pass;
    }
    // "no EPT" with no other positive condition: survive until timeout.
    if ex.ept == Some(false) && ex.phase.is_none() && ex.p_min.is_none() {
        return ExpectStatus::Wait;
    }
    ExpectStatus::Pass
}

fn phase_matches(actual: QiPhase, spec: &str) -> bool {
    let s = spec.trim().to_ascii_lowercase();
    actual.as_str() == s || (s == "power_transfer" && actual == QiPhase::Pt)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brief::LiveBrief;

    #[test]
    fn wait_pt_then_pass() {
        let plan = TestPlan::from_str(
            r#"{
              "id": "t",
              "abort": { "timeout_s": 30 },
              "steps": [ { "wait": { "phase": "pt", "timeout_s": 5 } } ]
            }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let mut b = LiveBrief::new();
        assert!(matches!(d.tick(&b, &[]), TickAction::None));
        b.ingest_line(0, "ASK 01 80 81 F ");
        b.ingest_line(1, "ASK 03 00 03 F ");
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn unknown_macro_fails() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "action": "NOPE" } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        let act = d.tick(&b, &[]);
        assert!(matches!(act, TickAction::Done) || d.verdict == RunVerdict::Failed);
        assert_eq!(d.verdict, RunVerdict::Failed);
    }

    #[test]
    fn macro_send() {
        let plan = TestPlan::from_str(
            r#"{
              "id": "t",
              "macros": { "PING": "AA55" },
              "steps": [ { "action": "PING" } ]
            }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        match d.tick(&b, &[]) {
            TickAction::Send { hex, .. } => assert_eq!(hex, "AA55"),
            other => panic!("expected send, got {other:?}"),
        }
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn raw_hex_requires_flag() {
        let plan = TestPlan::from_str(r#"{ "id": "t", "steps": [ { "action": "AABB" } ] }"#).unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Failed);

        let plan = TestPlan::from_str(
            r#"{ "id": "t", "allow_raw_hex": true, "steps": [ { "action": "AABB" } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        match d.tick(&b, &[]) {
            TickAction::Send { hex, .. } => assert_eq!(hex, "AABB"),
            other => panic!("expected send, got {other:?}"),
        }
    }

    #[test]
    fn wait_packet_uses_counts_not_notables() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "wait": { "packet": "CE", "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let mut b = LiveBrief::new();
        assert!(matches!(d.tick(&b, &[]), TickAction::None));
        b.ingest_line(0, "ASK 03 00 03 F ");
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn abort_timeout_fails_empty_plan() {
        let plan = TestPlan::from_str(r#"{ "id": "t", "abort": { "timeout_s": 0 } }"#).unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Failed);
        assert!(d.reason.contains("timeout_s"));
    }

    #[test]
    fn wait_packet_rising_ignores_existing() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "wait": { "packet": "CE", "rising": true, "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let mut b = LiveBrief::new();
        b.ingest_line(0, "ASK 03 00 03 F ");
        assert!(matches!(d.tick(&b, &[]), TickAction::None));
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Running);
        b.ingest_line(1, "ASK 03 00 03 F ");
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn wait_header_hex() {
        assert_eq!(parse_header(&serde_json::json!("0x71")), Some(0x71));
        assert_eq!(parse_header(&serde_json::json!(0x71)), Some(0x71));
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "wait": { "header": "0x03", "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let mut b = LiveBrief::new();
        assert!(matches!(d.tick(&b, &[]), TickAction::None));
        b.ingest_line(0, "ASK 03 00 03 F ");
        let _ = d.tick(&b, &[]);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn wait_line_rising() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "wait_line": { "regex": "ASK\\s+71", "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let mut b = LiveBrief::new();
        let old = vec!["ASK 71 already on screen".to_string()];
        b.ingest_line(0, &old[0]);
        assert!(matches!(d.tick(&b, &old), TickAction::None));
        let _ = d.tick(&b, &old);
        assert_eq!(d.verdict, RunVerdict::Running);
        let new_line = "ASK 71 01 02 F ".to_string();
        b.ingest_line(1, &new_line);
        let lines = vec![old[0].clone(), new_line.clone()];
        let _ = d.tick(&b, &lines);
        assert_eq!(d.verdict, RunVerdict::Passed);
        assert_eq!(d.last_hit_line.as_deref(), Some(new_line.as_str()));
    }

    #[test]
    fn capture_save_blocks_until_notify() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "capture_scope": { "tag": "x", "save": true, "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        match d.tick(&b, &[]) {
            TickAction::Capture { save, tag, .. } => {
                assert!(save);
                assert_eq!(tag, "x");
            }
            other => panic!("expected capture, got {other:?}"),
        }
        assert_eq!(d.verdict, RunVerdict::Running);
        let _ = d.notify_job(true, "saved");
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn instrument_command_parses_string_unit() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "instrument.command": { "command": "ScopeStop", "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        match d.tick(&b, &[]) {
            TickAction::Command { command, .. } => {
                assert_eq!(command, serde_json::json!("ScopeStop"));
            }
            other => panic!("expected command, got {other:?}"),
        }
        let _ = d.notify_job(true, "ok");
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn waveform_source_blocks() {
        let plan = TestPlan::from_str(
            r#"{ "id": "t", "steps": [ { "instrument.waveform_source": { "dir": "D:/isf", "timeout_s": 5 } } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        match d.tick(&b, &[]) {
            TickAction::WaveformSource { dir, .. } => assert_eq!(dir, "D:/isf"),
            other => panic!("expected waveform_source, got {other:?}"),
        }
        assert_eq!(d.verdict, RunVerdict::Running);
        let _ = d.notify_job(true, r"D:/isf/wave.isf");
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn parse_scope_stop_string_and_null() {
        use crate::instrument::parse_control_command;
        assert!(matches!(
            parse_control_command(&serde_json::json!("ScopeStop")).unwrap(),
            crate::instrument::ControlCommand::ScopeStop
        ));
        assert!(matches!(
            parse_control_command(&serde_json::json!({ "ScopeStop": null })).unwrap(),
            crate::instrument::ControlCommand::ScopeStop
        ));
    }
}
