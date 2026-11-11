//! One request owns read authorization, evidence and a bounded execution loop.
use super::*;
use wiparse_core::ai::{tools, AiReply};
pub(super) const MAX_CALLS: usize = 32;
pub(super) struct Run {
    pub id: String,
    pub question:String,
    pub uncertain:Vec<String>,
    pub pending_readback:Option<(String,String)>,
    pub correction:Option<String>,
    pub journal:Option<crate::ai::journal::Journal>,
    pub started: Instant,
    pub phase: &'static str,
    pub control_cap: bool,
    pub repairs:usize,
    pub first_evidence_ms:Option<u64>,
    pub repair_count: u8,
    pub json_fallback: bool,
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
    pub last_read: Option<String>,
    pub initial_evidence: Option<serde_json::Value>,
    pub evidence_ids: Vec<String>,
    pub previous_evidence: Vec<serde_json::Value>,
}
impl Run {
    pub fn new() -> Self {
        Self {
            question:String::new(),uncertain:vec![],pending_readback:None,correction:None,journal:None,
            id: uuid::Uuid::new_v4().to_string(),
            started: Instant::now(),
            phase: "reading",
            control_cap: true,
            repairs:0,first_evidence_ms:None,repair_count: 0,json_fallback:false,
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
            last_read: None,
            initial_evidence: None,
            evidence_ids: vec![],
            previous_evidence: vec![],
        }
    }
    pub fn finish(&mut self,phase:&'static str){
        if matches!(self.phase,"completed"|"needs_input"|"failed"|"partial"|"cancelled"){return}
        self.phase=phase;
        if let Some(j)=&self.journal{j.record(&self.id,"serial",&self.question,phase,&self.workflow,None);}
        tracing::info!(target:"wiparse_ai_outcome",task_id=%self.id,outcome=phase,elapsed_ms=self.started.elapsed().as_millis() as u64,first_evidence_ms=self.first_evidence_ms,repairs=self.repairs,usage=%json!(self.usage), evidence_count=self.evidence_sequence);
    }
    pub fn record(&mut self, result: serde_json::Value) {
        if let Some(j)=&self.journal{j.record(&self.id,"serial",&self.question,"running",&self.workflow,Some((result["method"].as_str().unwrap_or("serial observation"),&result)));}
        if result["ok"] == true {
            self.repair_count = 0;
            if self.first_evidence_ms.is_none()&&result["kind"]!="task_management"{self.first_evidence_ms=Some(self.started.elapsed().as_millis() as u64);}
        }
        if let Some(id) = result["evidence_id"]
            .as_str()
            .filter(|_| result["kind"] != "task_management")
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
    }
}
pub(super) fn functions(scope: DataScope, controls: bool) -> Vec<tools::Function> {
    let mut methods = vec![
        "agent.plan",
        "agent.evidence",
        "agent.help",
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
    methods.extend(wiparse_core::engineering_agent::TOOLS.iter().filter(|t|crate::ai::broker::supports(t.method)&& (controls || !matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test))).map(|t|t.method));
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
            let Some(Action::Platform { method, params }) = next else {
                return false;
            };
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
        if chat.messages.is_empty() {
            chat.queue_paused = false;
        }
        self.ai_scoped_observation(chat.scope)?;
        if !chat.ready() {
            return Err("请先保存并启用全局AI配置 / Enable shared AI first".into());
        }
        let scope_key = self.local_log_scope_key(chat);
        let previous = if chat.run.as_ref().is_some_and(|r| r.scope_key == scope_key) {
            chat.tool_results
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let continuing=chat.run.as_ref().is_some_and(|r|r.scope_key==scope_key&&r.phase=="needs_input");
        let mut workflow = if chat.run.as_ref().is_some_and(|r| r.scope_key == scope_key) {
            std::mem::take(&mut chat.run.as_mut().unwrap().workflow)
        } else {
            Default::default()
        };
        if !continuing {workflow.new_task();workflow.bind_request(&question);}else{workflow.amend_request(&question);}
        chat.tool_round = 0;
        chat.tool_results.clear();
        let continuation=if continuing {chat.run.take()} else {None};
        chat.run = Some(Run::new());
        if let Some(run)=chat.run.as_mut(){
            run.journal=crate::ai::journal::Journal::for_project(ctx,&state.binding.project);
            run.question=question.clone();
            if let Some(previous)=continuation {
                run.id=previous.id;
                run.evidence_sequence=previous.evidence_sequence;
                run.uncertain=previous.uncertain;
                run.pending_readback=previous.pending_readback;
                run.question=format!("{}\n{}",previous.question,question);
            }
            if let Some(recovery)=chat.recovery.take().filter(|r|r.draft==question && r.project==state.binding.project) {
                recovery.restore(&mut workflow);
                run.question=recovery.question.clone();
                run.uncertain=recovery.uncertain.clone();
                if let Some(j)=&run.journal{recovery.checkpoint_reports(j,&run.id,"serial",&workflow);}
                if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"resumed",&workflow,Some(("task.resume",&json!({"previous_task_id":recovery.origin()}))));}
            }else if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"running",&workflow,None);}
        }
        chat.run.as_mut().unwrap().control_cap = control_cap && chat.allow_automation;
        chat.run.as_mut().unwrap().workflow = workflow;
        chat.run.as_mut().unwrap().evidence_ids = previous
            .iter()
            .filter_map(|v| v["evidence_id"].as_str().map(str::to_owned))
            .collect();
        chat.run.as_mut().unwrap().previous_evidence = previous;
        chat.run.as_mut().unwrap().scope_key = self.local_log_scope_key(chat);
        chat.library.bind_scope(self.local_log_scope_key(chat));
        chat.entry("user", question.clone());
        chat.history.push(AiMessage::User(bounded(&question, 8000)));
        chat.run.as_mut().unwrap().phase = "planning";
        let data = self.ai_scoped_observation(chat.scope)?;
        chat.submit(ctx, state, data, question, false)
    }
}
