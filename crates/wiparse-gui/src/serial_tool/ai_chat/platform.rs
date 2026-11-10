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
    job: Option<u64>,
    next_poll: Instant,
}

pub(super) fn catalog() -> &'static str {
    r#"Platform tools: request ONE action {"type":"platform.call","method":"...","params":{...}} per answer; wait for the real host result before the next step. Supported:
serial.monitor.status {} (actual CURRENT source status, read-only; not a global aggregate);
serial.monitor.start {} / serial.monitor.stop {} (control permission required; CURRENT live source only, no port/baud overrides; start waits for actual connection, stop confirms reception stopped and explicitly reports asynchronous port release). Prefer these over legacy serial actions for multi-step workflows: after the host result you may discover a scope and stop it, without guessing success;
instrument.list {} (discover connected device IDs, kinds and capabilities first);
instrument.overview {} (cached device state and measurements);
instrument.command {"device_id":ID,"command":...} (typed ControlCommand only): strings ScopeRun, ScopeStop, ScopeSingle, ScopeAutoset, ProbeStatus, BridgeInfo, DongleInfo; objects {"ScopeMeasure":{"channel":1,"meas_type":"Frequency"}}, {"ScopeChannel":{"channel":1,"enabled":true}}, {"ScopeScale":{"channel":1,"volts_per_div":1}}, {"ScopeTimebase":0.001}, {"ScopeTrigger":{"source":"CH1","level":1,"slope":"POS"}}, {"SourceVoltage":{"channel":1,"value":5}}, {"SourceCurrent":{"channel":1,"value":1}}, {"SourceOutput":{"channel":1,"enabled":false}}, {"LoadInput":false}, {"LoadMode":"CC"}, {"LoadLevel":{"mode":"CC","value":1}};
instrument.capture {"device_id":ID} (screen capture to the GUI, job completion awaited; text AI cannot inspect image pixels);
instrument.waveform_source {"device_id":ID} (capture full source, save to host-owned project evidence directory; await completion and use returned metadata/metrics);
parse.line {"text":"one Qi ASK/FSK line"}; parse.metrics {"text":"AA55 metric frame"}; convert.radix {"input":"255","source_base":10,"target_base":16};
test.status {} (current serial test state); log.brief {} (cached serial metrics); ui.calc.get {} (calculator state).
Serial start/stop and instrument control/capture require device-control permission and no active test. Serial status, device lists, cached state and analysis queries are read-only and do not require control permission. Existing connected devices only; never connect guessed addresses. Never invent power setpoints/limits or perform a destructive flash/reset; ask for unspecified settings. No raw SCPI, shell, file paths, erase or arbitrary serial bytes. Platform calls run through the existing device worker. A queue acknowledgement is not completion; unknown/timeout is not success. At most six platform calls per user task/sample; results are supplied automatically for continued reasoning. Do not repeat a failed or unknown output-changing command. Use only fields named above. The host rejects unsupported calls."#
}

fn keys(params: &Value, allowed: &[&str]) -> bool {
    params
        .as_object()
        .is_some_and(|p| p.keys().all(|k| allowed.contains(&k.as_str())))
}

pub(super) fn validate(method: &str, params: &Value) -> Result<bool, String> {
    if library::is_local(method){library::validate(method,params)?;return Ok(false);}
    let invalid = || "AI.TOOL_INVALID: unsupported platform method or parameters".to_string();
    match method {
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
            if !keys(params, &["device_id", "command"]) || params["device_id"].as_u64().is_none() {
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
        if library::is_local(&method)&&(automatic||chat.scope==DataScope::None){return Err("LOG.DATA_SCOPE: choose a manual log scope first".into());}
        if chat.tool.is_some() || chat.tool_round >= 6 {
            return Err("AI.TOOL_LIMIT: tool loop is busy or reached six calls".into());
        }
        if controls && (!chat.allow_automation || self.ai_controls_locked(self.ai_test_locked)) {
            return Err(
                "AI.CONTROL_LOCKED: platform device control is disabled or test running".into(),
            );
        }
        if method == "instrument.capture" {
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
        let reply = if library::is_local(&method){
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
        let deadline = Instant::now()
            + Duration::from_secs(if method == "serial.monitor.start" {
                15
            } else {
                90
            });
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
            job: None,
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
                } else if let Some(job) = reply.data["job_id"].as_u64() {
                    tool.job = Some(job);
                    None
                } else {
                    Some(json!({"ok":true,"data":reply.data}))
                }
            } else {
                None
            }
        };
        if let Some(result) = result {
            tool.guard.cancelled.store(true, Ordering::Release);
            let serialized = result.to_string();
            let result = if serialized.len() > 8000 {crate::ai::agent::bounded_evidence(result,0)}else{result};
            let local=library::is_local(&tool.method);
            let parameters=if local{json!({"query_id":result["data"]["query_id"],"offset":tool.params["offset"]})}else{tool.params.clone()};
            let record = json!({"method":tool.method,"params":parameters,"result":result});
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
                entry.error = result["ok"] == false;
            }
            if !tool.automatic && !local {
                chat.history.push(AiMessage::User(bounded(
                    &format!("Verified platform result: {record}"),
                    8000,
                )));
            }
            if local{chat.tool_results.retain(|record|!record["method"].as_str().is_some_and(library::is_local));}
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
                chat.allow_automation = false;
                chat.note("设备调用失败或状态未知，已关闭设备控制；AI将解释实际结果 / Device call failed or state unknown; controls disabled; AI will explain the result",true);
            }
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
                        json!({"job_id":tool.job}),
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
