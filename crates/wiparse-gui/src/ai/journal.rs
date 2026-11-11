use egui::Context;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use wiparse_core::ai::workflow::Workflow;

/// Project-local investigation journal. Checkpoints contain goals and source
/// references; neither credentials nor pending instrument commands are restorable.
#[derive(Clone)]
pub(crate) struct Journal {
    path: PathBuf,
}

pub(crate) fn command_fingerprint(method:&str,params:&Value)->String {
    let (method, params) = if method == "platform.invoke" {
        (params["method"].as_str().unwrap_or(method), &params["params"])
    } else { (method, params) };
    let mut payload=params.clone();
    if let Some(fields)=payload.as_object_mut(){for transient in ["timeout_s","timeout_ms","command_id"]{fields.remove(transient);}}
    wiparse_core::update::hex_sha256(json!([method,payload]).to_string().as_bytes())
}
pub(crate) struct Recovery {
    pub(crate) project: String,
    pub(crate) draft: String,
    pub(crate) uncertain: Vec<String>,
    pub(crate) question: String,
    state: Value,
    reports: Vec<(String,String)>,
}
impl Recovery {
    pub(crate) fn restore(&self, workflow:&mut Workflow) {
        workflow.resume_checkpoint(&self.state,&self.question,&self.reports);
    }
    pub(crate) fn checkpoint_reports(&self,journal:&Journal,id:&str,entry:&str,workflow:&Workflow) {
        for (report,_) in &self.reports {
            let hash=self.state["saved_reports"].as_array().into_iter().flatten().filter_map(Value::as_str)
                .filter_map(|s|serde_json::from_str::<Value>(s).ok()).find(|r|r["report_id"]==report.as_str())
                .and_then(|r|r["report_sha256"].as_str().map(str::to_owned));
            if let Some(sha)=hash {
                journal.record(id,entry,&self.question,"resumed",workflow,Some(("report.create",&json!({
                    "report_id":report,"report_sha256":sha,"evidence_id":format!("restored-report-{report}")
                }))));
            }
        }
    }
    pub(crate) fn origin(&self)->&str {self.state["previous_task_id"].as_str().unwrap_or("")}
}
impl Journal {
    pub(crate) fn for_project(ctx: &Context, project: &str) -> Option<Self> {
        let root = ctx.data(|d| d.get_temp::<PathBuf>(egui::Id::new("wiparse-ai-project-root")))?;
        let directory = wiparse_core::project::project_dir(&root, project);
        if directory.file_name()?.to_str()? != project {
            return None;
        }
        Some(Self {
            path: directory.join("agent-tasks.sqlite3"),
        })
    }

    fn accessible(&self) -> bool {
        let Some(project) = self.path.parent() else {
            return false;
        };
        let Some(parent) = project.parent() else {
            return false;
        };
        let (Ok(real_project), Ok(real_parent)) = (project.canonicalize(), parent.canonicalize())
        else {
            return false;
        };
        if real_project.parent() != Some(real_parent.as_path()) {
            return false;
        }
        !self.path.exists()
            || self
                .path
                .canonicalize()
                .is_ok_and(|p| p.parent() == Some(real_project.as_path()))
    }

    fn save(
        &self,
        id: &str,
        entry: &str,
        question: &str,
        phase: &str,
        workflow: &Workflow,
        reference: Option<&Value>,
    ) -> rusqlite::Result<()> {
        if !self.accessible() {
            return Err(rusqlite::Error::InvalidPath(self.path.clone()));
        }
        let mut db = Connection::open(&self.path)?;
        db.busy_timeout(std::time::Duration::from_millis(100))?;
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY,entry TEXT NOT NULL,question TEXT NOT NULL,phase TEXT NOT NULL,progress TEXT NOT NULL,updated INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS observations(task TEXT NOT NULL,seq INTEGER PRIMARY KEY,reference TEXT NOT NULL); CREATE INDEX IF NOT EXISTS observations_by_task ON observations(task,seq);")?;
        tx.execute("INSERT INTO tasks VALUES(?1,?2,?3,?4,?5,unixepoch()) ON CONFLICT(id) DO UPDATE SET question=excluded.question,phase=excluded.phase,progress=excluded.progress,updated=excluded.updated",
            params![id,entry,question,phase,workflow.state().to_string()])?;
        if let Some(reference) = reference {
            tx.execute(
                "INSERT INTO observations(task,reference) VALUES(?1,?2)",
                params![id, reference.to_string()],
            )?;
        }
        tx.commit()
    }

    pub(crate) fn record(
        &self,
        id: &str,
        entry: &str,
        question: &str,
        phase: &str,
        w: &Workflow,
        evidence: Option<(&str, &Value)>,
    ) {
        let reference = evidence.map(|(method, v)| {
            let payload = v.get("result").unwrap_or(v);
            let data = payload.get("data").unwrap_or(payload);
            let d = data.get("job").filter(|j|j.is_object()).unwrap_or(data);
            let mut r = json!({"tool":method,"ok":v.get("ok"),"state":d.get("state")});
            for key in [
                "report_id",
                "report_sha256",
                "previous_task_id",
                "evidence_id",
                "source_id",
                "query_id",
                "decoded_text_sha256",
                "start_line",
                "next_line",
                "first_frame",
                "last_frame",
                "device_id",
                "run_id",
                "command_id",
                "command_fingerprint",
                "job_id",
                "job_session",
            ] {
                if let Some(x) = d
                    .get(key)
                    .or_else(|| v.get(key))
                    .filter(|x| x.is_number() || x.as_str().is_some_and(|s| s.len() <= 256))
                {
                    r[key] = x.clone();
                }
            }
            r
        });
        if let Err(error) = self.save(id, entry, question, phase, w, reference.as_ref()) {
            tracing::warn!(%error,"could not persist agent task");
        }
    }

    fn recent(&self) -> Result<Vec<(String, String, String, Value)>, String> {
        if !Path::new(&self.path).is_file() {
            return Ok(vec![]);
        }
        if !self.accessible() {
            return Err("任务记录路径已变化".into());
        }
        let db =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| e.to_string())?;
        let mut stmt=db.prepare("SELECT entry,question,phase,progress,id FROM tasks ORDER BY updated DESC,rowid DESC LIMIT 16").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let (e, q, p, s, id) = row.map_err(|e| e.to_string())?;
            let mut state: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
            let mut stmt = db
                .prepare(
                    "SELECT reference FROM observations WHERE task=?1 ORDER BY seq DESC LIMIT 8",
                )
                .map_err(|e| e.to_string())?;
            state["previous_task_id"]=json!(&id);
            let refs = stmt
                .query_map([&id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            state["references"] = json!(refs);
            let mut reports=db.prepare("SELECT reference FROM observations WHERE task=?1 AND json_extract(reference,'$.tool')='report.create' AND json_extract(reference,'$.report_sha256') IS NOT NULL ORDER BY seq DESC LIMIT 64").map_err(|e|e.to_string())?;
            let ids=reports.query_map([&id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            // A task may be resumed more than once. Restrict readbacks to that
            // branch and to earlier dispatches of the same operation.
            let lineage = "WITH RECURSIVE lineage(id) AS (
                SELECT ?1 UNION SELECT json_extract(o.reference,'$.previous_task_id')
                FROM observations o JOIN lineage l ON o.task=l.id
                WHERE json_extract(o.reference,'$.tool')='task.resume'
            )";
            let readback = "NOT EXISTS (SELECT 1 FROM observations a JOIN lineage l ON l.id=a.task
                WHERE a.seq>o.seq AND json_extract(a.reference,'$.state')='readback_acknowledged'
                AND json_extract(a.reference,'$.command_fingerprint')=json_extract(o.reference,'$.command_fingerprint'))";
            let unresolved = |condition: &str| -> Result<Vec<String>,String> {
                let sql = format!("{lineage} SELECT o.reference FROM observations o JOIN lineage l ON o.task=l.id
                                   WHERE {condition} AND {readback} ORDER BY o.seq DESC");
                let mut stmt=db.prepare(&sql).map_err(|e|e.to_string())?;
                let rows=stmt.query_map([&id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
                rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
            };
            let outstanding=unresolved("json_extract(o.reference,'$.state')='accepted' AND json_extract(o.reference,'$.job_id') IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM observations c WHERE c.task=o.task AND c.seq>o.seq
                AND json_extract(c.reference,'$.state')='completed' AND json_extract(c.reference,'$.job_id')=json_extract(o.reference,'$.job_id')
                AND json_extract(c.reference,'$.job_session')=json_extract(o.reference,'$.job_session'))")?;
            let unknown=unresolved("json_extract(o.reference,'$.state')='dispatching' AND json_extract(o.reference,'$.command_fingerprint') IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM observations c WHERE c.task=o.task AND c.seq>o.seq
                AND json_extract(c.reference,'$.state') IN ('completed','accepted')
                AND json_extract(c.reference,'$.command_id')=json_extract(o.reference,'$.command_id'))")?;
            state["unacknowledged"]=json!(unknown);
            state["outstanding_jobs"]=json!(outstanding);
            state["saved_reports"]=json!(ids);
            Ok((e, q, p, state))
        })
        .collect()
    }
    pub(crate) fn menu(&self, ui: &mut egui::Ui) -> Option<Recovery> {
        let mut draft = None;
        ui.menu_button("任务记录",|ui|{
            // Read once per opening, including empty results.
            let id=ui.id().with((&self.path,"journal-list"));
            let rows=ui.data_mut(|d| {
                let old=d.get_temp::<(u64,Result<Vec<(String,String,String,Value)>,String>)>(id);
                old
            });
            let frame=ui.ctx().cumulative_pass_nr();
            let rows=match rows {Some((previous,rows)) if previous+1>=frame=>rows,_=>self.recent()};
            ui.data_mut(|d|d.insert_temp(id,(frame,rows.clone())));
            match rows {
                Err(e)=>{ui.label(e);}
                Ok(rows) if rows.is_empty()=>{ui.weak("当前工程暂无记录");}
                Ok(rows)=>{for (entry,question,phase,state) in rows {
                    ui.horizontal_wrapped(|ui|{
                        ui.weak(format!("{entry} · {phase}"));
                        if ui.button(question.chars().take(40).collect::<String>()).on_hover_text(&question).clicked(){
                            let remaining=state["open_goals"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("、");
                            let refs=state["references"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                            let dir=self.path.parent().unwrap();
                            let project=dir.file_name().unwrap().to_string_lossy().into_owned();
                            let reports=state["saved_reports"].as_array().into_iter().flatten().filter_map(Value::as_str)
                                .filter_map(|raw|serde_json::from_str::<Value>(raw).ok()).filter_map(|reference|{
                                    let id=reference["report_id"].as_str()?;
                                    uuid::Uuid::parse_str(id).ok()?;
                                    let root=dir.canonicalize().ok()?.join("ai-reports").join(id);
                                    let path=root.join("report.md").canonicalize().ok()?;
                                    if path.parent()!=Some(root.as_path()) {return None;}
                                    let expected=reference["report_sha256"].as_str()?;
                                    wiparse_core::update::verify_file_sha256(&path,expected).ok()?;
                                    Some((id.to_owned(),reference["evidence_id"].as_str().unwrap_or("").to_owned()))
                                }).collect::<Vec<_>>();
                            let report_ids=reports.iter().map(|(id,_)|id.as_str()).collect::<Vec<_>>().join("、");
                            let receipts=ui.ctx().data(|d|d.get_temp::<(String,crate::instrument_control::InstrumentReceipts)>(egui::Id::new("wiparse-ai-instrument-receipts")));
                            let mut uncertain=state["unacknowledged"].as_array().into_iter().flatten().filter_map(Value::as_str)
                                .filter_map(|raw|serde_json::from_str::<Value>(raw).ok()).filter_map(|r|r["command_fingerprint"].as_str().map(str::to_owned)).collect::<Vec<_>>();
                            let jobs=state["outstanding_jobs"].as_array().into_iter().flatten().filter_map(Value::as_str)
                              .filter_map(|raw|serde_json::from_str::<Value>(raw).ok()).map(|job|{
                                let completed=receipts.as_ref().filter(|(session,_)|Some(session.as_str())==job["job_session"].as_str())
                                  .and_then(|(_,history)|history.lookup(job["job_id"].as_u64()?))
                                  .filter(|r|r.device_id==job["device_id"].as_u64());
                                if completed.is_none(){if let Some(key)=job["command_fingerprint"].as_str(){uncertain.push(key.to_owned());}}
                                json!({"method":job["tool"],"job_id":job["job_id"],"device_id":job["device_id"],
                                    "outcome":completed.map_or("unknown_readback_required",|r|if r.ok{"driver_completed"}else{"driver_failed"}),"physical_verified":false})
                              }).collect::<Vec<_>>();
                            let text=format!("继续调查。原始目标：{question}\n上次未决交付：{remaining}\n已校验报告正文：{report_ids}\n设备任务回执：{}\n历史定位信息（须核验来源）：{refs}\n从未决步骤继续；设备动作先读回，不重发历史命令。",json!(jobs));
                            let mut text=text;
                            if text.len()>15000 {let mut end=15000;while !text.is_char_boundary(end){end-=1;}text.truncate(end);}
                            draft=Some(Recovery{project,draft:text,question,state,reports,uncertain});
                            ui.close_menu();
                        }
                    });
                }}
            }
        });
        draft
    }
}

pub(crate) fn show_progress(ui: &mut egui::Ui, w: &Workflow) {
    use wiparse_core::ai::workflow::State;
    let rows = w.progress();
    if rows.is_empty() {
        return;
    }
    let done = rows.iter().filter(|(_, s)| s == &State::Done).count();
    egui::CollapsingHeader::new(format!("任务进度 {done}/{}", rows.len()))
        .id_salt("agent-goals")
        .show(ui, |ui| {
            for (title, state) in &rows {
                let label = match state {
                    State::Pending => "待办",
                    State::Running => "执行中",
                    State::Done => "已验证",
                    State::Blocked => "阻塞",
                };
                ui.label(format!("{label} · {title}"));
            }
            for step in &w.steps {
                if let Some(reason) = &step.reason {
                    ui.weak(reason);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_resume_preserves_unknown_writes_until_branch_readback() {
        let dir=tempfile::tempdir().unwrap();
        let journal=Journal{path:dir.path().join("agent.sqlite3")};
        let flow=Workflow::default();
        let write=|id:&str, event:Value| journal.record(id,"serial","diagnose","running",&flow,Some(("instrument.command",&event)));
        let resume=|id:&str,parent:&str| journal.record(id,"serial","diagnose","resumed",&flow,Some(("task.resume",&json!({"previous_task_id":parent}))));
        let unknown=|id:&str| journal.recent().unwrap().into_iter().find(|r|r.3["previous_task_id"]==id).unwrap().3["unacknowledged"].as_array().unwrap().len();
        write("parent",json!({"state":"dispatching","command_id":"one","command_fingerprint":"set-v"}));
        resume("child","parent");
        resume("next","child");
        resume("sibling","parent");
        assert_eq!(unknown("next"),1);
        write("child",json!({"state":"readback_acknowledged","command_fingerprint":"set-v"}));
        assert_eq!(unknown("next"),0);
        assert_eq!(unknown("sibling"),1);
        write("next",json!({"state":"dispatching","command_id":"two","command_fingerprint":"set-v"}));
        assert_eq!(unknown("next"),1);
    }

    #[test]
    fn reopened_task_retains_goal_and_receipt_without_command_parameters() {
        let root = std::env::temp_dir().join(format!("wiparse-journal-{}", uuid::Uuid::new_v4()));
        let directory = wiparse_core::project::project_dir(&root, "lab");
        std::fs::create_dir_all(&directory).unwrap();
        let journal = Journal {
            path: directory.join("agent-tasks.sqlite3"),
        };
        let mut workflow = Workflow::default();
        workflow.bind_request("生成报告");
        journal.record(
            "t",
            "serial",
            "生成报告",
            "awaiting_readback",
            &workflow,
            Some((
                "instrument.command",
                &json!({"result":{"data":{"command_id":"c1","params":{"payload":"test-secret"}}}}),
            )),
        );
        let reopened = Journal {
            path: journal.path.clone(),
        };
        let rows = reopened.recent().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "awaiting_readback");
        assert!(rows[0].3["references"].to_string().contains("c1"));
        assert!(!rows[0].3.to_string().contains("test-secret"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
