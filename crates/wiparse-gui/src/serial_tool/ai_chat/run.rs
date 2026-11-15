//! One request owns read authorization, evidence and a bounded execution loop.
use super::*;
use wiparse_core::ai::{tools, AiReply};
pub(super) const BASE_CALLS: usize = 32;
const MAX_READ_CREDITS:usize=64;
fn owned_test_control_followup(question:&str)->bool {
    let text=question.trim().to_lowercase();
    if ["新任务","另一个任务","new task","start over"].iter().any(|prefix|text.starts_with(prefix)){return false;}
    let target=["测试","运行","test","run"].iter().any(|word|text.contains(word));
    let control=["停止","暂停","继续","stop","pause","resume"].iter().any(|word|text.contains(word));
    target && control
}
#[cfg(test)]
mod owned_test_followup_tests {
    use super::owned_test_control_followup;
    #[test]
    fn recognizes_explicit_run_control_without_capturing_new_tasks() {
        for text in ["暂停测试", "继续当前运行", "stop the test", "resume run"] {
            assert!(owned_test_control_followup(text), "{text}");
        }
        for text in ["新任务：停止测试", "new task: stop test", "停止串口", "分析测试结果"] {
            assert!(!owned_test_control_followup(text), "{text}");
        }
    }
}
#[derive(Clone)]
pub(super) struct ReviewedLog {
    pub source: wiparse_core::log_analysis::Source,
    pub range: Option<(u64,u64)>,
    pub snapshot_id: String,
    pub retained_rows: usize,
}
pub(super) struct Run {
    pub reviewed_log: Option<wiparse_core::ai::ReviewedSource<ReviewedLog>>,
    pub id: String,
    credited_reads:std::collections::HashSet<u64>,
    consecutive_rejections:usize,
    pub summary_only:bool,
    pub question:String,
    pub uncertain:Vec<String>,
    pub pending_readback:Option<(String,String)>,
    pub correction:Option<String>,
    pub journal:Option<crate::ai::journal::Journal>,
    pub started: Instant,
    pub phase: &'static str,
    pub control_cap: bool,
    pub test_control_cap: bool,
    pub repairs:usize,
    pub first_evidence_ms:Option<u64>,
    pub recovery_policy:crate::ai::execution::ProtocolRecovery,
    pub evidence_sequence: u64,
    pub native_pending: Option<tools::Assistant>,
    pub native_outputs: Vec<serde_json::Value>,
    pub queued: std::collections::VecDeque<Action>,
    pub workflow: wiparse_core::ai::workflow::Workflow,
    pub usage: wiparse_core::ai::usage::Usage,
    pub native_base: Option<(String, Vec<AiMessage>)>,
    pub native_rounds: Vec<tools::Round>,
    pub feedback: Option<String>,
    pub scope_key: String,
    pub source_binding: Option<Binding>,
    pub last_read: Option<String>,
    pub initial_evidence: Option<serde_json::Value>,
    pub evidence_ids: Vec<String>,
    pub previous_evidence: Vec<serde_json::Value>,
    pub auto_log_scan: bool,
    pub auto_log_bytes: u64,
    pub recovery_needs_fresh_log: bool,
    pub recovery_scan_started: bool,
    pub recovery_query_id: Option<String>,
}
impl Run {
    pub(super) fn accept_recovery_scan(&mut self, params:&serde_json::Value, result:&serde_json::Value) {
        if !self.recovery_needs_fresh_log || !self.recovery_scan_started { return; }
        if self.recovery_query_id.is_none() && params.get("cursor").is_none() {
            self.recovery_query_id=result["data"]["query_id"].as_str()
                .filter(|id|uuid::Uuid::parse_str(id).is_ok()).map(str::to_owned);
            if self.recovery_query_id.is_none() {self.recovery_scan_started=false;return;}
        }
        if self.recovery_query_id.as_deref().is_some_and(|id|result["data"]["query_id"].as_str()==Some(id)
            && params.get("cursor").is_none_or(|cursor|cursor.as_str()==Some(id)))
            && wiparse_core::ai::workflow::fresh_log_scan_complete(result) {
            self.recovery_needs_fresh_log=false;
            self.recovery_scan_started=false;
            self.workflow.fresh_log_source();
        }
    }
    pub(super) fn compatible_history(&self, scope: &str, binding: &Binding, skills: &[String], control_cap: bool) -> bool {
        self.scope_key == scope && self.source_binding.as_ref() == Some(binding) && self.workflow.skill_ids == skills && self.control_cap == control_cap
    }
    pub fn new() -> Self {
        Self {
            reviewed_log:None, question:String::new(),uncertain:vec![],pending_readback:None,correction:None,journal:None,
            credited_reads:Default::default(),consecutive_rejections:0,summary_only:false,
            id: uuid::Uuid::new_v4().to_string(),
            started: Instant::now(),
            phase: "reading",
            control_cap: true,
            test_control_cap: true,
            repairs:0,first_evidence_ms:None,recovery_policy:Default::default(),
            evidence_sequence: 0,
            native_pending: None,
            native_outputs: vec![],
            queued: Default::default(),
            workflow: Default::default(),
            usage: Default::default(),
            native_base: None,
            native_rounds: vec![],
            feedback: None,
            scope_key: String::new(),
            source_binding: None,
            last_read: None,
            initial_evidence: None,
            evidence_ids: vec![],
            previous_evidence: vec![],
            auto_log_scan: false,
            auto_log_bytes: 0,
            recovery_needs_fresh_log: false,
            recovery_scan_started: false,
            recovery_query_id: None,
        }
    }
    pub fn call_limit(&self)->usize{BASE_CALLS+self.credited_reads.len()}
    pub fn limit_reason(&self,calls:usize)->Option<&'static str>{
        if self.started.elapsed()>=Duration::from_secs(600){Some("本次任务已运行 10 分钟")}
        else if self.consecutive_rejections>=3 {Some("连续三个请求未能派发")}
        else if calls>=self.call_limit(){Some("本次工具调用额度已用完")}
        else{None}
    }
    pub fn credit_read(&mut self,method:&str,params:&serde_json::Value,result:&serde_json::Value){
        use std::hash::{Hash,Hasher};
        if self.credited_reads.len()>=MAX_READ_CREDITS || result["ok"]!=true{return;}
        let data=&result["data"];
        let boundary=match method{
            "log.analyze"|"report.search"=>data["coverage"]["bytes_read"].as_u64(),
            "report.read"=>data["coverage"]["rows_scanned"].as_u64().or_else(||data["lines"].as_array().map(|v|v.len() as u64)),
            _=>None,
        };
        let Some(position)=boundary.filter(|p|*p>0) else{return;};
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        method.hash(&mut hash);params.to_string().hash(&mut hash);position.hash(&mut hash);
        self.credited_reads.insert(hash.finish());
    }
    pub fn finish(&mut self,phase:&'static str){
        if matches!(self.phase,"completed"|"needs_input"|"failed"|"partial"|"cancelled"){return}
        self.phase=phase;
        if let Some(j)=&self.journal{j.record(&self.id,"serial",&self.question,phase,&self.workflow,None);}
        tracing::info!(target:"wiparse_ai_outcome",task_id=%self.id,outcome=phase,elapsed_ms=self.started.elapsed().as_millis() as u64,first_evidence_ms=self.first_evidence_ms,repairs=self.repairs,usage=%json!(self.usage), evidence_count=self.evidence_sequence);
    }
    pub fn record(&mut self, result: serde_json::Value) {
        let _=self.record_checked(result);
    }
    pub(super) fn record_checked(&mut self, result: serde_json::Value) -> bool {
        let persisted=self.journal.as_ref().is_some_and(|j|j.record_checked(&self.id,"serial",&self.question,"running",&self.workflow,Some((result["method"].as_str().unwrap_or("serial observation"),&result))).is_ok());
        if !persisted && self.journal.is_some() { tracing::warn!("could not persist agent task"); }
        if result["executed"]==false{self.consecutive_rejections+=1;}
        else if result["ok"]==true{self.consecutive_rejections=0;}
        if result["ok"] == true {
            self.recovery_policy.made_progress();
            if self.first_evidence_ms.is_none()&&result["kind"]!="task_management"{self.first_evidence_ms=Some(self.started.elapsed().as_millis() as u64);}
        }
        if let Some(id) = result["evidence_id"]
            .as_str()
            .filter(|_| result["ok"]==true && result["kind"] != "task_management")
        {
            self.evidence_ids.push(id.into());
        }
        if self.initial_evidence.is_none() && result["kind"] != "task_management" {
            self.initial_evidence = Some(result.clone());
        }
        if self.native_pending.is_some() {
            self.native_outputs.push(result.clone());
            if self
                .native_pending
                .as_ref()
                .is_some_and(|a| a.calls.len() == self.native_outputs.len())
            {
                let assistant = self.native_pending.take().unwrap();
                self.native_rounds.push(tools::Round {
                    assistant,
                    outputs: std::mem::take(&mut self.native_outputs),
                });
            }
        }
        while self.native_rounds.len() > 6
            || serde_json::to_vec(&self.native_rounds).map_or(usize::MAX, |v| v.len()) > 96 * 1024
        {
            if self.native_rounds.len() <= 1 {
                break;
            }
            self.native_rounds.remove(0);
        }
        if result["kind"] != "task_management" {
            self.evidence_sequence += 1;
        }
        self.phase = "analysing";
        persisted
    }
}
pub(super) fn functions(scope: DataScope, controls: bool) -> Vec<tools::Function> {
    functions_for_skills(scope, controls, &[])
}
pub(super) fn functions_for_skills(scope: DataScope, controls: bool, skills: &[String]) -> Vec<tools::Function> {
    functions_for_skills_with_test(scope,controls,skills,false)
}
pub(super) fn functions_for_skills_with_test(scope: DataScope, controls: bool, skills: &[String], test_controls:bool) -> Vec<tools::Function> {
    let mut methods = vec![
        "agent.plan",
        "agent.evidence",
        "agent.help",
        "agent.recipe",
        "engineering.inspect",
        "engineering.wave.measure",
        "engineering.wave.frames",
        "engineering.calc.evaluate",
        "instrument.list",
        "instrument.overview",
        "serial.monitor.status",
        "serial.view",
        "serial.search",
        "serial.jump",
        "test.status",
        "log.brief",
        "ui.calc.get",
        "parse.line",
        "parse.metrics",
        "convert.radix",
        "monitor.configure",
        "monitor.start",
        "monitor.stop",
    ];
    if scope != DataScope::None {
        methods.extend(["log.files", "log.analyze","agent.align"]);
    }
    if controls {
        methods.extend([
            "serial.start",
            "serial.stop",
            "serial.monitor.start",
            "serial.monitor.stop",
            "instrument.command",
            "instrument.capture",
            "instrument.waveform_source",
        ]);
    }
    methods.extend(wiparse_core::engineering_agent::TOOLS.iter().filter(|t|crate::ai::broker::supports(t.method)&& (controls || test_controls && matches!(t.method,"testing.stop"|"testing.pause"|"testing.resume") || !matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test))).map(|t|t.method));
    methods.retain(|method| wiparse_core::ai::skills::permits(skills, method));
    tools::host_functions(methods)
}
pub(super) fn decode(reply: &AiReply) -> protocol::Response {
    let Some(message) = &reply.tool_message else {
        return protocol::parse(&reply.text);
    };
    let invalid = || protocol::Response {
        reply: String::new(),
        actions: vec![],
        error: Some("AI.TOOL_PROTOCOL: one listed call with valid arguments is required".into()),
        finalized: false, needs_input:false,
    };
    if message.calls.len() > 1 {
        let mut actions = vec![];
        for call in &message.calls {
            let mut one = reply.clone();
            one.tool_message = Some(tools::Assistant {
                content: None,
                reasoning_content: None,
                calls: vec![call.clone()],
            });
            let response = decode(&one);
            if response.error.is_some() || response.actions.len() != 1 {
                return invalid();
            }
            let action = response.actions.into_iter().next().unwrap();
            if !matches!(&action,Action::Platform{method,params} if !method.starts_with("agent.")&&method!="platform.invoke"&&!matches!(method.as_str(),"serial.search"|"serial.jump")&&method!="report.create"&&wiparse_core::engineering_agent::tool(method).is_none_or(|t|matches!(t.effect,wiparse_core::engineering_agent::Effect::Read|wiparse_core::engineering_agent::Effect::LocalCompute))&&platform::validate(method,params)==Ok(false))
            {
                return invalid();
            }
            actions.push(action);
        }
        return protocol::Response {
            reply: String::new(),
            actions,
            error: None,
            finalized: false, needs_input:false,
        };
    }
    if message.calls.is_empty() {
        return invalid();
    }
    let mut call = message.calls[0].clone();
    if matches!(
        call.name.as_str(),
        "wiparse_log_files"
            | "wiparse_log_analyze"
            | "wiparse_log_events"
            | "wiparse_log_timeline"
            | "wiparse_plan"
            | "wiparse_evidence"
            | "wiparse_help"
            | "wiparse_report_read"
    ) {
        let Ok((method, params)) = tools::normalized(&call) else {
            return invalid();
        };
        call.name = "wiparse_call".into();
        call.arguments = json!({"method":method,"params":params}).to_string();
    }
    let call = &call;
    let Ok(params) = serde_json::from_str::<serde_json::Value>(&call.arguments) else {
        return invalid();
    };
    match call.name.as_str() {
        "wiparse_answer" => {
            if !params.as_object().is_some_and(|o| o.len() == 3)
                || !params["evidence_ids"].as_array().is_some_and(|ids| {
                    ids.len() <= 32 && ids.iter().all(|id| id.as_str().is_some())
                })
                || !matches!(params["status"].as_str(), Some("completed" | "needs_input"))
                || !params["reply"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.len() <= 1024 * 1024)
            {
                return invalid();
            }
            protocol::Response {
                reply: params["reply"].as_str().unwrap().into(),
                actions: vec![],
                error: None,
                finalized: true, needs_input: params["status"]=="needs_input",
            }
        }
        "wiparse_call" => {
            if !params.as_object().is_some_and(|o| o.len() == 2) || !params["params"].is_object() {
                return invalid();
            }
            let Some(method) = params["method"].as_str() else {
                return invalid();
            };
            let method = match method {
                "serial.start" => "serial.monitor.start",
                "serial.stop" => "serial.monitor.stop",
                m => m,
            };
            let action = if method.starts_with("monitor.") {
                let mut a = params["params"].clone();
                a["type"] = json!(method);
                a
            } else {
                json!({"type":"platform.call","method":method,"params":params["params"]})
            };
            protocol::parse(&json!({"reply":"","actions":[action],"status":"working"}).to_string())
        }
        _ => invalid(),
    }
}
impl SerialToolPanel {
    pub(super) fn dispatch_queued_reads(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        question: &str,
    ) -> bool {
        loop {
            let next = chat.run.as_mut().and_then(|r| r.queued.pop_front());
            let Some(next) = next else { return false; };
            if chat.run.as_ref().is_some_and(|run|run.reviewed_log.is_some())
                && !matches!(&next,Action::Platform{method,..} if matches!(method.as_str(),"log.files"|"log.analyze"|"agent.plan"|"agent.evidence"|"agent.help")) {
                chat.cancel();chat.queue_paused=true;chat.note("LOG.REVIEW_TOOL_NOT_ALLOWED: 审阅日志任务仅支持只读日志与任务管理工具",true);return true;
            }
            let (method, params) = match next {
                Action::Platform { method, params } => (method, params),
                Action::SerialStart => ("serial.monitor.start".into(), json!({})),
                Action::SerialStop => ("serial.monitor.stop".into(), json!({})),
                action @ (Action::MonitorStart { .. } | Action::MonitorConfigure { .. } | Action::MonitorStop) => {
                    let method = match &action { Action::MonitorStart { .. } => "monitor.start", Action::MonitorConfigure { .. } => "monitor.configure", _ => "monitor.stop" };
                    if matches!(&action, Action::MonitorStart { .. } | Action::MonitorConfigure { .. } | Action::MonitorStop)
                        && wiparse_core::ai::skills::permits(&chat.selected_skills, method) {
                        let state = self.ai_state(&chat.project, self.ai_test_locked);
                        let blocked = chat.run.as_ref().and_then(|run| run.limit_reason(chat.tool_round))
                            .or_else(|| (matches!(&action, Action::MonitorStart { .. })
                                && (!chat.ready() || !state.binding.live || !state.reception_requested))
                                .then_some("请先启动实时串口接收并配置 AI 服务 / Start live serial reception and configure AI first"));
                        if let Some(reason) = blocked {
                            if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":method,"error":reason})); }
                            chat.cancel(); chat.queue_paused = true; chat.note(reason, true);
                            return true;
                        }
                        chat.control_approval = Some(ControlApproval { action, binding: state.binding,
                            skills: chat.selected_skills.clone(), question: question.into(), automatic: false,
                            created: Instant::now(), run_id: chat.run.as_ref().map(|run| run.id.clone()),
                            monitor_revision: Some(chat.monitor_revision()) });
                        chat.control_approval_focus_pending = true;
                        chat.note("待确认监控调整，当前规则尚未改变 / Monitoring change awaits confirmation", false);
                        return true;
                    }
                    let result = if let Some(reason) = chat.run.as_ref().and_then(|run| run.limit_reason(chat.tool_round)) {
                        Err(reason.to_owned())
                    } else if wiparse_core::ai::skills::permits(&chat.selected_skills, method) {
                        let lang = self.last_lang;
                        let testing = self.ai_test_locked;
                        self.apply_ai_action(chat, action, question, lang, testing)
                    } else { Err("SKILL.TOOL_NOT_ALLOWED".into()) };
                    chat.tool_round = chat.tool_round.saturating_add(1);
                    if let Some(run) = &mut chat.run {
                        run.record(match &result { Ok(()) => json!({"ok":true,"method":method}), Err(error) => json!({"ok":false,"executed":false,"method":method,"error":error}) });
                    }
                    if let Err(error) = result {
                        chat.cancel(); chat.queue_paused = true; chat.note(error, true);
                        return true;
                    }
                    continue;
                }
            };
            // Batched native tool calls are not automatically safe: every
            // queued control proposal needs its own concrete confirmation.
            match super::platform::validate(&method, &params) {
                Ok(true) => {
                    let state = self.ai_state(&chat.project, self.ai_test_locked);
                    let own_test_control=matches!(method.as_str(),"testing.stop"|"testing.pause"|"testing.resume");
                    if (if own_test_control {!chat.owned_test_control_allowed()} else {!chat.control_allowed(&state) || state.automation_locked})
                        || !wiparse_core::ai::skills::permits(&chat.selected_skills, &method) {
                        if let Some(run) = &mut chat.run {
                            run.record(json!({"ok":false,"executed":false,"method":method,"error":"AI.CONTROL_LOCKED"}));
                        }
                        chat.cancel();
                        chat.queue_paused = true;
                        chat.note("排队设备动作未获授权，未执行 / Queued device action not authorized", true);
                        return true;
                    }
                    if !method.starts_with("serial.monitor.") {
                        let result = if super::platform::broker_reviews_control(ctx, &method) {
                            self.begin_platform_call(ctx, chat, method.clone(), params, question.into(), false)
                        } else { Err("AI.VERIFIED_DEVICE_SESSION_AND_MANUAL_TASK_REQUIRED".into()) };
                        if let Err(error) = result {
                            if let Some(run) = &mut chat.run {
                                run.record(json!({"ok":false,"executed":false,"method":method,"error":error}));
                            }
                            chat.cancel(); chat.queue_paused = true; chat.note(error, true);
                        }
                        return true;
                    }
                    chat.control_approval = Some(ControlApproval {
                        action: Action::Platform { method, params }, binding: state.binding,
                        skills: chat.selected_skills.clone(), question: question.into(), automatic: false,
                        created: Instant::now(), run_id: chat.run.as_ref().map(|run| run.id.clone()),
                        monitor_revision: None,
                    });
                    chat.control_approval_focus_pending = true;
                    chat.note("待确认排队设备动作，尚未执行 / Queued device action awaits confirmation", false);
                    return true;
                }
                Err(error) => {
                    if let Some(run) = &mut chat.run {
                        run.record(json!({"ok":false,"executed":false,"method":method,"error":error}));
                    }
                    chat.cancel(); chat.queue_paused = true;
                    chat.note(format!("{error} · 批次已停止，后续动作未执行 / Batch stopped; later actions not executed"), true);
                    return true;
                }
                Ok(false) => {}
            }
            match self.begin_platform_call(
                ctx,
                chat,
                method.clone(),
                params,
                question.into(),
                false,
            ) {
                Ok(()) => return true,
                Err(error) => {
                    chat.entry(
                        "tool",
                        format!("未执行 / Not executed · {method}\n\n{error}"),
                    );
                    if let Some(run) = &mut chat.run {
                        run.record(json!({"ok":false,"error":error,"executed":false}));
                    }
                    chat.cancel(); chat.queue_paused = true;
                    chat.note(format!("{error} · 批次已停止，后续动作未执行 / Batch stopped; later actions not executed"), true);
                    return true;
                }
            }
        }
    }
    pub(super) fn begin_manual_agent(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        state: &Snapshot,
        question: String,
    ) -> Result<(), String> {
        let controls = chat.allow_automation;
        self.begin_manual_agent_capped(ctx, chat, state, question, controls)
    }
    pub(super) fn begin_manual_agent_capped(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        state: &Snapshot,
        question: String,
        control_cap: bool,
    ) -> Result<(), String> {
        self.begin_manual_agent_with_caps(ctx,chat,state,question,control_cap,control_cap)
    }
    pub(super) fn begin_manual_agent_with_caps(
        &mut self,ctx:&egui::Context,chat:&mut SerialAiChat,state:&Snapshot,question:String,
        control_cap:bool,test_control_cap:bool,
    ) -> Result<(), String> {
        if question.trim().is_empty() || question.len()>16*1024 {
            return Err("请输入问题（最多16 KiB）/ Enter a question (maximum 16 KiB)".into());
        }
        if chat.messages.is_empty() {
            chat.queue_paused = false;
        }
        self.ai_scoped_observation(chat.scope)?;
        if !chat.ready() {
            return Err("请先保存并启用全局AI配置 / Enable shared AI first".into());
        }
        let reviewed_log_requested=question.trim_start().starts_with("【审阅串口任务】");
        let reviewed_log=if reviewed_log_requested {
            if !chat.review_outbound || !matches!(chat.scope,DataScope::Current|DataScope::Range) {
                return Err("审阅串口任务需要开启外发审阅并选择当前日志或行范围 / Enable review and select current log or row range".into());
            }
            let tab=self.tabs.get(self.active_tab).ok_or("LOG.NO_CURRENT_LOG")?;
            if !tab.live { return Err("LOG.REVIEW_EXTERNAL_SOURCE_UNAVAILABLE: 仅支持当前实时采集的内存数据 / Only the current in-memory live capture can be frozen".into()); }
            let range=(chat.scope==DataScope::Range).then(||(chat.range_first.max(1),chat.range_last.max(chat.range_first.max(1))));
            let snapshot_id=uuid::Uuid::new_v4().to_string();
            let captured=wiparse_core::ai::ReviewedSource::capture(&self.serial_review_lease,|| {
                let lines=tab.ai_shared_lines(); let retained_rows=lines.len();
                let source=wiparse_core::log_analysis::Source::memory(&format!("live-snapshot-{snapshot_id}"),lines,tab.dropped() as u64+1,tab.dropped() as u64);
                ReviewedLog {source,range,snapshot_id,retained_rows}
            }).ok_or("LOG.REVIEW_SOURCE_BUSY")?;
            Some(captured)
        }else{None};
        if reviewed_log_requested { chat.monitor_lease.mutate_always(|| chat.selected_skills.clear());chat.auto_selected_skill=false;chat.library.reset(); }
        // Validate before taking the current workflow or consuming recovery.
        // A stale checkpoint must leave both the draft and recovery available
        // so the user can inspect it or choose a new task.
        let recovery_matches = chat.recovery.as_ref()
            .is_some_and(|r| r.draft == question && r.project == state.binding.project);
        if let Some(recovery) = chat.recovery.as_ref()
            .filter(|_| recovery_matches)
        {
            recovery.validate_skills()?;
        }
        let owner=ctx.data(|data|data.get_temp::<Option<(String,String)>>(egui::Id::new("wiparse-serial-test-owner"))).flatten();
        let owner_followup=owned_test_control_followup(&question) && chat.run.as_ref().is_some_and(|run|
            owner.as_ref().is_some_and(|(task,_)|task==&run.id));
        if chat.auto_skill_route && !reviewed_log_requested && !recovery_matches && !owner_followup
            && !(chat.recovery.is_none() && chat.run.as_ref().is_some_and(|run| run.scope_key == self.local_log_scope_key(chat)
                && run.source_binding.as_ref() == Some(&state.binding)
                && run.workflow.skill_ids == chat.selected_skills
                && wiparse_core::ai::workflow::continues_task(&question, chat.awaiting_reply())))
            && (chat.selected_skills.is_empty() || chat.auto_selected_skill) {
            chat.monitor_lease.mutate_always(|| chat.selected_skills.clear());
            chat.auto_selected_skill=false;
            let methods: Vec<_>=wiparse_core::ai::skills::BUILTINS.iter().flat_map(|skill|skill.tools.iter().copied())
                .filter(|method| (library::is_local(method) && chat.scope!=DataScope::None)
                    || (crate::ai::broker::supports(method) && wiparse_core::engineering_agent::tool(method)
                        .is_some_and(|tool| control_cap || !matches!(tool.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test))))
                .collect();
            let source=Some(chat.scope!=DataScope::None && !state.source_title.is_empty()
                && (chat.scope!=DataScope::Selection || state.selected_bytes>0));
            let sources=wiparse_core::ai::skills::RouteSources{log:source,..Default::default()};
            let candidates=wiparse_core::ai::skills::contextual_candidates_scoped(&question,"serial",sources,Some(!state.binding.project.is_empty()),None,&methods);
            if let [candidate]=candidates.as_slice() {
                if candidate.missing.is_empty() {
                    chat.monitor_lease.mutate_always(|| chat.selected_skills.push(candidate.skill.id.to_owned()));
                    chat.auto_selected_skill=true;
                }
            }
        }
        let scope_key = self.local_log_scope_key(chat);
        let compatible_history = !reviewed_log_requested && chat.run.as_ref().is_some_and(|r| {
            let device_cap=control_cap && chat.allow_automation && !state.automation_locked;
            r.scope_key==scope_key && r.source_binding.as_ref()==Some(&state.binding) && r.workflow.skill_ids==chat.selected_skills
                && (!device_cap || r.control_cap) && r.test_control_cap==(test_control_cap && chat.allow_automation)
        });
        let previous = if compatible_history {
            chat.tool_results
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let recovering=recovery_matches && !reviewed_log_requested;
        let continuing=chat.recovery.is_none() && compatible_history && chat.run.as_ref().is_some_and(|r|
            wiparse_core::ai::workflow::continues_task(&question,r.phase=="needs_input")
            || owner_followup);
        let mut workflow = if compatible_history {
            std::mem::take(&mut chat.run.as_mut().unwrap().workflow)
        } else {
            Default::default()
        };
        if !continuing {workflow.new_task();workflow.bind_request(&question);}else{workflow.follow_up(&question);}
        workflow.skill_ids = chat.selected_skills.clone();
        if !continuing {workflow.bind_skill_route(chat.auto_selected_skill,&question);}
        if !wiparse_core::ai::skills::explanatory_only(&question) { workflow.initialize_skill_plan_for(&question)?; }
        chat.tool_round = 0;
        if !continuing { chat.tool_results.clear(); chat.history.clear(); }
        // The active manual run can constrain control_allowed for the watch.
        // Publish its replacement and capability together as a monitor update.
        let monitor_update = chat.monitor_lease.begin_update();
        let continuation=if continuing {chat.run.take()} else {None};
        chat.run = Some(Run::new());
        if let Some(run)=chat.run.as_mut(){
            run.journal=crate::ai::journal::Journal::for_project(ctx,&state.binding.project);
            run.question=question.clone();
            run.reviewed_log=reviewed_log;
            if let Some(previous)=continuation {
                run.reviewed_log=previous.reviewed_log;
                run.id=previous.id;
                run.control_cap=previous.control_cap;
                run.test_control_cap=previous.test_control_cap;
                run.usage=previous.usage;
                run.evidence_sequence=previous.evidence_sequence;
                run.evidence_ids=previous.evidence_ids;
                run.initial_evidence=previous.initial_evidence;
                run.previous_evidence=previous.previous_evidence;
                run.uncertain=previous.uncertain;
                run.pending_readback=previous.pending_readback;
                run.question=format!("{}\n{}",previous.question,question);
            }
            if let Some(recovery)=if recovering { chat.recovery.take() } else { None } {
                run.recovery_needs_fresh_log=true;
                recovery.restore(&mut workflow);
                wiparse_core::ai::skills::select(&workflow.skill_ids)?;
                chat.monitor_lease.mutate_always(|| chat.selected_skills = workflow.skill_ids.clone());
                run.question=recovery.question.clone();
                run.uncertain=recovery.uncertain.clone();
                if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"resumed",&workflow,Some(("task.resume",&json!({"previous_task_id":recovery.origin()}))));}
            }else {
                // The checkpoint remains in the journal; clearing this UI
                // selection allows follow-ups to continue the new task.
                chat.recovery = None;
                if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"running",&workflow,None);}
            }
        }
        chat.run.as_mut().unwrap().control_cap &= control_cap && chat.allow_automation && !state.automation_locked;
        chat.run.as_mut().unwrap().test_control_cap &= test_control_cap && chat.allow_automation;
        chat.run.as_mut().unwrap().workflow = workflow;
        if !continuing { chat.run.as_mut().unwrap().previous_evidence = previous; }
        chat.run.as_mut().unwrap().scope_key = self.local_log_scope_key(chat);
        chat.run.as_mut().unwrap().source_binding = Some(state.binding.clone());
        drop(monitor_update);
        chat.library.bind_scope(self.local_log_scope_key(chat));
        chat.entry("user", question.clone());
        chat.history.push(AiMessage::User(bounded(&question, 8000)));
        chat.run.as_mut().unwrap().phase = "planning";
        let mut data = self.ai_scoped_observation(chat.scope)?;
        if continuing {
            // Carry recent observations into the new turn, including measurement detail beyond the index.
            data["current_followup"] = json!({"request":question,"instruction":"Answer the latest turn using the earlier task only for context. If the user explicitly asks for only a specific value or a fixed-length answer, output exactly that requested content and no additional explanation, table, heading or citation unless requested. Retain the supporting evidence in the task record."});
            data["platform_results"] = json!(chat.tool_results.iter().rev()
                .filter(|r| r["result"]["kind"] != "task_management")
                .take(3).collect::<Vec<_>>());
        }
        let excludes_read = wiparse_core::ai::skills::explanatory_only(&question);
        // Independent read-only skills in a combination each need their own
        // observation. Queue them before the first scan, so its early return
        // cannot silently suppress report or protocol collection.
        if !continuing && !recovering && !excludes_read && !reviewed_log_requested {
            if let Some(run) = &mut chat.run {
                // When running a test, the currently selected report may belong to
                // an earlier run. Let the agent wait for the run and explicitly
                // select its report instead of pre-reading a stale one.
                if chat.selected_skills.iter().any(|id| id == "test_result")
                    && !chat.selected_skills.iter().any(|id| id == "test_execution")
                    && !wiparse_core::ai::skills::skill_excluded(&question,"test_result") {
                    run.queued.push_back(Action::Platform { method: "report.read".into(), params: json!({}) });
                }
                if chat.selected_skills.iter().any(|id| id == "qi_protocol") && !wiparse_core::ai::skills::skill_excluded(&question,"qi_protocol") {
                    run.queued.push_back(Action::Platform { method: "wave.frames".into(), params: json!({}) });
                }
            }
        }
        if !continuing && !excludes_read && !wiparse_core::ai::skills::skill_excluded(&question,"serial_log") && chat.scope != DataScope::None && chat.selected_skills.iter().any(|id| id == "serial_log" || id == "charging_protection") {
            match self.begin_platform_call(ctx, chat, "log.analyze".into(),
                json!({"discover":true,"samples":4}), question.clone(), false) {
                Ok(()) => { if let Some(run)=&mut chat.run { run.auto_log_scan=true; run.recovery_scan_started=recovering; } return Ok(()); },
                Err(error) => data["local_analysis_error"] = json!(error),
            }
        }
        if self.dispatch_queued_reads(ctx, chat, &question) { return Ok(()); }
        chat.submit(ctx, state, data, question, false)
    }
}

#[cfg(test)]
mod budget_tests{
    use super::*;
    #[test]
    fn recovery_only_accepts_the_original_host_scan_and_complete_final_page() {
        let mut run=Run::new();run.recovery_needs_fresh_log=true;run.recovery_scan_started=true;
        let id="11111111-1111-4111-8111-111111111111";
        let other="22222222-2222-4222-8222-222222222222";
        let partial=json!({"ok":true,"data":{"query_id":id,"continuation":id,"coverage":{"complete":false}}});
        run.accept_recovery_scan(&json!({}),&partial);
        let complete=|query|json!({"ok":true,"data":{"query_id":query,"coverage":{"complete":true,"files":1},"source_snapshot":{"algorithm":"sha256","sources":[{"id":"source","sha256":"a".repeat(64),"bytes":3}]}}});
        run.accept_recovery_scan(&json!({"cursor":other}),&complete(other));
        assert!(run.recovery_needs_fresh_log);
        run.accept_recovery_scan(&json!({"cursor":id}),&complete(id));
        assert!(!run.recovery_needs_fresh_log);
    }
    #[test]
    fn only_distinct_successful_reads_extend_the_budget(){
        let mut run=Run::new();
        let receipt=|bytes|json!({"ok":true,"data":{"coverage":{"bytes_read":bytes}}});
        let cursor=json!({"cursor":"same-log"});
        for bytes in 1..=100{
            run.credit_read("instrument.command",&cursor,&receipt(bytes));
            run.credit_read("log.analyze",&cursor,&json!({"ok":false,"data":{"coverage":{"bytes_read":bytes}}}));
        }
        assert_eq!(run.call_limit(),32);
        for _ in 0..100{run.credit_read("log.analyze",&cursor,&receipt(500));}
        assert_eq!(run.call_limit(),33);assert!(run.limit_reason(32).is_none());
        for bytes in 501..700{run.credit_read("log.analyze",&cursor,&receipt(bytes));}
        assert_eq!(run.call_limit(),96);assert!(run.limit_reason(96).is_some());
    }
    #[test] fn rejected_dispatches_stop_without_spending_successful_read_allowance(){
        let mut run=Run::new();for _ in 0..3{run.record(json!({"ok":false,"executed":false}));}
        assert!(run.limit_reason(0).is_some());assert_eq!(run.call_limit(),32);
        run.record(json!({"ok":true}));assert!(run.limit_reason(0).is_none());
        run.started=Instant::now()-Duration::from_secs(601);assert!(run.limit_reason(0).is_some());
    }
}
