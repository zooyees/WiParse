//! Workspace navigation and configuration cards. No plugin or hardware semantics.
use super::*;
#[cfg(test)]
mod tests;

pub(super) const CONFIG_ROW_H: f32 = 50.0;

impl TestToolPanel {
    pub(in crate::test_tool) fn paint_workspace_heading(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        plugin: &PluginInfo,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let title = if zh && !plugin.name_zh.is_empty() {
            &plugin.name_zh
        } else {
            &plugin.name
        };
        let title_row = |panel: &mut Self, ui: &mut egui::Ui| {
            ui.set_min_height(ui_theme::CTRL_H);
            let title_w = (ui.available_width() - 90.0).max(8.0);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(title_w, ui_theme::CTRL_H), egui::Sense::hover());
            text_in_rect(ui, rect, title, ui_theme::FONT_SECTION, tokens.text_primary);
            response.on_hover_text(title);
            ui.label(
                RichText::new(&plugin.version)
                    .small()
                    .color(tokens.text_muted),
            );
            ui.menu_button(if zh { "更多" } else { "More" }, |ui| {
                if ui
                    .button(if zh {
                        "刷新工作区"
                    } else {
                        "Refresh workspace"
                    })
                    .clicked()
                {
                    panel.workspace.reload = true;
                    ui.close_menu();
                }
                if ui.button(tr(lang, "test_tool.clear_log")).clicked() {
                    panel.log.clear();
                    ui.close_menu();
                }
                if plugin.source == "marketplace"
                    && panel.job.is_none()
                    && !panel.catalog_busy
                    && ui.button(tr(lang, "test_tool.market_uninstall")).clicked()
                {
                    panel.uninstall_marketplace_plugin(&plugin.id, lang);
                    ui.close_menu();
                }
            });
        };
        if ui.available_width() >= 1000.0 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = ui_theme::SPACE_MD;
                ui.allocate_ui_with_layout(
                    egui::vec2(290.0, ui_theme::CTRL_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| title_row(self, ui),
                );
                ui.separator();
                self.paint_workspace_tabs(ui, lang);
            });
        } else {
            ui.horizontal(|ui| title_row(self, ui));
            self.paint_workspace_tabs(ui, lang);
        }
        ui.add_space(ui_theme::SPACE_SM);
    }

    fn paint_workspace_tabs(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let zh = matches!(lang, Lang::Zh);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
            for (index, cn, en) in [
                (0, "配置", "Configurations"),
                (1, "测试矩阵", "Test matrix"),
                (2, "本轮参数", "Run parameters"),
                (3, "运行日志", "Run log"),
                (4, "工程设置", "Project settings"),
            ] {
                if index == 4 && !self.workspace.snapshot["project_config"].is_object() {
                    continue;
                }
                ui.selectable_value(&mut self.workspace.tab, index, if zh { cn } else { en });
            }
            if self.workspace.rx.is_some() {
                ui.spinner();
            }
        });
    }

    pub(super) fn paint_workspace_command_band(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
    ) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .corner_radius(ui_theme::RADIUS_CARD)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if ui.available_width() >= 1000.0
                    && self.workspace.snapshot["project_files"].is_object()
                {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(300.0, 54.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.paint_project_file_bar(ui, lang),
                        );
                        ui.add_space(ui_theme::SPACE_MD);
                        let (divider, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 54.0), egui::Sense::hover());
                        ui.painter().vline(
                            divider.center().x,
                            divider.y_range(),
                            Stroke::new(1.0, tokens.divider),
                        );
                        ui.add_space(ui_theme::SPACE_MD);
                        ui.vertical(|ui| self.paint_workspace_execution(ui, lang, tokens));
                    });
                } else {
                    self.paint_project_file_bar(ui, lang);
                    if self.workspace.snapshot["project_files"].is_object() {
                        ui.separator();
                    }
                    self.paint_workspace_execution(ui, lang, tokens);
                }
            });
    }

    pub(super) fn paint_config_edit_actions(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        locked: bool,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let dirty = self.workspace.document != self.workspace.original;
        let busy = self.workspace.rx.is_some() || locked;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
            if ui
                .add_enabled(
                    !busy && dirty,
                    egui::Button::new(
                        RichText::new(if zh { "保存修改" } else { "Save changes" })
                            .color(tokens.accent_text),
                    )
                    .fill(tokens.accent),
                )
                .clicked()
            {
                self.workspace_save_outstanding();
            }
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if zh { "校验配置" } else { "Validate" }),
                )
                .clicked()
            {
                self.workspace_send("config.save", self.workspace_save_params(true));
            }
            if dirty
                && ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(if zh { "放弃修改" } else { "Discard" }),
                    )
                    .clicked()
            {
                self.workspace_discard();
            }
        });
    }

    pub(super) fn paint_config_batch_menu(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        rows: &[Value],
    ) {
        let zh = matches!(lang, Lang::Zh);
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.menu_button(if zh { "批量操作" } else { "Selection & batch" }, |ui| {
                if ui.button(if zh { "勾选筛选结果" } else { "Select filtered items" }).clicked() {
                    self.workspace.selected.extend(rows.iter().filter(|r|r["draft"]!=true)
                        .filter_map(|r|r["id"].as_str().map(str::to_owned))); ui.close_menu();
                }
                if ui.button(if zh { "反选筛选结果" } else { "Invert filtered selection" }).clicked() {
                    for row in rows.iter().filter(|r|r["draft"]!=true) {
                        let id=row["id"].as_str().unwrap_or("").to_owned();
                        if !self.workspace.selected.remove(&id) { self.workspace.selected.insert(id); }
                    }
                    ui.close_menu();
                }
                if ui.button(if zh { "清空运行勾选" } else { "Clear execution selection" }).clicked() {
                    self.workspace.selected.clear(); ui.close_menu();
                }
                ui.separator();
                let allowed = self.workspace.rx.is_none() && !self.workspace.selected.is_empty();
                for (enabled, cn, en) in [(true,"启用所选配置","Enable selected"),(false,"禁用所选配置","Disable selected")] {
                    if ui.add_enabled(allowed, egui::Button::new(if zh { cn } else { en })).clicked() {
                        let expected = self.config_batch_revisions();
                        self.workspace_send("config.batch",json!({"ids":self.workspace.selected,"expected":expected,"enabled":enabled}));
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.label(if zh { "批量标签" } else { "Batch tags" });
                ui.add(egui::TextEdit::singleline(&mut self.workspace.batch_tags).desired_width(200.0)
                    .hint_text(if zh { "逗号分隔" } else { "Comma-separated" }));
                if ui.add_enabled(allowed,egui::Button::new(if zh { "应用到勾选项" } else { "Apply to selected" })).clicked() {
                    let expected = self.config_batch_revisions();
                    let tags: Vec<_> = self.workspace.batch_tags.split(',').map(str::trim).filter(|s|!s.is_empty()).collect();
                    self.workspace_send("config.batch",json!({"ids":self.workspace.selected,"expected":expected,"tags":tags}));
                    ui.close_menu();
                }
            });
        });
    }

    fn config_batch_revisions(&self) -> Value {
        let mut expected = json!({});
        for row in self.workspace.snapshot["items"]
            .as_array()
            .into_iter()
            .flatten()
        {
            expected[row["id"].as_str().unwrap_or("")] =
                json!({"expected_revision":row["revision"],"expected_sha256":row["sha256"]});
        }
        expected
    }
}

pub(super) fn config_card(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    name: &str,
    status: &str,
    tooltip: &str,
    zh: bool,
    tone: egui::Color32,
    current: bool,
    can_select: bool,
    selected: &mut bool,
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), CONFIG_ROW_H),
        egui::Sense::hover(),
    );
    ui.painter().rect(
        rect,
        6,
        if current {
            tokens.accent_soft
        } else {
            tokens.panel_bg
        },
        Stroke::new(
            1.0,
            if current {
                tokens.accent
            } else {
                tokens.divider
            },
        ),
        egui::StrokeKind::Inside,
    );
    let check = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 18.0, rect.center().y),
        egui::vec2(22.0, 24.0),
    );
    let edit = egui::Rect::from_min_max(egui::pos2(rect.left() + 36.0, rect.top()), rect.max);
    let changed = ui_theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .id_salt("run_check")
            .max_rect(check)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::LeftToRight,
            )),
        |ui| ui.add_enabled(can_select, egui::Checkbox::without_text(selected)),
    )
    .inner
    .on_hover_text(if zh {
        "勾选此项参与测试运行"
    } else {
        "Include this item in the test run"
    })
    .changed();
    let name_rect = egui::Rect::from_min_max(
        edit.min + egui::vec2(5.0, 5.0),
        egui::pos2(rect.right() - 8.0, rect.top() + 26.0),
    );
    let status_rect = egui::Rect::from_min_max(
        egui::pos2(name_rect.left(), rect.top() + 27.0),
        rect.max - egui::vec2(8.0, 4.0),
    );
    text_in_rect(
        ui,
        name_rect,
        name,
        ui_theme::FONT_BODY,
        tokens.text_primary,
    );
    text_in_rect(ui, status_rect, status, ui_theme::FONT_CAPTION, tone);
    let clicked = ui
        .interact(edit, ui.id().with("edit"), egui::Sense::click())
        .on_hover_text(tooltip)
        .clicked();
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        let key = egui::Id::new("config_card_audit");
        let mut cards = data
            .get_temp::<Vec<(egui::Rect, egui::Rect, egui::Rect)>>(key)
            .unwrap_or_default();
        cards.push((rect, check, edit));
        data.insert_temp(key, cards);
    });
    (changed, clicked)
}

fn text_in_rect(ui: &egui::Ui, rect: egui::Rect, text: &str, size: f32, color: egui::Color32) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(size),
        color,
        rect.width().max(1.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(ui.clip_rect().intersect(rect))
        .galley(pos, galley, color);
}
