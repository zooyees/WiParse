//! One bounded task at a time over the existing shared provider transport.
use super::*;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use wiparse_core::engineering_agent::{self as contract, Call, Effect};

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ContextState {
    pub page: String,
    pub project: String,
    pub binding: String,
    pub testing: bool,
}
#[derive(Clone)]
pub(crate) struct HostCall {
    pub id: String,
    pub call: Call,
    pub context: ContextState,
    pub cancelled: Arc<AtomicBool>,
    pub internal: bool,
}
pub(crate) struct HostReply {
    pub value: Value,
    pub receiver: Option<crossbeam_channel::Receiver<Value>>,
}
impl HostReply {
    pub fn value(value: Value) -> Self {
        Self {
            value,
            receiver: None,
        }
    }
    pub fn work(receiver: crossbeam_channel::Receiver<Value>) -> Self {
        Self {
            value: Value::Null,
            receiver: Some(receiver),
        }
    }
}
struct Step {
    id: String,
    call: Call,
    title: String,
    status: String,
    data: Value,
    elapsed: f64,
    recorded_unix_ms: u128,
    task: String,
    scope: ContextState,
}
struct Message {
    role: &'static str,
    text: String,
    markdown: crate::test_report::TextMarkdown,
}
struct Waiting {
    origin: Call,
    task: String,
    dispatched: bool,
    request: HostCall,
    at: Instant,
    receiver: Option<crossbeam_channel::Receiver<Value>>,
    poll: Option<Call>,
    next: Instant,
}
pub(crate) struct Session {
    pub read_data: bool,
    pub cross_tools: bool,
    pub application: bool,
    pub(crate) data_job: Option<crate::data_analysis::AgentDataStats>,
    pub(crate) log_jobs: LogJobs,
    pub(crate) report_jobs: LogJobs,
    owned_test_key: Option<String>,
    owned_test_context: Option<(ContextState, String, Instant)>,
    context: Option<ContextState>,
    task: String,
    question: String,
    prompt: String,
    model: Option<AiTask>,
    waiting: Option<Waiting>,
    outgoing: VecDeque<HostCall>,
    approval: Option<Call>,
    cancel: Arc<AtomicBool>,
    messages: Vec<Message>,
    steps: Vec<Step>,
    plan: Vec<String>,
    observations: Vec<Value>,
    round: usize,
    max_rounds: usize,
    started: Option<Instant>,
    status: String,
    error: bool,
    last_call: String,
    repeated: usize,
    owned_test: bool,
    confirmed: Option<Call>,
    config: Option<AiConfig>,
    resume_model: bool,
    evidence_sequence: u64,
    repair_attempted: bool,
    watched: std::collections::HashSet<String>,
    background_test: Option<Value>,
    pub navigation: Option<ContextState>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            read_data: true,
            cross_tools: false,
            application: false,
            data_job: None,
            log_jobs: Default::default(),
            report_jobs: Default::default(),
            owned_test_key: None,
            owned_test_context: None,
            context: None,
            task: String::new(),
            question: String::new(),
            prompt: String::new(),
            model: None,
            waiting: None,
            outgoing: VecDeque::new(),
            approval: None,
            cancel: Arc::new(AtomicBool::new(false)),
            messages: vec![],
            steps: vec![],
            plan: vec![],
            observations: vec![],
            round: 0,
            max_rounds: 8,
            started: None,
            status: String::new(),
            error: false,
            last_call: String::new(),
            repeated: 0,
            owned_test: false,
            confirmed: None,
            config: None,
            resume_model: false,
            evidence_sequence: 0,
            repair_attempted: false,
            watched: Default::default(),
            background_test: None,
            navigation: None,
        }
    }
}
fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].into()
}
fn binding_for(context: &ContextState, page: &str) -> String {
    serde_json::from_str::<Value>(&context.binding)
        .ok()
        .and_then(|v| v[page].as_str().map(str::to_owned))
        .unwrap_or_else(|| context.binding.clone())
}
pub(crate) fn bounded_evidence(value: Value, depth: usize) -> Value {
    match value {
        Value::String(s) if s.len() > 2048 => {
            json!({"text":clip(&s,2048),"truncated":true,"original_bytes":s.len()})
        }
        Value::Array(values) if values.len() > 16 => {
            let len = values.len();
            json!({"items":values.into_iter().take(16).map(|v|bounded_evidence(v,depth+1)).collect::<Vec<_>>(),"truncated":true,"original_count":len})
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|v| bounded_evidence(v, depth + 1))
                .collect(),
        ),
        Value::Object(fields) if depth < 12 => Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k, bounded_evidence(v, depth + 1)))
                .collect(),
        ),
        Value::Object(_) => json!({"truncated":true,"reason":"nesting budget"}),
        v => v,
    }
}
fn ok(data: Value) -> Value {
    json!({"ok":true,"data":data})
}
pub(crate) fn failure(error: impl AsRef<str>) -> Value {
    json!({"ok":false,"error":error.as_ref()})
}
impl Session {
    fn call_page<'a>(&'a self, call: &Call) -> &'a str {
        let call = if call.method.ends_with(".result") || call.method == "serial.ready" {
            self.waiting.as_ref().map_or(call, |w| &w.origin)
        } else {
            call
        };
        match contract::tool(&call.method).map(|t| t.page) {
            Some("any" | "current") | None => self.context.as_ref().map_or("", |c| c.page.as_str()),
            Some(page) => page,
        }
    }
    pub fn busy(&self) -> bool {
        self.model.is_some()
            || self.waiting.is_some()
            || self.approval.is_some()
            || self.resume_model
    }
    pub fn origin_page(&self) -> Option<&str> {
        self.busy()
            .then(|| self.context.as_ref().map(|c| c.page.as_str()))
            .flatten()
    }
    pub fn tracking_test(&self) -> bool {
        self.owned_test_key.is_some()
    }
    pub fn update_test(&mut self, status: Value) {
        if let Some(key) = self.owned_test_key.clone() {
            if status["running"] == false || status["run_id"].as_str().is_some_and(|id| id != key) {
                let (context, task, at) = self.owned_test_context.clone().unwrap_or_else(|| {
                    (
                        self.context.clone().unwrap(),
                        self.task.clone(),
                        Instant::now(),
                    )
                });
                let wait = Waiting {
                    task,
                    dispatched: true,
                    origin: Call {
                        method: "testing.status".into(),
                        params: json!({}),
                    },
                    request: HostCall {
                        id: key.clone(),
                        call: Call {
                            method: "testing.status".into(),
                            params: json!({}),
                        },
                        context,
                        cancelled: self.cancel.clone(),
                        internal: true,
                    },
                    at,
                    receiver: None,
                    poll: None,
                    next: Instant::now(),
                };
                let matched = status["completed_run_identity"] == key;
                self.record_step(&wait,json!({"ok":matched&&status["process_exit_code"]==0,"state":if matched {"completed"}else{"outcome_unknown"},"run_id":key,"process_completed":matched,"formal_verdict":"consult test report; exit status is not product verdict","status":status}),"后台测试已结束");
                self.owned_test_key = None;
                self.owned_test = false;
            }
        }
        self.background_test = Some(status);
    }
    fn clear_changed_caches(&mut self, context: &ContextState) {
        if let Some(old) = &self.context {
            let project = old.project != context.project;
            if project || binding_for(old, "serial") != binding_for(context, "serial") {
                self.log_jobs = Default::default();
            }
            if project || binding_for(old, "test_report") != binding_for(context, "test_report") {
                self.report_jobs = Default::default();
            }
            if project || binding_for(old, "data_analysis") != binding_for(context, "data_analysis")
            {
                self.data_job = None;
            }
        }
    }
    pub fn set_context(&mut self, context: ContextState) {
        self.clear_changed_caches(&context);
        if self.context.as_ref().is_some_and(|old| {
            old.project != context.project
                || if self.watched.is_empty() {
                    binding_for(old, &old.page) != binding_for(&context, &old.page)
                } else {
                    self.watched
                        .iter()
                        .any(|page| binding_for(old, page) != binding_for(&context, page))
                }
        }) {
            if self.busy() {
                self.cancel_task("页面、工程或数据源已变化，旧任务已停止。请在新来源上重新提问。");
            }
            if self
                .context
                .as_ref()
                .is_some_and(|old| old.project != context.project)
            {
                self.messages.clear();
                self.observations.clear();
            }
            self.owned_test = self.owned_test_key.is_some();
        }
        self.context = Some(context);
    }
    pub fn configure(&mut self, config: &AiConfig) {
        if self.config.as_ref().is_some_and(|old| old != config) {
            self.cancel_task("AI连接设置已变化，请重新发起任务。");
            self.messages.clear();
            self.observations.clear();
            self.plan.clear();
        }
        self.config = Some(config.clone());
    }
    pub fn evidence(&self, id: &str) -> Value {
        self.steps.iter().find(|s|s.id==id&&self.context.as_ref().is_some_and(|c|c.project==s.scope.project)).map_or_else(||failure("EVIDENCE.EXPIRED_OR_DIFFERENT_PROJECT"),|s|json!({"ok":true,"data":{"evidence_id":s.id,"task":s.task,"scope":s.scope,"call":s.call,"status":s.status,"result":s.data,"historical":true}}))
    }
    pub fn take_request(&mut self) -> Option<HostCall> {
        let request = self.outgoing.pop_front();
        if let (Some(r), Some(wait)) = (&request, &mut self.waiting) {
            if !r.internal && r.id == wait.request.id {
                wait.dispatched = true;
            }
        }
        request
    }
    pub fn owns_test(&self, key: Option<&str>) -> bool {
        key.is_some()
            && self.owned_test_key.as_deref() == key
            && self
                .owned_test_context
                .as_ref()
                .is_some_and(|(scope, _, _)| {
                    self.context
                        .as_ref()
                        .is_some_and(|c| c.project == scope.project)
                })
    }
    pub fn mark_test(&mut self, key: Option<String>) {
        if key.is_some() {
            self.owned_test_context = self
                .context
                .clone()
                .map(|c| (c, self.task.clone(), Instant::now()));
        }
        self.owned_test_key = key;
        self.owned_test = self.owned_test_key.is_some();
    }
    pub fn authorize(&self, request: &HostCall, current: &ContextState) -> bool {
        if request.cancelled.load(Ordering::Acquire)
            || request.context.project != current.project
            || binding_for(&request.context, self.call_page(&request.call))
                != binding_for(current, self.call_page(&request.call))
        {
            return false;
        }
        let Some(wait) = &self.waiting else {
            return false;
        };
        if wait.request.id != request.id || wait.request.call != request.call {
            return false;
        }
        if request.internal {
            return true;
        }
        let Ok(tool) = contract::validate(&request.call) else {
            return false;
        };
        if tool.effect != Effect::LocalCompute
            && (!self.read_data
                || (!self.cross_tools
                    && !matches!(tool.page, "any" | "current")
                    && tool.page != current.page))
        {
            return false;
        }
        if tool.effect == Effect::Application && !self.application {
            return false;
        }
        if matches!(tool.effect, Effect::Device | Effect::Test)
            && (current.testing
                && !matches!(
                    request.call.method.as_str(),
                    "testing.stop" | "testing.pause" | "testing.resume"
                )
                || self.confirmed.as_ref() != Some(&request.call))
        {
            return false;
        }
        if request.call.method == "testing.stop" && !self.owned_test {
            return false;
        }
        true
    }
    pub fn attach(&mut self, id: &str, reply: HostReply) {
        if self
            .waiting
            .as_ref()
            .is_none_or(|wait| wait.request.id != id)
        {
            return;
        }
        if let Some(rx) = reply.receiver {
            self.waiting.as_mut().unwrap().receiver = Some(rx);
        } else {
            self.accept_host(reply.value);
        }
    }
    pub fn host_transition(&mut self, context: ContextState) {
        // Only the already-authorized host may update the binding of its own transition.
        self.clear_changed_caches(&context);
        self.context = Some(context.clone());
        if let Some(wait) = &mut self.waiting {
            wait.request.context = context;
        }
    }
    fn message(&mut self, role: &'static str, text: String) {
        let text = clip(&text, 48000);
        let markdown = crate::test_report::TextMarkdown::new(&text);
        self.messages.push(Message {
            role,
            text,
            markdown,
        });
        while self.messages.len() > 24
            || self.messages.iter().map(|m| m.text.len()).sum::<usize>() > 192 * 1024
        {
            self.messages.remove(0);
        }
    }
    pub fn cancel_task(&mut self, reason: &str) {
        self.cancel.store(true, Ordering::Release);
        if let Some(model) = self.model.take() {
            model.cancel();
        }
        if let Some(wait) = self.waiting.take() {
            let dispatched = wait.dispatched;
            let effect = contract::tool(&wait.origin.method).map(|t| t.effect);
            let unknown = dispatched && matches!(effect, Some(Effect::Device | Effect::Test));
            self.record_step(&wait, json!({"ok":false,"state":if unknown {"outcome_unknown"} else {"cancelled"},"reason":reason,"dispatched":dispatched,"pending_call":wait.request.call,"job":wait.poll}), if unknown {"结果未确认：请核对设备"} else {"已取消"});
        }
        self.outgoing.clear();
        self.resume_model = false;
        self.approval = None;
        self.confirmed = None;
        self.started = None;
        self.status = reason.into();
        self.error = false;
    }
    fn enqueue(&mut self, call: Call, internal: bool) {
        let Some(context) = self.context.clone() else {
            return;
        };
        self.watched.insert(self.call_page(&call).to_owned());
        let request = HostCall {
            id: uuid::Uuid::new_v4().to_string(),
            call,
            context,
            cancelled: self.cancel.clone(),
            internal,
        };
        self.waiting = Some(Waiting {
            origin: request.call.clone(),
            task: self.task.clone(),
            dispatched: false,
            request: request.clone(),
            at: Instant::now(),
            receiver: None,
            poll: None,
            next: Instant::now(),
        });
        self.outgoing.push_back(request);
    }
    fn request_model(&mut self, service: &AiService) {
        if self.round >= self.max_rounds {
            self.finish_error("达到本任务工具轮数上限；已保留证据。可缩小范围或继续提问。");
            return;
        }
        let Some(context) = &self.context else { return };
        let catalog = contract::catalog(
            &context.page,
            self.cross_tools,
            self.read_data,
            self.application,
        );
        let history = self
            .messages
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|m| {
                if m.role == "user" {
                    AiMessage::User(clip(&m.text, 6000))
                } else {
                    AiMessage::Assistant(clip(&m.text, 8000))
                }
            })
            .collect();
        let request=AiRequest{purpose:format!("{}\n{}",contract::CONTRACT,catalog),text:json!({"task_id":self.task,"user_request":self.question,"page":context.page,"project":context.project,"permissions":{"read_current_data":self.read_data,"cross_tools":self.cross_tools,"application_changes":self.application,"device_and_test_actions":"explicit operator confirmation per concrete action"},"verified_evidence":self.observations,"budget":{"round":self.round,"max_rounds":self.max_rounds},"testing_active":context.testing}).to_string()};
        match service.submit_conversation(request, history) {
            Ok(task) => {
                self.model = Some(task);
                self.round += 1;
                self.status = "正在规划或分析…".into();
                self.error = false;
            }
            Err(error) => self.finish_error(&error_message(&error)),
        }
    }
    fn start(&mut self, service: &AiService) {
        let question = self.prompt.trim().to_owned();
        if question.is_empty() || question.len() > 16000 {
            return;
        }
        self.cancel_task("");
        self.cancel = Arc::new(AtomicBool::new(false));
        self.task = uuid::Uuid::new_v4().to_string();
        self.question = question.clone();
        self.prompt.clear();
        self.round = 0;
        self.repair_attempted = false;
        self.watched.clear();
        self.plan.clear();
        self.last_call.clear();
        self.repeated = 0;
        self.owned_test = self.owned_test_key.is_some();
        self.started = Some(Instant::now());
        self.message("user", question);
        self.request_model(service);
    }
    fn finish_error(&mut self, message: &str) {
        self.cancel_task(message);
        self.error = true;
    }
    fn accept_host(&mut self, value: Value) {
        let Some(mut wait) = self.waiting.take() else {
            return;
        };
        wait.receiver = None;
        if let Some(next) = value.get("_agent_wait") {
            let Some(method) = next["method"].as_str() else {
                self.finish_error("宿主返回了无效的异步状态。");
                return;
            };
            wait.poll = Some(Call {
                method: method.into(),
                params: next["params"].clone(),
            });
            wait.next = Instant::now()
                + Duration::from_millis(
                    next["interval_ms"].as_u64().unwrap_or(250).clamp(16, 1000),
                );
            self.waiting = Some(wait);
            return;
        }
        let method = wait.origin.method.clone();
        if value["state"] == "pending" || value["data"]["state"] == "pending_or_unknown" {
            wait.poll = Some(wait.request.call.clone());
            wait.next = Instant::now() + Duration::from_millis(250);
            self.waiting = Some(wait);
            return;
        }
        if method == "testing.run" && value["ok"] != false {
            self.owned_test = true;
        }
        let data = bounded_evidence(value, 0);
        let status = if data["ok"] == false {
            "失败 / 状态未确认"
        } else {
            "完成"
        };
        self.record_step(&wait, data, status);
        self.confirmed = None;
        self.resume_model = true;
        self.status = "已取得宿主结果，继续分析…".into();
    }
    fn record_step(&mut self, wait: &Waiting, data: Value, status: &str) {
        self.evidence_sequence += 1;
        let id = format!("E{}", self.evidence_sequence);
        let method = wait.origin.method.clone();
        self.steps.push(Step {
            id: id.clone(),
            call: wait.origin.clone(),
            title: contract::tool(&method).map_or(method.clone(), |t| t.title.into()),
            status: status.into(),
            data: data.clone(),
            elapsed: wait.at.elapsed().as_secs_f64(),
            recorded_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            task: wait.task.clone(),
            scope: wait.request.context.clone(),
        });
        if self
            .context
            .as_ref()
            .is_some_and(|c| c.project == wait.request.context.project)
        {
            self.observations.push(json!({"evidence_id":id,"task_id":wait.task,"tool":method,"params":wait.origin.params,"result":data,"scope":wait.request.context,"algorithm_version":wiparse_core::VERSION}));
        }
        while self.observations.len() > 8 {
            self.observations.remove(0);
        }
        while self.steps.len() > 128 {
            self.steps.remove(0);
        }
    }
    pub fn poll(&mut self, ctx: &Context, service: &AiService) {
        if self
            .started
            .is_some_and(|at| at.elapsed() > Duration::from_secs(240))
            && self.approval.is_none()
        {
            self.finish_error(
                "任务超过4分钟工作时限，已停止等待；已发送的设备动作不会自动重试或回滚。",
            );
            return;
        }
        let host = self
            .waiting
            .as_ref()
            .and_then(|wait| wait.receiver.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(value) => Some(value),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(failure("AGENT.WORKER_DISCONNECTED"))
                }
                Err(_) => None,
            });
        if let Some(value) = host {
            self.accept_host(value);
        }
        if self
            .waiting
            .as_ref()
            .is_some_and(|wait| wait.at.elapsed() > Duration::from_secs(90))
        {
            self.finish_error("工具超过90秒仍未确认完成；不会重试设备动作，请核对设备状态。");
            return;
        }
        if let Some(wait) = &mut self.waiting {
            if wait.poll.is_some()
                && wait.receiver.is_none()
                && Instant::now() >= wait.next
                && self.outgoing.is_empty()
            {
                let call = wait.poll.take().unwrap();
                let request = HostCall {
                    id: wait.request.id.clone(),
                    call,
                    context: wait.request.context.clone(),
                    cancelled: self.cancel.clone(),
                    internal: true,
                };
                wait.request = request.clone();
                self.outgoing.push_back(request);
            }
        }
        let model = self
            .model
            .as_ref()
            .and_then(|task| match task.result.try_recv() {
                Ok(value) => Some(value),
                Err(crossbeam_channel::TryRecvError::Disconnected) => Some(Err(AiError::Network)),
                Err(_) => None,
            });
        if let Some(result) = model {
            self.model = None;
            match result {
                Err(error) => self.finish_error(&error_message(&error)),
                Ok(reply) => {
                    if reply
                        .finish_reason
                        .as_deref()
                        .is_some_and(|r| !matches!(r, "stop" | "completed"))
                    {
                        self.message("assistant", reply.text);
                        self.finish_error("模型回复被截断或未正常完成，未执行其中任何操作；请缩小问题范围后重试。");
                        return;
                    }
                    match contract::parse(&reply.text) {
                        Err(error) => {
                            if !self.repair_attempted {
                                self.repair_attempted = true;
                                self.message("assistant",format!("工具请求格式无效，未执行操作：{error}。请按工具契约重新生成一次请求。"));
                                self.resume_model = true;
                            } else {
                                self.finish_error(&format!(
                                    "模型工具请求无效，未执行操作：{error}"
                                ));
                            }
                        }
                        Ok(answer) => {
                            if !answer.reply.trim().is_empty() {
                                self.message("assistant", answer.reply);
                            }
                            if !answer.plan.is_empty() {
                                self.plan = answer.plan;
                            }
                            if let Some(call) = answer.call {
                                let encoded = serde_json::to_string(&call).unwrap_or_default();
                                if encoded == self.last_call {
                                    self.repeated += 1;
                                } else {
                                    self.last_call = encoded;
                                    self.repeated = 0;
                                }
                                if self.repeated > 0
                                    && !matches!(
                                        call.method.as_str(),
                                        "testing.status"
                                            | "testing.inspect"
                                            | "serial.inspect"
                                            | "instrument.list"
                                    )
                                {
                                    self.finish_error("模型重复了相同调用，已终止循环；已有结果可在执行记录中查看。");
                                    return;
                                }
                                let tool = contract::validate(&call).unwrap();
                                let Some(context) = self.context.as_ref() else {
                                    return;
                                };
                                self.watched
                                    .insert(if matches!(tool.page, "any" | "current") {
                                        context.page.clone()
                                    } else {
                                        tool.page.into()
                                    });
                                if tool.effect != Effect::LocalCompute
                                    && (!self.read_data
                                        || !self.cross_tools
                                            && !matches!(tool.page, "current" | "any")
                                            && tool.page != context.page)
                                {
                                    self.finish_error("当前数据范围不允许此调用，请调整“使用本页数据 / 跨工具”后重新发送。");
                                    return;
                                }
                                if tool.effect == Effect::Application && !self.application {
                                    self.finish_error("此操作需要开启“允许调整界面”，当前未执行。");
                                    return;
                                }
                                if matches!(tool.effect, Effect::Device | Effect::Test) {
                                    if context.testing
                                        && !matches!(
                                            call.method.as_str(),
                                            "testing.stop" | "testing.pause" | "testing.resume"
                                        )
                                    {
                                        self.finish_error("测试正在占用设备，当前设备操作被锁定。");
                                        return;
                                    }
                                    self.approval = Some(call);
                                    self.status = "等待确认具体操作，尚未执行。".into();
                                } else {
                                    self.enqueue(call, false);
                                    self.status = format!("正在{}…", tool.title);
                                }
                            } else {
                                self.started = None;
                                self.status = "分析完成；证据与执行记录已保留。".into();
                            }
                        }
                    }
                }
            }
        }
        if self.resume_model
            && self.model.is_none()
            && self.waiting.is_none()
            && self.approval.is_none()
        {
            self.resume_model = false;
            self.request_model(service);
        }
        if self.model.is_some() || self.waiting.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        lang: Lang,
        t: &Tokens,
        service: &AiService,
        ready: bool,
    ) {
        self.poll(ctx, service);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(text(lang, "工程智能任务", "Engineering task"))
                    .size(16.)
                    .strong(),
            );
            if let Some(context) = &self.context {
                ui.weak(format!("{} · {}", context.project, context.page));
            }
        });
        ui.label(
            RichText::new(
                text(lang,"读取真实数据 → 本地处理 → 工具执行 → 验证结果；设备与测试动作逐次确认。","Read evidence → local processing → tool execution → verify results. Device and test actions require confirmation."),
            )
            .size(12.)
            .color(t.text_muted),
        );
        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.read_data, text(lang,"使用本页数据","Use page data"));
                ui.checkbox(&mut self.cross_tools, text(lang,"跨工具","Across tools"));
                ui.checkbox(&mut self.application, text(lang,"允许调整界面","Allow UI changes"))
                    .on_hover_text(text(lang,"允许调整光标、协议配置及计算器输入；不代表授权硬件动作。","Allows cursor, protocol and calculator changes. Hardware actions require separate confirmation."));
                egui::ComboBox::from_id_salt("agent-round-budget")
                    .selected_text(format!("{}轮预算", self.max_rounds))
                    .show_ui(ui, |ui| {
                        for n in [4, 8, 12] {
                            ui.selectable_value(&mut self.max_rounds, n, format!("{n}轮"));
                        }
                    });
            });
        });
        if let Some(status) = &self.background_test {
            ui.weak(format!(
                "后台测试：{} · {}",
                if status["running"] == true {
                    "运行中（独立于AI等待时限）"
                } else {
                    "已结束"
                },
                status["status"].as_str().unwrap_or("")
            ));
        }
        if !self.plan.is_empty() {
            egui::CollapsingHeader::new(text(lang, "任务计划", "Task plan"))
                .default_open(true)
                .show(ui, |ui| {
                    for (i, item) in self.plan.iter().enumerate() {
                        ui.label(format!("{}. {item}", i + 1));
                    }
                });
        }
        let height = (ui.available_height() - 180.).clamp(120., 480.);
        egui::ScrollArea::vertical()
            .id_salt("engineering-agent-messages")
            .max_height(height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for message in &self.messages {
                    ui.add_space(6.);
                    ui.label(
                        RichText::new(if message.role == "user" {
                            text(lang, "你", "You")
                        } else {
                            "AI"
                        })
                        .size(12.)
                        .strong()
                        .color(t.text_muted),
                    );
                    message.markdown.ui(ui, t);
                    if message.role == "assistant"
                        && ui.small_button(text(lang, "复制", "Copy")).clicked()
                    {
                        ctx.copy_text(message.text.clone());
                    }
                }
            });
        if !self.steps.is_empty() {
            egui::CollapsingHeader::new(format!("执行记录与证据 · {}项", self.steps.len()))
                .default_open(self.approval.is_some())
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(200.)
                        .id_salt("engineering-agent-evidence")
                        .show(ui, |ui| {
                            for step in self.steps.iter().filter(|s|self.context.as_ref().is_some_and(|c|c.project==s.scope.project)) {
                                egui::CollapsingHeader::new(format!(
                                    "{} · {} · {} · {:.1}s",
                                    step.id, if matches!(lang,Lang::Zh){step.title.as_str()}else{step.call.method.as_str()}, step.status, step.elapsed
                                ))
                                .show(ui, |ui| {
                                    ui.label(format!("{} · {} · {}",step.call.method,step.scope.page,step.scope.project));
                                    ui.weak(format!("任务 {}",step.task));
                                    ui.label(step.call.params.to_string());
                                    let data=&step.data["data"];
                                    if let Some(unit)=data["unit"].as_str(){ui.weak(format!("单位：{unit}"));}
                                    if let Some(n)=data["sample_count"].as_u64(){ui.label(format!("有效范围样本：{n}"));}
                                    if let Some(v)=data["quality_complete"].as_bool(){ui.label(if v {text(lang,"当前范围数据完整","Complete within this scope")}else{text(lang,"数据不完整，请检查质量说明","Incomplete data; inspect quality notes")});}
                                    if data["coverage"].is_object(){ui.weak(format!("扫描范围：{}",data["coverage"]));}
                                    if !data["continuation"].is_null(){ui.weak(text(lang,"扫描尚未结束；可用续扫游标继续，当前结果不能代表全部日志。","Scan incomplete. Continue with the cursor; this is not a full-log conclusion."));}
                                    if ui.small_button(text(lang,"定位来源页面","Open source page")).clicked(){self.navigation=Some(step.scope.clone());}
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(
                                                serde_json::to_string_pretty(&step.data)
                                                    .unwrap_or_default(),
                                            )
                                            .monospace(),
                                        )
                                        .wrap()
                                        .selectable(true),
                                    );
                                });
                            }
                        });
                });
        }
        if let Some(call) = self.approval.clone() {
            Frame::NONE
                .fill(t.accent_soft)
                .inner_margin(8)
                .corner_radius(6)
                .show(ui, |ui| {
                    let title = contract::tool(&call.method).map_or("操作", |tool| tool.title);
                    ui.strong(format!("待确认：{title}"));
                    if call.method == "testing.apply" {
                        if let Some(step) = self
                            .steps
                            .iter()
                            .rev()
                            .find(|s| s.data["data"]["draft_id"] == call.params["draft_id"])
                        {
                            ui.label(
                                serde_json::to_string_pretty(&step.data["data"]["changes"])
                                    .unwrap_or_default(),
                            );
                        }
                    }
                    ui.label(text(lang,"仅执行下列具体参数；取消等待不会自动撤回已经发送的设备动作。","Only these parameters will be executed. Cancelling the wait does not undo a dispatched device action."));
                    ui.add(
                        egui::Label::new(
                            serde_json::to_string_pretty(&call.params).unwrap_or_default(),
                        )
                        .wrap()
                        .selectable(true),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(text(lang,"确认执行本次操作","Confirm this action")).clicked() {
                            self.approval = None;
                            self.confirmed = Some(call.clone());
                            self.started = Some(Instant::now());
                            self.enqueue(call, false);
                        }
                        if ui.button(text(lang,"拒绝并停止任务","Reject and stop task")).clicked() {
                            self.cancel_task("已拒绝，操作未发送。");
                        }
                    });
                });
        }
        if !self.status.is_empty() {
            ui.colored_label(
                if self.error { t.warning } else { t.text_muted },
                &self.status,
            );
        }
        if self.messages.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for prompt in [
                    "检查当前数据异常，给出证据和下一步建议",
                    "汇总本页状态与缺失信息",
                    "制定测试计划，执行前先列出具体操作",
                ] {
                    if ui.small_button(prompt).clicked() {
                        self.prompt = prompt.into();
                    }
                }
            });
        }
        let editor = ui.add(
            egui::TextEdit::multiline(&mut self.prompt)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .char_limit(16000)
                .hint_text(text(
                    lang,
                    "说明测试目标、数据范围或希望执行的操作… Ctrl+Enter 发送",
                    "Describe the goal, data scope or action… Ctrl+Enter to send",
                )),
        );
        if ready
            && !self.busy()
            && editor.has_focus()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter))
        {
            self.start(service);
        }
        ui.horizontal_wrapped(|ui|{if ui.add_enabled(ready&&!self.busy()&&!self.prompt.trim().is_empty(),egui::Button::new(RichText::new(text(lang,"开始任务","Start task")).color(t.accent_text)).min_size(egui::vec2(100.,28.)).fill(t.accent_fill)).clicked(){self.start(service);}if self.busy()&&ui.button(text(lang,"停止任务","Stop waiting")).clicked(){self.cancel_task("已停止等待，已发送的硬件动作不会自动回滚。请核对设备状态。");}if !self.busy()&&ui.button(text(lang,"清空记录","Clear history")).clicked(){self.messages.clear();self.steps.clear();self.plan.clear();self.observations.clear();self.status.clear();}if !self.busy()&&!self.steps.is_empty()&&ui.button(text(lang,"导出分析记录…","Export analysis…")).clicked(){if let Some(path)=rfd::FileDialog::new().set_file_name("AI分析记录.md").save_file(){let mut report=format!("# WiParse AI分析记录\n\n工程：{}\n\n任务：{}\n\n辅助工程分析，不改写正式测试判定。\n",self.context.as_ref().map_or("",|c|c.project.as_str()),self.task);for message in &self.messages{report+=&format!("\n## {}\n\n{}\n",message.role,message.text);}for step in &self.steps{report+=&format!("\n## {} · {} · {}\n\n```json\n{}\n```\n",step.id,step.title,step.status,serde_json::to_string_pretty(&json!({"task_id":step.task,"scope":step.scope,"call":step.call,"elapsed_s":step.elapsed,"recorded_unix_ms":step.recorded_unix_ms,"result":step.data})).unwrap_or_default());}self.status=match std::fs::write(path,report){Ok(())=>"已导出分析记录。".into(),Err(e)=>format!("导出失败：{e}")};}}});
        if !ready {
            ui.weak(text(
                lang,
                "请先保存并启用AI连接设置。",
                "Save and enable the AI connection first.",
            ));
        }
    }
}

static WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) fn spawn_work(
    ctx: &Context,
    work: impl FnOnce() -> Value + Send + 'static,
) -> HostReply {
    if WORKERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 2).then_some(n + 1)
        })
        .is_err()
    {
        return HostReply::value(failure("本地分析工作线程仍在结束，请稍后重试。"));
    }
    let permit = Permit;
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    let result = std::thread::Builder::new()
        .name("wiparse-engineering-analysis".into())
        .spawn(move || {
            let _permit = permit;
            let value = work();
            let _ = tx.send(value);
            ctx.request_repaint();
        });
    match result {
        Ok(_) => HostReply::work(rx),
        Err(e) => HostReply::value(failure(e.to_string())),
    }
}
#[derive(Clone, Default)]
pub(crate) struct LogJobs(
    Arc<
        std::sync::Mutex<
            VecDeque<(
                String,
                Arc<std::sync::Mutex<wiparse_core::log_analysis::Scan>>,
            )>,
        >,
    >,
);
impl LogJobs {
    pub fn analyze(
        &self,
        ctx: &Context,
        params: Value,
        spec: wiparse_core::log_analysis::LibrarySpec,
        memory: Option<wiparse_core::log_analysis::Source>,
        cancel: Arc<AtomicBool>,
    ) -> HostReply {
        let cache = self.0.clone();
        spawn_work(ctx, move || {
            let run = || -> Result<Value, String> {
                let (id, job) = if let Some(id) = params["cursor"].as_str() {
                    let job = cache
                        .lock()
                        .map_err(|_| "LOG.CACHE")?
                        .iter()
                        .find(|(key, _)| key == id)
                        .map(|(_, job)| job.clone())
                        .ok_or("LOG.CURSOR_EXPIRED")?;
                    (id.to_owned(), job)
                } else {
                    let mut sources = wiparse_core::log_analysis::discover(&spec, &cancel)?;
                    if let Some(source) = memory {
                        sources.sources.push(source);
                    }
                    let query = serde_json::from_value(params).map_err(|e| e.to_string())?;
                    let scan = wiparse_core::log_analysis::Scan::new(sources, query)?;
                    let id = uuid::Uuid::new_v4().to_string();
                    let scan = Arc::new(std::sync::Mutex::new(scan));
                    let mut jobs = cache.lock().map_err(|_| "LOG.CACHE")?;
                    while jobs.len() >= 4 {
                        jobs.pop_front();
                    }
                    jobs.push_back((id.clone(), scan.clone()));
                    (id, scan)
                };
                let mut job = job.lock().map_err(|_| "LOG.JOB")?;
                job.run_chunk(1024 * 1024 * 1024, Duration::from_secs(15), &cancel)?;
                let mut result = job.report();
                result["continuation"] = if job.finished() {
                    Value::Null
                } else {
                    json!(id)
                };
                result["query_id"] = json!(id);
                Ok(result)
            };
            match run() {
                Ok(value) => ok(value),
                Err(error) => failure(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> ContextState {
        ContextState {
            page: "serial".into(),
            project: "fixture".into(),
            binding: "source-1".into(),
            testing: false,
        }
    }
    fn session() -> Session {
        let mut s = Session::default();
        s.set_context(context());
        s
    }
    #[test]
    fn device_approval_is_required_and_rechecked_at_dispatch() {
        let mut s = session();
        let call = Call {
            method: "serial.start".into(),
            params: json!({}),
        };
        s.enqueue(call.clone(), false);
        let request = s.take_request().unwrap();
        assert!(!s.authorize(&request, &context()));
        s.confirmed = Some(call);
        assert!(s.authorize(&request, &context()));
        let mut c = context();
        c.testing = true;
        assert!(!s.authorize(&request, &c));
        s.cancel_task("stop");
        assert!(!s.authorize(&request, &context()));
    }
    #[test]
    fn stale_source_cross_tool_and_forged_internal_requests_are_rejected() {
        let mut s = session();
        s.enqueue(
            Call {
                method: "serial.inspect".into(),
                params: json!({}),
            },
            false,
        );
        let request = s.take_request().unwrap();
        assert!(s.authorize(&request, &context()));
        let mut stale = context();
        stale.binding = "another".into();
        assert!(!s.authorize(&request, &stale));
        let mut forged = request.clone();
        forged.call.method = "instrument.result".into();
        forged.internal = true;
        assert!(!s.authorize(&forged, &context()));
        s.set_context(stale);
        assert!(!s.busy());
        s.enqueue(
            Call {
                method: "wave.channels".into(),
                params: json!({}),
            },
            false,
        );
        let r = s.take_request().unwrap();
        assert!(!s.authorize(&r, s.context.as_ref().unwrap()));
    }
    #[test]
    fn queue_ack_is_not_evidence_and_internal_completion_keeps_origin() {
        let mut s = session();
        s.confirmed = Some(Call {
            method: "serial.start".into(),
            params: json!({}),
        });
        s.enqueue(s.confirmed.clone().unwrap(), false);
        let request = s.take_request().unwrap();
        s.attach(
            &request.id,
            HostReply::value(json!({"_agent_wait":{"method":"serial.ready","params":{}}})),
        );
        assert!(s.steps.is_empty());
        assert!(s.busy());
        s.waiting.as_mut().unwrap().request.call = Call {
            method: "serial.ready".into(),
            params: json!({}),
        };
        s.accept_host(json!({"ok":true,"data":{"connected":true}}));
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.steps[0].call.method, "serial.start");
        assert_eq!(s.observations[0]["evidence_id"], "E1");
        assert!(s.resume_model);
    }
    #[test]
    fn only_owned_test_identity_can_be_stopped() {
        let mut s = session();
        s.mark_test(Some("run-1".into()));
        assert!(s.owns_test(Some("run-1")));
        assert!(!s.owns_test(Some("run-2")));
        assert!(!s.owns_test(None));
        let mut c = context();
        c.project = "other".into();
        s.set_context(c);
        assert!(!s.owns_test(Some("run-1")));
    }
    #[test]
    fn evidence_payload_is_bounded_and_partial_is_explicit() {
        let mut s = session();
        s.enqueue(
            Call {
                method: "serial.inspect".into(),
                params: json!({}),
            },
            false,
        );
        s.take_request();
        s.accept_host(json!({"ok":true,"data":"x".repeat(30000)}));
        assert!(s.observations[0]["result"]["data"]["truncated"] == true);
        assert!(s.observations[0].to_string().len() < 12000);
    }
    #[test]
    fn task_chat_renders_at_narrow_widths() {
        for width in [420., 640., 1100.] {
            let ctx = Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let t = crate::theme::apply_theme(&ctx, false);
            let mut s = session();
            s.message("user", "检查串口日志中的超时".into());
            s.message("assistant","## 分析结论\n\n**共 3 次超时** [E1]。尚不能判定硬件故障。\n\n| 指标 | 结果 |\n| --- | --- |\n| 证据覆盖 | 当前日志快照 |".into());
            s.plan = vec!["统计本地日志".into(), "依据证据定位原因".into()];
            let service = AiService::new(AiConfig::default());
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 850.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| s.ui(ui, ctx, Lang::Zh, &t, &service, false));
                },
            );
            assert!(!output.shapes.is_empty());
            if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                let mut output = output;
                output
                    .textures_delta
                    .set
                    .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                output.textures_delta.set.push((
                    egui::TextureId::Managed(0),
                    egui::epaint::ImageDelta::full(
                        ctx.fonts(|f| f.image()),
                        egui::TextureOptions::LINEAR,
                    ),
                ));
                crate::ui_audit::export_frame(
                    &ctx,
                    output,
                    &std::path::PathBuf::from(dir),
                    &format!("agent-{width:.0}"),
                    egui::vec2(width, 850.),
                );
            }
        }
    }
}

#[cfg(test)]
mod workflow_test {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn mock_provider_uses_local_evidence_then_finishes_without_uploading_raw_log() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for i in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut block = [0u8; 4096];
                let boundary = loop {
                    let n = socket.read(&mut block).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&block[..n]);
                    if let Some(at) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..boundary]);
                let length = header
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                while bytes.len() < boundary + length {
                    let n = socket.read(&mut block).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&block[..n]);
                }
                tx.send(
                    serde_json::from_slice::<Value>(&bytes[boundary..boundary + length]).unwrap(),
                )
                .unwrap();
                let answer = if i == 0 {
                    json!({"reply":"先读取本地日志统计","call":{"method":"serial.logs","params":{"contains":["timeout"]}}})
                } else {
                    json!({"reply":"## 结论\n\n发现 3 次超时 [E1]。这只是当前快照，不代表完整充电周期。","call":null})
                };
                let body=json!({"model":"mock","choices":[{"message":{"content":answer.to_string()},"finish_reason":"stop"}]}).to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let service = AiService::new(AiConfig {
            enabled: true,
            base_url: endpoint,
            model: "mock".into(),
            ..Default::default()
        });
        let mut s = Session::default();
        s.set_context(ContextState {
            page: "serial".into(),
            project: "fixture".into(),
            binding: "log1".into(),
            testing: false,
        });
        s.prompt = "检查超时".into();
        s.start(&service);
        let ctx = Context::default();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut called = 0;
        while Instant::now() < deadline {
            s.poll(&ctx, &service);
            if let Some(r) = s.take_request() {
                assert_eq!(r.call.method, "serial.logs");
                assert!(s.authorize(&r, s.context.as_ref().unwrap()));
                s.attach(&r.id,HostReply::value(json!({"ok":true,"data":{"matching_lines":3,"coverage":"retained snapshot","raw_samples_uploaded":false}})));
                called += 1;
            }
            if s.started.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(called, 1);
        assert!(s.started.is_none(), "{}", s.status);
        assert_eq!(s.steps.len(), 1);
        assert!(s.messages.last().unwrap().text.contains("[E1]"));
        let requests = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].to_string().contains("matching_lines"));
        server.join().unwrap();
    }
}

#[cfg(test)]
mod remediation_tests;
