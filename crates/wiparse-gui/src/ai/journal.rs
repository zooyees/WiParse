use egui::Context;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use wiparse_core::ai::workflow::Workflow;

fn is_link_or_reparse(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else { return false; };
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

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
#[derive(Clone)]
pub(crate) struct Recovery {
    pub(crate) project: String,
    pub(crate) draft: String,
    pub(crate) uncertain: Vec<String>,
    pub(crate) question: String,
    state: Value,
    reports: Vec<(String,String)>,
}
impl Recovery {
    #[cfg(test)]
    pub(crate) fn incompatible_test_fixture(project: &str, draft: &str) -> Self {
        Self {
            project: project.into(), draft: draft.into(), uncertain: Vec::new(),
            question: draft.into(), state: json!({"skill_ids":["serial_log"],"skills":[]}),
            reports: Vec::new(),
        }
    }
    pub(crate) fn validate_skills(&self) -> Result<(), String> {
        let ids: Vec<String> = if self.state["skill_ids"].is_null() {
            Vec::new()
        } else {
            serde_json::from_value(self.state["skill_ids"].clone())
                .map_err(|_| "SKILL.INVALID_CHECKPOINT".to_owned())?
        };
        let current = wiparse_core::ai::skills::references(&ids);
        wiparse_core::ai::skills::select(&ids)?;
        if !ids.is_empty() && self.state["skills"] != json!(current) {
            return Err("SKILL.VERSION_CHANGED: 技能版本不兼容，请重新创建任务".into());
        }
        Ok(())
    }
    pub(crate) fn restore(&self, workflow:&mut Workflow) {
        workflow.resume_checkpoint(&self.state,&self.question,&self.reports);
    }
    pub(crate) fn origin(&self)->&str {self.state["previous_task_id"].as_str().unwrap_or("")}
}
pub(crate) fn report_input(journal:Option<&Journal>,workflow:&Workflow,id:&str)->Result<Value,String>{
    if let Ok(v)=workflow.report_input(id){return Ok(v);}
    let reference=workflow.restored_input(id).ok_or("REPORT.EVIDENCE_EXPIRED_OR_CHANGED")?;
    if reference["superseded"] == true {return Err("REPORT.EVIDENCE_SOURCE_CHANGED: historical evidence must be reacquired".into());}
    let journal=journal.filter(|j|j.accessible()).ok_or("REPORT.JOURNAL_UNAVAILABLE")?;
    let db=Connection::open_with_flags(&journal.path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
    let (bytes,digest):(Vec<u8>,String)=db.query_row("SELECT receipt,digest FROM report_inputs WHERE task=?1 AND evidence=?2",
        params![reference["task"].as_str(),reference["id"].as_str()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if reference["sha256"]!=digest || wiparse_core::update::hex_sha256(&bytes)!=digest{return Err("REPORT.EVIDENCE_DIGEST_CHANGED".into());}
    let mut result:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    result["historical"]=json!(true);result["evidence_id"]=json!(id);Ok(result)
}
impl Journal {
    pub(crate) fn prepare_control_storage(&self) -> Result<(), String> {
        let project = self.path.parent().ok_or("AGENT.JOURNAL_UNAVAILABLE")?;
        let projects = project.parent().ok_or("AGENT.JOURNAL_UNAVAILABLE")?;
        if is_link_or_reparse(projects) || is_link_or_reparse(project) || is_link_or_reparse(&self.path) {
            return Err("AGENT.JOURNAL_UNAVAILABLE: links and reparse points are not allowed".into());
        }
        if !project.exists() {
            std::fs::create_dir_all(project).map_err(|error| format!("AGENT.JOURNAL_UNAVAILABLE: {error}"))?;
        }
        if !self.accessible() { return Err("AGENT.JOURNAL_UNAVAILABLE: project journal is outside its project directory".into()); }
        Ok(())
    }
    /// The previous task may have closed without ever observing the worker's result.
    /// Query by operation across tasks in this project, including older journal files.
    pub(crate) fn needs_readback(&self, method:&str, p:&Value)->Result<bool,String> {
        // Instrument writes use the device service's durable reservation keyed by physical identity.
        if method=="instrument.command" || (method=="platform.invoke" && p["method"]=="instrument.command")
            || !self.path.exists() {return Ok(false);}
        if !self.accessible() {return Err("AGENT.JOURNAL_UNAVAILABLE".into());}
        let db=Connection::open_with_flags(&self.path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e|e.to_string())?;
        db.query_row("SELECT EXISTS(
            SELECT 1 FROM observations d
            WHERE json_extract(d.reference,'$.state')='dispatching'
            AND json_extract(d.reference,'$.command_fingerprint')=?1
            AND NOT EXISTS(SELECT 1 FROM observations r WHERE r.seq>d.seq AND (
              (json_extract(r.reference,'$.state')='readback_acknowledged'
               AND json_extract(r.reference,'$.command_fingerprint')=?1)
              OR (r.task=d.task AND json_extract(r.reference,'$.state')='completed'
               AND COALESCE(json_extract(r.reference,'$.outcome'),'')!='driver_failed'
               AND json_extract(r.reference,'$.command_id')=json_extract(d.reference,'$.command_id'))
              OR (r.task=d.task AND json_extract(r.reference,'$.state')='completed'
               AND COALESCE(json_extract(r.reference,'$.outcome'),'')!='driver_failed' AND EXISTS(
                SELECT 1 FROM observations a WHERE a.task=d.task AND a.seq BETWEEN d.seq AND r.seq
                AND json_extract(a.reference,'$.state')='accepted'
                AND json_extract(a.reference,'$.command_id')=json_extract(d.reference,'$.command_id')
                AND json_extract(a.reference,'$.job_id')=json_extract(r.reference,'$.job_id')
                AND json_extract(a.reference,'$.job_session')=json_extract(r.reference,'$.job_session'))))))",
            [command_fingerprint(method,p)],|row|row.get(0)).map_err(|e|e.to_string())
    }
    /// Append a late host observation without rewriting the task checkpoint.
    /// This is intentionally not a completion or readback acknowledgement.
    pub(crate) fn append_serial_release(&self, task:&str, command:&str, fingerprint:&str,
        session:&str, port:&str, opened:Option<bool>, dropped:Option<bool>) -> Result<(),String> {
        if !self.accessible() {return Err("AGENT.JOURNAL_UNAVAILABLE".into());}
        let mut db=Connection::open(&self.path).map_err(|e|e.to_string())?;
        db.busy_timeout(std::time::Duration::from_millis(100)).map_err(|e|e.to_string())?;
        let tx=db.transaction().map_err(|e|e.to_string())?;
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM observations WHERE task=?1 AND json_extract(reference,'$.state')='dispatching' AND json_extract(reference,'$.tool')='serial.monitor.stop' AND json_extract(reference,'$.command_id')=?2 AND json_extract(reference,'$.command_fingerprint')=?3)",params![task,command,fingerprint],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exists {return Err("AGENT.STOP_INTENT_NOT_FOUND".into());}
        let reference=json!({"tool":"serial.monitor.stop","state":"host_release_observation","command_id":command,
            "command_fingerprint":fingerprint,"job_session":session,"target_port":port,"opened":opened,
            "host_session_dropped":dropped,"physical_verified":false,"outcome":"not_a_readback_acknowledgement"});
        tx.execute("INSERT INTO observations(task,reference) VALUES(?1,?2)",params![task,reference.to_string()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())
    }
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

    pub(crate) fn at(root:&Path, project:&str)->Self {
        Self{path:wiparse_core::project::project_dir(root,project).join("agent-tasks.sqlite3")}
    }
    fn reader(&self)->Result<Connection,String>{
        if !self.accessible(){return Err("AGENT.JOURNAL_UNAVAILABLE".into());}
        Connection::open_with_flags(&self.path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())
    }
    pub(crate) fn verify_report(&self,dir:&Path,id:&str)->Result<(),String>{
        let db=self.reader()?;
        let receipt:String=db.query_row("SELECT reference FROM observations WHERE json_extract(reference,'$.report_id')=?1 AND json_extract(reference,'$.report_sha256') IS NOT NULL ORDER BY (json_extract(reference,'$.manifest_sha256') IS NOT NULL) DESC, seq ASC LIMIT 1",[id],|r|r.get(0)).map_err(|_|"缺少创建时的报告凭据".to_owned())?;
        super::artifact::verify_bundle(dir,&serde_json::from_str(&receipt).map_err(|e|e.to_string())?)
    }
    pub(crate) fn replay(&self,p:&Value)->Result<Value,String>{
        let limit=p["limit"].as_u64().unwrap_or(32).clamp(1,128) as usize;
        if !self.path.exists() && p.get("task_id").is_none(){return Ok(json!({"tasks":[]}));}
        let db=self.reader()?;
        if let Some(id)=p.get("task_id"){
            let id=id.as_str().filter(|v|!v.is_empty() && v.len()<=128).ok_or("AGENT.INVALID_TASK_ID")?;
            let omit_checkpoint=p["omit_checkpoint"]==true;
            let (entry,phase,progress):(String,String,String)=if omit_checkpoint {
                db.query_row("SELECT entry,phase FROM tasks WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,String::new())))
            } else {
                db.query_row("SELECT entry,phase,progress FROM tasks WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
            }.map_err(|e|e.to_string())?;
            let after=p["after_seq"].as_i64().unwrap_or(0);
            let mut q=db.prepare("SELECT seq,reference FROM observations WHERE task=?1 AND seq>?2 ORDER BY seq LIMIT ?3").map_err(|e|e.to_string())?;
            let rows=q.query_map(params![id,after,limit+1],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
            let mut observations=Vec::new();let mut next=None;
            for row in rows{let (seq,s)=row.map_err(|e|e.to_string())?;if observations.len()==limit{next=observations.last().map(|v:&Value|v["seq"].clone());break;}
                observations.push(json!({"seq":seq,"receipt":serde_json::from_str::<Value>(&s).map_err(|e|e.to_string())?}));}
            let checkpoint=if omit_checkpoint {Value::Null}else{serde_json::from_str::<Value>(&progress).map_err(|e|e.to_string())?};
            return Ok(json!({"task_id":id,"entry":entry,"phase":phase,"checkpoint":checkpoint,"observations":observations,"next_after_seq":next}));
        }
        let mut q=db.prepare("SELECT id,entry,phase,updated FROM tasks ORDER BY updated DESC,id LIMIT ?1").map_err(|e|e.to_string())?;
        let rows=q.query_map([limit],|r|Ok(json!({"task_id":r.get::<_,String>(0)?,"entry":r.get::<_,String>(1)?,"phase":r.get::<_,String>(2)?,"updated":r.get::<_,i64>(3)?}))).map_err(|e|e.to_string())?;
        Ok(json!({"tasks":rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?}))
    }
    fn accessible(&self) -> bool {
        let Some(project) = self.path.parent() else {
            return false;
        };
        let Some(parent) = project.parent() else {
            return false;
        };
        if is_link_or_reparse(parent) || is_link_or_reparse(project) || is_link_or_reparse(&self.path) { return false; }
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
            CREATE TABLE IF NOT EXISTS observations(task TEXT NOT NULL,seq INTEGER PRIMARY KEY,reference TEXT NOT NULL); CREATE INDEX IF NOT EXISTS observations_by_task ON observations(task,seq);
            CREATE INDEX IF NOT EXISTS observations_by_operation ON observations(json_extract(reference,'$.command_fingerprint'),json_extract(reference,'$.state'),seq);")?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS report_inputs(task TEXT,evidence TEXT,digest TEXT NOT NULL,receipt BLOB NOT NULL,PRIMARY KEY(task,evidence))")?;
        let mut snapshot=workflow.checkpoint();
        for evidence in workflow.report_input_ids(){
            use rusqlite::OptionalExtension;
            let existing=tx.query_row("SELECT digest FROM report_inputs WHERE task=?1 AND evidence=?2",params![id,evidence],|r|r.get::<_,String>(0)).optional()?;
            let digest=match existing{
                Some(digest)=>digest,
                None=>{
                    let Ok(v)=workflow.report_input(evidence) else{continue};
                    let bytes=serde_json::to_vec(&v).map_err(|e|rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    let sha=wiparse_core::update::hex_sha256(&bytes);
                    tx.execute("INSERT INTO report_inputs VALUES(?1,?2,?3,?4)",params![id,evidence,sha,bytes])?;sha
                }
            };
            snapshot["report_input_refs"][evidence]=json!({"task":id,"id":evidence,"sha256":digest});
        }
        tx.execute("INSERT INTO tasks VALUES(?1,?2,?3,?4,?5,unixepoch()) ON CONFLICT(id) DO UPDATE SET question=excluded.question,phase=excluded.phase,progress=excluded.progress,updated=excluded.updated",
            params![id,entry,question,phase,snapshot.to_string()])?;
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
        if let Err(error) = self.record_checked(id, entry, question, phase, w, evidence) {
            tracing::warn!(%error,"could not persist agent task");
        }
    }

    pub(crate) fn record_checked(
        &self,
        id: &str,
        entry: &str,
        question: &str,
        phase: &str,
        w: &Workflow,
        evidence: Option<(&str, &Value)>,
    ) -> rusqlite::Result<()> {
        let reference = evidence.map(|(method, v)| {
            let payload = v.get("result").unwrap_or(v);
            let data = payload.get("data").unwrap_or(payload);
            let d = data.get("job").filter(|j|j.is_object()).unwrap_or(data);
            let mut r = json!({"tool":method,"ok":v.get("ok"),"state":d.get("state"),"skills":wiparse_core::ai::skills::references(&w.skill_ids),"host_source_epoch":w.source_epoch()});
            for key in [
                "report_id",
                "report_sha256",
                "manifest_sha256",
                "previous_task_id",
                "evidence_id",
                "source_id",
                "query_id",
                "decoded_text_sha256",
                "start_line",
                "next_line",
                "next_cursor",
                "next_offset",
                "source_bytes",
                "partial",
                "continuation",
                "source_snapshot",
                "first_frame",
                "last_frame",
                "device_id",
                "target_port",
                "run_id",
                "command_id",
                "command_fingerprint",
                "job_id",
                "job_session",
                "outcome",
            ] {
                if let Some(x) = d
                    .get(key)
                    .or_else(|| v.get(key))
                    .filter(|x| x.is_number() || x.as_str().is_some_and(|s| s.len() <= 256))
                {
                    r[key] = x.clone();
                }
            }
            if let Some(executed) = v.get("executed").or_else(|| d.get("executed")).and_then(Value::as_bool) {
                r["executed"] = json!(executed);
            }
            if let Some(snapshot) = data.get("source_snapshot").filter(|snapshot|
                method == "log.analyze" && payload["ok"] == true && data["coverage"]["complete"] == true
                    && data["continuation"].is_null()
                    &&
                snapshot["algorithm"] == "sha256" && snapshot["sources"].as_array().is_some_and(|sources|
                    !sources.is_empty() && sources.len() <= 16 && sources.iter().all(|source|
                        source["id"].as_str().is_some_and(|id| !id.is_empty())
                            && source["sha256"].as_str().is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                            && source["bytes"].as_u64().is_some()))
                    && snapshot.to_string().len() <= 8192) {
                r["source_snapshot"] = snapshot.clone();
                r["source_snapshot_complete"] = json!(true);
            }
            if matches!(method,"report.read"|"wave.frames"|"engineering.wave.frames") {
                let params=&v["params"];
                for key in ["cursor","offset","start_line","start_byte"] {
                    if let Some(value)=params.get(key).filter(|value|
                        value.is_u64() || value.as_str().is_some_and(|text|text.len()<=256)) {
                        r[format!("input_{key}")]=value.clone();
                    }
                }
                for key in ["source_id","source_version","source_bytes","next_cursor","next_offset","total_matches","reached_eof","partial","protocol"] {
                    if let Some(value)=data.get(key).filter(|value|
                        value.is_boolean() || value.is_u64() || value.as_str().is_some_and(|text|text.len()<=256)) {
                        r[key]=value.clone();
                    }
                }
                if let Some(lines)=data["lines"].as_array(){r["returned_lines"]=json!(lines.len());}
                if let Some(frames)=data["frames"].as_array(){r["returned_frames"]=json!(frames.len());}
                if data["provenance"].is_object(){r["provenance_sha256"]=json!(wiparse_core::update::hex_sha256(data["provenance"].to_string().as_bytes()));}
            }
            if method=="agent.recipe" {
                for key in ["recipe","recipe_version","evidence_id","source_method","product_verdict"]{
                    if let Some(value)=data.get(key){r[key]=value.clone();}
                }
                if let Some(findings)=data.get("findings"){
                    let raw=findings.to_string();r["findings_sha256"]=json!(wiparse_core::update::hex_sha256(raw.as_bytes()));
                    if raw.len()<=32768{r["findings"]=findings.clone();}else{r["findings_omitted_bytes"]=json!(raw.len());}
                }
            }
            r
        });
        self.save(id, entry, question, phase, w, reference.as_ref())
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
            use rusqlite::OptionalExtension;
            let snapshot: Option<String> = if state["host_source_epoch_scope"] == "process_local_historical_only" {
                if let Some(epoch)=state["host_source_epoch"].as_u64().and_then(|v|i64::try_from(v).ok()) {
                    db.query_row(
                        "SELECT reference FROM observations WHERE task=?1 AND json_extract(reference,'$.tool')='log.analyze' AND json_extract(reference,'$.source_snapshot_complete')=1 AND json_type(reference,'$.source_snapshot')='object' AND json_extract(reference,'$.host_source_epoch')=?2 ORDER BY seq DESC LIMIT 1",
                        params![&id,epoch], |row| row.get(0)).optional().map_err(|e| e.to_string())?
                } else {None}
            } else {None};
            if let Some(snapshot) = snapshot.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) {
                state["log_snapshot"] = snapshot["source_snapshot"].clone();
            }
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
                AND json_extract(c.reference,'$.state')='completed' AND (
                  (json_extract(o.reference,'$.command_id') IS NOT NULL AND json_extract(c.reference,'$.command_id')=json_extract(o.reference,'$.command_id'))
                  OR (json_extract(o.reference,'$.job_session') IS NOT NULL
                      AND json_extract(c.reference,'$.job_id')=json_extract(o.reference,'$.job_id')
                      AND json_extract(c.reference,'$.job_session')=json_extract(o.reference,'$.job_session'))))")?;
            let mut unknown=unresolved("json_extract(o.reference,'$.state')='dispatching' AND json_extract(o.reference,'$.command_fingerprint') IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM observations c WHERE c.task=o.task AND c.seq>o.seq
                AND json_extract(c.reference,'$.state') IN ('completed','accepted')
                AND json_extract(c.reference,'$.command_id')=json_extract(o.reference,'$.command_id'))")?;
            unknown.extend(unresolved("json_extract(o.reference,'$.state')='completed'
                AND json_extract(o.reference,'$.outcome')='driver_failed'
                AND json_extract(o.reference,'$.command_fingerprint') IS NOT NULL")?);
            state["unacknowledged"]=json!(unknown);
            state["outstanding_jobs"]=json!(outstanding);
            state["saved_reports"]=json!(ids);
            Ok((e, q, p, state))
        })
        .collect()
    }
    fn recovery_key(&self,entry:&str)->egui::Id{
        egui::Id::new(("verify-report-bundle",&self.path,entry))
    }
    pub(crate) fn poll_recovery(&self,ctx:&Context,entry:&str)->Option<Recovery>{
        let key=self.recovery_key(entry);
        let rx=ctx.data(|d|d.get_temp::<crossbeam_channel::Receiver<Recovery>>(key))?;
        match rx.try_recv(){
            Ok(item)=>{ctx.data_mut(|d|d.remove::<crossbeam_channel::Receiver<Recovery>>(key));Some(item)}
            Err(crossbeam_channel::TryRecvError::Empty)=>None,
            Err(_)=>{ctx.data_mut(|d|d.remove::<crossbeam_channel::Receiver<Recovery>>(key));None}
        }
    }
    pub(crate) fn menu(&self,ui:&mut egui::Ui,entry:&str,lang:wiparse_core::i18n::Lang){
        let label=|zh,en|if lang==wiparse_core::i18n::Lang::Zh{zh}else{en};
        let pending=self.recovery_key(entry);
        if ui.data(|d|d.get_temp::<crossbeam_channel::Receiver<Recovery>>(pending).is_some()){
            ui.weak(label("正在核验任务工件…","Verifying task artifacts…"));return;
        }
        ui.menu_button(label("任务记录","Task history"),|ui|{
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
                Ok(rows) if rows.is_empty()=>{ui.weak(label("当前工程暂无记录","No records in this project"));}
                Ok(rows)=>{for (entry,question,phase,state) in rows {
                    let history_task_id=state["previous_task_id"].as_str().map(str::to_owned);
                    ui.horizontal_wrapped(|ui|{
                        ui.weak(format!("{entry} · {phase}"));
                        if let Some(id)=state["previous_task_id"].as_str(){
                            if ui.small_button(label("复制任务 ID","Copy task ID")).on_hover_text(id).clicked(){ui.ctx().copy_text(id.to_owned());}
                        }
                        if ui.button(question.chars().take(40).collect::<String>()).on_hover_text(&question).clicked(){
                            let ctx=ui.ctx().clone();
                            let receipts=ctx.data(|d|d.get_temp::<(String,crate::instrument_control::InstrumentReceipts)>(egui::Id::new("wiparse-ai-instrument-receipts")));
                            let dir=self.path.parent().unwrap().to_owned();
                            let(tx,rx)=crossbeam_channel::bounded(1);
                            ui.data_mut(|d|d.insert_temp(pending,rx));
                            std::thread::spawn(move||{
                            let remaining=state["open_goals"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("、");
                            let refs=state["references"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
                            let mut problems=Vec::new();
                            let project=dir.file_name().unwrap().to_string_lossy().into_owned();
                            let reports=state["saved_reports"].as_array().into_iter().flatten().filter_map(Value::as_str)
                                .filter_map(|raw|serde_json::from_str::<Value>(raw).ok()).filter_map(|reference|{
                                    let id=reference["report_id"].as_str()?;
                                    uuid::Uuid::parse_str(id).ok()?;
                                    let root=dir.canonicalize().ok()?.join("ai-reports").join(id);
                                    if let Err(error)=super::artifact::verify_bundle(&root,&reference){problems.push(format!("{id}: {error}"));return None;}
                                    Some((id.to_owned(),reference["evidence_id"].as_str().unwrap_or("").to_owned()))
                                }).collect::<Vec<_>>();
                            let report_ids=reports.iter().map(|(id,_)|id.as_str()).collect::<Vec<_>>().join("、");
                            let issues=problems.join("\n");
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
                            let text=format!("继续调查。原始目标：{question}\n上次未决交付：{remaining}\n已校验报告及附件：{report_ids}\n需复核的工件：{issues}\n设备任务回执：{}\n历史定位信息（须核验来源）：{refs}\n从未决步骤继续；设备动作先读回，不重发历史命令。",json!(jobs));
                            let mut text=text;
                            if text.len()>15000 {let mut end=15000;while !text.is_char_boundary(end){end-=1;}text.truncate(end);}
                            let _=tx.send(Recovery{project,draft:text,question,state,reports,uncertain});ctx.request_repaint();
                            });
                            ui.close_menu();
                        }
                    });
                    if let Some(task_id)=history_task_id.as_deref() {
                        ui.push_id(task_id,|ui|{
                            ui.collapsing(label("历史证据索引","Historical evidence index"),|ui|{
                                ui.weak(label("仅显示历史元数据；恢复后须重新核验来源，不能代替原始数据。","Historical metadata only; revalidate sources after recovery. This is not raw data."));
                                let key=ui.id().with((&self.path,"receipt-pages"));
                                let mut history=ui.data(|d|d.get_temp::<Vec<i64>>(key)).unwrap_or_else(||vec![0]);
                                let after=*history.last().unwrap_or(&0);
                                let cache=ui.id().with((&self.path,"receipt-page-cache"));
                                let frame=ui.ctx().cumulative_pass_nr();
                                let cached=ui.data(|d|d.get_temp::<(u64,i64,Result<Value,String>)>(cache));
                                let page=match cached {Some((last,cursor,value)) if last+1>=frame && cursor==after=>value,
                                    _=>self.replay(&json!({"task_id":task_id,"after_seq":after,"limit":32,"omit_checkpoint":true}))};
                                ui.data_mut(|d|d.insert_temp(cache,(frame,after,page.clone())));
                                match page {
                                    Err(error)=>{ui.label(error);},
                                    Ok(page)=>{
                                        egui::ScrollArea::vertical().max_height(200.0).show(ui,|ui|{
                                            for observation in page["observations"].as_array().into_iter().flatten() {
                                                let receipt=&observation["receipt"];
                                                ui.small(format!("#{} · {} · {}",observation["seq"],receipt["tool"].as_str().unwrap_or("?"),receipt["evidence_id"].as_str().unwrap_or("—")))
                                                    .on_hover_text(receipt.to_string());
                                            }
                                        });
                                        ui.horizontal(|ui|{
                                            if ui.add_enabled(history.len()>1,egui::Button::new(label("上一页","Previous"))).clicked(){history.pop();}
                                            if let Some(next)=page["next_after_seq"].as_i64(){if ui.button(label("下一页","Next")).clicked(){history.push(next);}}
                                            ui.weak(format!("{} {}",label("页","Page"),history.len()));
                                        });
                                    }
                                }
                                ui.data_mut(|d|d.insert_temp(key,history));
                            });
                        });
                    }
                }}
            }
        });
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
    fn old_test_execution_skill_version_requires_new_task() {
        let mut recovery=Recovery::incompatible_test_fixture("p","draft");
        recovery.state=json!({"skill_ids":["test_execution"],"skills":[{"id":"test_execution","version":2}]});
        assert!(recovery.validate_skills().unwrap_err().contains("SKILL.VERSION_CHANGED"));
    }

    #[test]
    fn job_receipt_without_session_closes_only_its_own_command() {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::at(root.path(), "p1");
        journal.prepare_control_storage().unwrap();
        let flow = Workflow::default();
        let record = |event: Value| journal.record_checked("task", "serial", "capture", "running",
            &flow, Some(("instrument.capture", &event))).unwrap();
        record(json!({"state":"accepted","command_id":"first","job_id":1,"job_session":null}));
        record(json!({"state":"accepted","command_id":"second","job_id":1,"job_session":null}));
        record(json!({"state":"completed","command_id":"first","job_id":1,"job_session":null}));
        let task = journal.recent().unwrap().into_iter().find(|r| r.3["previous_task_id"] == "task").unwrap();
        let remaining = task.3["outstanding_jobs"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].as_str().unwrap().contains("second"));
    }

    #[test]
    fn failed_driver_receipt_closes_job_but_requires_readback_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::at(root.path(), "p1");
        journal.prepare_control_storage().unwrap();
        let flow = Workflow::default();
        let method = "instrument.capture";
        let params = json!({"device_id": 4});
        let key = command_fingerprint(method, &params);
        let record = |event: Value| journal.record_checked("task", "serial", "capture", "running",
            &flow, Some((method, &event))).unwrap();
        record(json!({"state":"dispatching","command_id":"c","command_fingerprint":key}));
        record(json!({"state":"accepted","command_id":"c","job_id":1,"job_session":"s"}));
        record(json!({"state":"completed","outcome":"driver_failed","command_id":"c","command_fingerprint":key,"job_id":1,"job_session":"s"}));
        assert!(journal.needs_readback(method, &params).unwrap());
        let state = journal.recent().unwrap().into_iter().find(|r| r.3["previous_task_id"] == "task").unwrap().3;
        assert_eq!(state["outstanding_jobs"].as_array().unwrap().len(), 0);
        assert_eq!(state["unacknowledged"].as_array().unwrap().len(), 1);
        record(json!({"state":"readback_acknowledged","command_fingerprint":key}));
        assert!(!journal.needs_readback(method, &params).unwrap());
    }

    #[test]
    fn unknown_serial_action_survives_restart_without_blocking_other_ports() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path(), "p1");
        journal.prepare_control_storage().unwrap();
        let method = "serial.monitor.stop";
        let first = json!({"target_port":"COM1"});
        let second = json!({"target_port":"COM2"});
        let fingerprint = command_fingerprint(method, &first);
        journal.record_checked("task-1", "serial-watch", "stop", "awaiting_readback", &Workflow::default(),
            Some((method, &json!({"state":"dispatching","command_id":"cmd-1","command_fingerprint":fingerprint,"target_port":"COM1"})))).unwrap();
        let reopened = Journal::at(dir.path(), "p1");
        assert!(reopened.needs_readback(method, &first).unwrap());
        assert!(!reopened.needs_readback(method, &second).unwrap());
        reopened.record_checked("task-2", "serial", "checked", "readback", &Workflow::default(),
            Some(("device.readback", &json!({"state":"readback_acknowledged","command_fingerprint":fingerprint})))).unwrap();
        assert!(!reopened.needs_readback(method, &first).unwrap());
    }

    #[test]
    fn accepted_serial_stop_with_unconfirmed_release_still_requires_readback() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::at(dir.path(), "p1");
        journal.prepare_control_storage().unwrap();
        let method = "serial.monitor.stop";
        let params = json!({"target_port":"COM1"});
        let fingerprint = command_fingerprint(method, &params);
        let workflow = Workflow::default();
        journal.record_checked("watch", "serial-watch", "stop", "awaiting_readback", &workflow,
            Some((method, &json!({"state":"dispatching","command_id":"c1","command_fingerprint":fingerprint})))).unwrap();
        journal.record_checked("watch", "serial-watch", "stop", "awaiting_readback", &workflow,
            Some((method, &json!({"state":"unknown","command_id":"c1","command_fingerprint":fingerprint,
                "outcome":"host_stop_requested","port_release":"unconfirmed_async"})))).unwrap();
        assert!(journal.append_serial_release("watch","wrong",&fingerprint,"session","COM1",Some(true),Some(true)).is_err());
        journal.append_serial_release("watch","c1",&fingerprint,"session","COM1",Some(true),Some(true)).unwrap();
        assert!(journal.needs_readback(method, &params).unwrap());
    }

    #[test]
    fn new_task_can_check_prior_writes_without_restoring_the_original_chat() {
        let dir=tempfile::tempdir().unwrap();
        let journal=Journal{path:dir.path().join("agent.sqlite3")};
        let flow=Workflow::default();
        let p=json!({"port":"COM8"});
        let key=command_fingerprint("serial.start",&p);
        let record=|task,event:Value| journal.record(task,"serial","run","running",&flow,Some(("serial.start",&event)));
        record("old",json!({"state":"dispatching","command_id":"A","command_fingerprint":key}));
        assert!(journal.needs_readback("serial.start",&p).unwrap());
        assert!(!journal.needs_readback("serial.start",&json!({"port":"COM9"})).unwrap());
        record("new",json!({"state":"readback_acknowledged","command_fingerprint":key}));
        assert!(!journal.needs_readback("serial.start",&p).unwrap());
        record("new",json!({"state":"dispatching","command_id":"B","command_fingerprint":key}));
        record("new",json!({"state":"completed","command_id":"A"}));
        assert!(journal.needs_readback("serial.start",&p).unwrap());
        record("new",json!({"state":"completed","command_id":"B"}));
        assert!(!journal.needs_readback("serial.start",&p).unwrap());
    }

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
    #[test] fn replay_and_archived_receipt_survive_reopen(){
        let root=tempfile::tempdir().unwrap();let j=Journal::at(root.path(),"demo");
        std::fs::create_dir_all(j.path.parent().unwrap()).unwrap();
        let mut w=Workflow::default();
        w.record("E1","calc.evaluate",&json!({"ok":true,"data":{"result":17}}));
        w.update(json!({"steps":[{"id":"s","state":"done","title":"calculate","evidence_ids":["E1"]}]})).unwrap();
        j.record("task-a","serial","calculate","done",&w,Some(("calc.evaluate",&json!({"ok":true,"evidence_id":"E1"}))));
        for n in 0..110{w.record(&format!("x{n}"),"page.inspect",&json!({"ok":true}));}
        assert!(w.get("E1").is_err());assert_eq!(w.report_input("E1").unwrap()["result"]["data"]["result"],17);
        let r=j.replay(&json!({"task_id":"task-a"})).unwrap();
        let mut restored=Workflow::default();restored.resume_checkpoint(&r["checkpoint"],"calculate",&[]);
        assert!(report_input(Some(&j),&restored,"prior-E1").unwrap_err().contains("SOURCE_CHANGED"));
        assert_eq!(r["observations"][0]["receipt"]["evidence_id"],"E1");
        let reference=restored.restored_input("prior-E1").unwrap();
        assert_eq!(reference["superseded"],true);
        let db=Connection::open(&j.path).unwrap();db.execute("UPDATE report_inputs SET receipt=x'00'",[]).unwrap();
        assert!(report_input(Some(&j),&restored,"prior-E1").is_err());
    }

    #[test]
    fn recovery_does_not_reuse_log_snapshot_from_an_older_or_unknown_epoch() {
        let root=tempfile::tempdir().unwrap();
        let journal=Journal::at(root.path(),"demo");
        std::fs::create_dir_all(journal.path.parent().unwrap()).unwrap();
        let mut workflow=Workflow::default();
        let snapshot=json!({"algorithm":"sha256","sources":[{"id":"log-a","sha256":"a".repeat(64),"bytes":10}]});
        journal.record_checked("task","serial","analysis","running",&workflow,Some(("log.analyze",&json!({"ok":true,"data":{"coverage":{"complete":true},"source_snapshot":snapshot}})))).unwrap();
        assert!(journal.recent().unwrap()[0].3["log_snapshot"].is_object());
        workflow.source_transition();
        journal.record_checked("task","serial","analysis","running",&workflow,Some(("source.transition",&json!({})))).unwrap();
        assert!(journal.recent().unwrap()[0].3.get("log_snapshot").is_none());
        let db=Connection::open(&journal.path).unwrap();
        db.execute("UPDATE tasks SET progress=json_remove(progress,'$.host_source_epoch','$.host_source_epoch_scope')",[]).unwrap();
        assert!(journal.recent().unwrap()[0].3.get("log_snapshot").is_none());
    }

}
