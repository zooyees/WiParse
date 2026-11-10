use super::*;

impl SerialToolPanel {

    pub(super) fn ai_state(&self, project: &str, testing: bool) -> Snapshot {
        let tab = self.tabs.get(self.active_tab);
        let (selected, selected_baud) = self.current_port_baud();
        let live = tab.is_some_and(|tab| tab.live);
        let device = live
            .then(|| {
                self.port_for_tab(self.active_tab)
                    .map(str::to_owned)
                    .or_else(|| selected.clone())
            })
            .flatten();
        let requested = device
            .as_deref()
            .is_some_and(|port| self.device_is_open(port));
        let confirmed = device
            .as_ref()
            .and_then(|port| self.ai_confirmed.get(port))
            .copied();
        let status = if !live {
            "离线日志 / Offline log".into()
        } else if confirmed.is_some() {
            "已连接 / Connected".into()
        } else if let Some(error) = device
            .as_ref()
            .and_then(|port| self.ai_serial_error.get(port))
        {
            error.clone()
        } else if confirmed.is_some() {
            "已连接 / Connected".into()
        } else if requested {
            "正在连接 / Connecting".into()
        } else {
            "已停止 / Stopped".into()
        };
        Snapshot {
            binding: Binding {
                project: project.into(),
                source_id: tab.map_or(0, LogTabPage::ai_view_id),
                device,
                selected_device: selected,
                baud: confirmed.unwrap_or(selected_baud),
                capture_epoch: self.ai_epoch,
                live: tab.is_some_and(|tab| tab.live),
            },
            source_title: tab.map(|tab| tab.display_title()).unwrap_or_default(),
            status,
            reception_requested: requested,
            connection_confirmed: confirmed.is_some(),
            absolute_lines: tab.map_or(0, |tab| tab.dropped().saturating_add(tab.line_count())),
            retained_lines:tab.map_or(0,|tab|tab.line_count()),
            selected_bytes: tab.map_or(0, |tab| tab.ai_selection().len()),
            automation_locked: self.ai_controls_locked(testing),
        }
    }

    pub(super) fn ai_observation(
        &self,
        include: bool,
        since: Option<usize>,
        stats: Option<&WindowStats>,
    ) -> serde_json::Value {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return json!({"data_included":false});
        };
        let end = tab.dropped().saturating_add(tab.line_count());
        let requested_start = since.unwrap_or_else(|| end.saturating_sub(100));
        let start = requested_start
            .max(tab.dropped())
            .max(end.saturating_sub(100))
            .min(end);
        let lines = if include {
            tab.lines_slice(start.saturating_sub(tab.dropped()), end - start)
                .iter()
                .map(|line| bounded(line, 400))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let selected = if include && since.is_none() {
            bounded(&tab.ai_selection(), 16 * 1024)
        } else {
            String::new()
        };
        json!({"data_included":include,"sampling":"at most 100 recent lines, 400 UTF-8 bytes each; selected text at most 16 KiB; not exhaustive",
            "new_line_count":end.saturating_sub(requested_start),"omitted_lines":if include {start.saturating_sub(requested_start)} else {end.saturating_sub(requested_start)},
            "recent_lines":lines,"selected_text":selected,"keyword_statistics":if include { stats } else { None },
            "keyword_counts_are_hints_not_test_verdicts":true,"unsaved_editor_changes_included":false})
    }

    pub(super) fn ai_scoped_observation(
        &self,
        scope: DataScope,
    ) -> Result<serde_json::Value, String> {
        let mut data = self.ai_observation(false, None, None);
        data["sampling"]=json!("Manual analysis uses the local engine across the selected scope, not a recent-100-lines sample. Raw logs are not attached. Request log.files / log.analyze for evidence.");
        data["data_scope"] = json!(match scope {
            DataScope::Current => "current_log",
            DataScope::Range => "current_log_row_range",
            DataScope::Selection => "selection",
            DataScope::None => "none",
            DataScope::Library => "local_library",
        });
        if scope==DataScope::Selection && self.tabs.get(self.active_tab).is_none_or(|tab|tab.ai_selection().is_empty()) {
            return Err("请先在日志中选中要分析的内容 / Select log text to analyse first".into());
        }
        if let Some(tab)=self.tabs.get(self.active_tab){data["available_rows"]=json!({"retained":tab.line_count(),"evicted":tab.dropped(),"live_snapshot":tab.live,"disk_file":tab.filepath.is_some()});}
        Ok(data)
    }

    pub(crate) fn tick_ai(
        &mut self,
        ctx: &egui::Context,
        lang: Lang,
        project: String,
        config: &AiConfig,
        testing: bool,
    ) {
        self.ai_test_locked = testing;
        let mut chat = std::mem::take(&mut self.ai_chat);
        chat.set_context(project.clone(), config);
        if !chat.open {
            self.ai_chat = chat;
            return;
        }
        let state = self.ai_state(&project, testing);
        let source = (state.binding.source_id, state.binding.device.clone());
        if chat
            .conversation_source
            .as_ref()
            .is_some_and(|old| old != &source)
        {
            chat.cancel();
            chat.watch = None;
            chat.awaiting_connection = None;
            chat.entries.clear();
            chat.history.clear();
            chat.retry = None;
            chat.allow_automation = false;
            chat.reports = 0;
            chat.rules = Default::default();
            chat.alert_count = 0;
            chat.alert_last.clear();
            chat.notice = None;
            chat.note("数据源已切换，已开启新对话；控制权限已关闭 / Source changed; new conversation started and controls disabled",false);
        }
        chat.conversation_source = Some(source);
        if chat
            .pending
            .as_ref()
            .is_some_and(|pending| pending.binding != state.binding)
        {
            chat.cancel();
            chat.note("页面或串口会话已变化，旧请求已取消 / Source or session changed; old request cancelled",false);
        }
        if chat
            .watch
            .as_ref()
            .is_some_and(|watch| watch.binding != state.binding || !state.reception_requested)
        {
            chat.watch = None;
            chat.note("目标已变化或接收已停止，AI监控结束 / Source changed or reception stopped; AI watch ended",false);
        }
        if let Some((port, deadline)) = chat.awaiting_connection.clone() {
            if self.ai_confirmed.contains_key(&port) {
                chat.awaiting_connection = None;
                chat.note(
                    format!("串口连接已确认 / Connection confirmed: {port}"),
                    false,
                );
            } else if !self.device_is_open(&port) || Instant::now() >= deadline {
                chat.awaiting_connection = None;
                chat.note(
                    format!(
                        "串口连接未成功 / Connection not confirmed: {port} · {}",
                        self.ai_serial_error
                            .get(&port)
                            .map(String::as_str)
                            .unwrap_or("timeout or port closed")
                    ),
                    true,
                );
            }
        }
        let result =
            chat.pending
                .as_ref()
                .and_then(|pending| match pending.task.result.try_recv() {
                    Ok(reply) => Some(reply),
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        Some(Err(wiparse_core::ai::AiError::Network))
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => None,
                });
        if let Some(result) = result {
            let pending = chat.pending.take().unwrap();
            match result {
                Ok(reply) => {
                    let mut decoded = protocol::parse(&reply.text);
                    if reply.finish_reason.as_deref() == Some("length") {
                        decoded.actions.clear();
                        decoded.error = Some("回复达到输出上限，仅显示已有内容；未执行AI动作 / Output limit reached; no AI actions executed".into());
                    }
                    chat.entry("assistant", decoded.reply.clone());
                    if pending.automatic {
                        chat.reports += 1;
                        if let Some(entry) = chat.entries.last_mut() {
                            entry.automatic = true;
                        }
                    } else {
                        chat.history
                            .push(AiMessage::Assistant(bounded(&decoded.reply, 8000)));
                    }
                    chat.status = format!(
                        "{} · {}",
                        reply.model,
                        label(lang, "分析完成", "Analysis complete")
                    );
                    chat.status_error = false;
                    if let Some(watch) = &mut chat.watch {
                        watch.failures = 0;
                        watch.next = if watch.alert_waiting {
                            (Instant::now() + Duration::from_secs(1))
                                .max(watch.last_request + Duration::from_secs(MIN_INTERVAL_SECONDS))
                        } else {
                            (watch.last_request + watch.interval).max(Instant::now())
                        };
                    }
                    if let Some(error) = decoded.error {
                        chat.note(error, true);
                    } else if !decoded.actions.is_empty() {
                        let current = self.ai_state(&project, testing);
                        if current.binding != pending.binding {
                            chat.note(
                                "目标已变化，未执行AI动作 / Source changed; actions blocked",
                                true,
                            );
                        } else {
                            for action in decoded.actions {
                                if pending.automatic
                                    && matches!(action, Action::MonitorConfigure { .. })
                                {
                                    chat.note("AI监控不能自行更改用户预警规则 / Automatic monitoring cannot change user alert rules",true);
                                    break;
                                }
                                let controls = match &action {
                                    Action::SerialStart | Action::SerialStop => true,
                                    Action::Platform { method, params } => {
                                        platform::validate(method, params).unwrap_or(true)
                                    }
                                    _ => false,
                                };
                                if controls
                                    && (!pending.automation_granted
                                        || !chat.allow_automation
                                        || current.automation_locked)
                                {
                                    chat.note("设备控制未授权或测试运行中 / Device control disabled or test running",true);
                                    break;
                                }
                                if let Action::Platform { method, params } = action {
                                    if let Err(error) = self.begin_platform_call(
                                        ctx,
                                        &mut chat,
                                        method,
                                        params,
                                        pending.question.clone(),
                                        pending.automatic,
                                    ) {
                                        chat.note(error, true);
                                    }
                                } else if let Err(error) = self.apply_ai_action(
                                    &mut chat,
                                    action,
                                    &pending.question,
                                    lang,
                                    testing,
                                ) {
                                    chat.note(error, true);
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    if !pending.automatic {
                        chat.history.truncate(pending.history_before);
                    }
                    chat.retry = (!pending.automatic).then_some(pending.question);
                    chat.note(error.to_string(), true);
                    if let Some(watch) = chat.watch.as_mut().filter(|_| pending.automatic) {
                        watch.failures = watch.failures.saturating_add(1);
                        watch.next = Instant::now()
                            + watch
                                .interval
                                .max(Duration::from_secs(30 * (1u64 << watch.failures.min(3))));
                        if watch.failures >= 3 {
                            watch.ai_paused = true;
                            chat.note("AI连续失败，已暂停云端分析，本地预警继续 / AI analysis paused after repeated failures; local alerts continue",true);
                        }
                    }
                }
            }
        }
        self.poll_platform(ctx, &mut chat, lang);
        if chat.pending.is_none()
            && chat.tool.is_none()
            && chat
                .watch
                .as_ref()
                .is_some_and(|watch| !watch.ai_paused && Instant::now() >= watch.next)
        {
            let state = self.ai_state(&project, testing);
            let watch = chat.watch.as_ref().unwrap();
            if state.connection_confirmed && state.absolute_lines > watch.cursor {
                let cursor = watch.cursor;
                let question = watch.goal.clone();
                let stats = watch.stats.clone();
                let data = self.ai_observation(true, Some(cursor), Some(&stats));
                chat.tool_round = 0;
                chat.tool_results.clear();
                match chat.submit(ctx, &state, data, question, true) {
                    Ok(()) => {
                        if let Some(watch) = &mut chat.watch {
                            watch.last_request = Instant::now();
                            watch.alert_waiting = false;
                            watch.cursor = state.absolute_lines;
                            watch.stats = WindowStats::default();
                        }
                    }
                    Err(error) => {
                        chat.status = error;
                        chat.status_error = true;
                        if let Some(watch) = &mut chat.watch {
                            watch.next = Instant::now() + watch.interval;
                        }
                    }
                }
            } else if let Some(watch) = &mut chat.watch {
                watch.next = Instant::now() + watch.interval;
            }
        }
        if chat.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(watch) = &chat.watch {
            if watch.ai_paused {
                ctx.request_repaint_after(Duration::from_secs(1));
            } else {
                ctx.request_repaint_after(
                    watch
                        .next
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(1)),
                );
            }
        }
        if chat.awaiting_connection.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.ai_chat = chat;
    }

    pub(super) fn begin_ai_watch(
        &mut self,
        chat: &mut SerialAiChat,
        goal: String,
        testing: bool,
    ) -> Result<(), String> {
        if chat.watch.is_some() {
            return Ok(());
        }
        if !chat.ready() {
            return Err(
                "请先配置并启用 AI 服务 / Configure and enable the AI service first".into(),
            );
        }
        let state = self.ai_state(&chat.project, testing);
        if !state.binding.live || !state.reception_requested {
            return Err("请先启动实时串口接收 / Start live serial reception first".into());
        }
        let interval = Duration::from_secs(
            chat.interval_seconds
                .clamp(MIN_INTERVAL_SECONDS, MAX_INTERVAL_SECONDS),
        );
        chat.reports = 0;
        chat.watch = Some(Watch {
            binding: state.binding,
            goal,
            interval,
            next: Instant::now() + interval,
            cursor: state.absolute_lines,
            failures: 0,
            ai_paused: false,
            stats: WindowStats::default(),
            last_request: Instant::now() - Duration::from_secs(MIN_INTERVAL_SECONDS),
            alert_waiting: false,
        });
        chat.note("AI监控已开启，等待新增数据（有界定时采样）/ AI watch enabled; waiting for new data (bounded periodic samples)",false);
        Ok(())
    }

    pub(super) fn apply_ai_action(
        &mut self,
        chat: &mut SerialAiChat,
        action: Action,
        goal: &str,
        lang: Lang,
        testing: bool,
    ) -> Result<(), String> {
        let state = self.ai_state(&chat.project, testing);
        if matches!(action, Action::SerialStart | Action::SerialStop)
            && (!chat.allow_automation || !state.binding.live || state.automation_locked)
        {
            return Err("AI.CONTROL_LOCKED: 当前目标不允许自动化 / Automation not permitted for this source".into());
        }
        match action {
            Action::MonitorConfigure {
                goal,
                interval_seconds,
                contains,
                limits,
            } => {
                alerts::validate(&contains, &limits)?;
                if interval_seconds
                    .is_some_and(|v| !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&v))
                    || goal.as_ref().is_some_and(|s| s.len() > 8192)
                {
                    return Err("AI.RULE_INVALID".into());
                }
                if let Some(goal) = goal {
                    chat.watch_goal = goal.clone();
                    if let Some(w) = &mut chat.watch {
                        w.goal = goal;
                    }
                }
                if let Some(interval) = interval_seconds {
                    chat.interval_seconds = interval;
                    if let Some(w) = &mut chat.watch {
                        w.interval = Duration::from_secs(interval);
                        w.next = (w.last_request + w.interval).max(Instant::now());
                    }
                }
                if let Some(contains) = contains {
                    chat.rules.contains = contains;
                }
                if let Some(limits) = limits {
                    chat.rules.limits = limits;
                }
                chat.alert_last.clear();
                chat.note(
                    "监控目标与预警规则已更新 / Monitoring goal and alert rules updated",
                    false,
                );
            }
            Action::Platform { .. } => {
                return Err("Platform calls require the asynchronous dispatcher".into())
            }
            Action::SerialStart => {
                let port = state
                    .binding
                    .device
                    .ok_or("未选择串口 / No port selected")?;
                if self.device_is_open(&port) {
                    chat.note(
                        format!("目标已在接收或连接中 / Already receiving or connecting: {port}"),
                        false,
                    );
                } else {
                    self.api_start_monitor(lang, Some(port.clone()), Some(state.binding.baud))?;
                    chat.awaiting_connection =
                        Some((port.clone(), Instant::now() + Duration::from_secs(15)));
                    chat.note(format!("已提交启动接收，等待连接确认 / Reception requested; waiting for connection: {port}"),false);
                }
            }
            Action::SerialStop => {
                let port = state
                    .binding
                    .device
                    .ok_or("未选择串口 / No port selected")?;
                self.api_stop_device(lang, &port);
                chat.awaiting_connection = None;
                chat.watch = None;
                chat.note(format!("已停止目标串口接收；后台释放端口 / Reception stopped; port release is asynchronous: {port}"),false);
            }
            Action::MonitorStart { interval_seconds } => {
                if interval_seconds
                    .is_some_and(|v| !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&v))
                {
                    return Err("AI.INTERVAL_INVALID: use 1..300 seconds".into());
                }
                if let Some(interval) = interval_seconds {
                    chat.interval_seconds = interval;
                }
                if let Some(watch) = &mut chat.watch {
                    watch.interval = Duration::from_secs(chat.interval_seconds);
                    watch.ai_paused = false;
                    watch.failures = 0;
                    watch.next = Instant::now()
                        .max(watch.last_request + Duration::from_secs(MIN_INTERVAL_SECONDS));
                }
                if chat.watch.is_none() {
                    if let Some(interval) = interval_seconds {
                        chat.interval_seconds = interval;
                    }
                }
                let goal = if chat.watch_goal.trim().is_empty() {
                    goal.into()
                } else {
                    chat.watch_goal.clone()
                };
                self.begin_ai_watch(chat, goal, testing)?;
            }
            Action::MonitorStop => {
                chat.watch = None;
                chat.note("AI监控已停止，串口接收保持原状态 / AI watch stopped; serial reception unchanged",false);
            }
        }
        Ok(())
    }

    pub(in crate::serial_tool) fn paint_ai_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        lang: Lang,
        t: &Tokens,
    ) {
        let mut chat = std::mem::take(&mut self.ai_chat);
        let state = self.ai_state(&chat.project, self.ai_test_locked);
        let mut events = Vec::new();
        ui_theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                Frame::NONE
                    .fill(t.surface_bg)
                    .stroke(Stroke::new(1.0, t.divider))
                    .corner_radius(ui_theme::RADIUS_CARD)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width((rect.width() - 20.0).max(1.0));
                        ui.set_min_height((rect.height() - 20.0).max(1.0));
                        events = chat.ui(ui, lang, t, &state);
                    });
            },
        );
        for event in events {
            match event {
                UiAction::Close => chat.close(),
                UiAction::Cancel => {
                    if chat.pending.as_ref().is_some_and(|p| p.automatic)
                        || chat.tool.as_ref().is_some_and(|t| t.automatic)
                    {
                        chat.watch = None;
                    }
                    chat.cancel();
                    chat.status = "已取消等待 / Cancelled".into();
                }
                UiAction::Settings => self.ai_settings_requested = true,
                UiAction::Clear => {
                    chat.cancel();
                    chat.library.reset();
                    chat.tool_results.clear();
                    chat.watch = None;
                    chat.entries.clear();
                    chat.history.clear();
                    chat.retry = None;
                    chat.status.clear();
                    chat.prompt.clear();
                    chat.reports = 0;
                    chat.allow_automation = false;
                    chat.awaiting_connection = None;
                }
                UiAction::ResumeAi => {
                    if chat.ready() {
                        if let Some(watch) = &mut chat.watch {
                            watch.ai_paused = false;
                            watch.failures = 0;
                            watch.next = Instant::now();
                        }
                        chat.note("AI分析已恢复，本地监控保持运行 / AI analysis resumed; local monitoring remains active",false);
                    }
                }
                UiAction::ToggleWatch => {
                    if chat.watch.take().is_some() {
                        chat.note("AI监控已停止 / AI watch stopped", false);
                    } else {
                        let goal = if chat.watch_goal.trim().is_empty() {
                            label(lang,"监控新增串口数据，识别异常并给出依据和建议。","Monitor new serial data for anomalies; explain evidence and next steps.").into()
                        } else {
                            chat.watch_goal.clone()
                        };
                        if let Err(error) =
                            self.begin_ai_watch(&mut chat, goal, self.ai_test_locked)
                        {
                            chat.note(error, true);
                        }
                    }
                }
                UiAction::ChooseLogDirectory=>{
                    if let Some(root)=rfd::FileDialog::new().pick_folder(){chat.cancel();chat.library.directory=Some(root);chat.library.reset();chat.scope=DataScope::Library;chat.tool_results.clear();chat.history.clear();chat.note(label(lang,"已更新本地日志目录，后续只发送有限统计和证据。","Local log directory updated. Only bounded statistics and evidence are sent."),false);}
                }
                UiAction::ClearLogDirectory=>{chat.cancel();chat.library.directory=None;chat.library.reset();chat.tool_results.clear();chat.history.clear();}
                UiAction::PreviewLogs=>{
                    if chat.pending.is_none()&&chat.tool.is_none(){
                        chat.tool_round=0;chat.tool_results.clear();chat.library.bind_scope(self.local_log_scope_key(&chat));let params=chat.library.preview_params();
                        match self.begin_platform_call(ui.ctx(),&mut chat,"log.analyze".into(),params,"本地日志预分析".into(),false){
                            Ok(())=>{if let Some(tool)=chat.tool.as_mut(){tool.local_only=true;}},Err(error)=>chat.note(error,true),
                        }
                    }
                }
                UiAction::Send(question) => {
                    if chat.pending.is_some() || chat.tool.is_some() {
                        continue;
                    }
                    chat.tool_round = 0;
                    chat.tool_results.clear();
                    chat.library.bind_scope(self.local_log_scope_key(&chat));
                    let mut data = match self.ai_scoped_observation(chat.scope) {
                        Ok(data) => data,
                        Err(error) => {
                            chat.note(error, true);
                            continue;
                        }
                    };
                    if chat.scope==DataScope::Range{data["requested_rows"]=json!({"first":chat.range_first,"last":chat.range_last});}
                    match chat.submit(ui.ctx(), &state, data, question, false) {
                        Ok(()) => chat.prompt.clear(),
                        Err(error) => {
                            chat.status = error;
                            chat.status_error = true;
                        }
                    }
                }
            }
            ui.ctx().request_repaint();
        }
        self.ai_chat = chat;
    }

    pub(crate) fn take_ai_settings_request(&mut self) -> bool {
        std::mem::take(&mut self.ai_settings_requested)
    }
    pub(crate) fn open_ai_window(&mut self) {
        self.ai_chat.open = true;
    }
}

impl SerialToolPanel {
    pub(crate) fn ai_controls_locked(&self, integrated_test: bool) -> bool { integrated_test || self.live_test.is_some() }

 pub(crate) fn agent_identity(&self,project:&str)->String{serde_json::to_string(&self.ai_state(project,self.ai_test_locked).binding).unwrap_or_default()}
 pub(crate) fn agent_inspect(&self,project:&str)->serde_json::Value{let state=self.ai_state(project,self.ai_test_locked);json!({"source_id":state.binding.source_id,"live":state.binding.live,"status":state.status,"requested":state.reception_requested,"connected":state.connection_confirmed,"total_received_rows":state.absolute_lines,"retained_rows":state.retained_lines,"evicted_rows":state.absolute_lines.saturating_sub(state.retained_lines),"baud":state.binding.baud,"source_title":state.source_title})}
 pub(crate) fn agent_log_input(&self)->Result<(wiparse_core::log_analysis::LibrarySpec,Option<wiparse_core::log_analysis::Source>),String>{
  let tab=self.tabs.get(self.active_tab).ok_or("请先打开或接收串口日志")?;
  if !tab.live{if let Some(path)=&tab.filepath{return Ok((wiparse_core::log_analysis::LibrarySpec{directory:None,files:vec![PathBuf::from(path)]},None));}}
  Ok((Default::default(),Some(wiparse_core::log_analysis::Source::memory(&tab.display_title(),tab.ai_shared_lines(),tab.dropped() as u64+1,tab.dropped() as u64))))
 }
 pub(crate) fn agent_control(&mut self,project:&str,start:bool,lang:Lang,testing:bool)->Result<serde_json::Value,String>{
  let mut chat=SerialAiChat::default();chat.project=project.into();chat.allow_automation=true;
  self.apply_ai_action(&mut chat,if start{Action::SerialStart}else{Action::SerialStop},"工程智能任务",lang,testing)?;
  Ok(self.agent_inspect(project))
 }
 pub(crate) fn validate_agent_instrument(method:&str,params:&serde_json::Value)->Result<bool,String>{super::platform::validate(method,params)}
}

impl SerialToolPanel {
 pub(crate) fn agent_library(&mut self, method:&str, params:serde_json::Value, ctx:&egui::Context, cancel:Arc<AtomicBool>)->crate::ai::agent::HostReply {
  let key=self.local_log_scope_key(&self.ai_chat);self.ai_chat.library.bind_scope(key);
  let work=||->Result<Receiver<crate::backend::InvokeReply>,String>{
   let chat=&self.ai_chat;
   if chat.scope==DataScope::None{return Err("LOG.DATA_SCOPE: 请在串口AI中选择日志范围".into());}
   let mut files=Vec::new();let mut memory=None;
   if chat.scope==DataScope::Library {
    files=self.tabs.iter().filter_map(|t|t.filepath.as_ref().map(PathBuf::from)).collect();
    if self.save_live_to_disk {if let Some(p)=&self.live_file{files.push(p.clone());}}
   }else{
    let tab=self.tabs.get(self.active_tab).ok_or("LOG.NO_CURRENT_LOG")?;
    if chat.scope==DataScope::Selection {let text=tab.ai_selection();if text.is_empty(){return Err("LOG.EMPTY_SELECTION".into());}memory=Some(wiparse_core::log_analysis::Source::memory("selection",text.lines().map(Arc::<str>::from).collect(),1,0));}
    else if !tab.live&&tab.filepath.is_some(){files.push(PathBuf::from(tab.filepath.as_ref().unwrap()));}
    else{memory=Some(wiparse_core::log_analysis::Source::memory(&tab.display_title(),tab.ai_shared_lines(),tab.dropped() as u64+1,tab.dropped() as u64));}
   }
   let range=if chat.scope==DataScope::Range{Some((chat.range_first.max(1),chat.range_last.max(chat.range_first.max(1))))}else{None};
   chat.library.start_scoped(method,params,files,memory,range,chat.scope==DataScope::Library,cancel,ctx.clone())
  };
  match work(){Ok(rx)=>{let(tx,out)=crossbeam_channel::bounded(1);let ctx=ctx.clone();std::thread::spawn(move||{if let Ok(r)=rx.recv(){let _=tx.send(json!({"ok":r.ok,"data":r.data,"error":r.error}));ctx.request_repaint();}});crate::ai::agent::HostReply::work(out)},Err(e)=>crate::ai::agent::HostReply::value(crate::ai::agent::failure(e))}
 }
}

impl SerialToolPanel {
 pub(crate) fn agent_monitor(&mut self,method:&str,params:&serde_json::Value,project:&str,lang:Lang,testing:bool)->serde_json::Value{
  let mut action=params.clone();action["type"]=json!(method.strip_prefix("serial.").unwrap_or(method));
  let result=serde_json::from_value::<Action>(action).map_err(|e|e.to_string()).and_then(|action|{
   let mut chat=std::mem::take(&mut self.ai_chat);chat.project=project.into();
   let result=self.apply_ai_action(&mut chat,action,"工程智能监控",lang,testing);self.ai_chat=chat;result
  });
  match result{Ok(())=>json!({"ok":true,"data":{"monitoring":self.ai_chat.watch.is_some(),"interval_seconds":self.ai_chat.interval_seconds,"local_rules":"full retained incoming lines evaluated locally; cloud analysis is periodic, not a safety interlock","reception":self.agent_inspect(project)}}),Err(e)=>crate::ai::agent::failure(e)}
 }
}
