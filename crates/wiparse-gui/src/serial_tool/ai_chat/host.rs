use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum WatchReply { Invalid, Continue, Final }
pub(super) fn watch_reply(response: &protocol::Response) -> WatchReply {
    if response.error.is_some() || response.needs_input || (response.actions.is_empty() && (!response.finalized || response.reply.trim().is_empty())) {
        WatchReply::Invalid
    } else if response.finalized && response.actions.is_empty() { WatchReply::Final }
    else { WatchReply::Continue }
}
impl SerialAiChat {
    fn finish_budget_summary(&mut self, reply: Option<String>, needs_input: bool) {
        let Some(run) = self.run.as_mut() else { return };
        let reason = run.limit_reason(self.tool_round).unwrap_or("本次执行已结束");
        let goals = run.workflow.open_goals();
        let partial = reply.is_none() || needs_input || !goals.is_empty()
            || run.workflow.partially_completed() || run.evidence_ids.is_empty();
        let mut summary = reply.unwrap_or_else(|| {
            let mut text = format!("已保留 {} 项工具证据。", run.evidence_ids.len());
            for record in self.tool_results.iter().rev().filter(|r| r["result"]["kind"] != "task_management").take(3) {
                let result = &record["result"];
                let method = record["method"].as_str().unwrap_or("");
                let id = record["evidence_id"].as_str().unwrap_or("");
                let outcome = if result["ok"] == true { "已取得结果" } else { result["error"].as_str().unwrap_or("执行失败") };
                text.push_str(&format!("\n- {id} · {method} · {outcome}"));
                if let Some(rows) = result["data"]["coverage"]["lines_read"].as_u64() {
                    text.push_str(&format!("，已扫描 {rows} 行"));
                }
            }
            run.workflow.append_latest_timeline_ranges(&mut text);
            run.workflow.append_verified_results(&mut text);
            text
        });
        if !goals.is_empty() {
            summary.push_str(&format!("\n\n待完成：{}", goals.join("、")));
        }
        run.finish(if needs_input { "needs_input" } else if partial { "partial" } else { "completed" });
        self.history.push(AiMessage::Assistant(bounded(&summary, 8000)));
        self.entry("assistant", summary);
        self.status = format!("{reason} · {}", if partial { "部分完成" } else { "分析完成" });
        self.status_error = false;
    }
}

impl SerialToolPanel {
    // The placeholder and detached chat must share the same generation.
    // Host callbacks may mutate panel-owned state while the chat is detached.
    pub(super) fn take_ai_chat(&mut self) -> SerialAiChat {
        let mut placeholder = SerialAiChat::default();
        placeholder.monitor_lease = self.ai_chat.monitor_lease.clone();
        std::mem::replace(&mut self.ai_chat, placeholder)
    }

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
        let serial_error = device.as_ref().and_then(|port| self.ai_serial_error.get(port));
        let status = if !live {
            "离线日志 / Offline log".into()
        } else if let Some(error) = serial_error {
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
            connection_confirmed: confirmed.is_some() && serial_error.is_none(),
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
            "window":{"requested_start_zero_based":requested_start,"retained_start_zero_based":tab.dropped(),
                "sample_start_zero_based":start,"end_exclusive_zero_based":end,
                "capture_completeness":"retained host rows only; physical capture completeness not established"},
            "new_line_count":end.saturating_sub(requested_start),"omitted_lines":if include {start.saturating_sub(requested_start)} else {end.saturating_sub(requested_start)},
            "recent_lines":lines,"selected_text":selected,"keyword_statistics":if include { stats } else { None },
            "keyword_counts_are_hints_not_test_verdicts":true,"unsaved_editor_changes_included":false})
    }

    pub(super) fn ai_scoped_observation(
        &self,
        scope: DataScope,
    ) -> Result<serde_json::Value, String> {
        let mut data=json!({"analysis_mode":"local_engine","raw_logs_uploaded":false,"read_authorized_for_selected_scope":scope!=DataScope::None});
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
        if self.ai_test_locked != testing {
            // The panel's source lease survives while ai_chat is temporarily
            // moved out; its default replacement is not the active chat.
            self.serial_review_lease.clone().mutate_always(|| self.ai_test_locked = testing);
        }
        let mut chat = self.take_ai_chat();
        chat.set_context(project.clone(), config);
        if !chat.open {
            self.ai_chat = chat;
            return;
        }
        platform::poll_reviewed_stop(&mut chat, ctx);
        let state = self.ai_state(&project, testing);
        if chat.outbound_grant_binding.as_ref().is_some_and(|old| old != &state.binding) {
            chat.approved_outbound.clear();
        }
        chat.outbound_grant_binding = Some(state.binding.clone());
        if chat.review_outbound && chat.pending.as_ref().is_some_and(|pending| {
            pending.binding != state.binding || (!pending.automatic && chat.run.as_ref().and_then(|run|run.reviewed_log.as_ref()).is_some_and(|source|!chat.review_outbound || !source.is_current())) || (!pending.automatic && match &chat.run {
                Some(run) => pending.review_task_id.as_deref() != Some(run.id.as_str())
                    || pending.review_scope_key != run.scope_key
                    || run.scope_key != self.local_log_scope_key(&chat),
                None => pending.review_task_id.is_some(),
            })
        }) {
            let automatic = chat.pending.as_ref().is_some_and(|pending| pending.automatic);
            let before_dispatch = chat.pending.as_ref().is_some_and(|pending| pending.task.cancel_outbound_before_dispatch());
            chat.cancel();
            let message = if before_dispatch {"来源或任务已变化，外发请求未发送 / Source or task changed; request not sent"} else {"来源或任务已变化，已停止等待；请求可能已发送 / Source or task changed; request may have been sent"};
            if automatic { chat.note_background(message, true); } else { chat.queue_paused = true; chat.note(message, true); }
        }
        // Release stale automatic reviews promptly, even if no provider reply arrives.
        // This check is not an atomic dispatch guard; unsupported reviewed sources
        // remain blocked until all source and permission writes join a lease.
        if chat.pending.as_ref().is_some_and(|pending| pending.automatic && chat.review_outbound
            && !chat.automatic_pending_current(pending, &state)) {
            let before_dispatch = chat.pending.as_ref().is_some_and(|pending| pending.task.cancel_outbound_before_dispatch());
            chat.cancel();
            chat.note_background(if before_dispatch {"监控配置或授权已变化，外发请求未发送 / Monitor or authorization changed; request not sent"} else {"监控配置或授权已变化，请求可能已发送 / Monitor or authorization changed; request may have been sent"}, true);
        }
        if chat.control_approval.is_some() { ctx.request_repaint_after(Duration::from_secs(1)); }
        if chat.control_approval.as_ref().is_some_and(|approval| !approval.valid(&state, &chat.selected_skills,
            chat.run.as_ref().map(|run| run.id.as_str()), chat.control_allowed(&state), &chat.monitor_revision(), Instant::now())
            || approval.automatic && chat.review_outbound && chat.monitor_review.as_ref().is_none_or(|(_, sample)| !sample.is_current())
            || !approval.automatic && chat.run.as_ref().is_some_and(|run|run.scope_key!=self.local_log_scope_key(&chat))) {
            let _monitor_update = chat.monitor_lease.begin_update();
            let automatic = chat.control_approval.as_ref().is_some_and(|approval| approval.automatic);
            let same_watch=chat.control_approval.as_ref().is_some_and(|approval|approval.belongs_to_watch(chat.watch.as_ref()));
            chat.reject_control_approval("AI.APPROVAL_EXPIRED_OR_REVOKED");
            if automatic {
                if same_watch {chat.pause_watch();}
                chat.note_background("确认已失效，操作未执行 / Approval expired or revoked; action not executed", true);
            } else {
                chat.cancel(); chat.queue_paused = true;
                chat.note("确认已失效，操作未执行 / Approval expired or revoked; action not executed", true);
            }
        }
        let source = (state.binding.source_id, state.binding.device.clone());
        if chat
            .conversation_source
            .as_ref()
            .is_some_and(|old| old != &source)
        {
            let _monitor_update = chat.monitor_lease.begin_update();
            let unknown_control = platform::control_pending(&chat.tool);
            let automatic_control = chat.tool.as_ref().is_some_and(|tool| tool.automatic);
            chat.cancel();chat.run=None;chat.queue_paused=true;chat.usage_total=Default::default();
            let unknown_receipt = if unknown_control {
                if automatic_control { chat.automatic_tool_results.last().cloned() } else { chat.tool_results.last().cloned() }
            } else { None };
            chat.watch = None;
            chat.discard_stale_watch_transaction();
            chat.awaiting_connection = None;
            chat.history.clear();
            chat.tool_results.clear();
            chat.automatic_tool_results.clear();
            chat.preview_library.reset();
            chat.retry = None;
            chat.allow_automation = false;chat.revoke_controls();
            chat.reports = 0;
            chat.rules = Default::default();
            chat.alert_count = 0;
            chat.alert_last.clear();
            chat.notice = None;
            if let Some(receipt)=unknown_receipt {chat.entry("tool",format!("历史设备动作回执（来源切换前） / Previous-source device receipt\n\n```json\n{}\n```",receipt));}
            chat.note(if unknown_control {"数据源已切换；之前设备动作结果可能未知，请先核对设备状态。上方记录属于旧来源 / Source changed; previous outcome may be unknown. Earlier entries belong to the old source"} else {"数据源已切换，控制权限已关闭；上方记录属于旧来源，不用于新任务 / Source changed; controls disabled. Earlier entries are historical and excluded from new tasks"},unknown_control);
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
            let _monitor_update = chat.monitor_lease.begin_update();
            chat.watch = None;
            chat.discard_stale_watch_transaction();
            chat.note("目标已变化或接收已停止，AI监控结束 / Source changed or reception stopped; AI watch ended",false);
        }
        if let Some((port, deadline)) = chat.awaiting_connection.clone() {
            if self.ai_confirmed.contains_key(&port) && !self.ai_serial_error.contains_key(&port) {
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
        if let Some(pending) = &mut chat.pending {
            if let Some(text) = pending.task.progress.lock().ok().and_then(|mut p|p.take()) {
                pending.preview = Some(crate::test_report::TextMarkdown::new(&text));
                ctx.request_repaint();
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
            let current_task = chat.run.as_ref().map(|run| run.id.as_str());
            let stale = (!pending.automatic && chat.run.as_ref().and_then(|run|run.reviewed_log.as_ref()).is_some_and(|source|!chat.review_outbound || !source.is_current())) || pending.binding != state.binding || pending.skills != chat.selected_skills
                || pending.authorization_revoked || pending.automation_granted != chat.control_allowed(&state)
                || pending.test_control_granted != (!pending.automatic && chat.owned_test_control_allowed())
                || (!pending.automatic && (pending.review_task_id.as_deref() != current_task
                    || chat.run.as_ref().is_some_and(|run| run.scope_key != pending.review_scope_key
                        || run.scope_key != self.local_log_scope_key(&chat))))
                || (pending.automatic && (pending.watch_revision.as_deref() != Some(chat.monitor_revision().as_str())
                    || chat.review_outbound && !chat.reviewed_monitor_sources_current()
                    || chat.watch.as_ref().is_none_or(|watch| watch.binding != state.binding || watch.ai_paused || Some(&watch.id) != pending.watch_id.as_ref())));
            if stale {
                if pending.automatic {
                    if chat.watch.as_ref().is_some_and(|watch| Some(&watch.id) == pending.watch_id.as_ref() && watch.binding == pending.binding) {
                        chat.finish_watch_transaction(false);
                    } else { chat.discard_watch_transaction_for(pending.watch_id.as_deref()); }
                    chat.note_background("任务、来源或权限已变化，已放弃旧 AI 回复。",true);
                }
                else { chat.queue_paused=true; chat.note("任务、来源或权限已变化，已放弃旧 AI 回复。",true); }
                self.ai_chat=chat;
                ctx.request_repaint();
                return;
            }
            match result {
                Ok(reply) => {
                    chat.usage_total.record(&reply.usage);if !pending.automatic{if let Some(run)=&mut chat.run{run.usage.record(&reply.usage);}}
                    let mut decoded = run::decode(&reply);
                    // Final text is a valid native response; known read-confirmation loops and
                    // unfinished declared goals receive feedback instead of false completion.
                    let native_text_final=!pending.automatic && reply.tool_message.is_none() && matches!(reply.finish_reason.as_deref(),Some("stop"|"completed")) && !reply.text.trim().is_empty() && !reply.text.trim_start().starts_with(['{','[']) && !reply.text.trim_start().starts_with("```json") && decoded.error.is_none() && decoded.actions.is_empty() && serde_json::from_str::<serde_json::Value>(&reply.text).is_err() && !wiparse_core::ai::workflow::is_read_confirmation_loop(&reply.text);
                    if native_text_final { decoded.finalized=true; }
                    if !pending.automatic && protocol::standalone_explanation(&pending.question) {
                        decoded.actions.clear();
                        decoded.reply = reply.text.clone();
                        decoded.finalized = reply.tool_message.is_none() && !reply.text.trim().is_empty();
                        decoded.error = None;
                    }

                    if reply.finish_reason.as_deref() == Some("length") {
                        decoded.actions.clear();
                        decoded.error = Some("回复达到输出上限，仅显示已有内容；未执行AI动作 / Output limit reached; no AI actions executed".into());
                    }
                    if !pending.automatic && decoded.finalized {
                        if let Some(message)=&reply.tool_message{if message.calls.first().is_some_and(|c|c.name=="wiparse_answer"){
                            let p:serde_json::Value=serde_json::from_str(&message.calls[0].arguments).unwrap_or_default();
                            let valid=p["evidence_ids"].as_array().is_some_and(|ids|(decoded.needs_input||chat.scope==DataScope::None||!ids.is_empty())&&ids.iter().all(|id|id.as_str().is_some_and(|id|chat.run.as_ref().is_some_and(|run|run.workflow.citation_method(id).is_some()))));
                            if !valid{decoded.finalized=false;decoded.error=Some("AI.EVIDENCE_REFERENCE_REQUIRED: cite actual evidence IDs returned by host tools".into());}
                        }}
                    }
                    if !pending.automatic && decoded.needs_input && wiparse_core::ai::workflow::is_read_confirmation_loop(&decoded.reply){decoded.finalized=false;decoded.error=Some("AGENT.READ_ALREADY_AUTHORIZED".into());}
                    let stale=self.ai_state(&project,testing).binding!=pending.binding||!pending.automatic&&chat.run.as_ref().is_some_and(|r|r.scope_key!=self.local_log_scope_key(&chat));
                    if stale {
                        if pending.automatic { chat.discard_watch_transaction_for(pending.watch_id.as_deref()); }
                        if pending.automatic { chat.note_background("来源已变化，已放弃旧来源的 AI 回复。",true); }
                        else { chat.note("来源已变化，已放弃旧来源的 AI 回复。",true); }
                        self.ai_chat=chat;
                        ctx.request_repaint();
                        return;
                    }
                    let automatic_invalid = pending.automatic && watch_reply(&decoded) == WatchReply::Invalid;
                    let automatic_final = pending.automatic && watch_reply(&decoded) == WatchReply::Final;
                    if automatic_invalid {
                        chat.finish_watch_transaction(false);
                        if decoded.needs_input { chat.pause_watch(); }
                        decoded.actions.clear();
                    } else if automatic_final {
                        chat.finish_watch_transaction(true);
                    }
                    if !pending.automatic && decoded.finalized {
                        let validation = chat.run.as_mut().map(|run| {
                            if run.summary_only {
                                run.workflow.validate_conclusion(&decoded.reply)
                                    .and_then(|_| run.workflow.validate_plot_claim(&decoded.reply))
                            } else {
                                run.workflow.validate_reply(&decoded.reply)
                            }
                        });
                        if let Some(Err(error)) = validation {
                            decoded.finalized=false;decoded.error=Some(error);
                        }else if let Some(run)=&chat.run {run.workflow.append_verified_results(&mut decoded.reply);}
                    }
                    if !pending.automatic && chat.run.as_ref().is_some_and(|r|r.summary_only) {
                        let summary = (decoded.finalized && decoded.error.is_none() && decoded.actions.is_empty()
                            && !decoded.reply.trim().is_empty()).then_some(decoded.reply);
                        chat.finish_budget_summary(summary, decoded.needs_input);
                        self.ai_chat=chat;ctx.request_repaint();return;
                    }
                    if !pending.automatic && decoded.finalized && !decoded.needs_input
                        && protocol::requests_transition_count(&pending.question)
                        && chat.tool_results.iter().any(|v| v["method"]=="log.analyze" && v["result"]["ok"]==true)
                        && !chat.tool_results.iter().any(|v| v["method"]=="log.analyze" && (v["params"]["events"].is_object()
                            || v["result"]["data"]["state_transition_analysis"].is_object()))
                        && protocol::claims_event_count(&decoded.reply) {
                        decoded.finalized=false;
                        decoded.error=Some("AGENT.INCOMPLETE_TRANSITION_EVIDENCE: use wiparse_log_events / log.analyze.events before claiming an exact count from sampled lines".into());
                    }
                    if !pending.automatic&&decoded.finalized {
                        let mut goals=chat.run.as_ref().map_or_else(Vec::new,|r|r.workflow.open_goals());
                        let explicit_plan = protocol::requests_task_plan(&pending.question);
                        if explicit_plan && chat.run.as_ref().is_some_and(|r| r.workflow.state()["steps"].as_array().is_some_and(Vec::is_empty)) {
                            goals.push("用户要求可核验的任务计划。调用 agent.plan 记录交付、实际状态与证据。".into());
                        }
                        if !decoded.needs_input&&!goals.is_empty(){decoded.finalized=false;decoded.error=Some(format!("AGENT.UNFINISHED_OUTCOMES: {}. Complete actual work then update agent.plan, or mark genuine blockers with reasons.",goals.join("; ")));}
                    }
                    let incomplete=!pending.automatic&&!stale&&reply.finish_reason.as_deref()!=Some("length")&&(decoded.error.is_some()||decoded.actions.is_empty()&&!decoded.finalized);
                    if incomplete {
                        let reason=decoded.error.as_deref().unwrap_or("AI.NO_ACTION: working reply omitted the tool call");
                        let category=if reason.starts_with("AGENT.PLOT_") {"图表回执核对"} else {"协议诊断"};
                        chat.entry("tool",format!("{category} · {reason}\n结束标记 {:?}，工具调用 {} 个。",reply.finish_reason,reply.tool_message.as_ref().map_or(0,|m|m.calls.len())));
                        let native=chat.config.as_ref().is_some_and(wiparse_core::ai::context::native);
                        let retry=chat.run.as_mut().is_some_and(|r|r.recovery_policy.retry(native,reason));
                        if let Some(run)=chat.run.as_mut().filter(|_|retry){
                            run.repairs+=1;
                            let recovery=if reason.starts_with("AGENT.PLOT_") {"Use the actual plot receipt; correct missing extraction or report observed fields."} else {protocol::recovery(native && !run.recovery_policy.json_fallback)};
                            run.feedback=Some(format!("{recovery} Last validation: {reason}. Available evidence IDs: {:?}. Open goals: {:?}.",run.evidence_ids,run.workflow.open_goals()));
                            let mut data=self.ai_scoped_observation(chat.scope).unwrap_or_else(|_|json!({}));
                            data["platform_results"]=json!(chat.tool_results);
                            if let Err(e)=chat.submit(ctx,&self.ai_state(&project,testing),data,pending.question.clone(),false){chat.note(e,true);}
                        } else if reason.starts_with("AGENT.PLOT_") {
                            chat.finish_budget_summary(None, false);
                        }else{
                            let goals=chat.run.as_ref().map(|r|r.workflow.open_goals()).unwrap_or_default();
                            let detail=if goals.is_empty(){String::new()}else{format!("；待完成：{}",goals.join("、"))};
                            let evidence=chat.run.as_ref().map_or(0,|r|r.evidence_ids.len());
                            chat.note(format!("本次分析未完成。最后一次回复未通过校验：{reason}。活动记录有 {evidence} 项可引用证据{detail}。"),true);
                        }
                    } else {
                    let has_reply = !decoded.reply.trim().is_empty();
                    if has_reply {chat.entry("assistant", decoded.reply.clone());}
                    if pending.automatic {
                        if automatic_final { chat.reports += 1; }
                        if let Some(entry) = chat.entries.last_mut().filter(|_| has_reply) {
                            entry.automatic = true;
                        }
                    } else if decoded.finalized && !decoded.reply.trim().is_empty() {
                        chat.history
                            .push(AiMessage::Assistant(bounded(&decoded.reply, 8000)));
                    }
                    if !pending.automatic {
                        chat.status = format!(
                            "{} · {}",
                            reply.model,
                            if decoded.needs_input {label(lang,"等待补充信息","Input required")} else if !decoded.finalized {label(lang,"处理中","Working")} else if chat.run.as_ref().is_some_and(|r|r.workflow.partially_completed()){label(lang,"部分完成","Partial")}else{label(lang, "分析完成", "Analysis complete")}
                        );
                        chat.status_error = false;
                    }
                    if decoded.needs_input&&!pending.automatic&&!chat.messages.is_empty(){chat.queue_paused=true;}
                    if decoded.finalized&&!pending.automatic{if let Some(run)=&mut chat.run{run.finish(if decoded.needs_input{"needs_input"}else if run.workflow.partially_completed(){"partial"}else{"completed"});}}
                    let will_pause = automatic_invalid && chat.watch.as_ref().is_some_and(|watch| watch.failures >= 2);
                    let _monitor_update = will_pause.then(|| chat.monitor_lease.begin_update());
                    if let Some(watch) = chat.watch.as_mut().filter(|_| pending.automatic) {
                        if automatic_invalid {
                            watch.failures = watch.failures.saturating_add(1);
                            watch.next = Instant::now() + watch.interval.max(Duration::from_secs(30 * (1u64 << watch.failures.min(3))));
                            if watch.failures >= 3 { watch.ai_paused = true; }
                        } else if automatic_final {
                            watch.failures = 0;
                            watch.next = if watch.alert_waiting {
                            (Instant::now() + Duration::from_secs(1))
                                .max(watch.last_request + Duration::from_secs(MIN_INTERVAL_SECONDS))
                            } else { (watch.last_request + watch.interval).max(Instant::now()) };
                        }
                    }
                    drop(_monitor_update);
                    if automatic_invalid && decoded.error.is_none() {
                        chat.note_background("周期分析未完成，已保留采样并等待重试或补充信息 / Monitor analysis incomplete; samples retained", true);
                    }
                    if let Some(error) = decoded.error {
                        if pending.automatic { chat.note_background(error, true); } else { chat.note(error, true); }
                    } else if !decoded.actions.is_empty() {
                        if !pending.automatic{if let Some(run)=&mut chat.run{run.native_pending=reply.tool_message.clone();run.phase="executing";}}
                        if !pending.automatic { chat.status="正在执行分析工具… / Executing analysis tools…".into(); }
                        let current = self.ai_state(&project, testing);
                        if stale {
                            if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"error":"AI.CANCELLED_OR_STALE"})); }
                            chat.cancel(); chat.queue_paused = true;
                            chat.note(
                                "目标已变化，未执行AI动作 / Source changed; actions blocked",
                                true,
                            );
                        } else {
                            let mut actions=decoded.actions;
                            if actions.len() > 1 && (pending.automatic || chat.run.is_none()) {
                                if pending.automatic { chat.finish_watch_transaction(false); }
                                if !pending.automatic {
                                    if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"error":"AI.BATCH_REQUIRES_ACTIVE_TASK"})); }
                                    chat.cancel(); chat.queue_paused = true;
                                }
                                chat.pause_watch();
                                if pending.automatic { chat.note_background("批量动作需要明确的手动任务；本批未执行 / Batched actions require an active manual task; none dispatched", true); }
                                else { chat.note("批量动作需要明确的手动任务；本批未执行 / Batched actions require an active manual task; none dispatched", true); }
                                self.ai_chat = chat;
                                return;
                            }
                            if actions.len()>1 && !pending.automatic {if let Some(run)=&mut chat.run {run.queued=actions.drain(1..).collect();}}
                            for action in actions {
                                let skill_method = match &action {
                                    Action::SerialStart => "serial.start",
                                    Action::SerialStop => "serial.stop",
                                    Action::MonitorStart { .. } => "monitor.start",
                                    Action::MonitorConfigure { .. } => "monitor.configure",
                                    Action::MonitorStop => "monitor.stop",
                                    Action::Platform { method, .. } => method,
                                };
                                if !pending.automatic && chat.run.as_ref().is_some_and(|run|run.reviewed_log.is_some()) && !matches!(skill_method,"log.files"|"log.analyze"|"agent.plan"|"agent.evidence"|"agent.help") {
                                    chat.cancel();chat.queue_paused=true;chat.note("LOG.REVIEW_TOOL_NOT_ALLOWED: 审阅日志不能执行设备或界面动作",true);break;
                                }
                                let reviewed_monitor_local = pending.automatic && chat.review_outbound
                                    && monitor_review::is_local(skill_method);
                                if !reviewed_monitor_local && !wiparse_core::ai::skills::permits(&chat.selected_skills, skill_method) {
                                    if pending.automatic {
                                        chat.finish_watch_transaction(false);
                                        chat.pause_watch();
                                        chat.note_background("SKILL.TOOL_NOT_ALLOWED: 当前技能不支持此工具", true);
                                    } else {
                                        if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":skill_method,"error":"SKILL.TOOL_NOT_ALLOWED"})); }
                                        chat.cancel(); chat.queue_paused = true;
                                        chat.note("SKILL.TOOL_NOT_ALLOWED: 当前技能不支持此工具", true);
                                    }
                                    break;
                                }
                                if pending.automatic
                                    && matches!(action, Action::MonitorConfigure { .. } | Action::MonitorStart { .. } | Action::MonitorStop)
                                {
                                    chat.finish_watch_transaction(false);
                                    chat.note_background("自动分析不能自行修改或停止监控；本地预警继续 / Automatic analysis cannot modify or stop monitoring; local alerts continue",true);
                                    break;
                                }
                                if !pending.automatic && matches!(&action, Action::MonitorStart { .. } | Action::MonitorConfigure { .. } | Action::MonitorStop) {
                                    let blocked = chat.run.as_ref().and_then(|run| run.limit_reason(chat.tool_round))
                                        .or_else(|| (matches!(&action, Action::MonitorStart { .. })
                                            && (!chat.ready() || !current.binding.live || !current.reception_requested))
                                            .then_some("请先启动实时串口接收并配置 AI 服务 / Start live serial reception and configure AI first"));
                                    if let Some(reason) = blocked {
                                        if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":skill_method,"error":reason})); }
                                        chat.cancel(); chat.queue_paused = true; chat.note(reason, true);
                                        break;
                                    }
                                    chat.control_approval = Some(ControlApproval {
                                        action, binding: current.binding.clone(), skills: chat.selected_skills.clone(),
                                        question: pending.question.clone(), automatic: false, created: Instant::now(),
                                        run_id: chat.run.as_ref().map(|run| run.id.clone()),
                                        monitor_revision: Some(chat.monitor_revision()),
                                    });
                                    chat.control_approval_focus_pending = true;
                                    chat.note("待确认监控调整，当前规则尚未改变 / Monitoring change awaits confirmation", false);
                                    break;
                                }
                                let controls = match &action {
                                    Action::SerialStart | Action::SerialStop => true,
                                    Action::Platform { method, params } => {
                                        platform::validate(method, params).unwrap_or(false)
                                    }
                                    _ => false,
                                };
                                let own_test_control = !pending.automatic && matches!(&action,Action::Platform{method,..}
                                    if matches!(method.as_str(),"testing.stop"|"testing.pause"|"testing.resume"));
                                if controls
                                    && (if own_test_control { !pending.test_control_granted || !chat.owned_test_control_allowed() }
                                       else { !pending.automation_granted || !chat.control_allowed(&current) || current.automation_locked })
                                {
                                    if !pending.automatic { if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":skill_method,"error":"AI.CONTROL_LOCKED"})); } }
                                    if !pending.automatic { chat.cancel(); chat.queue_paused = true; }
                                    else { chat.finish_watch_transaction(false); }
                                    chat.pause_watch();
                                    let message=if current.automation_locked{"测试正在占用设备，未执行AI设备动作 / Test owns devices; action blocked"}else{"本条消息未获得设备操作授权，未执行；授权后请提交新消息，旧消息不会自动补授权。 / This message has no device grant; submit a new message after authorization."};
                                    if pending.automatic { chat.note_background(message,true); } else { chat.note(message,true); }
                                    break;
                                }
                                if controls {
                                    if pending.automatic && chat.review_outbound && !(matches!(&action,Action::SerialStop) || matches!(&action,Action::Platform { method, .. } if method == "serial.monitor.stop")) {
                                        chat.finish_watch_transaction(false);
                                        chat.pause_watch();
                                        chat.note_background("审阅监控暂不支持自动设备动作，请在手动任务中操作 / Reviewed monitoring cannot dispatch automatic device actions; use a manual task", true);
                                        break;
                                    }
                                    if let Action::Platform { method, params } = &action {
                                        if !method.starts_with("serial.monitor.") {
                                            if pending.automatic || !platform::broker_reviews_control(ctx, method) || chat.run.is_none() {
                                                if pending.automatic { chat.finish_watch_transaction(false); }
                                                else {
                                                    if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":method,"error":"AI.VERIFIED_DEVICE_SESSION_AND_MANUAL_TASK_REQUIRED"})); }
                                                    chat.cancel(); chat.queue_paused = true;
                                                }
                                                chat.pause_watch();
                                                if pending.automatic { chat.note_background("设备动作需要可核验会话及手动任务；本地预警继续 / Device actions require a verified session and manual task; local alerts continue", true); }
                                                else { chat.note("设备动作需要可核验会话及手动任务；本地预警继续 / Device actions require a verified session and manual task; local alerts continue", true); }
                                            } else if let Err(error) = self.begin_platform_call(ctx, &mut chat, method.clone(), params.clone(), pending.question.clone(), false) {
                                                if let Some(run) = &mut chat.run { run.record(json!({"ok":false,"executed":false,"method":method,"error":error})); }
                                                chat.cancel(); chat.queue_paused = true; chat.note(error, true);
                                            }
                                            // The broker owns the only confirmation for these
                                            // actions and rechecks the device session before dispatch.
                                            break;
                                        }
                                    }
                                    // A model proposal is not user authorization. Capture
                                    // the exact action and source before any host dispatch.
                                    chat.control_approval = Some(ControlApproval {
                                        action, binding: current.binding.clone(), skills: chat.selected_skills.clone(),
                                        question: pending.question.clone(), automatic: pending.automatic,
                                        created: Instant::now(), run_id: chat.run.as_ref().map(|run| run.id.clone()),
                                        monitor_revision: None,
                                    });
                                    chat.control_approval_focus_pending = true;
                                    if pending.automatic {
                                        chat.pause_watch();
                                        // A new or resumed watch must not inherit an old approval.
                                        let revision = chat.monitor_revision();
                                        if let Some(approval) = &mut chat.control_approval { approval.monitor_revision = Some(revision); }
                                    }
                                    if pending.automatic { chat.note_background("待确认具体设备动作，尚未执行 / Device action awaits confirmation; not dispatched", false); }
                                    else { chat.note("待确认具体设备动作，尚未执行 / Device action awaits confirmation; not dispatched", false); }
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
                                        if pending.automatic {
                                            chat.finish_watch_transaction(false);chat.pause_watch();chat.note_background(format!("{error} · AI 周期分析已暂停，本地预警继续 / AI monitoring paused; local alerts continue"), true);
                                        } else {
                                            if let Some(run)=&mut chat.run { run.record(json!({"ok":false,"error":error,"executed":false})); }
                                            chat.cancel(); chat.queue_paused=true;
                                            chat.note(format!("{error} · 批次已停止，后续动作未执行 / Batch stopped; later actions not executed"), true);
                                        }
                                        break;
                                    } else { break; }
                                } else {
                                    let action_method=match &action{Action::MonitorConfigure{..}=>"monitor.configure",Action::MonitorStart{..}=>"monitor.start",Action::MonitorStop=>"monitor.stop",_=>"serial.action"};
                                    if !pending.automatic && chat.run.as_ref().is_some_and(|r|r.limit_reason(chat.tool_round).is_some()){
                                        if let Some(run)=&mut chat.run{run.record(json!({"ok":false,"executed":false,"error":"AI.SUMMARY_ONLY"}));}
                                        let data=self.ai_scoped_observation(chat.scope).unwrap_or_else(|_|json!({}));
                                        if let Err(e)=chat.submit(ctx,&current,data,pending.question.clone(),false){chat.note(e,true);}break;
                                    }
                                    if !pending.automatic{chat.tool_round+=1;}
                                    let result=self.apply_ai_action(&mut chat,action,&pending.question,lang,testing);
                                    if !pending.automatic&&chat.run.as_ref().is_some_and(|r|r.native_pending.is_some()){
                                        let mut evidence=match result{Ok(())=>json!({"ok":true,"monitoring":chat.watch.is_some(),"reception":self.ai_state(&project,testing)}),Err(e)=>json!({"ok":false,"error":e})};
                                        chat.evidence_counter=chat.evidence_counter.saturating_add(1);evidence["evidence_id"]=json!(format!("S{}",chat.evidence_counter));if let Some(run)=&mut chat.run{run.workflow.record(evidence["evidence_id"].as_str().unwrap_or(""),action_method,&evidence);run.record(evidence.clone());}
                                        if evidence["ok"] != true { chat.cancel(); chat.queue_paused=true; chat.note("批次动作失败，后续动作未执行 / Batch action failed; later actions not executed",true); break; }
                                        if self.dispatch_queued_reads(ctx,&mut chat,&pending.question){break;}
                                        let mut data=self.ai_scoped_observation(chat.scope).unwrap_or_else(|_|json!({}));data["monitor_result"]=evidence;
                                        if let Err(e)=chat.submit(ctx,&self.ai_state(&project,testing),data,pending.question.clone(),false){chat.note(e,true);}
                                    }else if let Err(error)=result{if pending.automatic{chat.note_background(error,true)}else{chat.note(error,true)};break;}
                                    else if !pending.automatic && self.dispatch_queued_reads(ctx,&mut chat,&pending.question){break;}
                                }
                            }
                        }
                    }
                }
                }
                Err(error) => {
                    if pending.automatic { chat.finish_watch_transaction(false); }
                    chat.usage_total.failure();if !pending.automatic{if let Some(run)=&mut chat.run{run.usage.failure();}}
                    if !pending.automatic {
                        chat.history.truncate(pending.history_before);
                    }
                    // A provider may return a native function that was not advertised.
                    // Reject that reply, then repair the protocol using the existing
                    // bounded JSON fallback; never dispatch or replay its tool call.
                    let unexpected_tool = matches!(&error, wiparse_core::ai::AiError::ResponseDetail("回复调用了本次请求未提供的工具"));
                    let current = self.ai_state(&project, testing);
                    let scope_valid = current.binding == pending.binding
                        && chat.run.as_ref().is_some_and(|r| r.scope_key == self.local_log_scope_key(&chat));
                    if !pending.automatic && unexpected_tool && scope_valid
                        && !chat.run.as_ref().is_some_and(|r| r.summary_only) {
                        let retry = chat.run.as_mut().is_some_and(|r| r.recovery_policy.retry(true, "AI.UNADVERTISED_TOOL"));
                        if retry {
                            if let Some(run) = chat.run.as_mut() {
                                run.recovery_policy.json_fallback = true;
                                run.repairs += 1;
                                run.feedback = Some("The previous response used a function not advertised in that request and was rejected without execution. Use the JSON action contract and only the listed tools.".into());
                            }
                            chat.entry("tool", "协议诊断 · 模型返回了未提供的工具，已拒绝并切换 JSON 兼容模式；未执行该调用。");
                            let mut data = self.ai_scoped_observation(chat.scope).unwrap_or_else(|_| json!({}));
                            data["platform_results"] = json!(chat.tool_results);
                            if let Err(e) = chat.submit(ctx, &current, data, pending.question.clone(), false) { chat.note(e, true); }
                            self.ai_chat = chat;
                            ctx.request_repaint();
                            return;
                        }
                    }
                    if !pending.automatic && chat.run.as_ref().is_some_and(|r|r.summary_only && r.scope_key==self.local_log_scope_key(&chat))
                        && self.ai_state(&project,testing).binding==pending.binding {
                        chat.entry("tool",format!("总结请求失败 · {error}"));
                        chat.finish_budget_summary(None, false);
                        self.ai_chat = chat;
                        ctx.request_repaint();
                        return;
                    }
                    if !pending.automatic { chat.retry = Some(pending.question); }
                    if pending.automatic { chat.note_background(error.to_string(), true); }
                    else { chat.note(error.to_string(), true); }
                    let will_pause = pending.automatic && chat.watch.as_ref().is_some_and(|watch| watch.failures >= 2);
                    let _monitor_update = will_pause.then(|| chat.monitor_lease.begin_update());
                    if let Some(watch) = chat.watch.as_mut().filter(|_| pending.automatic) {
                        watch.failures = watch.failures.saturating_add(1);
                        watch.next = Instant::now()
                            + watch
                                .interval
                                .max(Duration::from_secs(30 * (1u64 << watch.failures.min(3))));
                        if watch.failures >= 3 {
                            watch.ai_paused = true;
                            chat.note_background("AI连续失败，已暂停云端分析，本地预警继续 / AI analysis paused after repeated failures; local alerts continue",true);
                        }
                    }
                }
            }
        }
        self.poll_platform(ctx, &mut chat, lang);
        if chat.pending.is_none()&&chat.tool.is_none()&&chat.reviewed_stop_pending.is_none()&&chat.control_approval.is_none(){
            if let Some((data,question,bound))=chat.deferred.take(){
                let current=self.ai_state(&project,testing);
                let guard=chat.deferred_guard.take();
                if current.binding==bound.binding && guard.as_ref().is_some_and(|guard|
                    !guard.revoked && guard.controls==chat.control_allowed(&current) && guard.test_controls==chat.owned_test_control_allowed() && guard.skills==chat.selected_skills
                    && guard.scope_key==self.local_log_scope_key(&chat)
                    && chat.run.as_ref().is_some_and(|run|run.id==guard.task_id && run.scope_key==guard.scope_key)) {
                    if let Err(error)=chat.submit(ctx,&current,data,question,false){chat.note(error,true);}
                }else{chat.queue_paused=true;chat.note("任务、来源或权限已变化，已取消等待中的分析请求 / Task, source or permissions changed; deferred analysis cancelled",true);}
            }
        }
        let queue_state=self.ai_state(&project,testing);self.pump_message_queue(ctx,&mut chat,&queue_state);
        if chat.pending.is_none()
            && chat.tool.is_none()
            && chat.deferred.is_none()
            && chat.control_approval.is_none()
            && chat
                .watch
                .as_ref()
                .is_some_and(|watch| !watch.ai_paused && Instant::now() >= watch.next)
        {
            let mut state = self.ai_state(&project, testing);
            let watch = chat.watch.as_ref().unwrap();
            if state.connection_confirmed && state.absolute_lines > watch.cursor {
                let cursor = watch.cursor;
                let watch_id=watch.id.clone();
                let question = watch.goal.clone();
                let stats = watch.stats.clone();
                let data = if chat.review_outbound {
                    // Each value is captured under its owner's boundary. The
                    // two permits must eventually be joined at every dispatch;
                    // until writer coverage is complete the review stays blocked.
                    let window = wiparse_core::ai::ReviewedSource::capture(&self.serial_review_lease, ||
                        (self.ai_state(&project, self.ai_test_locked), self.ai_observation(true, Some(cursor), Some(&stats))));
                    if let Some(source) = &window { state = source.value().0.clone(); }
                    let service=crate::ai::service(ctx);
                    let service_revision=service.as_ref().zip(chat.config.as_ref()).and_then(|(service,config)|service.revision_for_config(config));
                    let identity = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, ||
                        json!({"watch_id":watch_id,"revision":chat.monitor_revision(),"binding":state.binding,
                            "skills":chat.selected_skills,"control_allowed":chat.control_allowed(&state),
                            "service_id":service.as_ref().map(|service|service.instance_id().to_string()),"service_revision":service_revision}));
                    match (identity, window) {
                        (Some(identity), Some(window)) if state.connection_confirmed && state.absolute_lines > cursor
                            && chat.watch.as_ref().is_some_and(|active| active.id == watch_id && active.binding == state.binding && !active.ai_paused) => {
                            let window = window.map(|(_, sample)| sample);
                            let data = window.value().clone();
                            chat.monitor_review = Some((identity, window));
                            data
                        }
                        _ => {
                            chat.note_background("监控来源正在变化，采样尚未发送 / Monitor source is changing; sample not sent", true);
                            if let Some(watch) = &mut chat.watch { watch.next = Instant::now() + watch.interval; }
                            self.ai_chat = chat;
                            return;
                        }
                    }
                } else { self.ai_observation(true, Some(cursor), Some(&stats)) };
                chat.automatic_tool_round = 0;
                chat.automatic_tool_results.clear();
                match chat.submit(ctx, &state, data, question, true) {
                    Ok(()) => {
                        chat.monitor_inflight = Some((watch_id, state.binding.clone(), cursor, stats));
                        if let Some(watch) = &mut chat.watch {
                            watch.last_request = Instant::now();
                            watch.alert_waiting = false;
                            watch.cursor = state.absolute_lines;
                            watch.stats = WindowStats::default();
                        }
                    }
                    Err(error) => {
                        chat.monitor_review = None;
                        chat.note_background(error, true);
                        let unavailable=chat.review_outbound && !chat.reviewed_monitor_dispatch_ready();
                        let _monitor_update=unavailable.then(||chat.monitor_lease.begin_update());
                        if let Some(watch) = &mut chat.watch {
                            if unavailable { watch.ai_paused = true; }
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
        let _monitor_update = chat.monitor_lease.begin_update();
        chat.reports = 0;
        chat.watch = Some(Watch {
            id: uuid::Uuid::new_v4().to_string(),
            pretrigger: Default::default(), first_trigger: None, observed_rows: 0,
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
        let _monitor_update = chat.monitor_lease.begin_update();
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
                if !self.device_is_open(&port) {
                    return Err("SERIAL.NOT_RECEIVING: 当前端口未接收，未发送停止请求 / Port is not receiving; no stop request dispatched".into());
                }
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
                chat.discard_stale_watch_transaction();
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
        let mut chat = self.take_ai_chat();
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
            // Do not invalidate on ApproveOutbound: its permit must survive
            // until the worker atomically commits the dispatch.
            let reviewed_stop_confirmation = matches!(&event,UiAction::ConfirmControl) && chat.review_outbound
                && chat.control_approval.as_ref().is_some_and(|approval|approval.automatic && (matches!(&approval.action,Action::SerialStop)
                    || matches!(&approval.action,Action::Platform { method, .. } if method == "serial.monitor.stop")));
            let _monitor_update = (matches!(&event, UiAction::RejectOutbound | UiAction::RejectControl
                | UiAction::ConfirmControl | UiAction::Cancel | UiAction::Close | UiAction::Clear
                | UiAction::ResumeAi | UiAction::ToggleWatch) && !reviewed_stop_confirmation).then(|| chat.monitor_lease.begin_update());
            match event {
                UiAction::ApproveOutbound => {
                    let valid = chat.pending.as_ref().is_some_and(|pending| pending.reviewed_source_guarded && pending.binding == state.binding
                        && (pending.automatic || chat.run.as_ref().and_then(|run|run.reviewed_log.as_ref()).is_none_or(|source|chat.review_outbound && source.is_current()))
                        && (!pending.automatic || chat.automatic_pending_current(pending, &state))
                        && (pending.automatic || match &chat.run {
                            Some(r) => pending.review_task_id.as_deref() == Some(r.id.as_str())
                                && pending.review_scope_key == r.scope_key
                                && r.scope_key == self.local_log_scope_key(&chat),
                            None => pending.review_task_id.is_none(),
                        }));
                    if valid {
                        let key = chat.pending.as_ref().and_then(|pending| pending.task.outbound_preview()
                            .and_then(|preview| chat.outbound_grant_key(pending, &preview)));
                        let approved = chat.pending.as_ref().is_some_and(|pending| pending.task.approve_outbound());
                        if approved && chat.remember_identical_outbound {
                            if let Some(key) = key {
                                if chat.approved_outbound.len() >= 64 { chat.approved_outbound.clear(); }
                                chat.approved_outbound.insert(key);
                            }
                        }
                    } else {
                        chat.cancel();
                        chat.note("来源保护不可用或任务已变化，审阅请求未提交 / Source guard unavailable or task changed; reviewed request not committed", true);
                    }
                }
                UiAction::RejectOutbound => {
                    let automatic = chat.pending.as_ref().is_some_and(|pending| pending.automatic);
                    let same_watch = chat.pending.as_ref().is_some_and(|pending| pending.automatic && chat.watch.as_ref().is_some_and(|watch| pending.watch_id.as_deref() == Some(watch.id.as_str())));
                    let rejected_before_send = chat.pending.as_ref().is_some_and(|pending| pending.task.reject_outbound());
                    chat.cancel();
                    if automatic {
                        if same_watch { chat.pause_watch(); }
                        chat.note_background(if rejected_before_send {"已拒绝外发，AI 周期分析暂停，本地预警继续 / Outbound request rejected; AI monitoring paused, local alerts continue"} else {"已停止等待；请求可能已发送，AI 周期分析暂停，本地预警继续 / Wait stopped; request may have been sent, AI monitoring paused, local alerts continue"}, false);
                    } else {
                        chat.queue_paused = true;
                        chat.note(if rejected_before_send {"已拒绝外发，当前请求未发送 / Outbound request rejected; not sent"} else {"已停止等待；请求可能已发送 / Wait stopped; request may have been sent"}, false);
                    }
                }
                UiAction::RejectControl => {
                    let automatic = chat.control_approval.as_ref().is_some_and(|approval| approval.automatic);
                    let same_watch=chat.control_approval.as_ref().is_some_and(|approval|approval.belongs_to_watch(chat.watch.as_ref()));
                    if chat.reject_control_approval("AI.ACTION_REJECTED") {
                        if automatic {
                            if same_watch {chat.pause_watch();}
                            chat.note_background("已拒绝，操作未执行 / Rejected; action not executed", false);
                        } else {
                            chat.cancel(); chat.queue_paused = true;
                            chat.note("已拒绝，操作未执行 / Rejected; action not executed", false);
                        }
                    }
                }
                UiAction::ConfirmControl => {
                    if let Some(approval) = chat.control_approval.as_ref() {
                        let ctx = ui.ctx().clone();
                        let current = self.ai_state(&chat.project, self.ai_test_locked);
                        let valid = approval.valid(&current, &chat.selected_skills,
                            chat.run.as_ref().map(|run| run.id.as_str()), chat.control_allowed(&current), &chat.monitor_revision(), Instant::now())
                            && (!approval.automatic || !chat.review_outbound || chat.monitor_review.as_ref().is_some_and(|(_, sample)| sample.is_current()))
                            && (approval.automatic || chat.run.as_ref().is_none_or(|run|run.scope_key==self.local_log_scope_key(&chat)));
                        if !valid {
                            let automatic = approval.automatic;
                            let same_watch=approval.belongs_to_watch(chat.watch.as_ref());
                            chat.reject_control_approval("AI.APPROVAL_EXPIRED_OR_REVOKED");
                            if automatic {
                                if same_watch {chat.pause_watch();}
                                chat.note_background("确认已失效，操作未执行 / Approval expired or revoked; action not executed", true);
                            } else {
                                chat.cancel(); chat.queue_paused = true;
                                chat.note("确认已失效，操作未执行 / Approval expired or revoked; action not executed", true);
                            }
                        } else {
                            let approval = chat.control_approval.take().expect("approval checked above");
                            chat.control_approval_focus_pending = false;
                            // Non-serial device operations require the canonical
                            // broker's target-session recheck and review; the
                            // legacy bridge cannot bind a confirmed session.
                            if let Action::Platform { method, .. } = &approval.action {
                                if !method.starts_with("serial.monitor.")
                                    && !crate::ai::broker::supports(method)
                                    && !crate::ai::broker::has_instrument_executor(&ctx, method) {
                                    chat.control_approval = Some(approval);
                                    let automatic = chat.control_approval.as_ref().is_some_and(|item| item.automatic);
                                    chat.reject_control_approval("AI.VERIFIED_DEVICE_SESSION_REQUIRED");
                                    if automatic {
                                        chat.pause_watch();
                                        chat.note_background("缺少可核验的设备会话，动作未发送 / Verified device session unavailable; action not dispatched", true);
                                    } else {
                                        chat.cancel(); chat.queue_paused = true;
                                        chat.note("缺少可核验的设备会话，动作未发送 / Verified device session unavailable; action not dispatched", true);
                                    }
                                    continue;
                                }
                            }
                            let serial_permit = SerialControlPermit {
                                binding: approval.binding.clone(), scope_key:self.local_log_scope_key(&chat), skills: approval.skills.clone(),
                                run_id: approval.run_id.clone(), created: approval.created,
                            };
                            let result = match approval.action {
                                Action::Platform { method, params } => {
                                    chat.confirmed_platform_action = Some((method.clone(), params.clone()));
                                    if method.starts_with("serial.monitor.") { chat.confirmed_serial_permit = Some(serial_permit); }
                                    let result = self.begin_platform_call(&ctx, &mut chat, method, params, approval.question.clone(), approval.automatic);
                                    chat.confirmed_platform_action = None;
                                    chat.confirmed_serial_permit = None;
                                    result
                                },
                                action @ (Action::SerialStart | Action::SerialStop) => {
                                    let method = if matches!(action, Action::SerialStart) { "serial.monitor.start" } else { "serial.monitor.stop" }.to_owned();
                                    let params = json!({});
                                    chat.confirmed_platform_action = Some((method.clone(), params.clone()));
                                    chat.confirmed_serial_permit = Some(serial_permit);
                                    let result = self.begin_platform_call(&ctx, &mut chat, method, params, approval.question.clone(), approval.automatic);
                                    chat.confirmed_platform_action = None;
                                    chat.confirmed_serial_permit = None;
                                    result
                                }
                                action @ (Action::MonitorStart { .. } | Action::MonitorConfigure { .. } | Action::MonitorStop) => {
                                    let method = match &action { Action::MonitorStart { .. } => "monitor.start", Action::MonitorStop => "monitor.stop", _ => "monitor.configure" };
                                    let testing = self.ai_test_locked;
                                    let result = if let Some(reason) = chat.run.as_ref().and_then(|run| run.limit_reason(chat.tool_round)) {
                                        Err(reason.to_owned())
                                    } else { self.apply_ai_action(&mut chat, action, &approval.question, lang, testing) };
                                    chat.tool_round = chat.tool_round.saturating_add(1);
                                    chat.evidence_counter = chat.evidence_counter.saturating_add(1);
                                    let id = format!("S{}", chat.evidence_counter);
                                    let evidence = match &result {
                                        Ok(()) => json!({"evidence_id":id,"method":method,"ok":true,"monitoring":chat.watch.is_some()}),
                                        Err(error) => json!({"evidence_id":id,"method":method,"ok":false,"executed":false,"error":error}),
                                    };
                                    if let Some(run) = &mut chat.run { run.workflow.record(&id, method, &evidence); run.record(evidence.clone()); }
                                    chat.tool_results.push(evidence.clone());
                                    if result.is_ok() && !self.dispatch_queued_reads(&ctx, &mut chat, &approval.question) {
                                        let state = self.ai_state(&chat.project, self.ai_test_locked);
                                        let mut data = self.ai_scoped_observation(chat.scope).unwrap_or_else(|_| json!({}));
                                        data["monitor_result"] = evidence;
                                        if let Err(error) = chat.submit(&ctx, &state, data, approval.question.clone(), false) { chat.note(error, true); }
                                    }
                                    result
                                }
                                _ => Err("AI.APPROVAL_INVALID_ACTION".into()),
                            };
                            if let Err(error) = result {
                                let receipt = json!({"ok":false,"executed":false,"error":error});
                                if approval.automatic { chat.automatic_tool_results.push(receipt); } else { chat.tool_results.push(receipt); }
                                if !approval.automatic { chat.cancel(); }
                                else { chat.pause_watch(); }
                                if approval.automatic { chat.finish_watch_transaction(false); chat.note_background(error,true); }
                                else { chat.queue_paused=true; chat.note(error,true); }
                            }
                        }
                    }
                }
                UiAction::Close => chat.close(),
                UiAction::Cancel => {
                    chat.cancel_current_from_ui();
                }
                UiAction::Settings => self.ai_settings_requested = true,
                UiAction::Clear => {
                    let _monitor_update = chat.monitor_lease.begin_update();
                    chat.cancel();
                    chat.library.reset();
                    chat.preview_library.reset();
                    chat.tool_results.clear();
                    chat.automatic_tool_results.clear();
                    chat.watch = None;
                    chat.discard_stale_watch_transaction();
                    chat.entries.clear();chat.messages.clear();chat.queue_paused=false;chat.queue_notice=None;chat.run=None;chat.usage_total=Default::default();
                    chat.history.clear();
                    chat.retry = None;
                    chat.status.clear();
                    chat.prompt.clear();
                    chat.reports = 0;
                    chat.allow_automation = false;
                    chat.awaiting_connection = None;
                }
                UiAction::ResumeAi => {
                    let _monitor_update = chat.monitor_lease.begin_update();
                    if chat.review_outbound && !chat.reviewed_monitor_dispatch_ready() {
                        chat.note_background("当前构建暂不支持受审阅的周期 AI 分析，未恢复；本地告警继续运行。请使用手动分析，或在了解数据外发范围后调整审阅设置。 / Reviewed periodic AI analysis is unavailable in this build; local alerts continue. Use manual analysis or review outbound settings.",true);
                    } else if chat.ready() {
                        if let Some(watch) = &mut chat.watch {
                            watch.ai_paused = false;
                            watch.failures = 0;
                            watch.next = Instant::now();
                        }
                        chat.note("AI分析已恢复，本地监控保持运行 / AI analysis resumed; local monitoring remains active",false);
                    }
                }
                UiAction::ToggleWatch => {
                    let _monitor_update = chat.monitor_lease.begin_update();
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
                    if let Some(root)=rfd::FileDialog::new().pick_folder(){chat.cancel();chat.queue_paused=!chat.messages.is_empty();chat.run=None;chat.recovery=None;chat.library.directory=Some(root);chat.library.reset();chat.preview_library.reset();chat.scope=DataScope::Library;chat.tool_results.clear();chat.history.clear();chat.note(label(lang,"已更新本地日志目录，后续只发送有限统计和证据。","Local log directory updated. Only bounded statistics and evidence are sent."),false);}
                }
                UiAction::ClearLogDirectory=>{chat.cancel();chat.queue_paused=!chat.messages.is_empty();chat.run=None;chat.recovery=None;chat.library.directory=None;chat.library.reset();chat.preview_library.reset();chat.tool_results.clear();chat.history.clear();}
                UiAction::PreviewLogs=>{
                    if chat.pending.is_none()&&chat.tool.is_none(){
                        let scope_key=self.local_log_scope_key(&chat);
                        chat.preview_library.directory=chat.library.directory.clone();
                        chat.preview_library.budget_mib=chat.library.budget_mib;
                        chat.preview_library.bind_scope(scope_key);let params=chat.preview_library.preview_params();
                        chat.local_preview_request=true;
                        let preview=self.begin_platform_call(ui.ctx(),&mut chat,"log.analyze".into(),params,"本地日志预分析".into(),false);
                        chat.local_preview_request=false;
                        match preview {
                            Ok(())=>{},Err(error)=>chat.note_background(error,true),
                        }
                    }
                }
                UiAction::RemoveQueued(id)=>{chat.messages.retain(|m|m.id!=id);chat.queue_notice=None;},
                UiAction::EditQueued(id)=>{if chat.prompt.trim().is_empty(){if let Some(i)=chat.messages.iter().position(|m|m.id==id){chat.prompt=chat.messages.remove(i).unwrap().text;chat.queue_notice=None;}}else{chat.queue_notice=Some("输入区已有草稿，请先保存或发送，排队内容保持不变 / Draft exists; queued message preserved".into());}},
                UiAction::ClearQueue=>{chat.messages.clear();chat.queue_paused=false;chat.queue_notice=None;},
                UiAction::ResumeQueue=>{chat.queue_paused=false;chat.status_error=false;},
                UiAction::Correct(text)=>{
                    let scope=self.local_log_scope_key(&chat);
                    let Some(run)=chat.run.as_mut().filter(|r|r.scope_key==scope && matches!(r.phase,"planning"|"reading"|"executing"|"analysing")) else {continue;};
                    if text.trim().is_empty() || text.len()>16*1024 {chat.note("请输入 16 KiB 以内的修正说明。",true);continue;}
                    if chat.pending.as_ref().is_some_and(|p|p.automatic || p.binding!=state.binding) || chat.tool.as_ref().is_some_and(|t|t.automatic){continue;}
                    if run.correction.is_some(){chat.note("上一条修正正在等待当前操作返回，新输入已保留。",false);continue;}
                    run.correction=Some(text.clone());
                    run.queued.clear();
                    chat.prompt.clear();chat.entry("user",text);
                    if let Some(pending)=chat.pending.take(){
                        pending.task.cancel();
                        chat.history.truncate(pending.history_before);
                        match self.ai_scoped_observation(chat.scope).and_then(|data|chat.submit(ui.ctx(),&state,data,pending.question,false)){
                            Ok(())=>{},Err(error)=>chat.note(error,true)
                        }
                    }
                }
                UiAction::Send(question) => {
                    if chat.should_queue_input() {
                        let key=self.local_log_scope_key(&chat);
                        match chat.enqueue_message(question,&state,key){Ok(())=>{chat.prompt.clear();chat.queue_notice=None;},Err(error)=>{chat.queue_notice=Some(error);}}
                        continue;
                    }
                    match self.begin_manual_agent(ui.ctx(),&mut chat,&state,question) {
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
    pub(crate) fn ai_controls_locked(&self, integrated_test: bool) -> bool {
        integrated_test || self.integrated_test_active.load(Ordering::Acquire) || self.live_test.is_some()
    }

 pub(crate) fn agent_identity(&self,project:&str)->String{serde_json::to_string(&json!({"binding":self.ai_state(project,self.ai_test_locked).binding,"log_scope":self.local_log_scope_key(&self.ai_chat)})).unwrap_or_default()}
 pub(crate) fn agent_inspect(&self,project:&str)->serde_json::Value{let state=self.ai_state(project,self.ai_test_locked);let releases:Vec<_>=self.serial_release_receipts.iter().rev().filter_map(|receipt|receipt.stop_intent.as_ref().filter(|intent|intent.project==project).map(|intent|json!({"session_id":receipt.session_id.to_string(),"port":receipt.port,"command_id":intent.command_id,"command_fingerprint":intent.fingerprint,"opened":receipt.opened,"host_session_dropped":receipt.host_session_dropped,"journal_persisted":receipt.journal_persisted,"physical_verified":false,"scope":"historical host process observation; OS and physical state not independently verified"}))).take(4).collect();json!({"source_id":state.binding.source_id,"live":state.binding.live,"status":state.status,"requested":state.reception_requested,"connected":state.connection_confirmed,"total_received_rows":state.absolute_lines,"retained_rows":state.retained_lines,"evicted_rows":state.absolute_lines.saturating_sub(state.retained_lines),"baud":state.binding.baud,"source_title":state.source_title,"recent_host_session_releases":releases})}
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
 pub(crate) fn agent_dataset(&mut self,id:&str)->Result<wiparse_core::log_analysis::timeline::Dataset,String>{
  let key=self.local_log_scope_key(&self.ai_chat);self.ai_chat.library.bind_scope(key);
  self.ai_chat.library.dataset(id)
 }
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
   let mut chat=self.take_ai_chat();
   if let Some(config)=chat.config.clone() { chat.set_context(project.into(), &config); }
   else if chat.project != project { chat.monitor_lease.mutate_always(|| chat.project = project.into()); }
   let result=self.apply_ai_action(&mut chat,action,"工程智能监控",lang,testing);self.ai_chat=chat;result
  });
  match result{Ok(())=>json!({"ok":true,"data":{"monitoring":self.ai_chat.watch.is_some(),"interval_seconds":self.ai_chat.interval_seconds,"local_rules":"full retained incoming lines evaluated locally; cloud analysis is periodic, not a safety interlock","reception":self.agent_inspect(project)}}),Err(e)=>crate::ai::agent::failure(e)}
 }
}
