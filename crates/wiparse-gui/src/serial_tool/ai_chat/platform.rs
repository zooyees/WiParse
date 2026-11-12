//! Bounded platform calls using the existing GUI API and durable instrument jobs.
use super::*;
use crate::backend::{AiGuard, ApiBridge, InvokeReply, PendingRequest};
use serde_json::Value;

pub(super) struct ToolPending {
    reply: Option<Receiver<InvokeReply>>,
    guard: AiGuard,
    method: String,
    params: Value,
    question: String,
    pub(super) automatic: bool,
    pub(super) local_only: bool,
    local_scope: String,
    deadline: Instant,
    command_id:String,
    job: Option<u64>,
    job_session: Option<String>,
    next_poll: Instant,
}

pub(super) fn catalog() -> &'static str {
    r#"Platform tools: JSON compatibility mode requests ONE action {"type":"platform.call","method":"...","params":{...}} per answer; wait for the real host result before the next step. Supported:
serial.view {}: inspect current serial log view ID, absolute 1-based row range and search status. serial.search {view_id:actualID,query:string}: apply the existing log search UI and show matching rows; result search_pending means background search has not finished, inspect serial.view later. serial.jump {view_id:actualID,line:positiveInteger}: highlight and scroll current log to original 1-based file/absolute live row, pausing tail follow without stopping capture. These reversible UI actions need no device-control grant, only a manual current-log scope. Never use selection-relative rows or another file's lines. First inspect view identity; use log.analyze for full evidence searches and then jump to a verified row. No serial transmit, edits or capture stop.
To extract and display serial data: inspect log schema, call log.analyze timeline with retain_dataset:true, finish its cursor, then data.present {query_id:returnedQueryID}; the host imports actual local points into the data analysis page. Do not pass model-generated arrays.
Discover the complete published platform API with platform.catalog {filter,offset,limit}; then invoke an exact listed method using platform.invoke {method,params}. Catalogue params are examples, never defaults. Device/file/test mutations require concrete confirmation. Never guess targets or credentials. If service_required is true, report unavailable services honestly.
Canonical shared host tools also available: wave.channels/measure/timing/frames/decode/view, data.inspect/statistics/present, calc.inspect/evaluate/apply, report.inspect/read/search/create/open, testing.inspect/status/cases/propose/wait. Use agent.help {method:...} for the exact canonical schema. Reversible view/calculator/decoder operations require actual user intent; never change a protocol by guessing. testing.wait waits locally on an actual run_id; do not consume model calls polling. Testing execution/apply remains in the global Agent with its concrete action confirmation. For requested report files, report.create {title,markdown,evidence_ids:[actualIDs]} saves portable Markdown/JSON and real-series PNG/CSV; call report.open {report_id:returnedUUID} to show it in WiParse. Never invent evidence IDs or local file paths.
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
instrument.waveform_source {"device_id":ID} (capture full source, save to host-owned project evidence directory; await completion and use returned metadata/metrics);
parse.line {"text":"one Qi ASK/FSK line"}; parse.metrics {"text":"AA55 metric frame"}; convert.radix {"input":"255","source_base":10,"target_base":16};
test.status {} (current serial test state); log.brief {} (cached serial metrics); ui.calc.get {} (calculator state).
Serial start/stop and instrument control/capture require device-control permission and no active test. Serial status, device lists, cached state and analysis queries are read-only and do not require control permission. Existing connected devices only; never connect guessed addresses. Never invent power setpoints/limits or perform a destructive flash/reset; ask for unspecified settings. No raw SCPI, shell, file paths, erase or arbitrary serial bytes. Platform calls run through the existing device worker. A queue acknowledgement is not completion; unknown/timeout is not success. At most thirty-two platform calls per user task (monitor samples remain bounded); results are supplied automatically for continued reasoning. Do not repeat a failed or unknown output-changing command. Use only fields named above. The host rejects unsupported calls."#
}

fn keys(params: &Value, allowed: &[&str]) -> bool {
    params
        .as_object()
        .is_some_and(|p| p.keys().all(|k| allowed.contains(&k.as_str())))
}

pub(super) fn validate(method: &str, params: &Value) -> Result<bool, String> {
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
        "agent.plan"|"agent.evidence"|"agent.help"|"agent.align"|"agent.recipe"=>{wiparse_core::ai::workflow::validate_management(method,params)?;Ok(false)},
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
    pub(crate) fn ai_platform_result_guard_valid(&self, guard: &AiGuard) -> bool {
        let mut read_guard = guard.clone(); read_guard.controls = false;
        self.ai_platform_guard_valid(&read_guard) && (!guard.controls || self.ai_chat.allow_automation)
    }
    pub(crate) fn ai_platform_guard_for(&self, guard: &AiGuard, method: &str, params: &Value) -> bool {
        let mut read_guard = guard.clone(); read_guard.controls = false;
        self.ai_platform_guard_valid(&read_guard) && (!guard.controls || self.ai_chat.allow_automation &&
            (!self.ai_controls_locked(self.ai_test_locked) || wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call {method:method.into(),params:params.clone()})))
    }
    /// Acknowledge only the host-dispatched source transition, never a user switch.
    pub(crate) fn ai_accept_source_transition(&mut self, guard: &mut AiGuard) {
        let state = self.ai_state(&guard.project,self.ai_test_locked);
        guard.binding = serde_json::to_string(&state.binding).unwrap_or_default();
        let scope = self.local_log_scope_key(&self.ai_chat);
        self.ai_chat.conversation_source = Some((state.binding.source_id,state.binding.device));
        if let Some(tool) = &mut self.ai_chat.tool { tool.guard.binding = guard.binding.clone(); tool.local_scope = scope.clone(); }
        if let Some(run) = &mut self.ai_chat.run {
            run.scope_key = scope; run.last_read = None; run.initial_evidence = None;
            run.feedback = Some("HOST_SOURCE_TRANSITION: source changed by your approved call. Read the new source; prior evidence belongs to the previous source, never reuse its row coordinates.".into());
        }
        self.ai_chat.watch = None;
    }
    pub(crate) fn ai_platform_guard_valid(&self, guard: &AiGuard) -> bool {
        !guard.cancelled.load(Ordering::Acquire)
            && serde_json::to_string(&self.ai_state(&guard.project, self.ai_test_locked).binding)
                .ok()
                .as_deref()
                == Some(guard.binding.as_str())
            && (!guard.controls
                || (self.ai_chat.allow_automation
                    && !self.ai_controls_locked(self.ai_test_locked)))
    }

    pub(super) fn local_log_scope_key(&self,chat:&SerialAiChat)->String {
        use std::hash::{Hash,Hasher};
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        if chat.scope==DataScope::Selection{self.tabs.get(self.active_tab).map(|tab|tab.ai_selection()).hash(&mut hash);}
        format!("{:?}:{}:{}:{}:{}",chat.scope,chat.range_first,chat.range_last,self.tabs.get(self.active_tab).map_or(0,LogTabPage::ai_view_id),hash.finish())
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
        let controls = validate(&method, &params)?;
        let command_id=uuid::Uuid::new_v4().to_string();
        let fingerprint=crate::ai::journal::command_fingerprint(&method,&params);
        let previous_unknown=if controls {
            chat.run.as_ref().and_then(|r|r.journal.as_ref()).map(|j|j.needs_readback(&method,&params)).transpose()?.unwrap_or(false)
        } else {false};
        if controls && (previous_unknown || chat.run.as_ref().is_some_and(|r|r.uncertain.contains(&fingerprint)))
            && !wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call{method:method.clone(),params:params.clone()}) {
            if let Some(run)=chat.run.as_mut(){
                run.pending_readback=Some((fingerprint.clone(),format!("{method} · {}",params)));
            }
            return Err("AI.OUTCOME_UNKNOWN: inspect the instrument before repeating this command".into());
        }
        if library::is_local(&method)&&(automatic||chat.scope==DataScope::None){return Err("LOG.DATA_SCOPE: choose a manual log scope first".into());}
        if chat.tool.is_some() || chat.tool_round >= run::MAX_CALLS {
            return Err("AI.TOOL_LIMIT: tool loop is busy or reached the bounded task call budget".into());
        }
        if controls && (!chat.allow_automation || chat.run.as_ref().is_some_and(|r|!r.control_cap) || self.ai_controls_locked(self.ai_test_locked) && !wiparse_core::platform_api::interrupt_call(&wiparse_core::engineering_agent::Call{method:method.clone(),params:params.clone()})) {
            return Err(
                "AI.CONTROL_LOCKED: platform device control is disabled or test running".into(),
            );
        }
        if !automatic&&library::is_local(&method)&&params.get("cursor").is_none(){
            let signature=format!("{}:{}:{}",method,params,self.ai_state(&chat.project,self.ai_test_locked).absolute_lines);
            if let Some(run)=&mut chat.run{if run.last_read.as_ref()==Some(&signature){return Err("AI.NO_PROGRESS: identical read already completed; use prior evidence or a narrower query".into());}run.last_read=Some(signature);}
        }
        if controls && !automatic {
            if let Some(run)=&chat.run {if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"awaiting_readback",&run.workflow,Some((&method,&json!({"state":"dispatching","command_id":command_id,"command_fingerprint":fingerprint,"device_id":params.get("device_id")}))));}}
        }
        let canonical_instrument=crate::ai::broker::has_instrument_executor(ctx,&method);
        if params.get("verify").is_some() && method=="instrument.command" && !canonical_instrument{return Err("DEVICE.VERIFICATION_HOST_REQUIRED".into());}
        if method == "instrument.capture" && !canonical_instrument {
            params["wait"] = json!(true);
        }
        if method == "instrument.waveform_source" {
            if !chat
                .project
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err("AI.PROJECT_INVALID".into());
            }
            let root: PathBuf = ctx
                .data(|d| d.get_temp(egui::Id::new("wiparse-ai-project-root")))
                .ok_or("AI.PROJECT_UNAVAILABLE")?;
            params["dir"] = json!(root
                .join("projects")
                .join(&chat.project)
                .join("ai-evidence"));
            params["filename"] = json!(format!(
                "ai-wave-{}.isf",
                Local::now().format("%Y%m%d-%H%M%S-%f")
            ));
            params["overwrite"] = json!(false);
        }
        if method == "serial.monitor.start" || method == "serial.monitor.stop" {
            self.apply_ai_action(
                chat,
                if method.ends_with("start") {
                    Action::SerialStart
                } else {
                    Action::SerialStop
                },
                &question,
                self.last_lang,
                self.ai_test_locked,
            )?;
            // A successful start may open a new live tab (e.g. an additional port).
            // Bind only this host-owned transition; later user changes still cancel.
            let own = self.ai_state(&chat.project, self.ai_test_locked);
            chat.conversation_source = Some((own.binding.source_id, own.binding.device));
        }
        let guard = AiGuard {
            binding: serde_json::to_string(
                &self.ai_state(&chat.project, self.ai_test_locked).binding,
            )
            .map_err(|e| e.to_string())?,
            project: chat.project.clone(),
            controls,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let reply = if matches!(method.as_str(),"serial.view"|"serial.search"|"serial.jump") {
            if automatic{return Err("SERIAL.NAVIGATION_MANUAL_TASK_ONLY".into());}
            if chat.scope!=DataScope::Current{return Err("SERIAL.NAVIGATION_REQUIRES_CURRENT_SCOPE: current-log navigation cannot map selection-relative or other-file line numbers".into());}
            let tab=self.tabs.get_mut(self.active_tab).ok_or("SERIAL.NO_CURRENT_TAB")?;
            let value=tab.ai_navigate(&method,&params)?;
            let(tx,rx)=crossbeam_channel::bounded(1);let _=tx.send(crate::backend::invoke_ok(&method,value));ctx.request_repaint();Some(rx)
        } else if method.starts_with("agent.") {
            let run=chat.run.as_mut().ok_or("AGENT.NO_ACTIVE_TASK")?;
            let result=match method.as_str(){
                "agent.recipe"=>run.workflow.run_recipe(&params),
                "agent.align"=>run.workflow.align(&params),
                "agent.plan"=>run.workflow.update(params.clone()),
                "agent.evidence"=>run.workflow.page(&params),
                "agent.help"=>{
                    let target=params["method"].as_str().unwrap();
                    let allowed=run::functions(chat.scope,chat.allow_automation&&!self.ai_test_locked&&run.control_cap).into_iter().find(|f|f.name=="wiparse_call").unwrap();
                    if let Some(recipe)=wiparse_core::ai::workflow::recipe(target){Ok(recipe)}else if !allowed.parameters["properties"]["method"]["enum"].as_array().unwrap().iter().any(|v|v==target){Err("AGENT.UNKNOWN_OR_UNAUTHORIZED_TOOL".into())}else{
                        let help=if let Some(tool)=wiparse_core::engineering_agent::tool(target){tool.schema.to_owned()}else if target.starts_with("log."){library::catalog().to_owned()}else if target=="engineering.calc.evaluate"{wiparse_core::engineering_agent::CALC_SCHEMA.to_owned()}else{catalog().to_owned()};
                        Ok(json!({"ok":true,"method":target,"schema":help,"validated_keys":wiparse_core::engineering_agent::tool(target).and_then(|t|wiparse_core::engineering_agent::declared_keys(t.method))}))
                    }
                },_=>Err("AGENT.UNKNOWN_MANAGEMENT_TOOL".into())
            };
            let mut value=result.unwrap_or_else(|e|json!({"ok":false,"error":e}));value["kind"]=json!("task_management");
            let (tx,rx)=crossbeam_channel::bounded(1);let _=tx.send(InvokeReply{ok:value["ok"]!=false,cmd:method.clone(),error:value["error"].as_str().map(str::to_owned),data:value});Some(rx)
        }else if library::is_local(&method){
            let key=self.local_log_scope_key(chat);
            chat.library.bind_scope(key);
            let mut files=Vec::new();let mut memory=None;
            if chat.scope==DataScope::Library {
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
            let range=if chat.scope==DataScope::Range{Some((chat.range_first.max(1),chat.range_last.max(chat.range_first.max(1))))}else{None};
            Some(chat.library.start_scoped(&method,params.clone(),files,memory,range,chat.scope==DataScope::Library,guard.cancelled.clone(),ctx.clone())?)
        } else if method=="report.create" {
            let run=chat.run.as_ref().ok_or("REPORT.NO_ACTIVE_TASK")?;
            run.workflow.validate_conclusion(&format!("{}\n{}",params["title"],params["markdown"]))?;
            let evidence=params["evidence_ids"].as_array().ok_or("REPORT.MISSING_EVIDENCE")?.iter().map(|v|crate::ai::journal::report_input(run.journal.as_ref(),&run.workflow,v.as_str().unwrap_or(""))).collect::<Result<Vec<_>,_>>()?;
            let root=ctx.data(|d|d.get_temp::<PathBuf>(egui::Id::new("wiparse-ai-project-root"))).ok_or("REPORT.PROJECT_UNAVAILABLE")?;
            Some(crate::ai::artifact::start(root,chat.project.clone(),params.clone(),evidence,guard.cancelled.clone(),ctx.clone(),|value|InvokeReply{ok:value["ok"]==true,cmd:"report.create".into(),error:value["error"].as_str().map(str::to_owned),data:value["data"].clone()})?)
        } else if canonical_instrument || crate::ai::broker::supports(&method) {
            Some(crate::ai::broker::enqueue(ctx,&method,params.clone(),guard.clone())?)
        } else if method.starts_with("serial.monitor.") {
            None
        } else {
            let api = bridge(ctx).ok_or("AI.PLATFORM_UNAVAILABLE")?;
            Some(enqueue(&api, &method, params.clone(), guard.clone())?)
        };
        chat.tool_round += 1;
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
            guard,
            method,
            params,
            question,
            automatic,
            local_only:false,
            local_scope:self.local_log_scope_key(chat),
            deadline,
            command_id,job: None, job_session:None,
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
            != Some(tool.guard.binding.as_str()) || (library::is_local(&tool.method)&&self.local_log_scope_key(chat)!=tool.local_scope);
        let result = if changed || tool.guard.cancelled.load(Ordering::Acquire) {
            Some(
                json!({"ok":false,"error":"AI.CANCELLED_OR_STALE","unknown_if_already_dispatched":true}),
            )
        } else if Instant::now() >= tool.deadline {
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
                    Some(json!({"ok":false,"error":reply.error}))
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
                    if let Some(run)=&chat.run {if let Some(j)=&run.journal {
                        j.record(&run.id,"serial",&run.question,"awaiting_readback",&run.workflow,Some((&tool.method,&json!({"state":"accepted","command_id":tool.command_id,"command_fingerprint":crate::ai::journal::command_fingerprint(&tool.method,&tool.params),"job_id":job,"job_session":tool.job_session,"device_id":reply.data["device_id"]}))));
                    }}
                    None
                } else {
                    Some(json!({"ok":true,"data":reply.data}))
                }
            } else {
                None
            }
        };
        if let Some(result) = result {
            if result["ok"]==true && result["data"]["accepted"]!=true && tool.job.is_none() && validate(&tool.method,&tool.params)==Ok(true) {
                if let Some(run)=&chat.run{if let Some(j)=&run.journal{
                    j.record(&run.id,"serial",&run.question,"running",&run.workflow,Some((&tool.method,&json!({"state":"completed","command_id":tool.command_id}))));
                }}
            }
            tool.guard.cancelled.store(true, Ordering::Release);
            if !tool.automatic { if let Some(run)=&mut chat.run {
                run.workflow.record_call(&format!("S{}",chat.evidence_counter+1), &tool.method, &tool.params, &result);
            }}
            let serialized = result.to_string();
            let result = if serialized.len() > 8000 {crate::ai::agent::bounded_evidence(result,0)}else{result};
            let local=library::is_local(&tool.method);
            let parameters=if local{json!({"query_id":result["data"]["query_id"],"offset":tool.params["offset"]})}else{tool.params.clone()};
            chat.evidence_counter=chat.evidence_counter.saturating_add(1);
            let mut result=result;if tool.method.starts_with("agent."){result["kind"]=json!("task_management");}
            let mut result=result;result["method"]=json!(tool.method);result["evidence_id"]=json!(format!("S{}",chat.evidence_counter));
            let record=json!({"evidence_id":result["evidence_id"],"method":tool.method,"params":parameters,"result":result});
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
            if !tool.automatic && !local && !chat.config.as_ref().is_some_and(wiparse_core::ai::context::native) {
                chat.history.push(AiMessage::User(bounded(
                    &format!("Verified platform result: {record}"),
                    8000,
                )));
            }
            if !tool.automatic{if let Some(run)=&mut chat.run{run.workflow.record(result["evidence_id"].as_str().unwrap_or(""),&tool.method,&result);run.record(result.clone());}}
            while chat.tool_results.len()>=8{chat.tool_results.remove(0);}
            chat.tool_results.push(record);
            if tool.local_only{
                chat.status=label(lang,"本批本地处理结束（未调用AI）","Local analysis complete (no AI request)").into();chat.status_error=result["ok"]==false;return;
            }
            if changed {
                chat.status = label(
                    lang,
                    "工具调用失败或状态未知，请查看执行结果",
                    "Tool failed or state unknown; inspect the result",
                )
                .into();
                chat.status_error = true;
                return;
            }
            if result["ok"] == false && tool.guard.controls {
                chat.allow_automation = false;chat.revoke_controls();
                chat.note("设备调用失败或状态未知，已关闭设备控制；AI将解释实际结果 / Device call failed or state unknown; controls disabled; AI will explain the result",true);
            }
            if self.dispatch_queued_reads(ctx,chat,&tool.question){return;}
            let mut data = self
                .ai_scoped_observation(if tool.automatic {
                    DataScope::Current
                } else {
                    chat.scope
                })
                .unwrap_or_else(|_| json!({"data_included":false}));
            data["platform_results"] = json!(chat.tool_results);
            if let Err(error) = chat.submit(ctx, &current, data, tool.question, tool.automatic) {
                chat.note(error, true);
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
                            chat.status = error;
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

pub(super) fn cancel(tool: &mut Option<ToolPending>) -> bool {
    if let Some(tool) = tool.take() {
        tool.guard.cancelled.store(true, Ordering::Release);
        return tool.guard.controls;
    }
    false
}
