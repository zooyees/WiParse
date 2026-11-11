//! Serial-specific conversation and bounded monitoring over the shared AI service.
mod alerts;
mod host;
mod library;
mod platform;
mod protocol;
mod run;
mod queue;
mod timeline_view;
mod view;
const MIN_INTERVAL_SECONDS: u64 = 1;
const MAX_INTERVAL_SECONDS: u64 = 300;
use super::*;
use protocol::Action;
use serde::Serialize;
use wiparse_core::ai::{AiConfig, AiMessage, AiRequest, AiTask};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Binding {
    project: String,
    source_id: u64,
    device: Option<String>,
    selected_device: Option<String>,
    baud: u32,
    capture_epoch: u64,
    live: bool,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    #[serde(flatten)]
    binding: Binding,
    source_title: String,
    status: String,
    reception_requested: bool,
    connection_confirmed: bool,
    absolute_lines: usize,
    retained_lines:usize,
    selected_bytes: usize,
    automation_locked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Analyse,
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataScope {
    Current,
    Range,
    Selection,
    Library,
    None,
}

struct Entry {
    automatic: bool,
    error: bool,
    time: String,
    role: &'static str,
    text: String,
    markdown: crate::test_report::TextMarkdown,
    timeline: Option<serde_json::Value>,
}

struct Pending {
    task: AiTask,
    preview: Option<crate::test_report::TextMarkdown>,
    binding: Binding,
    question: String,
    automatic: bool,
    history_before: usize,
    automation_granted: bool,
}

#[derive(Default, Clone, Serialize)]
struct WindowStats {
    received_lines: usize,
    error_keyword_lines: usize,
    warning_keyword_lines: usize,
    alert_examples: std::collections::VecDeque<String>,
}

struct Watch {
    pretrigger: std::collections::VecDeque<String>,
    first_trigger: Option<String>,
    observed_rows: u64,
    binding: Binding,
    goal: String,
    interval: Duration,
    next: Instant,
    cursor: usize,
    failures: u8,
    ai_paused: bool,
    stats: WindowStats,
    last_request: Instant,
    alert_waiting: bool,
}

pub(super) struct SerialAiChat {
    pub open: bool,
    recovery:Option<crate::ai::journal::Recovery>,
    panel_width: f32,
    project: String,
    config: Option<AiConfig>,
    prompt: String,
    scope: DataScope,
    range_first: u64,
    range_last: u64,
    library: library::Library,
    mode: Mode,
    watch_goal: String,
    reports: usize,
    evidence_counter: u64,
    rules: alerts::Rules,
    alert_count: usize,
    alert_last: HashMap<String, Instant>,
    notice: Option<(String, Instant)>,
    conversation_source: Option<(u64, Option<String>)>,
    allow_automation: bool,
    interval_seconds: u64,
    entries: Vec<Entry>,
    messages: std::collections::VecDeque<queue::Message>,
    queue_paused: bool,
    queue_notice: Option<String>,
    usage_total: wiparse_core::ai::usage::Usage,
    history: Vec<AiMessage>,
    pending: Option<Pending>,
    deferred: Option<(serde_json::Value,String,Snapshot)>,
    watch: Option<Watch>,
    tool: Option<platform::ToolPending>,
    tool_round: usize,
    run: Option<run::Run>,
    tool_results: Vec<serde_json::Value>,
    status: String,
    status_error: bool,
    retry: Option<String>,
    awaiting_connection: Option<(String, Instant)>,
}

impl Default for SerialAiChat {
    fn default() -> Self {
        Self {
            open: false,
            recovery:None,
            panel_width: 420.0,
            project: String::new(),
            config: None,
            prompt: String::new(),
            scope: DataScope::Current,
            range_first:1,
            range_last:1000,
            library: Default::default(),
            mode: Mode::Analyse,
            watch_goal: String::new(),
            reports: 0,
            evidence_counter: 0,
            rules: Default::default(),
            alert_count: 0,
            alert_last: HashMap::new(),
            notice: None,
            conversation_source: None,
            allow_automation: false,
            interval_seconds: 5,
            entries: Vec::new(),
            messages: Default::default(),
            queue_paused: false,
            queue_notice: None,
            usage_total: Default::default(),
            history: Vec::new(),
            pending: None,
            deferred: None,
            watch: None,
            tool: None,
            tool_round: 0,
            run: None,
            tool_results: Vec::new(),
            status: String::new(),
            status_error: false,
            retry: None,
            awaiting_connection: None,
        }
    }
}

enum UiAction {
    Send(String),
    Correct(String),
    RemoveQueued(String),
    EditQueued(String),
    ResumeQueue,
    ClearQueue,
    ChooseLogDirectory,
    ClearLogDirectory,
    PreviewLogs,
    ToggleWatch,
    ResumeAi,
    Cancel,
    Clear,
    Settings,
    Close,
}

fn label(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    if lang == Lang::Zh {
        zh
    } else {
        en
    }
}

fn bounded(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

impl SerialAiChat {
    pub(super) fn monitoring_active(&self) -> bool {
        self.watch.is_some()
    }

    fn entry(&mut self, role: &'static str, text: impl Into<String>) {
        let text = bounded(&text.into(), 1024 * 1024);
        let markdown = crate::test_report::TextMarkdown::new(&text);
        self.entries.push(Entry {
            automatic: false,
            error: false,
            time: Local::now().format("%H:%M:%S").to_string(),
            role,
            text,
            markdown,
            timeline: None,
        });
        while self.entries.len() > 40 || self.entries.iter().map(|e|e.text.len()).sum::<usize>() > 8*1024*1024 { self.entries.remove(0); }
    }

    fn note(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_error = error;
        if error { if let Some(run) = &mut self.run { run.finish("failed"); run.native_pending = None; } }
        if self.entries.last().is_some_and(|entry| {
            entry.role == "host" && entry.text == self.status && entry.error == error
        }) {
            return;
        }
        self.entry("host", self.status.clone());
        if let Some(entry) = self.entries.last_mut() {
            entry.error = error;
        }
        if self.pending.is_none() {
            self.history.push(AiMessage::User(format!(
                "Verified host result: {}",
                self.status
            )));
        }
        if self.history.len() > 12 {
            self.history.drain(..self.history.len() - 12);
        }
    }

    fn cancel(&mut self) {
        self.deferred=None;
        if let Some(run)=&mut self.run{run.finish("cancelled");run.native_pending=None;run.native_outputs.clear();run.queued.clear();}
        if platform::cancel(&mut self.tool) {
            self.allow_automation = false;self.revoke_controls();
        }
        if let Some(pending) = self.pending.take() {
            pending.task.cancel();
            if !pending.automatic {
                self.history.truncate(pending.history_before);
            }
        }
    }

    pub(super) fn close(&mut self) {
        self.cancel();
        self.queue_paused=true;
        self.watch = None;
        self.awaiting_connection = None;
        self.allow_automation = false;
        self.open = false;
    }

    fn set_context(&mut self, project: String, config: &AiConfig) {
        let project_changed = !self.project.is_empty() && self.project != project;
        let config_changed = self.config.as_ref().is_some_and(|old| old != config);
        if project_changed || config_changed {
            self.cancel();self.queue_paused=!self.messages.is_empty();
            if project_changed {
                self.watch = None;
            } else if let Some(watch) = &mut self.watch {
                watch.ai_paused = true;
            }
            self.awaiting_connection = None;
            if project_changed {
                self.entries.clear();
                self.history.clear();
            self.run=None;
                self.allow_automation = false;self.revoke_controls();
                self.conversation_source = None;
                self.library=Default::default();
                if self.scope==DataScope::Library{self.scope=DataScope::Current;}
            }
            self.note(
                if project_changed {
                    "工程已切换，AI监控已停止 / Project changed; AI monitoring stopped"
                } else {
                    "AI配置已更改，AI分析已暂停，本地监控继续 / AI settings changed; AI analysis paused; local monitoring continues"
                },
                false,
            );
        }
        self.project = project;
        self.config = Some(config.clone());
    }

    fn working(&self)->bool{self.pending.is_some()||self.tool.is_some()||self.deferred.is_some()}

    fn submit(
        &mut self,
        ctx: &egui::Context,
        state: &Snapshot,
        mut data: serde_json::Value,
        question: String,
        automatic: bool,
    ) -> Result<(), String> {
        let mut question=question;
        if !automatic {
            if let Some(run)=self.run.as_mut() {
                if let Some(correction)=run.correction.take() {
                    run.question=format!("{}\n用户修正：{}",run.question,correction);
                    run.workflow=Default::default();
                    run.workflow.bind_request(&run.question);
                    run.native_base=None;
                    run.native_rounds.clear();
                    run.native_outputs.clear();
                    run.native_pending=None;
                    run.last_read=None;
                    run.initial_evidence=None;
                    run.previous_evidence.clear();
                    run.evidence_ids.clear();
                    self.tool_results.clear();
                    data["platform_results"]=json!([]);
                    run.feedback=Some("目标已修改。重新核查受影响的结论；已发出的设备命令先读回状态。".into());
                    if let Some(j)=&run.journal{j.record(&run.id,"serial",&run.question,"revised",&run.workflow,None);}
                }
                if !run.question.is_empty(){question=run.question.clone();}
            }
        }
        if !automatic&&self.tool_round>run::MAX_CALLS{return Err("分析已达本轮工具预算；已保留证据，不会重复设备操作。".into());}
        if !automatic&&self.run.as_ref().is_some_and(|r|r.started.elapsed()>Duration::from_secs(600)){return Err("本轮分析已达工作预算；证据与覆盖范围已保留，可继续提问。".into());}
        if self.pending.is_some() || self.tool.is_some() {
            return Err("AI.BUSY: 请等待当前回复 / Wait for the current reply".into());
        }
        if question.trim().is_empty() || question.len() > 16 * 1024 {
            return Err("请输入问题（最多16 KiB）/ Enter a question (maximum 16 KiB)".into());
        }
        let service =
            crate::ai::service(ctx).ok_or("请先配置全局AI / Configure global AI first")?;
        if self.history.len() > 10 {
            self.history.drain(..self.history.len() - 10);
        }
        if !automatic&&self.scope!=DataScope::None{
            let mut library=self.library.status();
            if data["platform_results"].as_array().is_some_and(|results|results.iter().any(|result|result["method"].as_str().is_some_and(library::is_local))){library["last_analysis"]=serde_json::Value::Null;}
            data["local_log_library"]=library;
        }
        let fallback = self.run.as_ref().is_some_and(|r|r.json_fallback);
        let native = !automatic && !fallback && self.config.as_ref().is_some_and(wiparse_core::ai::context::native);
        let tools=if native { String::new() } else if !automatic && self.scope!=DataScope::None{
            format!("{}\n{}",platform::catalog(),library::catalog())
        }else{platform::catalog().into()};
        if !automatic {
            if let Some(run)=&self.run{data["workflow"]=run.workflow.state();data["evidence_index"]=run.workflow.index();data["initial_evidence"]=json!(run.initial_evidence);data["previous_task_evidence"]=json!({"historical":true,"same_scope":true,"results":run.previous_evidence});data["task"]=json!({"id":run.id,"phase":run.phase,"calls":self.tool_round,"max_calls":run::MAX_CALLS,"protocol_feedback":run.feedback});if native && !run.native_rounds.is_empty(){data["platform_results"]=json!({"provided_as_native_tool_messages":true});}}
        }
        let mut request=AiRequest {
            purpose: if automatic { format!("{}\nDevice control allowed: {}\n{}",wiparse_core::ai::context::MONITOR,self.control_allowed(state),tools) } else {
                format!("{}\n{}\n{}\n{}",if native { format!("{} Native mode permits up to four independent read-only calls; device, monitor and management actions must be separate. Use agent.help before calling unfamiliar methods. Serial device control allowed: {}. Monitor interval 1..300 seconds (default 5); local alert rules remain immediate. Serial view/search/jump are reversible view operations; inspect view_id first.",wiparse_core::ai::context::NATIVE,self.control_allowed(state)) } else {protocol::contract(self.control_allowed(state))},tools,wiparse_core::ai::workflow::GUIDANCE,wiparse_core::ai::workflow::profile("serial"))
            },
            text: serde_json::to_string(&json!({"user_request":question,"source":if automatic {json!(state)}else{json!({"source_id":state.binding.source_id,"title":state.source_title,"kind":if state.binding.live{"live_retained_snapshot"}else{"offline_log"},"available_rows":state.absolute_lines,"retained_rows":state.retained_lines,"device_control_allowed":self.control_allowed(state)})},"ai_monitoring":self.watch.is_some(),"observation":data})).map_err(|e|e.to_string())?,
        };
        let mut history=if automatic{Vec::new()}else{self.history.clone()};
        wiparse_core::ai::context::deduplicate_current(&mut history,&question);
        let mut host_context=None;
        if native {
            if let Some(run)=&mut self.run {if !run.native_rounds.is_empty(){if let Ok(mut payload)=serde_json::from_str::<serde_json::Value>(&request.text){payload["observation"]["initial_evidence"]=json!({"details":"native tool messages or agent.evidence"});request.text=payload.to_string();}}let (base,base_history)=run.native_base.get_or_insert_with(||(request.text.clone(),history.clone()));if request.text!=*base{let mut update=serde_json::from_str::<serde_json::Value>(&request.text).unwrap();update.as_object_mut().unwrap().remove("user_request");update["observation"]["previous_task_evidence"]=json!({"details":"initial task context or agent.evidence"});host_context=Some(update.to_string());}request.text=base.clone();history=base_history.clone();}
        }
        let submitted=if automatic || !native{service.submit_conversation(request,history)}else{service.submit_agent(request,history,wiparse_core::ai::tools::Conversation{host_context,functions:run::functions(self.scope,self.control_allowed(state)),rounds:self.run.as_ref().map_or_else(Vec::new,|r|r.native_rounds.clone())})};
        let task=match submitted{Ok(task)=>task,Err(wiparse_core::ai::AiError::Busy) if !automatic&&self.run.is_some()=>{self.deferred=Some((data,question,state.clone()));self.status="等待AI资源，任务将自动继续… / Waiting for AI capacity…".into();ctx.request_repaint_after(Duration::from_millis(100));return Ok(());},Err(error)=>return Err(error.to_string())};
        let history_before = self.history.len();
        if !automatic && self.tool_round==0 && self.run.is_none() {
            self.history.push(AiMessage::User(bounded(&question, 8000)));
        }
        if !automatic && self.tool_round==0 && self.run.is_none() {
            self.entry("user", question.clone());
        }
        self.pending = Some(Pending {
            preview: None,
            task,
            binding: state.binding.clone(),
            question,
            automatic,
            history_before,
            automation_granted: self.control_allowed(state),
        });
        self.status = "正在分析 / Analysing…".into();
        self.status_error = false;
        self.retry = None;
        ctx.request_repaint_after(Duration::from_millis(100));
        Ok(())
    }

    pub(super) fn observe(&mut self, source: u64, lines: &[String]) {
        self.observe_rules(source, lines);
        let Some(watch) = self
            .watch
            .as_mut()
            .filter(|watch| watch.binding.source_id == source)
        else {
            return;
        };
        watch.stats.received_lines = watch.stats.received_lines.saturating_add(lines.len());
        for line in lines {
            let sample = bounded(line, 600);
            let lower = sample.to_ascii_lowercase();
            let error = [
                "error",
                "fail",
                "fault",
                "panic",
                "exception",
                "timeout",
                "错误",
                "异常",
                "故障",
            ]
            .iter()
            .any(|word| lower.contains(word));
            if error {
                watch.stats.error_keyword_lines += 1;
                if watch.stats.alert_examples.len() == 12 {
                    watch.stats.alert_examples.pop_front();
                }
                watch.stats.alert_examples.push_back(sample);
            } else if lower.contains("warn") || lower.contains("警告") {
                watch.stats.warning_keyword_lines += 1;
            }
        }
    }

    pub(super) fn split(
        &mut self,
        ui: &mut egui::Ui,
        full: egui::Rect,
        t: &Tokens,
    ) -> (egui::Rect, Option<egui::Rect>) {
        if !self.open {
            return (full, None);
        }
        let gap = 4.0;
        let max = (full.width() - 300.0 - gap)
            .max(160.0)
            .min(full.width() * 0.55);
        let width = self.panel_width.clamp(280.0, 720.0).min(max).max(1.0);
        let right =
            egui::Rect::from_min_max(egui::pos2(full.right() - width, full.top()), full.max);
        let main = egui::Rect::from_min_max(
            full.min,
            egui::pos2((right.left() - gap).max(full.left()), full.bottom()),
        );
        let drag = egui::Rect::from_min_max(
            egui::pos2(main.right() + 2.0, full.top()),
            egui::pos2(right.left() - 2.0, full.bottom()),
        );
        let response = ui
            .interact(drag, ui.id().with("serial-ai-resize"), egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            self.panel_width = (width - ui.input(|i| i.pointer.delta().x)).clamp(280.0, 720.0);
            ui.ctx().request_repaint();
        }
        if response.hovered() || response.dragged() {
            ui.painter()
                .vline(drag.center().x, drag.y_range(), Stroke::new(1.0, t.accent));
        }
        (main, Some(right))
    }
}

#[cfg(test)]
mod platform_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;

#[cfg(test)]
mod responsiveness_tests;

#[cfg(test)]
mod agent_workflow_tests;
#[cfg(test)]
mod queue_tests;
