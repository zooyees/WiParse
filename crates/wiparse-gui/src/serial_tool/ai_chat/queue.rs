//! FIFO user messages, distinct from the native read-tool batch queue.
use super::*;
pub(super) struct Message {
    pub id: String,
    pub text: String,
    pub source: String,
    binding: Binding,
    scope_key: String,
    pub(super) controls: bool,
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
        });
        Ok(())
    }
    pub(super) fn control_allowed(&self, state: &Snapshot) -> bool {
        self.allow_automation
            && !state.automation_locked
            && self.run.as_ref().is_none_or(|r| r.control_cap)
    }
    pub(super) fn revoke_controls(&mut self) {
        if let Some(run) = &mut self.run {
            run.control_cap = false;
        }
        for message in &mut self.messages {
            message.controls = false;
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
        if chat.working() || chat.queue_paused || chat.messages.is_empty() {
            return;
        }
        if chat.status_error {
            chat.queue_paused = true;
            return;
        }
        let front = chat.messages.front().unwrap();
        if front.binding != state.binding || front.scope_key != self.local_log_scope_key(chat) {
            chat.queue_paused = true;
            chat.note("排队消息的来源或范围已变化，已暂停；内容保留，可编辑后重新发送。 / Queue source changed; paused and drafts preserved.",false);
            return;
        }
        let message = chat.messages.pop_front().unwrap();
        let backup = message.text.clone();
        if let Err(error) =
            self.begin_manual_agent_capped(ctx, chat, state, message.text, message.controls)
        {
            chat.prompt = backup;
            chat.queue_paused = true;
            chat.note(error, true);
        }
    }
}
