//! Bounded platform calls using the existing GUI API and durable instrument jobs.
use super::*;
use crate::backend::{AiGuard, ApiBridge, InvokeReply, PendingRequest};
use serde_json::Value;

pub(super) fn control_failure_message(result: &Value, lang: Lang) -> &'static str {
    if result["executed"] == false && result["error"].as_str() == Some("PLATFORM.USER_DECLINED") {
        label(lang, "已取消设备操作，未派发；设备控制已关闭。", "Device action cancelled before dispatch; controls disabled.")
    } else if result["executed"] == false {
        label(lang, "设备操作未派发，已关闭设备控制；请查看执行结果。", "Device action was not dispatched; controls disabled. Inspect the result.")
    } else {
        label(lang, "设备调用失败或状态未知，已关闭设备控制；请查看执行结果。", "Device call failed or state unknown; controls disabled. Inspect the result.")
    }
}

pub(super) struct ToolPending {
    reply: Option<Receiver<InvokeReply>>,
    guard: AiGuard,
    method: String,
    params: Value,
    question: String,
    pub(super) automatic: bool,
    pub(super) local_only: bool,
    local_scope: String,
    watch_id: Option<String>,
    watch_revision: Option<String>,
    deadline: Instant,
    command_id:String,
    fingerprint:String,
    job: Option<u64>,
    job_session: Option<String>,
    next_poll: Instant,
    audit: Option<ControlAudit>,
}

#[derive(Clone)]
struct ControlAudit {
    journal: crate::ai::journal::Journal,
    task_id: String,
    entry: &'static str,
    question: String,
    workflow: wiparse_core::ai::workflow::Workflow,
}
pub(super) struct ReviewedStopPending {
    gate: dispatch_gate::Gate,
    command: dispatch_gate::Command,
    reply: Receiver<Result<(), String>>,
    slot: Arc<std::sync::Mutex<Option<SerialStopIntent>>>,
    audit: ControlAudit,
    deadline: Instant,
}
pub(super) fn cancel_reviewed_stop(pending: ReviewedStopPending) -> (dispatch_gate::Outcome, bool) {
    let outcome=pending.gate.cancel(pending.command.id).unwrap_or(dispatch_gate::Outcome::Unknown);
    if outcome==dispatch_gate::Outcome::CancelledBeforeClaim {
        if let Ok(mut value)=pending.slot.lock() {
            if value.as_ref().is_some_and(|intent|intent.command_id==pending.command.id.to_string()) {*value=None;}
        }
        let saved=pending.audit.not_dispatched("serial.monitor.stop",&pending.command.id.to_string()).is_ok();
        (outcome,saved)
    } else {
        let saved=pending.audit.journal.record_checked(&pending.audit.task_id,pending.audit.entry,&pending.audit.question,
            "awaiting_readback",&pending.audit.workflow,Some(("serial.monitor.stop",&json!({"state":"unknown",
                "command_id":pending.command.id.to_string(),"execution_state":"unknown; inspect before retry"})))).is_ok();
        (outcome,saved)
    }
}
pub(super) fn poll_reviewed_stop(chat:&mut SerialAiChat,ctx:&egui::Context) {
    let Some(pending)=chat.reviewed_stop_pending.take() else{return;};
    let result=match pending.reply.try_recv() {
        Ok(result)=>Some(result),
        Err(crossbeam_channel::TryRecvError::Disconnected)=>Some(Err("SERIAL.GATE_WORKER_DISCONNECTED".into())),
        Err(crossbeam_channel::TryRecvError::Empty) if Instant::now()>=pending.deadline=>Some(Err("SERIAL.GATE_CLAIM_TIMEOUT".into())),
        Err(crossbeam_channel::TryRecvError::Empty)=>None,
    };
    let Some(result)=result else{chat.reviewed_stop_pending=Some(pending);ctx.request_repaint_after(Duration::from_millis(100));return;};
    match result {
        Ok(())=>{
            let saved=pending.audit.journal.record_checked(&pending.audit.task_id,pending.audit.entry,&pending.audit.question,
                "awaiting_readback",&pending.audit.workflow,Some(("serial.monitor.stop",&json!({"state":"unknown",
                    "command_id":pending.command.id.to_string(),"outcome":"worker_claimed_stop",
                    "job_session":pending.command.target.session,"target_port":pending.command.target.port,
                    "physical_verified":false})))).is_ok();
            chat.finish_watch_transaction(false);
            chat.note_background(if saved {"串口工作线程已认领停止请求；会话释放与设备状态仍待观察 / Stop claimed; session release and device state remain unverified"}
                else {"停止请求已认领，但任务记录写入失败；请核查设备状态 / Stop claimed; journal write failed, inspect device"}, !saved);
        }
        Err(error)=>{
            let (outcome,saved)=cancel_reviewed_stop(pending);
            chat.finish_watch_transaction(false);chat.pause_watch();
            let status=if outcome==dispatch_gate::Outcome::CancelledBeforeClaim {"未认领，停止请求未执行 / Not claimed; stop not executed"} else {"结果未知，请核查设备后再重试 / Outcome unknown; inspect device before retry"};
            chat.note_background(format!("{error} · {status} · journal={saved}"),true);
        }
    }
}
impl ControlAudit {
    fn not_dispatched(&self, method: &str, command_id: &str) -> Result<(), String> {
        self.journal.record_checked(&self.task_id, self.entry, &self.question, if self.entry == "serial-watch" {"cancelled"} else {"running"}, &self.workflow,
            Some((method, &json!({"state":"completed","command_id":command_id,"outcome":"not_dispatched","executed":false}))))
            .map_err(|error| error.to_string())
    }
}

pub(super) fn catalog() -> &'static str {
    r#"Platform tools: JSON compatibility mode requests ONE action {"type":"platform.call","method":"...","params":{...}} per answer; wait for the real host result before the next step. Supported:
serial.view {}: inspect current serial log view ID, absolute 1-based row range and search status. serial.search {view_id:actualID,query:string}: apply the existing log search UI and show matching rows; result search_pending means background search has not finished, inspect serial.view later. serial.jump {view_id:actualID,line:positiveInteger}: highlight and scroll current log to original 1-based file/absolute live row, pausing tail follow without stopping capture. These reversible UI actions need no device-control grant, only a manual current-log scope. Never use selection-relative rows or another file's lines. First inspect view identity; use log.analyze for full evidence searches and then jump to a verified row. No serial transmit, edits or capture stop.
To extract and display serial data: inspect log schema, call log.analyze timeline with retain_dataset:true, finish its cursor, then data.present {query_id:returnedQueryID,expected_fields:[names requested by the user]}; use the returned curves and missing_fields to report exactly what was drawn; the host imports actual local points into the data analysis page. Do not pass model-generated arrays.
Discover the complete published platform API with platform.catalog {filter,offset,limit}; then invoke an exact listed method using platform.invoke {method,params}. Catalogue params are examples, never defaults. Device/file/test mutations require concrete confirmation. Never guess targets or credentials. If service_required is true, report unavailable services honestly.
Canonical shared host tools also available: wave.channels/measure/timing/frames/decode/view, data.inspect/statistics/present, calc.inspect/evaluate/apply, report.inspect/read/search/create/open, testing.inspect/status/cases/propose/wait. Use agent.help {method:...} for the exact canonical schema. Reversible view/calculator/decoder operations require actual user intent; never change a protocol by guessing. testing.wait waits locally on an actual run_id; do not consume model calls polling. A manually authorized test_execution task can request testing.preflight or testing.run through host confirmation; testing.pause, testing.resume and testing.stop apply only to its own verified run and each requires confirmation. testing.apply remains in the global Agent. For requested report files, report.create {title,markdown,evidence_ids:[actualIDs]} saves portable Markdown/JSON and real-series PNG/CSV; call report.open {report_id:returnedUUID} to show it in WiParse. Never invent evidence IDs or local file paths.
serial.monitor.status {} (actual CURRENT source status, read-only; not a global aggregate);
serial.monitor.start {} / serial.monitor.stop {} (control permission required; CURRENT live source only, no port/baud overrides; start waits for actual connection, stop confirms reception stopped and explicitly reports asynchronous port release). Prefer these over legacy serial actions for multi-step workflows: after the host result you may discover a scope and stop it, without guessing success;
engineering.inspect {"tool":"waveform"|"data_analysis"|"test_report"|"test_tool"|"calculator"|"instruments"}: relevant current-project tool context with source identity; cached context is not fresh measurement. Use other tools only when necessary to answer the engineering request.
engineering.wave.measure {"channel":index,"start_s":optionalSeconds,"end_s":optionalSeconds}: local original-sample measurements, units/provenance/quality, read-only background worker.
engineering.wave.frames {"contains":optionalText,"errors_only":optionalBool,"offset":0,"limit":10}: actual decoded data frames and configuration provenance.
engineering.calc.evaluate {"card":"lc|bandpass|q|rc|crc|converter","fields":{...}}: local deterministic calculation. Parameters must use canonical fields and SI units from the native calculator contract. Use calculator inspection for inputs, do not invent component values.
instrument.list {} (discover connected device IDs, kinds and capabilities first);
instrument.overview {} (cached device state and measurements);
instrument.command {"device_id":ID,"command":...} (typed ControlCommand only): strings ScopeRun, ScopeStop, ScopeSingle, ScopeAutoset, ProbeStatus, BridgeInfo, DongleInfo; objects {"ScopeMeasure":{"channel":1,"meas_type":"Frequency"}}, {"ScopeChannel":{"channel":1,"enabled":true}}, {"ScopeScale":{"channel":1,"volts_per_div":1}}, {"ScopeTimebase":0.001}, {"ScopeTrigger":{"source":"CH1","level":1,"slope":"POS"}}, {"SourceVoltage":{"channel":1,"value":5}}, {"SourceCurrent":{"channel":1,"value":1}}, {"SourceOutput":{"channel":1,"enabled":false}}, {"LoadInput":false}, {"LoadMode":"CC"}, {"LoadLevel":{"mode":"CC","value":1}};
instrument.capture {"device_id":ID} (screen capture to the GUI, job completion awaited; text AI cannot inspect image pixels);
instrument.waveform_source {"device_id":ID} (capture full source to a new host-owned project evidence file; await completion and use returned metadata/metrics);
parse.line {"text":"one Qi ASK/FSK line"}; parse.metrics {"text":"AA55 metric frame"}; convert.radix {"input":"255","source_base":10,"target_base":16};
test.status {} (current serial test state); log.brief {} (cached serial metrics); ui.calc.get {} (calculator state).
Serial start/stop and instrument control/capture require device-control permission and no active test. Serial status, device lists, cached state and analysis queries are read-only and do not require control permission. Existing connected devices only; never connect guessed addresses. Never invent power setpoints/limits or perform a destructive flash/reset; ask for unspecified settings. No raw SCPI, shell, file paths, erase or arbitrary serial bytes. Platform calls run through the existing device worker. A queue acknowledgement is not completion; unknown/timeout is not success. The host extends the read budget for distinct pages with verified coverage and reserves a final summary; results are supplied automatically for continued reasoning. Do not repeat a failed or unknown output-changing command. Use only fields named above. The host rejects unsupported calls."#
}

fn keys(params: &Value, allowed: &[&str]) -> bool {
    params
        .as_object()
        .is_some_and(|p| p.keys().all(|k| allowed.contains(&k.as_str())))
}

pub(super) fn validate(method: &str, params: &Value) -> Result<bool, String> {
    if monitor_review::is_local(method) {
        if !params.is_object() { return Err("AI.MONITOR_LOCAL_INVALID".into()); }
        return Ok(false);
    }
    if library::is_local(method){library::validate(method,params)?;return Ok(false);}
    if crate::ai::broker::supports(method) {
        let tool=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:method.into(),params:params.clone()})?;
        return Ok(matches!(tool.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test));
    }
    if matches!(method,"serial.view"|"serial.search"|"serial.jump") {
        let allowed=match method{"serial.view"=>vec![],"serial.search"=>vec!["view_id","query"],_=>vec!["view_id","line"]};
        if !keys(params,&allowed)|| (method!="serial.view"&&params["view_id"].as_u64().is_none()) || (method=="serial.jump"&&!params["line"].as_u64().is_some_and(|n|n>0)) || (method=="serial.search"&&!params["query"].as_str().is_some_and(|s|!s.trim().is_empty()&&s.len()<=256&&!s.chars().any(char::is_control))){return Err("SERIAL.INVALID_NAVIGATION".into());}
        return Ok(false);
    }
    let invalid = || "AI.TOOL_INVALID: unsupported platform method or parameters".to_string();
    match method {
        // Management tools are read-only/local; validate inside their handler so
        // an invalid model proposal becomes a tool result the model can repair.
        "agent.plan"|"agent.evidence"|"agent.help"|"agent.align"|"agent.recipe"=>Ok(false),
        "engineering.inspect"=>{
            if !keys(params,&["tool"])||!matches!(params["tool"].as_str(),Some("waveform"|"data_analysis"|"test_report"|"test_tool"|"calculator"|"instruments")){return Err(invalid());}Ok(false)
        },
        "engineering.wave.measure"|"engineering.wave.frames"|"engineering.calc.evaluate"=>{
            let target=match method{"engineering.wave.measure"=>"wave.measure","engineering.wave.frames"=>"wave.frames",_=>"calc.evaluate"};
            wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:target.into(),params:params.clone()})?;Ok(false)
        },
        "serial.monitor.status"
        | "instrument.list"
        | "instrument.overview"
        | "test.status"
        | "log.brief"
        | "ui.calc.get" => {
            if !keys(params, &[]) {
                return Err(invalid());
            }
            Ok(false)
        }
        "serial.monitor.start" | "serial.monitor.stop" => {
            if !keys(params, &[]) {
                return Err(invalid());
            }
            Ok(true)
        }
        "parse.line" | "parse.metrics" => {
            if !keys(params, &["text"])
                || !params["text"].as_str().is_some_and(|s| s.len() <= 16384)
            {
                return Err(invalid());
            }
            Ok(false)
        }
        "convert.radix" => {
            if !keys(params, &["input", "source_base", "target_base"])
                || !params["input"].as_str().is_some_and(|s| s.len() <= 512)
                || !params["source_base"]
                    .as_u64()
                    .is_some_and(|v| [2, 8, 10, 16].contains(&v))
                || !params["target_base"]
                    .as_u64()
                    .is_some_and(|v| [2, 8, 10, 16].contains(&v))
            {
                return Err(invalid());
            }
            Ok(false)
        }
        "instrument.capture" | "instrument.waveform_source" => {
            if !keys(params, &["device_id"]) || params["device_id"].as_u64().is_none() {
                return Err(invalid());
            }
            Ok(true)
        }
        "instrument.command" => {
            wiparse_core::engineering_agent::scope_verification(params)?;
            if !keys(params, &["device_id", "command","verify"]) || params["device_id"].as_u64().is_none() {
                return Err(invalid());
            }
            let command = &params["command"];
            let (name, fields) = if let Some(name) = command.as_str() {
                (name, None)
            } else {
                let obj = command
                    .as_object()
                    .filter(|o| o.len() == 1)
                    .ok_or_else(invalid)?;
                let (name, value) = obj.iter().next().unwrap();
                (name.as_str(), Some(value))
            };
            let allowed: &[&str] = match name {
                "ScopeRun" | "ScopeStop" | "ScopeSingle" | "ScopeAutoset" | "ProbeStatus"
                | "BridgeInfo" | "DongleInfo"
                    if fields.is_none() =>
                {
                    &[]
                }
                "ScopeMeasure" => &["channel", "meas_type"],
                "ScopeChannel" => &["channel", "enabled"],
                "ScopeScale" => &["channel", "volts_per_div"],
                "ScopeOffset" => &["channel", "volts"],
                "ScopePosition" => &["channel", "divisions"],
                "ScopeTrigger" => &["source", "level", "slope"],
                "SourceVoltage" | "SourceCurrent" | "SourceOvp" | "SourceOcp" => {
                    &["channel", "value"]
                }
                "SourceOutput" => &["channel", "enabled"],
                "LoadLevel" => &["mode", "value"],
                "ScopeTimebase" | "LoadMode" | "LoadInput" => &[],
                _ => return Err(invalid()),
            };
            if !allowed.is_empty() && !fields.is_some_and(|v| keys(v, allowed)) {
                return Err(invalid());
            }
            if let Some(channel) = fields.and_then(|v| v.get("channel")) {
                if !channel.as_u64().is_some_and(|n| (1..=8).contains(&n)) {
                    return Err(invalid());
                }
            }
            if name == "ScopeScale"
                && !fields
                    .and_then(|v| v["volts_per_div"].as_f64())
                    .is_some_and(|v| v.is_finite() && v > 0.)
            {
                return Err(invalid());
            }
            if name == "ScopeTimebase"
                && !fields
                    .and_then(|v| v.as_f64())
                    .is_some_and(|v| v.is_finite() && v > 0.)
            {
                return Err(invalid());
            }
            if name == "LoadLevel"
                && !fields
                    .and_then(|v| v["value"].as_f64())
                    .is_some_and(|v| v.is_finite() && v >= 0.)
            {
                return Err(invalid());
            }
            if name == "LoadMode"
                && !fields
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| ["CC", "CV", "CR", "CP"].contains(&v))
            {
                return Err(invalid());
            }
            let parsed =
                wiparse_core::instrument::parse_control_command(command).map_err(|_| invalid())?;
            // Parsing rejects unknown enum/value shapes; reject non-finite or negative supply magnitudes.
            let _ = parsed;
            if name.starts_with("Source")
                && name != "SourceOutput"
                && !fields
                    .and_then(|v| v["value"].as_f64())
                    .is_some_and(|v| v.is_finite() && v >= 0.0)
            {
                return Err(invalid());
            }
            Ok(true)
        }
        _ => Err(invalid()),
    }
}

/// Forwarding is safe only when the canonical broker will review and bind
/// the target session before dispatch. It is not an authorization to execute.
pub(super) fn broker_reviews_control(ctx: &egui::Context, method: &str) -> bool {
    !method.starts_with("serial.monitor.") && crate::ai::broker::available(ctx)
        && (crate::ai::broker::supports(method) || crate::ai::broker::has_instrument_executor(ctx, method))
}

fn persist_control_intent(ctx: &egui::Context, chat: &SerialAiChat, method: &str, params: &Value,
    command_id: &str, fingerprint: &str, question: &str, automatic: bool) -> Result<ControlAudit, String> {
    let (journal, task_id, workflow) = if automatic {
        let journal = crate::ai::journal::Journal::for_project(ctx, &chat.project)
            .ok_or("AGENT.JOURNAL_UNAVAILABLE: device action not dispatched")?;
        let mut workflow = wiparse_core::ai::workflow::Workflow::default();
        workflow.skill_ids = chat.selected_skills.clone();
        workflow.bind_skill_route(chat.auto_selected_skill, question);
        (journal, uuid::Uuid::new_v4().to_string(), workflow)
    } else {
        let run = chat.run.as_ref().ok_or("AGENT.TASK_REQUIRED: device action not dispatched")?;
        let journal = run.journal.clone().ok_or("AGENT.JOURNAL_UNAVAILABLE: device action not dispatched")?;
        (journal, run.id.clone(), run.workflow.clone())
    };
    journal.prepare_control_storage()?;
    let entry = if automatic { "serial-watch" } else { "serial" };
    journal.record_checked(&task_id, entry, question, "awaiting_readback", &workflow,
        Some((method, &json!({"state":"dispatching","command_id":command_id,
            "command_fingerprint":fingerprint,"device_id":params.get("device_id"),"target_port":params.get("target_port")}))))
        .map_err(|error| format!("AGENT.JOURNAL_UNAVAILABLE: device action not dispatched: {error}"))?;
    Ok(ControlAudit { journal, task_id, entry, question: question.to_owned(), workflow })
}

fn bridge(ctx: &egui::Context) -> Option<ApiBridge> {
    ctx.data(|d| d.get_temp(egui::Id::new("wiparse-ai-platform-bridge")))
}

fn enqueue(
    api: &ApiBridge,
    method: &str,
    params: Value,
    guard: AiGuard,
) -> Result<Receiver<InvokeReply>, String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    api.request_tx
        .try_send(PendingRequest {
            method: method.into(),
            params,
            reply: tx,
            deadline: Instant::now() + Duration::from_secs(10),
            ai_guard: Some(guard),
        })
        .map_err(|_| "AI.TOOL_BUSY: platform queue full".to_string())?;
    api.wake_ui();
    Ok(rx)
}

impl SerialToolPanel {
    fn enqueue_reviewed_stop(&mut self, ctx:&egui::Context, chat:&mut SerialAiChat, current:&Snapshot,
        permit:&SerialControlPermit, command_id:&str, fingerprint:&str, audit:ControlAudit) -> Result<(),String> {
        let port=current.binding.device.as_deref().ok_or("SERIAL.NO_CURRENT_PORT")?;
        let endpoint=self.serial_reviewed_stop_endpoint(port).ok_or("SERIAL.GATE_SESSION_UNAVAILABLE")?;
        let slot=self.serial_stop_command_slot(port).ok_or("SERIAL.GATE_SESSION_UNAVAILABLE")?;
        let (identity,sample)=chat.monitor_review.as_ref().ok_or("AI.MONITOR_REVIEW_SNAPSHOT_MISSING")?;
        let service=crate::ai::service(ctx).ok_or("AI.SERVICE_UNAVAILABLE")?;
        let config=chat.config.as_ref().ok_or("AI.CONFIG_UNAVAILABLE")?;
        let revision=service.revision_for_config(config).ok_or("AI.CONFIG_CHANGED")?;
        if identity.value()["service_id"].as_str()!=Some(service.instance_id().to_string().as_str())
            || identity.value()["service_revision"].as_u64()!=Some(revision as u64) {return Err("AI.CONFIG_CHANGED".into());}
        if !sample.is_current() || !permit.valid(current,chat,&self.local_log_scope_key(chat),Instant::now()) {
            return Err("AI.APPROVAL_EXPIRED_OR_REVOKED".into());
        }
        let authority=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||json!({"revision":chat.monitor_revision(),
            "skills":chat.selected_skills,"control_allowed":chat.control_allowed(current)})).ok_or("AI.MONITOR_REVIEW_BUSY")?;
        let elapsed=Instant::now().checked_duration_since(permit.created).ok_or("AI.APPROVAL_EXPIRED_OR_REVOKED")?;
        let lifetime=Duration::from_secs(600).checked_sub(elapsed).ok_or("AI.APPROVAL_EXPIRED_OR_REVOKED")?;
        let id=uuid::Uuid::parse_str(command_id).map_err(|_|"SERIAL.INVALID_COMMAND_ID")?;
        let target=dispatch_gate::Target{project:chat.project.clone(),port:port.to_owned(),session:endpoint.session_id.to_string(),
            baud:current.binding.baud,source_revision:sample.generation(),authorization_revision:authority.generation(),
            task:audit.task_id.clone()};
        let sources=sample.clone().join(authority);
        let worker_sources=sources.clone();
        let (tx,rx)=crossbeam_channel::bounded(1);
        let intent=SerialStopIntent{command_id:command_id.into(),project:chat.project.clone(),fingerprint:fingerprint.into(),
            audit:Some(SerialStopAudit{journal:audit.journal.clone(),task_id:audit.task_id.clone()})};
        let queued=service.commit_config_revision(revision,||sources.commit_in_memory(|| -> Result<dispatch_gate::Command,String> {
            endpoint.gate.publish(Some(target.clone()),true).map_err(str::to_owned)?;
            let command=endpoint.gate.confirm_with_id(&target,dispatch_gate::Operation::Stop,lifetime,id).map_err(str::to_owned)?;
            endpoint.gate.enqueue(&command).map_err(str::to_owned)?;
            let request=dispatch_gate::ReviewedStop{gate:endpoint.gate.clone(),command:command.clone(),sources:worker_sources,service:service.clone(),service_revision:revision,reply:tx,slot:slot.clone(),intent};
            if endpoint.tx.try_send(request).is_err() {let _=endpoint.gate.cancel(id);return Err("SERIAL.GATE_QUEUE_UNAVAILABLE".into());}
            Ok(command)
        })).flatten().ok_or("AI.MONITOR_REVIEW_SOURCE_OR_CONFIG_CHANGED")??;
        chat.reviewed_stop_pending=Some(ReviewedStopPending{gate:endpoint.gate,command:queued,reply:rx,slot,audit,
            deadline:Instant::now()+Duration::from_secs(30)});
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn test_broker_control_guard(&mut self, project: &str) -> AiGuard {
        self.ai_chat.allow_automation=true;
        self.ai_chat.project=project.into();
        self.ai_chat.run=Some(run::Run::new());
        let scope=self.local_log_scope_key(&self.ai_chat);
        self.ai_chat.run.as_mut().unwrap().scope_key=scope;
        let state=self.ai_state(project,false);
        AiGuard { task_id:self.ai_chat.run.as_ref().map(|run|run.id.clone()),
            skill_ids:self.ai_chat.selected_skills.clone(),binding:serde_json::to_string(&state.binding).unwrap(),
            project:project.into(),controls:true,cancelled:Arc::new(AtomicBool::new(false)) }
    }
    pub(crate) fn ai_platform_result_guard_valid(&self, guard: &AiGuard, method:&str) -> bool {
        let mut read_guard = guard.clone(); read_guard.controls = false;
        self.ai_platform_guard_valid(&read_guard) && (!guard.controls || self.ai_chat.allow_automation
            && self.ai_chat.run.as_ref().is_none_or(|run| if matches!(method,"testing.stop"|"testing.pause"|"testing.resume") {run.test_control_cap} else {run.control_cap}))
    }
    pub(crate) fn ai_platform_guard_for(&self, guard: &AiGuard, method: &str, params: &Value) -> bool {
        let mut read_guard = guard.clone(); read_guard.controls = false;
        self.ai_platform_guard_valid(&read_guard) && (!guard.controls || self.ai_chat.allow_automation
            && self.ai_chat.run.as_ref().is_none_or(|run| if matches!(method,"testing.stop"|"testing.pause"|"testing.resume") {run.test_control_cap} else {run.control_cap}) &&
            (!self.ai_controls_locked(self.ai_test_locked) || matches!(method,"testing.stop"|"testing.pause"|"testing.resume") || wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call {method:method.into(),params:params.clone()})))
    }
    /// Acknowledge only the host-dispatched source transition, never a user switch.
    pub(crate) fn ai_accept_source_transition(&mut self, guard: &mut AiGuard) {
        let _monitor_update = self.ai_chat.monitor_lease.begin_update();
        let state = self.ai_state(&guard.project,self.ai_test_locked);
        guard.binding = serde_json::to_string(&state.binding).unwrap_or_default();
        let scope = self.local_log_scope_key(&self.ai_chat);
        self.ai_chat.conversation_source = Some((state.binding.source_id,state.binding.device.clone()));
        if let Some(tool) = &mut self.ai_chat.tool { tool.guard.binding = guard.binding.clone(); tool.local_scope = scope.clone(); }
        self.ai_chat.rebind_host_source(&state.binding, &scope);
        self.ai_chat.watch = None;
    }
    pub(crate) fn ai_platform_guard_valid(&self, guard: &AiGuard) -> bool {
        !guard.cancelled.load(Ordering::Acquire)
            && guard.project == self.ai_chat.project
            && guard.task_id.as_ref().is_none_or(|id| self.ai_chat.run.as_ref().is_some_and(|run| &run.id == id && run.scope_key==self.local_log_scope_key(&self.ai_chat)))
            && guard.skill_ids == self.ai_chat.selected_skills
            && serde_json::to_string(&self.ai_state(&guard.project, self.ai_test_locked).binding)
                .ok()
                .as_deref()
                == Some(guard.binding.as_str())
            && (!guard.controls
                || (self.ai_chat.allow_automation
                    && self.ai_chat.run.as_ref().is_none_or(|run| run.control_cap)
                    && !self.ai_controls_locked(self.ai_test_locked)))
    }

    pub(super) fn local_log_scope_key(&self,chat:&SerialAiChat)->String {
        use std::hash::{Hash,Hasher};
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        if chat.scope==DataScope::Selection{self.tabs.get(self.active_tab).map(|tab|tab.ai_selection()).hash(&mut hash);}
        if chat.scope==DataScope::Library{chat.library.directory.hash(&mut hash);}
        format!("{:?}:{}:{}:{}:{}",chat.scope,chat.range_first,chat.range_last,self.tabs.get(self.active_tab).map_or(0,LogTabPage::ai_view_id),hash.finish())
    }
    #[cfg(test)]
    pub(super) fn test_bind_serial_permit(&self, chat: &mut SerialAiChat) {
        let state=self.ai_state(&chat.project,self.ai_test_locked);
        let scope=self.local_log_scope_key(chat);
        if let Some(run)=&mut chat.run {if run.scope_key.is_empty(){run.scope_key=scope.clone();}}
        chat.confirmed_serial_permit=Some(SerialControlPermit { binding:state.binding,scope_key:scope,
            skills:chat.selected_skills.clone(),run_id:chat.run.as_ref().map(|run|run.id.clone()),created:Instant::now() });
    }
    pub(super) fn begin_platform_call(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        method: String,
        mut params: Value,
        question: String,
        automatic: bool,
    ) -> Result<(), String> {
        if let Some(source)=chat.run.as_ref().filter(|_|!automatic && !chat.local_preview_request).and_then(|run|run.reviewed_log.as_ref()) {
            if !source.is_current() || !chat.review_outbound {return Err("LOG.REVIEW_SOURCE_CHANGED".into());}
            if !matches!(method.as_str(),"log.files"|"log.analyze"|"agent.plan"|"agent.evidence"|"agent.help") {
                return Err("LOG.REVIEW_TOOL_NOT_ALLOWED".into());
            }
        }
        // Reviewed automatic local helpers only read the frozen sample or
        // evaluate caller-supplied numbers. Other producers need their own
        // source permits before their results may be sent on a later round.
        let reviewed_local = automatic && chat.review_outbound && monitor_review::is_local(&method);
        if monitor_review::is_local(&method) && !reviewed_local {
            return Err("AI.MONITOR_LOCAL_REQUIRES_REVIEWED_WATCH".into());
        }
        // A concrete approval is one-shot even if a later validation fails.
        let serial_confirmation = method.starts_with("serial.monitor.").then(||
            (chat.confirmed_platform_action.take(), chat.confirmed_serial_permit.take()));
        if !reviewed_local && !(chat.local_preview_request && method == "log.analyze")
            && !wiparse_core::ai::skills::permits(&chat.selected_skills, &method) {
            return Err("SKILL.TOOL_NOT_ALLOWED: 当前技能不支持此工具".into());
        }
        if matches!(method.as_str(), "testing.run" | "testing.preflight" | "testing.stop" | "testing.pause" | "testing.resume") &&
            (!chat.selected_skills.iter().any(|id| id == "test_execution") || automatic || chat.run.is_none()) {
            return Err("TEST.EXPLICIT_SKILL_AND_TASK_REQUIRED".into());
        }
        // Local log validation failures are safe, deterministic non-executions.
        // Return them as tool results so the model can repair a rule instead
        // of aborting the whole task after the evidence was already collected.
        let mut local_validation_error=None;
        let controls = match validate(&method, &params) {
            Ok(value)=>value,
            Err(error) if library::is_local(&method) && !automatic && !chat.local_preview_request=>{local_validation_error=Some(error);false},
            Err(error)=>return Err(error),
        };
        // Source update intervals invalidate outbound snapshots but do not
        // exclude concurrent serial-worker updates. Until the host has an
        // exclusive target-bound dispatch mechanism, reviewed automatic
        // control cannot safely bridge approval to the actual operation.
        if automatic && chat.review_outbound && controls && method != "serial.monitor.stop" {
            return Err("AI.MONITOR_REVIEW_CONTROL_UNAVAILABLE: use a manual task for device control".into());
        }
        if automatic && chat.review_outbound && !reviewed_local && !controls {
            return Err("AI.MONITOR_REVIEW_TOOL_SOURCE_UNGUARDED: tool has no reviewed source permit".into());
        }
        let mut serial_permit = None;
        if controls {
            if method.starts_with("serial.monitor.") {
                let (action, permit) = serial_confirmation.unwrap_or_default();
                let exact = action.as_ref() == Some(&(method.clone(), params.clone()));
                let current = self.ai_state(&chat.project, self.ai_test_locked);
                if !exact || !permit.as_ref().is_some_and(|permit| permit.valid(&current, chat, &self.local_log_scope_key(chat), Instant::now())) {
                    return Err("AI.CONCRETE_CONFIRMATION_REQUIRED".into());
                }
                serial_permit = permit;
            } else if !broker_reviews_control(ctx, &method) || automatic || chat.run.is_none() {
                return Err("AI.VERIFIED_DEVICE_SESSION_AND_MANUAL_TASK_REQUIRED".into());
            }
        }
        if controls && !automatic && chat.run.as_ref().is_some_and(|run|run.scope_key!=self.local_log_scope_key(chat)) {
            return Err("AI.CONTROL_SCOPE_CHANGED: 操作未发送 / Analysis scope changed; action not dispatched".into());
        }
        if method=="serial.monitor.stop" && !self.ai_state(&chat.project,self.ai_test_locked).reception_requested {
            return Err("SERIAL.NOT_RECEIVING: 当前端口未接收，未发送停止请求 / Port is not receiving; no stop request dispatched".into());
        }
        let command_id=uuid::Uuid::new_v4().to_string();
        // Serial actions use an implicit current port; include that resolved
        // identity so an uncertain COM1 operation does not block COM2, and a
        // reconnect to COM1 cannot silently bypass the prior readback.
        let fingerprint_params = if method == "serial.monitor.start" || method == "serial.monitor.stop" {
            let state = self.ai_state(&chat.project, self.ai_test_locked);
            json!({"target_port":state.binding.device})
        } else { params.clone() };
        let fingerprint=crate::ai::journal::command_fingerprint(&method,&fingerprint_params);
        let previous_unknown=if controls {
            if automatic {
                let journal = crate::ai::journal::Journal::for_project(ctx, &chat.project)
                    .ok_or("AGENT.JOURNAL_UNAVAILABLE: device action not dispatched")?;
                journal.needs_readback(&method, &fingerprint_params)?
            } else {
                chat.run.as_ref().and_then(|r|r.journal.as_ref()).map(|j|j.needs_readback(&method,&fingerprint_params)).transpose()?.unwrap_or(false)
            }
        } else {false};
        if controls && (previous_unknown || chat.run.as_ref().is_some_and(|r|r.uncertain.contains(&fingerprint)))
            && !wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call{method:method.clone(),params:params.clone()}) {
            if let Some(run)=chat.run.as_mut(){
                run.pending_readback=Some((fingerprint.clone(),format!("{method} · {}",fingerprint_params)));
            }
            return Err(if automatic {"AI.OUTCOME_UNKNOWN: inspect the device and use a manual task to acknowledge readback before retrying"}
                else {"AI.OUTCOME_UNKNOWN: inspect the instrument before repeating this command"}.into());
        }
        if library::is_local(&method)&&(automatic||chat.scope==DataScope::None){return Err("LOG.DATA_SCOPE: choose a manual log scope first".into());}
        if chat.tool.is_some(){return Err("AI.TOOL_BUSY".into());}
        if automatic {
            if chat.automatic_tool_round>=run::BASE_CALLS{return Err("AI.MONITOR_TOOL_LIMIT".into());}
        }else if !chat.local_preview_request { if let Some(reason)=chat.run.as_ref().and_then(|r|r.limit_reason(chat.tool_round)){
            return Err(reason.into());
        }}
        let owned_test_control=matches!(method.as_str(),"testing.stop"|"testing.pause"|"testing.resume");
        if controls && (!chat.allow_automation || chat.run.as_ref().is_some_and(|r|if owned_test_control {!r.test_control_cap} else {!r.control_cap}) || self.ai_controls_locked(self.ai_test_locked) && !owned_test_control && !wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call{method:method.clone(),params:params.clone()})) {
            return Err(
                "AI.CONTROL_LOCKED: platform device control is disabled or test running".into(),
            );
        }
        if !automatic&&!chat.local_preview_request&&library::is_local(&method)&&params.get("cursor").is_none(){
            let signature=format!("{}:{}:{}",method,params,self.ai_state(&chat.project,self.ai_test_locked).absolute_lines);
            if let Some(run)=&mut chat.run{if run.last_read.as_ref()==Some(&signature){return Err("AI.NO_PROGRESS: identical read already completed; use prior evidence or a narrower query".into());}run.last_read=Some(signature);}
        }
        let canonical_instrument=crate::ai::broker::has_instrument_executor(ctx,&method);
        if params.get("verify").is_some() && method=="instrument.command" && !canonical_instrument{return Err("DEVICE.VERIFICATION_HOST_REQUIRED".into());}
        if method == "instrument.capture" && !canonical_instrument {
            params["wait"] = json!(true);
        }
        // Persist the exact intent before either local dispatch or broker enqueue.
        // This ordering does not depend on the consumer sharing the UI thread.
        let audit = if controls { Some(persist_control_intent(ctx, chat, &method, &fingerprint_params,
            &command_id, &fingerprint, &question, automatic)?) } else { None };
        if method == "serial.monitor.start" || method == "serial.monitor.stop" {
            let current = self.ai_state(&chat.project, self.ai_test_locked);
            if !serial_permit.as_ref().is_some_and(|permit| permit.valid(&current, chat, &self.local_log_scope_key(chat), Instant::now())) {
                if let Some(audit) = &audit {
                    if let Err(error) = audit.not_dispatched(&method, &command_id) {
                        return Err(format!("AI.APPROVAL_EXPIRED_OR_REVOKED; AGENT.JOURNAL_COMPLETION_UNAVAILABLE: {error}; action not dispatched"));
                    }
                }
                return Err("AI.APPROVAL_EXPIRED_OR_REVOKED: serial action not dispatched".into());
            }
            if automatic && chat.review_outbound && method == "serial.monitor.stop" {
                let durable=audit.as_ref().ok_or("AGENT.JOURNAL_UNAVAILABLE")?;
                let result=self.enqueue_reviewed_stop(ctx,chat,&current,serial_permit.as_ref().unwrap(),&command_id,&fingerprint,durable.clone());
                if let Err(error)=result {
                    if let Err(journal_error)=durable.not_dispatched(&method,&command_id) {
                        return Err(format!("{error}; AGENT.JOURNAL_COMPLETION_UNAVAILABLE: {journal_error}; action not dispatched"));
                    }
                    return Err(error);
                }
                chat.note_background("停止请求已入队，等待串口工作线程认领 / Stop queued; waiting for serial worker claim",false);
                return Ok(());
            }
            // Associate a stop intent only with the exact currently selected
            // worker's completion channel. Missing/poisoned tracking remains
            // unknown and is never treated as proof of release.
            let stop_slot = if method == "serial.monitor.stop" {
                current.binding.device.as_deref().and_then(|port|self.serial_stop_command_slot(port))
            } else { None };
            if let Some(slot) = &stop_slot {
                if let Ok(mut value) = slot.lock() { *value = Some(SerialStopIntent { command_id:command_id.clone(),project:chat.project.clone(),fingerprint:fingerprint.clone(),audit:audit.as_ref().map(|a|SerialStopAudit{journal:a.journal.clone(),task_id:a.task_id.clone()}) }); }
            }
            let saved_history = automatic.then(|| std::mem::take(&mut chat.history));
            let saved_status_error = chat.status_error;
            let saved_status = automatic.then(|| chat.status.clone());
            let applied = self.apply_ai_action(
                chat,
                if method.ends_with("start") {
                    Action::SerialStart
                } else {
                    Action::SerialStop
                },
                &question,
                self.last_lang,
                self.ai_test_locked,
            );
            if let Some(history) = saved_history { chat.history = history; chat.status_error = saved_status_error; }
            if let Some(status) = saved_status { chat.status = status; }
            if let Err(error) = applied {
                if let Some(slot) = &stop_slot {
                    if let Ok(mut value) = slot.lock() { if value.as_ref().is_some_and(|intent| intent.command_id == command_id) { *value = None; } }
                }
                if let Some(audit) = &audit {
                    if let Err(journal_error) = audit.not_dispatched(&method, &command_id) {
                        return Err(format!("{error}; AGENT.JOURNAL_COMPLETION_UNAVAILABLE: {journal_error}; action not dispatched, recovery may require manual review"));
                    }
                }
                return Err(error);
            }
            // A successful start may open a new live tab (e.g. an additional port).
            // Bind only this host-owned transition; later user changes still cancel.
            let own = self.ai_state(&chat.project, self.ai_test_locked);
            chat.conversation_source = Some((own.binding.source_id, own.binding.device.clone()));
            let scope = self.local_log_scope_key(chat);
            chat.rebind_host_source(&own.binding, &scope);
            if automatic && method == "serial.monitor.stop" {
                // The host accepted the stop, but OS handle release is
                // asynchronous. The watch has ended, so there is no valid
                // watch permit for a model continuation. Preserve an explicit
                // unconfirmed receipt instead of treating the transition as a
                // stale tool result or automatically replaying the command.
                let terminal = json!({"state":"unknown","command_id":command_id,
                    "command_fingerprint":fingerprint,"outcome":"host_stop_requested",
                    "physical_verified":false,"port_release":"unconfirmed_async"});
                let recorded = audit.as_ref().is_some_and(|audit| audit.journal.record_checked(
                    &audit.task_id, audit.entry, &audit.question, "awaiting_readback", &audit.workflow,
                    Some((&method, &terminal))).is_ok());
                chat.automatic_evidence_counter = chat.automatic_evidence_counter.saturating_add(1);
                let evidence = json!({"evidence_id":format!("W{}", chat.automatic_evidence_counter),
                    "method":method,"ok":true,"data":{"host_stop_requested":true,
                    "command_id":command_id,"target_port":fingerprint_params["target_port"],
                    "port_release":"unconfirmed_async","physical_verified":false,
                    "readback_required_before_replay":true,"journal_recorded":recorded}});
                chat.automatic_tool_results.push(evidence.clone());
                chat.entry("tool", format!("串口停止请求已提交，端口释放尚未确认；自动监控已结束，请核查设备状态后再重试。 / Stop requested; port release is not confirmed. Verify the device before retrying.\n\n```json\n{}\n```", evidence));
                if let Some(entry) = chat.entries.last_mut() { entry.automatic = true; entry.error = !recorded; }
                chat.finish_watch_transaction(false);
                return Ok(());
            }
        }
        let guard = AiGuard {
            task_id: if automatic { None } else { chat.run.as_ref().map(|run| run.id.clone()) },
            skill_ids: chat.selected_skills.clone(),
            binding: serde_json::to_string(
                &self.ai_state(&chat.project, self.ai_test_locked).binding,
            )
            .map_err(|e| e.to_string())?,
            project: chat.project.clone(),
            controls,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let reply = if let Some(error)=local_validation_error {
            let (tx,rx)=crossbeam_channel::bounded(1);
            let _=tx.send(InvokeReply{ok:false,cmd:method.clone(),error:Some(error),data:json!({"executed":false,"guidance":"Correct the local read rule or use agent.help; keep prior evidence."})});
            Some(rx)
        } else if reviewed_local {
            let (identity, sample) = chat.monitor_review.as_ref().ok_or("AI.MONITOR_REVIEW_SNAPSHOT_MISSING")?;
            if !identity.is_current() || !sample.is_current() { return Err("AI.MONITOR_REVIEW_SOURCE_CHANGED".into()); }
            let value = monitor_review::execute(&method, &params, sample.value())?;
            let (tx, rx) = crossbeam_channel::bounded(1);
            let _ = tx.send(crate::backend::invoke_ok(&method, value["data"].clone()));
            ctx.request_repaint();
            Some(rx)
        } else if matches!(method.as_str(),"serial.view"|"serial.search"|"serial.jump") {
            if automatic{return Err("SERIAL.NAVIGATION_MANUAL_TASK_ONLY".into());}
            if chat.scope!=DataScope::Current{return Err("SERIAL.NAVIGATION_REQUIRES_CURRENT_SCOPE: current-log navigation cannot map selection-relative or other-file line numbers".into());}
            let tab=self.tabs.get_mut(self.active_tab).ok_or("SERIAL.NO_CURRENT_TAB")?;
            let value=tab.ai_navigate(&method,&params)?;
            let(tx,rx)=crossbeam_channel::bounded(1);let _=tx.send(crate::backend::invoke_ok(&method,value));ctx.request_repaint();Some(rx)
        } else if method.starts_with("agent.") {
            let run=chat.run.as_mut().ok_or("AGENT.NO_ACTIVE_TASK")?;
            let result=wiparse_core::ai::workflow::validate_management(&method,&params).and_then(|_|match method.as_str(){
                "agent.recipe"=>run.workflow.run_recipe(&params),
                "agent.align"=>run.workflow.align(&params),
                "agent.plan"=>run.workflow.update(params.clone()),
                "agent.evidence"=>run.workflow.page(&params),
                "agent.help"=>{
                    let target=params["method"].as_str().unwrap();
                    let allowed=run::functions_for_skills_with_test(chat.scope,chat.allow_automation&&!self.ai_test_locked&&run.control_cap,&chat.selected_skills,chat.allow_automation&&run.test_control_cap&&chat.selected_skills.iter().any(|id|id=="test_execution")).into_iter().find(|f|f.name=="wiparse_call").unwrap();
                    if let Some(recipe)=wiparse_core::ai::workflow::recipe(target){Ok(recipe)}else if matches!(target,"agent.plan"|"agent.evidence"|"agent.help") && wiparse_core::ai::skills::permits(&chat.selected_skills,target) {
                        let schema=wiparse_core::ai::tools::host_functions(vec![target]).into_iter().find(|f|match target {"agent.plan"=>f.name=="wiparse_plan","agent.evidence"=>f.name=="wiparse_evidence","agent.help"=>f.name=="wiparse_help",_=>false}).map(|f|f.parameters);
                        Ok(json!({"ok":true,"method":target,"schema":schema,"guidance":wiparse_core::ai::workflow::GUIDANCE}))
                    }else if !allowed.parameters["properties"]["method"]["enum"].as_array().unwrap().iter().any(|v|v==target){Err("AGENT.UNKNOWN_OR_UNAUTHORIZED_TOOL".into())}else{
                        let help=if let Some(tool)=wiparse_core::engineering_agent::tool(target){tool.schema.to_owned()}else if target.starts_with("log."){library::catalog().to_owned()}else if target=="engineering.calc.evaluate"{wiparse_core::engineering_agent::CALC_SCHEMA.to_owned()}else{catalog().to_owned()};
                        Ok(json!({"ok":true,"method":target,"schema":help,"validated_keys":wiparse_core::engineering_agent::tool(target).and_then(|t|wiparse_core::engineering_agent::declared_keys(t.method))}))
                    }
                },_=>Err("AGENT.UNKNOWN_MANAGEMENT_TOOL".into())
            });
            let mut value=result.unwrap_or_else(|e|json!({"ok":false,"error":e,"executed":false,"guidance":"Correct the arguments using agent.help for this method; retain existing evidence and do not repeat device actions."}));value["kind"]=json!("task_management");
            let (tx,rx)=crossbeam_channel::bounded(1);let _=tx.send(InvokeReply{ok:value["ok"]!=false,cmd:method.clone(),error:value["error"].as_str().map(str::to_owned),data:value});Some(rx)
        }else if library::is_local(&method){
            let key=self.local_log_scope_key(chat);
            let use_frozen=!automatic && !chat.local_preview_request;
            let active_library = if chat.local_preview_request { &mut chat.preview_library } else { &mut chat.library };
            active_library.bind_scope(key);
            let mut files=Vec::new();let mut memory=None;
            let frozen=if use_frozen {chat.run.as_ref().and_then(|run|run.reviewed_log.as_ref())}else{None};
            if let Some(source)=frozen { memory=Some(source.value().source.clone()); }
            else if chat.scope==DataScope::Library {
                files=self.tabs.iter().filter_map(|tab|tab.filepath.as_ref().map(PathBuf::from)).collect();
                if self.save_live_to_disk {if let Some(path)=&self.live_file{files.push(path.clone());}}
            }else{
                let tab=self.tabs.get(self.active_tab).ok_or("LOG.NO_CURRENT_LOG")?;
                if chat.scope==DataScope::Selection {
                    let text=tab.ai_selection();if text.is_empty(){return Err("LOG.EMPTY_SELECTION".into());}
                    memory=Some(wiparse_core::log_analysis::Source::memory("选中内容（相对行号）",text.lines().map(Arc::<str>::from).collect(),1,0));
                }else if !tab.live && tab.filepath.is_some(){files.push(PathBuf::from(tab.filepath.as_ref().unwrap()));}
                else{memory=Some(wiparse_core::log_analysis::Source::memory(&tab.display_title(),tab.ai_shared_lines(),tab.dropped() as u64+1,tab.dropped() as u64));}
            }
            let range=if let Some(source)=frozen {source.value().range} else if chat.scope==DataScope::Range{Some((chat.range_first.max(1),chat.range_last.max(chat.range_first.max(1))))}else{None};
            Some(active_library.start_scoped(&method,params.clone(),files,memory,range,frozen.is_none() && chat.scope==DataScope::Library,guard.cancelled.clone(),ctx.clone())?)
        } else if method=="report.create" {
            let run=chat.run.as_ref().ok_or("REPORT.NO_ACTIVE_TASK")?;
            run.workflow.validate_conclusion(&format!("{}\n{}",params["title"],params["markdown"]))?;
            let evidence=params["evidence_ids"].as_array().ok_or("REPORT.MISSING_EVIDENCE")?.iter().map(|v|crate::ai::journal::report_input(run.journal.as_ref(),&run.workflow,v.as_str().unwrap_or(""))).collect::<Result<Vec<_>,_>>()?;
            let root=ctx.data(|d|d.get_temp::<PathBuf>(egui::Id::new("wiparse-ai-project-root"))).ok_or("REPORT.PROJECT_UNAVAILABLE")?;
            Some(crate::ai::artifact::start(root,chat.project.clone(),params.clone(),evidence,guard.cancelled.clone(),ctx.clone(),|value|InvokeReply{ok:value["ok"]==true,cmd:"report.create".into(),error:value["error"].as_str().map(str::to_owned),data:value["data"].clone()})?)
        } else if canonical_instrument || crate::ai::broker::supports(&method) {
            match crate::ai::broker::enqueue(ctx,&method,params.clone(),guard.clone()) {
                Ok(receiver) => Some(receiver),
                Err(error) => {
                    if let Some(audit) = &audit {
                        if let Err(journal_error) = audit.not_dispatched(&method, &command_id) {
                            return Err(format!("{error}; AGENT.JOURNAL_COMPLETION_UNAVAILABLE: {journal_error}; action not dispatched, recovery may require manual review"));
                        }
                    }
                    return Err(error);
                }
            }
        } else if method.starts_with("serial.monitor.") {
            None
        } else {
            let api = bridge(ctx).ok_or("AI.PLATFORM_UNAVAILABLE")?;
            Some(enqueue(&api, &method, params.clone(), guard.clone())?)
        };
        if automatic { chat.automatic_tool_round += 1; } else if !chat.local_preview_request { chat.tool_round += 1; }
        chat.entry(
            "tool",
            format!(
                "{} · {}",
                label(self.last_lang, "执行中", "Running"),
                method
            ),
        );
        if let Some(entry) = chat.entries.last_mut() {
            entry.automatic = automatic;
        }
        let deadline = Instant::now() + crate::ai::execution::timeout(&method);
        chat.tool = Some(ToolPending {
            reply,
            audit,
            guard,
            method,
            params,
            question,
            automatic,
            local_only:chat.local_preview_request,
            local_scope:self.local_log_scope_key(chat),
            watch_id:if automatic {chat.watch.as_ref().map(|watch|watch.id.clone())} else {None},
            watch_revision:automatic.then(||chat.monitor_revision()),
            deadline,
            command_id,fingerprint,job: None, job_session:None,
            next_poll: Instant::now(),
        });
        Ok(())
    }

    pub(super) fn poll_platform(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        lang: Lang,
    ) {
        let Some(mut tool) = chat.tool.take() else {
            return;
        };
        let current = self.ai_state(&chat.project, self.ai_test_locked);
        let changed = serde_json::to_string(&current.binding).ok().as_deref()
            != Some(tool.guard.binding.as_str()) || tool.guard.project != chat.project
            || tool.guard.skill_ids != chat.selected_skills
            || (!tool.automatic && (tool.guard.task_id.as_deref() != chat.run.as_ref().map(|run|run.id.as_str())
                || self.local_log_scope_key(chat) != tool.local_scope))
            || (tool.guard.controls && if matches!(tool.method.as_str(),"testing.stop"|"testing.pause"|"testing.resume") {!chat.owned_test_control_allowed()} else {!chat.control_allowed(&current)})
            || (tool.automatic && (tool.watch_revision.as_deref() != Some(chat.monitor_revision().as_str())
                || chat.review_outbound && !chat.reviewed_monitor_sources_current()
                || chat.watch.as_ref().is_none_or(|watch|watch.binding != current.binding || watch.ai_paused || Some(&watch.id) != tool.watch_id.as_ref())));
        let frozen_changed=!tool.automatic && !tool.local_only && chat.run.as_ref().and_then(|run|run.reviewed_log.as_ref()).is_some_and(|source|!chat.review_outbound || !source.is_current());
        if changed || frozen_changed || tool.guard.cancelled.load(Ordering::Acquire) {
            let restore_watch = tool.automatic && chat.watch.as_ref().is_some_and(|watch|
                Some(&watch.id) == tool.watch_id.as_ref() && watch.binding == current.binding);
            let same_task = !tool.automatic && tool.guard.task_id.is_some()
                && tool.guard.task_id.as_deref() == chat.run.as_ref().map(|run|run.id.as_str())
                && tool.guard.binding == serde_json::to_string(&current.binding).unwrap_or_default()
                && tool.local_scope == self.local_log_scope_key(chat);
            let stale_watch_id = tool.watch_id.clone();
            let stale_manual_task = !tool.automatic && tool.guard.task_id.is_some()
                && tool.guard.task_id.as_deref() == chat.run.as_ref().map(|run|run.id.as_str());
            let mut stale = Some(tool);
            if let Some((method, controls, fingerprint, automatic, journal_failed)) = cancel(&mut stale) {
                if controls && same_task {
                    if let Some(run)=&mut chat.run {if !run.uncertain.contains(&fingerprint){run.uncertain.push(fingerprint);}}
                    chat.tool_results.push(json!({"method":method,"result":{"ok":false,"error":"AI.CANCELLED_OR_STALE","unknown_if_already_dispatched":true,"cancellation_audit_persisted":!journal_failed}}));
                }
                chat.entry("tool", format!("历史工具调用已停止 · {method}\n{}", if controls {
                    if journal_failed {"设备动作结果未知，记录写入失败；请核查设备状态。"} else {"设备动作结果可能未知，请核查设备状态。"}
                } else {"任务、来源或权限已变化，结果不用于当前任务。"}));
                if let Some(entry)=chat.entries.last_mut(){entry.automatic=automatic;entry.error=controls;}
                if automatic {
                    if restore_watch {chat.finish_watch_transaction(false);} else {chat.discard_watch_transaction_for(stale_watch_id.as_deref());}
                }
            }
            if stale_manual_task { chat.cancel(); chat.queue_paused=true; }
            if frozen_changed {chat.cancel();chat.queue_paused=true;chat.note("LOG.REVIEW_SOURCE_CHANGED: 来源已变化，扫描结果已丢弃；请重新发起审阅任务 / Capture changed; scan discarded",true);}
            return;
        }
        let result = if Instant::now() >= tool.deadline {
            Some(
                json!({"ok":false,"error":"AI.TOOL_TIMEOUT","execution_state":"unknown; do not retry output-changing command"}),
            )
        } else if tool.method=="serial.search" {
            match self.tabs.get_mut(self.active_tab).ok_or_else(||"SERIAL.NO_CURRENT_TAB".to_owned()).and_then(|tab|tab.ai_navigate("serial.view",&json!({}))) {
                Ok(value) if value["search_query"]!=tool.params["query"]=>Some(json!({"ok":false,"error":"SERIAL.SEARCH_CHANGED_BY_USER"})),
                Ok(value) if value["search_pending"]==true=>None,
                Ok(value)=>Some(json!({"ok":true,"data":value})),
                Err(error)=>Some(json!({"ok":false,"error":error}))
            }
        } else if tool.method.starts_with("serial.monitor.") {
            if tool.method == "serial.monitor.start" && !current.connection_confirmed {
                if current.reception_requested {
                    None
                } else {
                    Some(
                        json!({"ok":false,"error":"SERIAL.CONNECTION_FAILED","status":current.status}),
                    )
                }
            } else {
                Some(json!({"ok":true,"data":current,
                    "port_release":if tool.method == "serial.monitor.stop" { "asynchronous; OS port release not yet confirmed" } else { "not_requested" }}))
            }
        } else {
            let reply = tool.reply.as_ref().and_then(|rx| match rx.try_recv() {
                Ok(r) => Some(r),
                Err(crossbeam_channel::TryRecvError::Disconnected) => Some(
                    crate::backend::invoke_err(&tool.method, "AI.TOOL_DISCONNECTED"),
                ),
                _ => None,
            });
            if let Some(reply) = reply {
                tool.reply = None;
                if !reply.ok {
                    // A failed test process can still provide a definitive
                    // completion identity and a report worth investigating.
                    // Preserve the receipt without changing its failure status.
                    let mut result = json!({"ok":false,"error":reply.error,"data":reply.data});
                    if result["data"]["executed"] == false { result["executed"] = json!(false); }
                    if tool.job.is_some() && result["data"]["state"] == "completed" {
                        result["command_completed"] = json!(true);
                    }
                    Some(result)
                } else if tool.job.is_some() {
                    if reply.data["state"] == "completed" {
                        Some(json!({"ok":reply.data["ok"],"data":reply.data,"command_completed":true,"physical_verified":false,"verification":"Device API job completion only; use actual measurements and explicit tolerances before a physical verdict"}))
                    } else {
                        tool.next_poll = Instant::now() + Duration::from_millis(250);
                        None
                    }
                } else if let Some(job) = reply.data["job_id"].as_u64().filter(|_|tool.method.starts_with("instrument.")) {
                    tool.job = Some(job);
                    tool.job_session = reply.data["job_session"].as_str().map(str::to_owned);
                    let recorded = if let Some(audit) = &tool.audit {
                        audit.journal.record_checked(&audit.task_id, audit.entry, &audit.question,
                            "awaiting_readback", &audit.workflow, Some((&tool.method, &json!({
                                "state":"accepted", "command_id":tool.command_id,
                                "command_fingerprint":tool.fingerprint, "job_id":job,
                                "job_session":tool.job_session, "device_id":reply.data["device_id"]
                            }))))
                    } else { Ok(()) };
                    match recorded {
                        Ok(()) => None,
                        Err(error) => Some(json!({"ok":false,
                            "error":format!("AGENT.JOURNAL_ACCEPTANCE_UNAVAILABLE: {error}"),
                            "data":{"job_id":job,"job_session":tool.job_session},
                            "execution_state":"unknown; job may be running; inspect before retry"})),
                    }
                } else {
                    Some(json!({"ok":true,"data":reply.data}))
                }
            } else {
                None
            }
        };
        if let Some(mut result) = result {
            if let Some(source)=chat.run.as_ref().filter(|_|!tool.automatic && !tool.local_only).and_then(|run|run.reviewed_log.as_ref()) {
                if !source.is_current() || !chat.review_outbound {chat.cancel();chat.queue_paused=true;chat.note("LOG.REVIEW_SOURCE_CHANGED: 来源已变化，结果未提交 / Capture changed; result discarded",true);return;}
                result["reviewed_log_snapshot"]=json!({"snapshot_id":source.value().snapshot_id,"source_id":source.value().source.id,"range":source.value().range,"retained_rows":source.value().retained_rows,"preceding_rows_unavailable":source.value().source.preceding_rows_unavailable});
            }
            result["host_source_binding"] = json!({
                "binding_sha256": wiparse_core::update::hex_sha256(tool.guard.binding.as_bytes()),
                "scope_sha256": wiparse_core::update::hex_sha256(tool.local_scope.as_bytes()),
                "epoch": chat.run.as_ref().map(|run|run.workflow.source_epoch()),
                "authority": "host_context_only; external source integrity not established"
            });
            if tool.automatic && result["ok"] == false {
                let _monitor_update = chat.monitor_lease.begin_update();
                chat.finish_watch_transaction(false);
                if let Some(watch) = &mut chat.watch { watch.ai_paused = true; }
            }
            if !tool.automatic && !tool.local_only && tool.method == "log.analyze" {
                if let Some(run)=&mut chat.run {run.accept_recovery_scan(&tool.params,&result);}
            }
            if ((result["ok"]==true && result["data"]["accepted"]!=true && tool.job.is_none()) || result["executed"] == false || result["command_completed"] == true)
                && validate(&tool.method,&tool.params)==Ok(true) {
                if let Some(audit) = &tool.audit {
                    let terminal = json!({"state":"completed","command_id":tool.command_id,
                        "command_fingerprint":tool.fingerprint,
                        "outcome":if result["executed"] == false {"not_dispatched"} else if result["ok"] == false {"driver_failed"} else {"driver_completed"},
                        "executed":if result["executed"] == false {Some(false)} else {None},
                        "job_id":tool.job,"job_session":tool.job_session.clone()});
                    if let Err(error) = audit.journal.record_checked(&audit.task_id,audit.entry,&audit.question,
                        if tool.automatic {"completed"} else {"running"},&audit.workflow,Some((&tool.method,&terminal))) {
                        let actual = result;
                        result = json!({"ok":false,"error":format!("AGENT.JOURNAL_COMPLETION_UNAVAILABLE: {error}"),
                            "data":{"device_result":actual},"execution_state":"inspect actual result and device before retry"});
                    }
                }
            }
            if !tool.automatic && tool.guard.controls && result["ok"] == false && result["executed"] != false {
                let fingerprint = tool.fingerprint.clone();
                if let Some(run) = &mut chat.run {
                    if !run.uncertain.contains(&fingerprint) { run.uncertain.push(fingerprint); }
                }
            }
            if tool.automatic && tool.guard.controls {
                let _monitor_update = chat.monitor_lease.begin_update();
                if let Some(watch) = &mut chat.watch {
                    watch.ai_paused = result["ok"] != true;
                    if !watch.ai_paused { watch.next = Instant::now() + watch.interval; }
                }
            }
            crate::ai::agent::cancel_host_call(&tool.guard.cancelled);
            if !tool.automatic && !tool.local_only { if let Some(run)=&mut chat.run {
                run.credit_read(&tool.method,&tool.params,&result);
                run.workflow.record_call(&format!("S{}",chat.evidence_counter+1), &tool.method, &tool.params, &result);
                if let Some(journal) = &run.journal {
                    journal.record(&run.id, "serial", &run.question, "running", &run.workflow,
                        Some((&tool.method, &json!({"result":result,"ok":result["ok"],"params":tool.params,
                            "evidence_id":format!("S{}",chat.evidence_counter+1)}))));
                }
            }}
            let serialized = result.to_string();
            let result = if serialized.len() > 8000 {crate::ai::agent::bounded_evidence(result,0)}else{result};
            let local=library::is_local(&tool.method);
            let parameters=if local{json!({"query_id":result["data"]["query_id"],"offset":tool.params["offset"]})}else{tool.params.clone()};
            let evidence_id = if tool.local_only {
                chat.preview_evidence_counter=chat.preview_evidence_counter.saturating_add(1);
                format!("P{}",chat.preview_evidence_counter)
            } else if tool.automatic {
                chat.automatic_evidence_counter=chat.automatic_evidence_counter.saturating_add(1);
                format!("W{}",chat.automatic_evidence_counter)
            } else {
                chat.evidence_counter=chat.evidence_counter.saturating_add(1);
                format!("S{}",chat.evidence_counter)
            };
            let mut result=result;if tool.method.starts_with("agent."){result["kind"]=json!("task_management");}
            let mut result=result;result["method"]=json!(tool.method);result["evidence_id"]=json!(evidence_id);
            let record=json!({"evidence_id":result["evidence_id"],"method":tool.method,"params":parameters,"result":result,"skills":wiparse_core::ai::skills::references(&chat.selected_skills)});
            if let Some(i)=chat.entries.iter().rposition(|entry|entry.role=="tool"&&(entry.text==format!("执行中 · {}",tool.method)||entry.text==format!("Running · {}",tool.method))){chat.entries.remove(i);}
            chat.entry(
                "tool",
                if local {library::readable(&result,lang)}else{format!(
                    "{}\n\n```json\n{}\n```",
                    format!(
                        "{} · {}",
                        if result["ok"] == false {
                            label(lang, "执行失败", "Failed")
                        } else {
                            label(lang, "执行完成", "Completed")
                        },
                        tool.method
                    ),
                    record
                )},
            );
            if let Some(entry) = chat.entries.last_mut() {
                entry.automatic = tool.automatic;
                entry.timeline = result["data"].get("timeline_analysis").cloned();
                entry.error = result["ok"] == false;
            }
            if !tool.automatic && !tool.local_only && !local && !chat.config.as_ref().is_some_and(wiparse_core::ai::context::native) {
                chat.history.push(AiMessage::User(bounded(
                    &format!("Verified platform result: {record}"),
                    8000,
                )));
            }
            if !tool.automatic && !tool.local_only {if let Some(run)=&mut chat.run{run.record(result.clone());}}
            if !tool.local_only {
                // Reviewed monitor continuations verify every receipt against
                // the frozen sample. Preserve the complete bounded sequence;
                // silently discarding its first receipt makes round nine fail
                // and prevents the model from seeing earlier evidence.
                let limit = if tool.automatic && chat.review_outbound { run::BASE_CALLS } else { 8 };
                let results = if tool.automatic { &mut chat.automatic_tool_results } else { &mut chat.tool_results };
                while results.len() >= limit { results.remove(0); }
                results.push(record);
            }
            if tool.local_only{
                chat.status=label(lang,"本批本地处理结束（未调用AI）","Local analysis complete (no AI request)").into();return;
            }
            if result["ok"] == false && tool.guard.controls {
                chat.monitor_lease.mutate_always(|| chat.allow_automation = false);chat.revoke_controls();
                let message=control_failure_message(&result, lang);
                if !tool.automatic {
                    if let Some(run)=&mut chat.run {run.queued.clear();run.native_pending=None;run.native_outputs.clear();run.finish("partial");}
                    chat.queue_paused=true;
                    chat.note(message,true);
                    return;
                }
                // The action may have reached the device even when its reply
                // failed. Do not let this watch issue another cycle or send a
                // continuation that could propose an automatic retry.
                chat.finish_watch_transaction(false);
                chat.pause_watch();
                chat.note_background(message,true);
                return;
            }
            // A failed earlier member must never release the remaining native
            // batch, which may contain separately confirmable device actions.
            if !tool.automatic && result["ok"] == false
                && chat.run.as_ref().is_some_and(|run| !run.queued.is_empty()) {
                if let Some(run)=&mut chat.run { run.queued.clear(); run.native_pending=None; run.native_outputs.clear(); run.finish("partial"); }
                chat.queue_paused=true;
                chat.note("批次工具失败，后续动作未执行 / Batch tool failed; later actions not executed",true);
                return;
            }
            if !tool.automatic && tool.method == "log.analyze" && result["ok"] == false {
                if let Some(run)=&mut chat.run {run.auto_log_scan=false;}
            }
            if !tool.automatic && tool.method == "log.analyze" && result["ok"] == true {
                let next=result["data"]["continuation"].as_str().map(str::to_owned);
                let bytes=result["data"]["coverage"]["bytes_read"].as_u64().unwrap_or(0);
                let was_auto=chat.run.as_ref().is_some_and(|run| run.auto_log_scan);
                let should_continue=chat.run.as_mut().is_some_and(|run| {
                    if !run.auto_log_scan { return false; }
                    let progress=bytes > run.auto_log_bytes;
                    run.auto_log_bytes=bytes;
                    match next.as_ref() {
                        Some(_) if progress => true,
                        _ => { run.auto_log_scan=false; false }
                    }
                });
                if should_continue {
                    let cursor=next.unwrap();
                    match self.begin_platform_call(ctx,chat,"log.analyze".into(),json!({"cursor":cursor}),tool.question.clone(),false) {
                        Ok(()) => return,
                        Err(error) => {
                            if let Some(run)=&mut chat.run {run.auto_log_scan=false;}
                            chat.entry("tool",format!("{}: {error}",label(lang,"自动续读停止","Automatic continuation stopped")));
                            if chat.run.as_ref().is_some_and(|run| !run.queued.is_empty()) {
                                chat.cancel(); chat.queue_paused=true;
                                chat.note("日志续读失败，后续批次未执行 / Log continuation failed; later batch actions not executed",true);
                                return;
                            }
                        }
                    }
                } else if was_auto && next.is_some() {
                    chat.entry("tool", label(lang,"自动续读未取得新的字节，已保留部分覆盖结果。","Automatic continuation made no progress; partial coverage was retained."));
                    if chat.run.as_ref().is_some_and(|run| !run.queued.is_empty()) {
                        chat.cancel(); chat.queue_paused=true;
                        chat.note("日志续读未完成，后续批次未执行 / Log continuation incomplete; later batch actions not executed",true);
                        return;
                    }
                }
            }
            if tool.automatic {
                if chat.watch.as_ref().is_none_or(|watch| watch.ai_paused) {
                    chat.finish_watch_transaction(false);
                    return;
                }
            } else if self.dispatch_queued_reads(ctx,chat,&tool.question) { return; }
            let mut data = self
                .ai_scoped_observation(if tool.automatic {
                    DataScope::Current
                } else {
                    chat.scope
                })
                .unwrap_or_else(|_| json!({"data_included":false}));
            data["platform_results"] = if tool.automatic {json!(chat.automatic_tool_results)} else {json!(chat.tool_results)};
            if let Err(error) = chat.submit(ctx, &current, data, tool.question, tool.automatic) {
                if tool.automatic { chat.finish_watch_transaction(false); chat.pause_watch(); chat.note_background(format!("{error} · AI 周期分析已暂停，本地预警继续 / AI monitoring paused; local alerts continue"), true); } else { chat.note(error, true); }
            }
        } else {
            if tool.reply.is_none() && tool.job.is_some() && Instant::now() >= tool.next_poll {
                if let Some(api) = bridge(ctx) {
                    let mut query_guard = tool.guard.clone();
                    query_guard.controls = false;
                    match enqueue(
                        &api,
                        "instrument.job.get",
                        json!({"job_id":tool.job,"job_session":tool.job_session}),
                        query_guard,
                    ) {
                        Ok(rx) => tool.reply = Some(rx),
                        Err(error) => {
                            if tool.automatic { chat.note_background(error, true); } else { chat.status = error; }
                            tool.next_poll = Instant::now() + Duration::from_millis(500);
                        }
                    }
                }
            }
            chat.tool = Some(tool);
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub(super) fn revoke_pending_control(tool: &Option<ToolPending>) {
    if let Some(tool) = tool {
        if tool.guard.controls { crate::ai::agent::cancel_host_call(&tool.guard.cancelled); }
    }
}

pub(super) fn control_pending(tool: &Option<ToolPending>) -> bool {
    tool.as_ref().is_some_and(|tool| tool.guard.controls)
}

pub(super) fn cancel(tool: &mut Option<ToolPending>) -> Option<(String, bool, String, bool, bool)> {
    if let Some(tool) = tool.take() {
        crate::ai::agent::cancel_host_call(&tool.guard.cancelled);
        let fingerprint = tool.fingerprint.clone();
        let journal_failed = tool.audit.as_ref().is_some_and(|audit|
            audit.journal.record_checked(&audit.task_id, audit.entry, &audit.question, "awaiting_readback", &audit.workflow,
                Some((&tool.method, &json!({"state":"unknown","command_id":tool.command_id,"command_fingerprint":fingerprint})))).is_err());
        return Some((tool.method, tool.guard.controls, fingerprint, tool.automatic, journal_failed));
    }
    None
}

#[cfg(test)]
mod reviewed_stop_handoff_tests {
    use super::*;
    #[test]
    fn queued_stop_revocation_prevents_claim_and_closes_durable_intent() {
        let root=tempfile::tempdir().unwrap();
        let journal=crate::ai::journal::Journal::at(root.path(),"p");
        journal.prepare_control_storage().unwrap();
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        panel.primary_device=Some("COM-test".into());
        panel.primary_stop_intent=Some(Arc::new(std::sync::Mutex::new(None)));
        let (tx,rx)=crossbeam_channel::bounded(1);
        panel.primary_reviewed_stop=Some(ReviewedStopEndpoint{session_id:uuid::Uuid::new_v4(),gate:dispatch_gate::Gate::default(),tx});
        let ctx=egui::Context::default();let config=AiConfig::default();let service=wiparse_core::ai::AiService::new(config.clone());
        ctx.data_mut(|data|data.insert_temp(egui::Id::new("wiparse-global-ai"),service.clone()));
        let mut chat=SerialAiChat::default();chat.project="p".into();chat.allow_automation=true;chat.review_outbound=true;chat.config=Some(config);
        let state=panel.ai_state("p",false);
        let identity=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||json!({"service_id":service.instance_id().to_string(),"service_revision":0})).unwrap();
        let sample=wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,||json!({})).unwrap();
        chat.monitor_review=Some((identity,sample));
        let permit=SerialControlPermit{binding:state.binding.clone(),scope_key:panel.local_log_scope_key(&chat),skills:vec![],run_id:None,created:Instant::now()};
        let id=uuid::Uuid::new_v4().to_string();let params=json!({"target_port":"COM-test"});
        let fingerprint=crate::ai::journal::command_fingerprint("serial.monitor.stop",&params);
        let workflow=wiparse_core::ai::workflow::Workflow::default();
        journal.record_checked("task","serial-watch","stop","awaiting_readback",&workflow,
            Some(("serial.monitor.stop",&json!({"state":"dispatching","command_id":id,"command_fingerprint":fingerprint})))).unwrap();
        let audit=ControlAudit{journal:journal.clone(),task_id:"task".into(),entry:"serial-watch",question:"stop".into(),workflow};
        panel.enqueue_reviewed_stop(&ctx,&mut chat,&state,&permit,&id,&fingerprint,audit).unwrap();
        chat.monitor_lease.mutate_always(||());
        let request=rx.try_recv().unwrap();let flag=AtomicBool::new(false);
        assert!(request.claim(&flag).is_err());assert!(!flag.load(Ordering::SeqCst));
        let pending=chat.reviewed_stop_pending.take().unwrap();
        assert_eq!(cancel_reviewed_stop(pending).0,dispatch_gate::Outcome::CancelledBeforeClaim);
        assert!(!journal.needs_readback("serial.monitor.stop",&params).unwrap());
    }
}
