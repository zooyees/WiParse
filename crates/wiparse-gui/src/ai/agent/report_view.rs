use super::*;

impl Session {
    pub(crate) fn set_report_context(&mut self, context:ContextState) {
        let changed=self.context.as_ref().is_some_and(|old|old.project!=context.project || binding_for(old,"test_report")!=binding_for(&context,"test_report"));
        self.set_context(context);
        if changed {self.messages.clear();self.steps.clear();self.observations.clear();self.plan.clear();self.stream_preview=None;self.status="报告来源已切换，请基于当前报告重新提问。".into();}
    }
    pub(crate) fn report_ui(&mut self,ui:&mut egui::Ui,ctx:&Context,lang:Lang,t:&Tokens,service:&AiService,ready:bool) {
        // Fixed composer, scrolling conversation: growing evidence never pushes input out of view.
        egui::TopBottomPanel::bottom("report-ai-composer").show_inside(ui,|ui| {
            ui.add_enabled_ui(!self.busy(),|ui|{ui.horizontal_wrapped(|ui|{
                ui.checkbox(&mut self.read_data,text(lang,"读取报告","Read reports"));
                ui.checkbox(&mut self.cross_tools,text(lang,"跨工具","Across tools"));
                ui.checkbox(&mut self.application,text(lang,"界面操作","UI actions"));
            });});
            let editor=ui.add(egui::TextEdit::multiline(&mut self.prompt).desired_rows(3).desired_width(f32::INFINITY).char_limit(16000)
                .hint_text(text(lang,"总结报告、核查异常或生成分析报告…\nCtrl+Enter 发送","Summarize, investigate or create an analysis…\nCtrl+Enter to send")));
            let send=ready&&!self.busy()&&!self.prompt.trim().is_empty();
            let shortcut=editor.has_focus()&&ui.input_mut(|i|i.consume_key(egui::Modifiers::CTRL,egui::Key::Enter));
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(send,egui::Button::new(text(lang,"发送","Send"))).clicked()||send&&shortcut {self.start(service);}
                if self.busy() && ui.button(text(lang,"停止","Stop")).clicked(){self.cancel_task("已停止分析；已发送的操作不会自动撤回。");}
                if !self.busy()&&ui.button(text(lang,"清空对话","Clear chat")).clicked(){self.messages.clear();self.steps.clear();self.observations.clear();self.plan.clear();self.workflow=Default::default();self.status.clear();}
            });
            if !ready {ui.weak(text(lang,"请通过设置启用并保存 AI 服务。","Enable and save the shared AI service in Settings."));}
        });
        egui::ScrollArea::vertical().id_salt("report-ai-conversation").auto_shrink([false,false]).stick_to_bottom(true).show(ui,|ui|{
            ui.set_width(ui.available_width());
            if self.messages.is_empty() {
                ui.label(text(lang,"基于报告原文与证据分析，不改写原始测试判定。","Analyse report evidence without changing the original test verdict."));
                for (zh,en) in [
                    ("总结当前报告：结论、关键指标与缺失证据","Summarize the current report: findings, metrics and missing evidence"),
                    ("分析失败与异常，引用报告行号并给出排查建议","Investigate failures with report line references and next checks"),
                    ("对比当前工程的报告，指出差异与可比性限制","Compare project reports, explaining differences and comparability limits"),
                    ("读取当前报告，生成独立的分析报告并打开","Read this report, create a separate analysis report and open it"),
                ] { if ui.add(egui::Button::new(text(lang,zh,en)).wrap()).clicked(){self.prompt=text(lang,zh,en).into();} }
            }
            for (i,message) in self.messages.iter().enumerate() {
                ui.push_id(i,|ui|{
                    ui.horizontal(|ui|{
                        ui.label(RichText::new(if message.role=="user"{text(lang,"你","You")}else{"AI"}).size(12.).color(t.text_muted));
                        if message.role!="user"&&ui.small_button(text(lang,"复制","Copy")).clicked(){ctx.copy_text(message.text.clone());}
                    });
                    message.markdown.ui_chat(ui,t,lang);
                    ui.add_space(8.);
                });
            }
            if let Some(preview)=&self.stream_preview {ui.push_id("report-stream",|ui|{ui.weak(text(lang,"AI · 正在生成（尚未完成）","AI · Generating (not final)"));preview.ui_chat(ui,t,lang);});}
            if !self.steps.is_empty(){egui::CollapsingHeader::new(format!("{} · {}",text(lang,"执行记录与证据","Activity and evidence"),self.steps.len())).show(ui,|ui|{
                for step in &self.steps {egui::CollapsingHeader::new(format!("{} · {}",step.title,step.status)).id_salt(&step.id).show(ui,|ui|{
                    ui.add(egui::Label::new(serde_json::to_string_pretty(&step.data).unwrap_or_default()).wrap().selectable(true));
                });}
            });}
            if let Some(call)=self.approval.clone(){
                ui.label(text(lang,"请确认本次具体操作","Confirm this specific action"));
                ui.label(&call.method);
                ui.add(egui::Label::new(serde_json::to_string_pretty(&call.params).unwrap_or_default()).wrap());
                ui.horizontal_wrapped(|ui|{
                    if ui.button(text(lang,"确认执行","Confirm")).clicked(){self.approval=None;self.confirmed=Some(call.clone());self.started=Some(Instant::now());self.enqueue(call,false);}
                    if ui.button(text(lang,"拒绝","Reject")).clicked(){self.cancel_task("已拒绝操作。");}
                });
            }
            if self.busy(){ui.spinner();}
            if !self.status.is_empty(){ui.add(egui::Label::new(RichText::new(&self.status).size(12.).color(if self.error{t.warning}else{t.text_muted})).wrap());}
            if self.usage.responses>0{ui.weak(self.usage.summary());}
        });
    }
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn changing_report_clears_old_conversation_without_crossing_sources(){
        let mut session=Session::default();
        let mut context=ContextState{page:"test_report".into(),project:"default".into(),binding:json!({"test_report":"first.md"}).to_string(),testing:false};
        session.set_report_context(context.clone());session.message("assistant","Old report evidence".into());
        session.set_report_context(context.clone());assert_eq!(session.messages.len(),1);
        context.binding=json!({"test_report":"second.md"}).to_string();session.set_report_context(context);
        assert!(session.messages.is_empty());assert!(session.observations.is_empty());assert!(!session.busy());
    }
}
