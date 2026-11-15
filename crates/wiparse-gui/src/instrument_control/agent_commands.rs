use super::*;
use rusqlite::{params, Connection};
use std::path::Path;

/// Persist before queueing. A lost reply or process exit leaves the reservation intact.
struct Reservation {
    db: Connection,
    operation: String,
    session: String,
    job: u64,
}

impl Reservation {
    fn open(root: &Path, operation: String, session: &str, job: u64) -> rusqlite::Result<Self> {
        let db = Connection::open(root.join("agent-device-operations.sqlite3"))?;
        db.busy_timeout(Duration::from_millis(100))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending(
            operation TEXT PRIMARY KEY, session TEXT NOT NULL, job INTEGER NOT NULL)",
        )?;
        Ok(Self {
            db,
            operation,
            session: session.into(),
            job,
        })
    }

    fn reserve(&mut self) -> rusqlite::Result<Option<(String, u64)>> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO pending VALUES(?1,?2,?3)",
            params![self.operation, self.session, self.job],
        )?;
        let prior = if inserted == 0 {
            Some(tx.query_row(
                "SELECT session,job FROM pending WHERE operation=?1",
                [&self.operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        } else {
            None
        };
        tx.commit()?;
        Ok(prior)
    }

    fn resolve(&self, evidence:&serde_json::Value)->rusqlite::Result<usize>{
        let tx=self.db.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS readbacks(operation TEXT,session TEXT,job INTEGER,observed_at INTEGER,evidence TEXT);CREATE INDEX IF NOT EXISTS readback_keys ON readbacks(operation)")?;
        let changed=tx.execute("DELETE FROM pending WHERE operation=?1 AND session=?2 AND job=?3",params![self.operation,self.session,self.job])?;
        if changed==1{
            tx.execute("INSERT INTO readbacks VALUES(?1,?2,?3,unixepoch(),?4)",params![self.operation,self.session,self.job,evidence.to_string()])?;
        }
        tx.commit()?;
        Ok(changed)
    }
    fn release(&self) -> rusqlite::Result<usize> {
        self.db.execute(
            "DELETE FROM pending WHERE operation=?1 AND session=?2 AND job=?3",
            params![self.operation, self.session, self.job],
        )
    }
}

#[derive(Default)]
pub(super) struct AgentCommands {
    active: HashMap<u64, (u64, Reservation)>,
    review: Option<(String, Reservation)>,
    observation: Readback,
}
#[derive(Default)]
struct Readback {
    kind:usize,
    source:String,
    value:String,
    unit:String,
    tolerance:String,
}
impl Readback {
    fn evidence(&self,target:&str)->Option<serde_json::Value>{
        let source=self.source.trim();
        let value=self.value.trim();
        if source.is_empty() || value.is_empty(){return None;}
        let mut record=serde_json::json!({"source":source,"target":target,"observation":value,"physical_verified":false,
            "kind":(["operator_observation","instrument_state_readback","independent_measurement"][self.kind])});
        if self.kind==2{
            let number=value.parse::<f64>().ok().filter(|v|v.is_finite())?;
            if self.unit.trim().is_empty(){return None;}
            record["value"]=serde_json::json!(number);record["unit"]=serde_json::json!(self.unit.trim());
            if !self.tolerance.trim().is_empty(){
                record["tolerance"]=serde_json::json!(self.tolerance.trim().parse::<f64>().ok().filter(|v|v.is_finite() && *v>=0.)?);
            }
        }
        Some(record)
    }
}

impl AgentCommands {
    pub(super) fn disconnected(&mut self, device: u64) {
        self.active.retain(|_, (id, _)| *id != device);
    }

    pub(super) fn completed(&mut self, job: u64, success: bool) {
        if let Some((_, reservation)) = self.active.remove(&job) {
            if success {
                if let Err(error) = reservation.resolve(&serde_json::json!({"kind":"driver_completed","physical_verified":false})) {
                    tracing::warn!(%error, "device operation still requires readback");
                }
            }
        }
    }

    pub(super) fn show_review(&mut self, ctx: &egui::Context) {
        let Some((label, reservation)) = &self.review else {
            return;
        };
        let mut open = true;
        let mut acknowledged = false;
        egui::Window::new("仪器操作结果待核对 / Check instrument state")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(label.as_str());
                ui.label("请读回仪器状态并核对实际输出。确认后可重新提交操作。");
                let r=&mut self.observation;
                egui::ComboBox::from_id_salt("readback-kind").selected_text(["人工观察","仪器状态读回","独立仪器测量"][r.kind]).show_ui(ui,|ui|{
                    for (i,name) in ["人工观察","仪器状态读回","独立仪器测量"].iter().enumerate(){ui.selectable_value(&mut r.kind,i,*name);}
                });
                ui.add(egui::TextEdit::singleline(&mut r.source).hint_text("观测来源 / 仪器编号").char_limit(160));
                ui.add(egui::TextEdit::singleline(&mut r.value).hint_text("观测值或状态").char_limit(160));
                if r.kind==2{
                    ui.add(egui::TextEdit::singleline(&mut r.unit).hint_text("单位").char_limit(32));
                    ui.add(egui::TextEdit::singleline(&mut r.tolerance).hint_text("测量容差（可选）").char_limit(32));
                }
                acknowledged=ui.add_enabled(r.evidence(label).is_some(),egui::Button::new("保存读回记录")).clicked();
            });
        if acknowledged {
            match reservation.resolve(&self.observation.evidence(label).unwrap()) {
                Ok(_) => open = false,
                Err(error) => tracing::warn!(%error, "could not acknowledge device readback"),
            }
        }
        if !open {
            self.review = None;
        }
    }
}

impl InstrumentControlPanel {
    pub(crate) fn independent_scope(&self,p:&serde_json::Value)->Result<serde_json::Value,String>{
        let v=&p["verify"];
        let find=|key:&serde_json::Value|self.devices.iter().find(|d|Some(d.id)==key.as_u64()).ok_or("DEVICE.NOT_CONNECTED");
        let source=find(&p["device_id"])?;let scope=find(&v["device_id"])?;
        if [source,scope].iter().any(|d|d.resource.to_ascii_uppercase().starts_with("DEMO::")||d.identity.serial.trim().is_empty()) ||
            (scope.identity.serial==source.identity.serial && scope.identity.manufacturer==source.identity.manufacturer) {
            return Err("DEVICE.PHYSICAL_IDENTITY_UNVERIFIED".into());
        }
        let read=serde_json::json!({"device_id":scope.id,"command":{"ScopeMeasure":{"channel":v["channel"],"meas_type":v["meas_type"]}}});
        self.agent_validate_device(&read,false)?;
        Ok(serde_json::json!({"source":source.identity,"observer":scope.identity,"read":read,"verify":v,
            "operation":self.operation_key(source.id,&parse_control_command(&p["command"])?)?}))
    }
    pub(crate) fn save_measurement(root:&Path,plan:&serde_json::Value,origin:&serde_json::Value,reading:&serde_json::Value)->Result<(),String>{
        let r=Reservation::open(root,plan["operation"].as_str().ok_or("DEVICE.IDENTITY_REQUIRED")?.to_owned(),
            origin["job_session"].as_str().ok_or("DEVICE.NO_SESSION")?,origin["job_id"].as_u64().ok_or("DEVICE.NO_RECEIPT")?).map_err(|e|e.to_string())?;
        r.db.execute_batch("CREATE TABLE IF NOT EXISTS readbacks(operation TEXT,session TEXT,job INTEGER,observed_at INTEGER,evidence TEXT);CREATE INDEX IF NOT EXISTS readback_keys ON readbacks(operation)").map_err(|e|e.to_string())?;
        r.db.execute("INSERT INTO readbacks VALUES(?1,?2,?3,unixepoch(),?4)",params![r.operation,r.session,r.job,reading.to_string()]).map_err(|e|e.to_string())?;
        Ok(())
    }
    fn operation_key(&self,id:u64,command:&ControlCommand)->Result<String,&'static str>{
        let device=self.devices.iter().find(|d|d.id==id).ok_or("DEVICE.NOT_CONNECTED")?;
        let serial = device.identity.serial.trim();
        let identity = if serial.is_empty() {
            device.resource.trim()
        } else {
            serial
        };
        let canonical = serde_json::json!([
            device.identity.manufacturer,
            device.identity.model,
            identity,
            command
        ]);
        Ok(wiparse_core::update::hex_sha256(canonical.to_string().as_bytes()))
    }
    pub(crate) fn agent_readback(&self,root:&Path,p:&serde_json::Value)->Result<serde_json::Value,String>{
        let command=parse_control_command(&p["command"])?;
        let key=self.operation_key(p["device_id"].as_u64().ok_or("DEVICE.ID_REQUIRED")?,&command)?;
        let path=root.join("agent-device-operations.sqlite3");
        let mut records=Vec::new();
        if path.exists(){
            let db=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='readbacks' AND type='table')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if exists {
                let mut query=db.prepare("SELECT session,job,observed_at,evidence FROM readbacks WHERE operation=?1 ORDER BY rowid DESC LIMIT 16").map_err(|e|e.to_string())?;
                let mut rows=query.query([key]).map_err(|e|e.to_string())?;
                while let Some(row)=rows.next().map_err(|e|e.to_string())?{
                    let raw:String=row.get(3).map_err(|e|e.to_string())?;
                    let obs:serde_json::Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
                    records.push(serde_json::json!({"session":row.get::<_,String>(0).map_err(|e|e.to_string())?,
                        "job_id":row.get::<_,u64>(1).map_err(|e|e.to_string())?,"utc_seconds":row.get::<_,i64>(2).map_err(|e|e.to_string())?,
                        "observation":obs}));
                }
            }
        }
        Ok(serde_json::json!({"device_id":p["device_id"],"records":records,"historical":true,"physical_verified":false}))
    }
    pub(crate) fn agent_command(
        &mut self,
        root: &Path,
        p: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err, invoke_ok};
        const METHOD: &str = "instrument.command";
        let Some(device) = self
            .devices
            .iter()
            .find(|d| Some(d.id) == p["device_id"].as_u64())
        else {
            return invoke_err(METHOD, "DEVICE.NOT_CONNECTED");
        };
        let command = match parse_control_command(&p["command"]) {
            Ok(command) => command,
            Err(e) => return invoke_err(METHOD, &e),
        };
        if matches!(
            command,
            ControlCommand::ScopeMeasure { .. }
                | ControlCommand::ProbeStatus
                | ControlCommand::ProbeRegs
                | ControlCommand::BridgeInfo
                | ControlCommand::DongleInfo
                | ControlCommand::ProbeMemRead { .. }
        ) {
            return self.api_command(p);
        }
        let key=match self.operation_key(device.id,&command){Ok(key)=>key,Err(e)=>return invoke_err(METHOD,e)};
        let label = format!(
            "{} · {} · {}",
            device.identity.model, device.resource, p["command"]
        );
        let device_id = device.id;
        let job = self.alloc_job_id();
        let mut reservation = match Reservation::open(root, key, &self.job_session, job) {
            Ok(r) => r,
            Err(e) => return invoke_err(METHOD, &format!("DEVICE.JOURNAL_UNAVAILABLE: {e}")),
        };
        match reservation.reserve() {
            Ok(Some((session, old_job))) => {
                if session == self.job_session && self.agent_commands.active.contains_key(&old_job)
                {
                    return invoke_err(METHOD, "DEVICE.OPERATION_IN_FLIGHT");
                }
                reservation.session = session;
                reservation.job = old_job;
                if self.agent_commands.review.as_ref().is_none_or(|(_,r)|r.operation!=reservation.operation){
                    self.agent_commands.observation=Readback::default();
                }
                self.agent_commands.review = Some((label, reservation));
                return invoke_err(
                    METHOD,
                    "DEVICE.READBACK_REQUIRED: inspect the device before repeating this operation",
                );
            }
            Err(e) => return invoke_err(METHOD, &format!("DEVICE.JOURNAL_UNAVAILABLE: {e}")),
            Ok(None) => {}
        }
        if let Err(e) = self.tx.send(Job::Command {
            id: device_id,
            command,
            job_id: Some(job),
        }) {
            let _ = reservation.release();
            return invoke_err(METHOD, &e.to_string());
        }
        self.agent_commands
            .active
            .insert(job, (device_id, reservation));
        invoke_ok(
            METHOD,
            serde_json::json!({"accepted":true,"device_id":device_id,"job_session":self.job_session,"job_id":job}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_failed_and_lost_demo_receipts_keep_distinct_audit_states() {
        let root = tempfile::tempdir().unwrap();
        let mut commands = AgentCommands::default();
        for (operation, job) in [("demo-complete", 1), ("demo-failed", 2), ("demo-lost", 3)] {
            let mut reservation = Reservation::open(root.path(), operation.into(), "session-a", job).unwrap();
            assert_eq!(reservation.reserve().unwrap(), None);
            commands.active.insert(job, (42, reservation));
        }
        commands.completed(1, true);
        commands.completed(2, false);
        // A duplicate late success for the already failed job cannot release it.
        commands.completed(2, true);
        // Disconnect drops the live owner without releasing its durable reservation.
        commands.disconnected(42);
        drop(commands);

        let mut success = Reservation::open(root.path(), "demo-complete".into(), "session-b", 4).unwrap();
        assert_eq!(success.reserve().unwrap(), None);
        let mut failure = Reservation::open(root.path(), "demo-failed".into(), "session-b", 5).unwrap();
        assert_eq!(failure.reserve().unwrap(), Some(("session-a".into(), 2)));
        let mut unknown = Reservation::open(root.path(), "demo-lost".into(), "session-b", 6).unwrap();
        assert_eq!(unknown.reserve().unwrap(), Some(("session-a".into(), 3)));
        // A receipt from another session or job cannot release the lost action.
        assert_eq!(unknown.resolve(&serde_json::json!({"kind":"driver_completed","physical_verified":false})).unwrap(), 0);
        assert_eq!(unknown.reserve().unwrap(), Some(("session-a".into(), 3)));
        let db = Connection::open_with_flags(root.path().join("agent-device-operations.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let evidence: String = db.query_row("SELECT evidence FROM readbacks WHERE operation='demo-complete'", [], |row| row.get(0)).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["kind"], "driver_completed");
        assert_eq!(evidence["physical_verified"], false);
        let unresolved: i64 = db.query_row("SELECT count(*) FROM pending WHERE operation IN ('demo-failed','demo-lost')", [], |row| row.get(0)).unwrap();
        assert_eq!(unresolved, 2);
    }

    #[test]
    fn readback_confirmation_requires_valid_observation_and_measurement() {
        let mut read = Readback::default();
        assert!(read.evidence("demo target").is_none());
        read.source = "  DMM-1  ".into();
        assert!(read.evidence("demo target").is_none());
        read.value = "  ".into();
        assert!(read.evidence("demo target").is_none());
        read.value = "observed off".into();
        let operator = read.evidence("demo target").unwrap();
        assert_eq!(operator["source"], "DMM-1");
        assert_eq!(operator["physical_verified"], false);

        read.kind = 2;
        for invalid in ["observed off", "NaN", "inf"] {
            read.value = invalid.into();
            read.unit = "V".into();
            assert!(read.evidence("demo target").is_none(), "{invalid}");
        }
        read.value = "4.99".into();
        read.unit.clear();
        assert!(read.evidence("demo target").is_none());
        read.unit = "V".into();
        for invalid in ["-0.1", "NaN", "not-a-number"] {
            read.tolerance = invalid.into();
            assert!(read.evidence("demo target").is_none(), "{invalid}");
        }
        read.tolerance = "0.02".into();
        let measurement = read.evidence("demo target").unwrap();
        assert_eq!(measurement["value"], 4.99);
        assert_eq!(measurement["tolerance"], 0.02);
        assert_eq!(measurement["physical_verified"], false);
    }
    #[test]
    fn concurrent_same_operation_has_one_owner_and_old_receipts_cannot_release_it() {
        use std::sync::{Arc, Barrier};
        let root=tempfile::tempdir().unwrap();
        let operation="fixture-scope/single";
        // Initialize schema before starting the contenders; only reservation
        // ownership is under test, not concurrent database creation.
        drop(Reservation::open(root.path(),operation.into(),"initializer",0).unwrap());
        let barrier=Arc::new(Barrier::new(5));let mut workers=Vec::new();
        for n in 0..4_u64 {
            let barrier=barrier.clone();let path=root.path().to_owned();
            workers.push(std::thread::spawn(move||{
                let session=format!("session-{n}");let mut reservation=Reservation::open(&path,operation.into(),&session,n).unwrap();
                barrier.wait();(session,reservation.reserve())
            }));
        }
        barrier.wait();
        let results:Vec<_>=workers.into_iter().map(|worker|worker.join().unwrap()).collect();
        let winners:Vec<_>=results.iter().filter(|(_,result)|matches!(result,Ok(None))).collect();
        assert_eq!(winners.len(),1,"reservation outcomes: {results:?}");
        let winner=&winners[0].0;
        for (_,result) in &results {if let Ok(Some((session,_)))=result {assert_eq!(session,winner);}}
        assert!(results.iter().all(|(_,result)|result.is_ok()),"reservation error: {results:?}");
        let stale=Reservation::open(root.path(),operation.into(),"stale",999).unwrap();
        assert_eq!(stale.release().unwrap(),0);
        let mut later=Reservation::open(root.path(),operation.into(),"later",1000).unwrap();
        assert_eq!(later.reserve().unwrap().unwrap().0,*winner);
        let winner_job=winner.strip_prefix("session-").unwrap().parse::<u64>().unwrap();
        let current=Reservation::open(root.path(),operation.into(),winner,winner_job).unwrap();
        let readback=serde_json::json!({"kind":"simulated_readback","physical_verified":false});
        assert_eq!(current.resolve(&readback).unwrap(),1);
        assert_eq!(current.resolve(&readback).unwrap(),0,"duplicate readback must not release twice");
        assert_eq!(later.reserve().unwrap(),None,"verified release must permit a new owner");
        assert_eq!(current.release().unwrap(),0,"old owner cannot release the new reservation");
    }
    #[test]
    fn reconnect_with_a_new_device_id_does_not_resubmit_a_scope_single() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        panel.begin_connect_demo(InstrumentKind::Oscilloscope, Lang::Zh);
        let deadline = Instant::now() + Duration::from_secs(5);
        while panel.devices.is_empty() {
            panel.pump(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let id = panel.devices[0].id;
        let command = |id| serde_json::json!({"device_id":id,"command":"ScopeSingle"});
        let accepted = panel.agent_command(dir.path(), &command(id));
        assert!(accepted.ok);
        let job = accepted.data["job_id"].as_u64().unwrap();
        assert_eq!(
            panel
                .agent_command(dir.path(), &command(id))
                .error
                .as_deref(),
            Some("DEVICE.OPERATION_IN_FLIGHT")
        );
        panel.agent_commands.disconnected(id);
        panel.devices[0].id = id + 1;
        panel.complete_job(Some(job), true, "command", Some(id), serde_json::json!({}));
        let repeated = panel.agent_command(dir.path(), &command(id + 1));
        assert!(repeated
            .error
            .unwrap()
            .starts_with("DEVICE.READBACK_REQUIRED"));
        assert!(panel.agent_commands.active.is_empty());
        let (_, old) = panel.agent_commands.review.take().unwrap();
        let read=Readback{kind:2,source:"DMM-12".into(),value:"4.99".into(),unit:"V".into(),tolerance:"0.02".into()};
        assert_eq!(old.resolve(&read.evidence("demo scope").unwrap()).unwrap(),1);
        let saved=panel.agent_readback(dir.path(),&command(id+1)).unwrap();
        assert_eq!(saved["records"][0]["observation"]["value"],4.99);
        assert_eq!(saved["records"][0]["observation"]["kind"],"independent_measurement");
        assert_eq!(saved["physical_verified"],false);
        panel.devices[0].id = id;
        let next = panel.agent_command(dir.path(), &command(id));
        assert!(next.ok);
        assert_ne!(next.data["job_id"], job);
    }

    #[test]
    fn restart_keeps_unknown_writes_and_old_receipts_cannot_clear_new_dispatches() {
        let root = tempfile::tempdir().unwrap();
        let open = |session: &str, job| {
            Reservation::open(root.path(), "scope-serial/single".into(), session, job).unwrap()
        };
        assert_eq!(open("first", 1).reserve().unwrap(), None);
        assert_eq!(
            open("second", 1).reserve().unwrap(),
            Some(("first".into(), 1))
        );
        assert_eq!(open("first", 1).release().unwrap(), 1);
        assert_eq!(open("second", 1).reserve().unwrap(), None);
        assert_eq!(open("first", 1).release().unwrap(), 0);
        assert_eq!(
            open("third", 1).reserve().unwrap(),
            Some(("second".into(), 1))
        );
    }
}
