//! Deterministic closed-loop test plan + driver (no LLM on the hot path).

use crate::brief::{LiveBrief, QiPhase};
use serde::{Deserialize, Serialize};
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
    #[serde(default)]
    pub packet: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_s: f64,
}

fn default_timeout() -> f64 {
    8.0
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TestStep {
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
            Self::Action { .. } => "action",
            Self::Sleep { .. } => "sleep",
            Self::Expect { .. } => "expect",
            Self::CaptureScope { .. } => "capture_scope",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Wait { wait } => format!(
                "wait {} {}",
                wait.phase.as_deref().unwrap_or("-"),
                wait.packet.as_deref().unwrap_or("")
            ),
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
        }
    }
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

#[derive(Debug, Clone)]
pub enum TickAction {
    None,
    Send { name: String, hex: String },
    Capture { tag: String },
    Done,
}

pub struct TestDriver {
    pub plan: TestPlan,
    pub verdict: RunVerdict,
    pub reason: String,
    pub step: usize,
    pub step_log: Vec<StepRecord>,
    run_t0: Instant,
    step_t0: Instant,
    action_sent: bool,
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
            run_t0: now,
            step_t0: now,
            action_sent: false,
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
        self.verdict = RunVerdict::Aborted;
        self.reason = reason.to_string();
        self.record("abort", reason);
        if self.plan.abort.capture_scope {
            TickAction::Capture {
                tag: "abort".into(),
            }
        } else {
            TickAction::Done
        }
    }

    pub fn tick(&mut self, brief: &LiveBrief) -> TickAction {
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
                if wait_met(&wait, brief) {
                    self.advance("ok");
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
                let tag = capture_scope
                    .tag
                    .clone()
                    .unwrap_or_else(|| format!("s{}", self.step));
                self.advance("queued");
                TickAction::Capture { tag }
            }
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
        if self.step >= self.plan.steps.len() {
            self.verdict = RunVerdict::Passed;
            self.reason = "all steps ok".into();
        }
    }

    pub fn fail(&mut self, reason: &str) -> TickAction {
        self.verdict = RunVerdict::Failed;
        self.reason = reason.to_string();
        self.record("fail", reason);
        if self.plan.abort.capture_scope {
            TickAction::Capture {
                tag: "fail".into(),
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

fn wait_met(wait: &WaitSpec, brief: &LiveBrief) -> bool {
    if let Some(phase) = wait.phase.as_deref() {
        if !phase_matches(brief.phase(), phase) {
            return false;
        }
    }
    if let Some(pkt) = wait.packet.as_deref() {
        if !brief.seen_packet(pkt) {
            return false;
        }
    }
    wait.phase.is_some() || wait.packet.is_some()
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
        assert!(matches!(d.tick(&b), TickAction::None));
        b.ingest_line(0, "ASK 01 80 81 F ");
        b.ingest_line(1, "ASK 03 00 03 F ");
        let _ = d.tick(&b);
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
        let act = d.tick(&b);
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
        match d.tick(&b) {
            TickAction::Send { hex, .. } => assert_eq!(hex, "AA55"),
            other => panic!("expected send, got {other:?}"),
        }
        let _ = d.tick(&b);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn raw_hex_requires_flag() {
        let plan = TestPlan::from_str(r#"{ "id": "t", "steps": [ { "action": "AABB" } ] }"#).unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        let _ = d.tick(&b);
        assert_eq!(d.verdict, RunVerdict::Failed);

        let plan = TestPlan::from_str(
            r#"{ "id": "t", "allow_raw_hex": true, "steps": [ { "action": "AABB" } ] }"#,
        )
        .unwrap();
        let mut d = TestDriver::new(plan);
        match d.tick(&b) {
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
        assert!(matches!(d.tick(&b), TickAction::None));
        b.ingest_line(0, "ASK 03 00 03 F ");
        let _ = d.tick(&b);
        assert_eq!(d.verdict, RunVerdict::Passed);
    }

    #[test]
    fn abort_timeout_fails_empty_plan() {
        let plan = TestPlan::from_str(r#"{ "id": "t", "abort": { "timeout_s": 0 } }"#).unwrap();
        let mut d = TestDriver::new(plan);
        let b = LiveBrief::new();
        let _ = d.tick(&b);
        assert_eq!(d.verdict, RunVerdict::Failed);
        assert!(d.reason.contains("timeout_s"));
    }
}
