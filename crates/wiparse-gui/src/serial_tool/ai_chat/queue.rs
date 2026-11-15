//! FIFO user messages, distinct from the native read-tool batch queue.
use super::*;
pub(super) struct Message {
    pub id: String,
    pub text: String,
    pub source: String,
    binding: Binding,
    scope_key: String,
    pub(super) controls: bool,
    pub(super) test_controls: bool,
    skills: Vec<String>,
}
impl SerialAiChat {
    pub(super) fn enqueue_message(
        &mut self,
        text: String,
        state: &Snapshot,
        scope_key: String,
    ) -> Result<(), String> {
        let text = text.trim().to_owned();
        if text.is_empty() || text.len() > 16 * 1024 {
            return Err("消息须为1–16384字节 / Message size invalid".into());
        }
        if self.messages.len() >= 16
            || self.messages.iter().map(|m| m.text.len()).sum::<usize>() + text.len() > 64 * 1024
        {
            return Err(
                "队列已满（最多16条、64 KiB）；输入内容已保留 / Queue full; draft preserved".into(),
            );
        }
        self.messages.push_back(Message {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            source: format!("{} · {}", state.binding.project, state.source_title),
            binding: state.binding.clone(),
            scope_key,
            controls: self.allow_automation && !state.automation_locked,
            test_controls: self.allow_automation,
            skills: self.selected_skills.clone(),
        });
        Ok(())
    }
    pub(super) fn control_allowed(&self, state: &Snapshot) -> bool {
        self.allow_automation
            && !state.automation_locked
            && self.run.as_ref().is_none_or(|r| r.control_cap)
    }
    pub(super) fn owned_test_control_allowed(&self) -> bool {
        self.allow_automation && self.run.as_ref().is_some_and(|run|run.test_control_cap)
            && self.selected_skills.iter().any(|skill|skill=="test_execution")
    }
    pub(super) fn revoke_controls(&mut self) {
        let _monitor_update = self.monitor_lease.begin_update();
        self.reviewed_task_lease.mutate_always(||());
        self.confirmed_platform_action = None;
        self.confirmed_serial_permit = None;
        if let Some(stop)=self.reviewed_stop_pending.take() {
            let (outcome,saved)=platform::cancel_reviewed_stop(stop);
            self.finish_watch_transaction(false);
            self.pause_watch();
            let message=if outcome==dispatch_gate::Outcome::CancelledBeforeClaim {
                "控制授权已撤销，排队停止请求未执行 / Control revoked; queued stop was not executed"
            } else {"控制授权已撤销，停止结果未知，请核查串口状态 / Control revoked; stop outcome unknown, inspect serial state"};
            self.note_background(format!("{message} · journal={saved}"),outcome!=dispatch_gate::Outcome::CancelledBeforeClaim || !saved);
        }
        platform::revoke_pending_control(&self.tool);
        // A later re-enable applies only to new requests. In-flight model
        // replies must not regain the authority they had before revocation.
        if let Some(pending) = &mut self.pending {
            pending.authorization_revoked |= pending.automation_granted || pending.test_control_granted;
            pending.automation_granted = false;
            pending.test_control_granted = false;
        }
        if let Some(deferred) = &mut self.deferred_guard { deferred.revoked |= deferred.controls || deferred.test_controls; }
        let automatic_approval = self.control_approval.as_ref().is_some_and(|approval| approval.automatic);
        let approval_watch_matches=self.control_approval.as_ref().is_some_and(|approval|approval.belongs_to_watch(self.watch.as_ref()));
        if self.control_approval.as_ref().is_some_and(ControlApproval::requires_device_grant)
            && self.reject_control_approval("AI.AUTHORIZATION_REVOKED") {
            if !automatic_approval { self.queue_paused = true; }
            if !automatic_approval || approval_watch_matches {if let Some(watch) = &mut self.watch { watch.ai_paused = true; }}
            if let Some(run) = self.run.as_mut().filter(|_| !automatic_approval) {
                run.finish("cancelled");
                run.native_pending = None;
                run.native_outputs.clear();
                run.queued.clear();
            }
            let message="控制授权已撤销，待确认动作未发送 / Control revoked; pending action not dispatched";
            if automatic_approval { self.note_background(message, true); } else { self.note(message, true); }
        }
        if let Some(run) = &mut self.run {
            run.control_cap = false;
            run.test_control_cap = false;
        }
        for message in &mut self.messages {
            message.controls = false;
            message.test_controls = false;
        }
    }
    pub(super) fn queue_ui(
        &self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        actions: &mut Vec<UiAction>,
    ) {
        if self.messages.is_empty() {
            return;
        }
        Frame::NONE
            .fill(t.header_bg)
            .stroke(Stroke::new(1.0_f32, t.border))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {}/16",
                            label(lang, "待发送", "Queued"),
                            self.messages.len()
                        ))
                        .strong()
                        .size(13.0),
                    );
                    if self.queue_paused
                        && ui
                            .small_button(label(lang, "继续队列", "Resume queue"))
                            .clicked()
                    {
                        actions.push(UiAction::ResumeQueue);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(label(lang, "清空", "Clear")).clicked() {
                            actions.push(UiAction::ClearQueue);
                        }
                    });
                });
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(if self.queue_paused {
                            label(
                                lang,
                                "队列已暂停；来源变化时请编辑后重发。",
                                "Queue paused; edit before resending on a changed source.",
                            )
                        } else {
                            label(
                                lang,
                                "本轮完成后依次发送。",
                                "Sent in order after the current task.",
                            )
                        })
                        .size(12.0)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("queued-user-messages")
                    .max_height(112.0)
                    .show(ui, |ui| {
                        for message in &self.messages {
                            ui.push_id(&message.id, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .small_button("×")
                                        .on_hover_text(label(
                                            lang,
                                            "移除排队消息",
                                            "Remove queued message",
                                        ))
                                        .clicked()
                                    {
                                        actions.push(UiAction::RemoveQueued(message.id.clone()));
                                    }
                                    if ui.small_button(label(lang, "编辑", "Edit")).clicked() {
                                        actions.push(UiAction::EditQueued(message.id.clone()));
                                    }
                                    ui.add(egui::Label::new(&message.text).truncate())
                                        .on_hover_text(format!(
                                            "{}\n{}",
                                            message.source, message.text
                                        ));
                                });
                            });
                        }
                    });
            });
    }
}
impl SerialToolPanel {
    pub(super) fn pump_message_queue(
        &mut self,
        ctx: &egui::Context,
        chat: &mut SerialAiChat,
        state: &Snapshot,
    ) {
        if chat.working() || chat.awaiting_reply() || chat.queue_paused || chat.messages.is_empty() {
            return;
        }
        if chat.status_error {
            chat.queue_paused = true;
            return;
        }
        let front = chat.messages.front().unwrap();
        if front.binding != state.binding || front.scope_key != self.local_log_scope_key(chat) || front.skills != chat.selected_skills {
            chat.queue_paused = true;
            chat.note("排队消息的来源或范围已变化，已暂停；内容保留，可编辑后重新发送。 / Queue source changed; paused and drafts preserved.",false);
            return;
        }
        let message = chat.messages.pop_front().unwrap();
        if let Err(error) =
            self.begin_manual_agent_with_caps(ctx, chat, state, message.text.clone(), message.controls, message.test_controls)
        {
            // Keep both the failed queued message and any draft entered while
            // the previous task was running. Replacing the draft loses input.
            chat.messages.push_front(message);
            chat.queue_paused = true;
            chat.note(error, true);
        }
    }
}
