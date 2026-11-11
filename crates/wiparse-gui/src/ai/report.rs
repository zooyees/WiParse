//! Dedicated report conversation; shared provider, permissions and host tools.
use super::*;

pub(crate) struct ReportAssistant {
    pub open: bool,
    pub session: agent::Session,
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn report_sidebar_fits_narrow_and_wide_viewports_in_both_themes(){
        for width in [620.,1200.] {for light in [false,true] {for lang in [Lang::Zh,Lang::En] {
            let ctx=Context::default();ctx.set_visuals(if light{egui::Visuals::light()}else{egui::Visuals::dark()});
            let cfg=AiConfig::default();ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-global-ai"),AiService::new(cfg.clone())));
            let mut panel=ReportAssistant::default();panel.open=true;
            let _=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,600.))),..Default::default()},|ctx|{
                assert!(!panel.ui(ctx,lang,&ui_theme::tokens_for_preference(light),&cfg,"default · 很长的测试报告名称 / selected-report.md"));
                assert!(ctx.available_rect().width()>=width*0.50);
                egui::CentralPanel::default().show(ctx,|ui|{ui.label("Report");});
            });
            assert!(!panel.session.busy());
        }}}
    }
}
impl Default for ReportAssistant {
    fn default()->Self { let mut session=agent::Session::default();session.application=true;Self{open:false,session} }
}
impl ReportAssistant {
    /// Returns a request to open shared connection settings.
    pub fn ui(&mut self, ctx:&Context, lang:Lang, t:&Tokens, config:&AiConfig, source:&str)->bool {
        if !self.open {return false;}
        let mut settings=false;
        let max=(ctx.available_rect().width()*0.48).max(180.0);
        egui::SidePanel::right("report-ai-sidebar").resizable(true)
            .default_width(400.0_f32.min(max)).min_width(280.0_f32.min(max)).max_width(max)
            .frame(Frame::NONE.fill(t.surface_bg).inner_margin(8).stroke(Stroke::new(1.0,t.divider)))
            .show(ctx,|ui| {
                ui.horizontal(|ui| {
                    ui.strong(text(lang,"报告 AI","Report AI"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                        if ui.small_button("×").on_hover_text(text(lang,"收起窗口，任务继续","Hide; task continues")).clicked(){self.open=false;}
                        if ui.small_button(text(lang,"设置","Settings")).clicked(){settings=true;}
                    });
                });
                ui.add(egui::Label::new(RichText::new(source).size(12.0).color(t.text_muted)).truncate()).on_hover_text(source);
                ui.separator();
                if let Some(service)=service(ctx) {
                    self.session.report_ui(ui,ctx,lang,t,&service,config.enabled);
                } else {ui.label(text(lang,"AI 服务尚未初始化","AI service is not initialized"));}
            });
        settings
    }
}
