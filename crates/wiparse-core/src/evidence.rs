//! On-disk evidence pack for a closed-loop test run.

use crate::brief::{BriefEvent, LiveBrief, MetricWindow};
use crate::testrun::{RunVerdict, StepRecord, TestPlan};
use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const PRE_S: f64 = 0.05;
const POST_S: f64 = 0.10;

#[derive(Debug, Clone, Serialize)]
pub struct CorrelateRow {
    pub t: f64,
    pub ev: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub m: MetricWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<ScopeNote>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeNote {
    pub tag: String,
    pub requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<u64>,
    pub ok: bool,
}

struct PendingCorr {
    close_at: f64,
    ev: BriefEvent,
}

pub struct EvidencePack {
    pub dir: PathBuf,
    pub run_id: String,
    events: BufWriter<File>,
    metrics: BufWriter<File>,
    serial: BufWriter<File>,
    correlate: Vec<CorrelateRow>,
    pending: Vec<PendingCorr>,
    pub plan: TestPlan,
    started_at: String,
    port: String,
    baud: u32,
}

impl EvidencePack {
    pub fn create(
        root: &Path,
        plan: TestPlan,
        port: &str,
        baud: u32,
    ) -> Result<Self, String> {
        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        let id = sanitize(&plan.id);
        let run_id = format!("{stamp}_{id}");
        let dir = root.join(&run_id);
        fs::create_dir_all(dir.join("scope")).map_err(|e| e.to_string())?;
        let events = BufWriter::new(
            File::create(dir.join("events.jsonl")).map_err(|e| e.to_string())?,
        );
        let mut metrics = BufWriter::new(
            File::create(dir.join("metrics.csv")).map_err(|e| e.to_string())?,
        );
        writeln!(metrics, "t,v_in,i_in,v_out,i_out,v_bat,i_bat,p,eff")
            .map_err(|e| e.to_string())?;
        let serial = BufWriter::with_capacity(
            64 * 1024,
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("serial.txt"))
                .map_err(|e| e.to_string())?,
        );
        let pack = Self {
            dir,
            run_id,
            events,
            metrics,
            serial,
            correlate: Vec::new(),
            pending: Vec::new(),
            plan,
            started_at: Local::now().to_rfc3339(),
            port: port.into(),
            baud,
        };
        pack.write_manifest(None, &[], None)?;
        Ok(pack)
    }

    pub fn append_serial_line(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.serial, "{line}").map_err(|e| e.to_string())
    }

    pub fn on_metric(&mut self, t: f64, m: &crate::metrics::MetricSample) -> Result<(), String> {
        writeln!(
            self.metrics,
            "{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2}",
            t, m.v_in, m.i_in, m.v_out, m.i_out, m.v_bat, m.i_bat, m.p, m.eff
        )
        .map_err(|e| e.to_string())
    }

    pub fn on_event(&mut self, ev: &BriefEvent) -> Result<(), String> {
        let line = serde_json::to_string(ev).map_err(|e| e.to_string())?;
        writeln!(self.events, "{line}").map_err(|e| e.to_string())?;
        self.pending.push(PendingCorr {
            close_at: ev.t + POST_S,
            ev: ev.clone(),
        });
        Ok(())
    }

    pub fn take_due_events(&mut self, now_s: f64) -> Vec<BriefEvent> {
        let mut due = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if now_s >= self.pending[i].close_at {
                due.push(self.pending.remove(i).ev);
            } else {
                i += 1;
            }
        }
        due
    }

    pub fn push_correlate(&mut self, row: CorrelateRow) {
        self.correlate.push(row);
    }

    pub fn note_scope(&mut self, tag: &str, device_id: Option<u64>, ok: bool, t: f64) {
        self.correlate.push(CorrelateRow {
            t,
            ev: format!("SCOPE {tag}"),
            reason: None,
            m: MetricWindow::default(),
            s: Some(ScopeNote {
                tag: tag.into(),
                requested: true,
                device_id,
                ok,
            }),
        });
    }

    pub fn note_artifact(&mut self, kind: &str, path: &str, t: f64) {
        self.correlate.push(CorrelateRow {
            t,
            ev: format!("{kind} {path}"),
            reason: None,
            m: MetricWindow::default(),
            s: None,
        });
    }

    pub fn close_pending(&mut self, now_s: f64, mut window: impl FnMut(f64, f64) -> crate::brief::MetricWindow) {
        for ev in self.take_due_events(now_s) {
            let w = window(ev.t - PRE_S, ev.t + POST_S);
            self.correlate.push(CorrelateRow {
                t: ev.t,
                ev: ev.k,
                reason: ev.x,
                m: w,
                s: None,
            });
        }
    }

    pub fn finalize(
        &mut self,
        brief: &LiveBrief,
        verdict: RunVerdict,
        reason: &str,
        steps: &[StepRecord],
        live_log: Option<&Path>,
    ) -> Result<Value, String> {
        self.close_pending(brief.elapsed_s() + POST_S, |a, b| brief.metrics_window(a, b));
        let _ = self.events.flush();
        let _ = self.metrics.flush();
        let _ = self.serial.flush();
        let brief_json = brief.snapshot_json(0, None);
        fs::write(
            self.dir.join("brief_final.json"),
            serde_json::to_vec(&brief_json).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::write(
            self.dir.join("correlate.json"),
            serde_json::to_vec_pretty(&self.correlate).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.write_manifest(Some((verdict, reason)), steps, live_log)?;
        let skeleton = render_skeleton(self, verdict, reason, steps, &brief_json);
        fs::write(self.dir.join("report.skeleton.md"), skeleton).map_err(|e| e.to_string())?;
        Ok(self.pack_summary(verdict, reason, steps, &brief_json))
    }

    pub fn pack_summary(
        &self,
        verdict: RunVerdict,
        reason: &str,
        steps: &[StepRecord],
        brief: &Value,
    ) -> Value {
        let corr: Vec<&CorrelateRow> = self.correlate.iter().take(12).collect();
        json!({
            "dir": self.dir.display().to_string(),
            "run_id": self.run_id,
            "plan_id": self.plan.id,
            "verdict": verdict,
            "reason": reason,
            "steps": steps,
            "brief": brief,
            "correlate": corr,
            "files": [
                "manifest.json",
                "serial.txt",
                "metrics.csv",
                "events.jsonl",
                "correlate.json",
                "brief_final.json",
                "report.skeleton.md"
            ],
            "skeleton": self.dir.join("report.skeleton.md").display().to_string(),
        })
    }

    fn write_manifest(
        &self,
        done: Option<(RunVerdict, &str)>,
        steps: &[StepRecord],
        live_log: Option<&Path>,
    ) -> Result<(), String> {
        let (verdict, reason) = match done {
            Some((v, r)) => (Some(v), Some(r)),
            None => (None, None),
        };
        let body = json!({
            "run_id": self.run_id,
            "plan_id": self.plan.id,
            "started_at": self.started_at,
            "ended_at": done.map(|_| Local::now().to_rfc3339()),
            "port": self.port,
            "baud": self.baud,
            "verdict": verdict,
            "reason": reason,
            "steps": steps,
            "live_log": live_log.map(|p| p.display().to_string()),
            "plan": self.plan,
        });
        fs::write(
            self.dir.join("manifest.json"),
            serde_json::to_vec_pretty(&body).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
}

fn sanitize(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "run".into()
    } else {
        s.chars().take(40).collect()
    }
}

fn render_skeleton(
    pack: &EvidencePack,
    verdict: RunVerdict,
    reason: &str,
    steps: &[StepRecord],
    brief: &Value,
) -> String {
    let mut md = String::new();
    md.push_str("# WiParse test report\n\n");
    md.push_str("## 1. Conclusion\n\n");
    md.push_str(&format!(
        "- Verdict: **{:?}**\n- Reason: {reason}\n- Plan: `{}`\n- Run: `{}`\n\n",
        verdict, pack.plan.id, pack.run_id
    ));
    md.push_str("_AI: write a 3–6 sentence conclusion and next-step advice here._\n\n");
    md.push_str("## 2. Timeline\n\n");
    md.push_str("| t_s | step | result |\n|-----|------|--------|\n");
    for s in steps {
        md.push_str(&format!(
            "| {:.2} | {} | {} |\n",
            s.t_s,
            escape_md(&s.label),
            s.result
        ));
    }
    md.push('\n');
    md.push_str("## 3. Protocol brief\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(brief).unwrap_or_else(|_| "{}".into()));
    md.push_str("\n```\n\n");
    md.push_str("## 4. Correlation\n\n");
    md.push_str("| t | event | ΔP | Vin |\n|---|-------|-----|-----|\n");
    for c in pack.correlate.iter().take(20) {
        md.push_str(&format!(
            "| {:.3} | {} | {:.3} | {:?}|\n",
            c.t,
            escape_md(&c.ev),
            c.m.dp,
            c.m.vin
        ));
    }
    md.push_str("\nScope captures live under `scope/` when an instrument was connected.\n\n");
    md.push_str("## 5. Evidence index\n\n");
    md.push_str(&format!("Directory: `{}`\n\n", pack.dir.display()));
    md.push_str("- `serial.txt` — full serial (do not paste into the model)\n");
    md.push_str("- `metrics.csv` — AA55 samples\n");
    md.push_str("- `events.jsonl` — notable Qi events\n");
    md.push_str("- `correlate.json` — event ↔ metrics windows\n");
    md.push_str("- `brief_final.json` — compact session facts\n\n");
    md.push_str("## 6. Advice\n\n_AI: fill this section._\n");
    md
}

fn escape_md(s: &str) -> String {
    s.replace('|', "\\|")
}

pub fn evidence_root() -> PathBuf {
    crate::paths::project_path("evidence")
}
