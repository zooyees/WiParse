//! Chat typography is intentionally separate from report/document typography.
use super::*;

impl TextMarkdown {
    pub(crate) fn ui_chat(&self, ui: &mut egui::Ui, t: &Tokens, lang: Lang) {
        for (index, block) in self.0.blocks.iter().enumerate() {
            ui.push_id(index, |ui| match block {
                MdBlock::Heading { level, text } => {
                    ui.add_space(if index == 0 { 0.0 } else { 10.0 });
                    inline(ui, text, if *level <= 2 { 14.0 } else { 13.0 }, true, t);
                    ui.add_space(2.0);
                }
                MdBlock::Paragraph { text } => {
                    inline(ui, text, 13.0, false, t);
                    ui.add_space(5.0);
                }
                MdBlock::Bullet { depth, text }
                | MdBlock::Ordered { depth, text, .. }
                | MdBlock::Task { depth, text, .. } => {
                    let marker = match block {
                        MdBlock::Ordered { n, .. } => format!("{n}."),
                        MdBlock::Task { checked, .. } => if *checked { "☑" } else { "☐" }.into(),
                        _ => "•".into(),
                    };
                    ui.horizontal_top(|ui| {
                        ui.add_space(f32::from(*depth) * 12.0);
                        ui.label(RichText::new(marker).size(13.0));
                        inline(ui, text, 13.0, false, t);
                    });
                    ui.add_space(3.0);
                }
                MdBlock::Quote { text } => {
                    let width = ui.available_width();
                    Frame::NONE
                        .fill(t.header_bg)
                        .inner_margin(10)
                        .corner_radius(6)
                        .show(ui, |ui| {
                            ui.set_width((width - 20.0).max(1.0));
                            inline(ui, text, 13.0, false, t);
                        });
                    ui.add_space(5.0);
                }
                MdBlock::Code {
                    lang: code_lang,
                    body,
                } => {
                    if code_lang.trim().eq_ignore_ascii_case("mermaid") {
                        super::mermaid::show(ui, body, lang);
                        return;
                    }
                    let width = ui.available_width();
                    Frame::NONE
                        .fill(t.input_bg)
                        .inner_margin(10)
                        .corner_radius(6)
                        .show(ui, |ui| {
                            ui.set_width((width - 20.0).max(1.0));
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if code_lang.is_empty() {
                                        "code"
                                    } else {
                                        code_lang
                                    })
                                    .size(12.0)
                                    .color(t.text_muted),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .button(
                                                RichText::new(if lang == Lang::Zh {
                                                    "复制"
                                                } else {
                                                    "Copy"
                                                })
                                                .size(12.0),
                                            )
                                            .clicked()
                                        {
                                            ui.ctx().copy_text(body.clone());
                                        }
                                    },
                                );
                            });
                            egui::ScrollArea::horizontal()
                                .id_salt("chat-code")
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(body)
                                                .monospace()
                                                .size(13.0)
                                                .color(t.text_primary),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Extend)
                                        .selectable(true),
                                    );
                                });
                        });
                    ui.add_space(6.0);
                }
                MdBlock::Table { rows } => {
                    let count = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                    let width = ui.available_width();
                    egui::ScrollArea::horizontal()
                        .id_salt("chat-table")
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let total = if count > 2 {
                                width.max(count as f32 * 115.0)
                            } else {
                                width
                            };
                            let gap = 12.0;
                            let widths: Vec<f32> = (0..count)
                                .map(|c| {
                                    if count == 2 {
                                        ((total - gap) * if c == 0 { 0.38 } else { 0.62 }).max(1.0)
                                    } else {
                                        ((total - gap * (count - 1) as f32) / count as f32).max(1.0)
                                    }
                                })
                                .collect();
                            egui::Grid::new("chat-table-grid")
                                .spacing([gap, 7.0])
                                .show(ui, |ui| {
                                    for (r, row) in rows.iter().enumerate() {
                                        for (c, cell_width) in widths.iter().enumerate() {
                                            ui.vertical(|ui| {
                                                ui.set_width(*cell_width);
                                                ui.set_max_width(*cell_width);
                                                inline(
                                                    ui,
                                                    row.get(c).map(String::as_str).unwrap_or(""),
                                                    13.0,
                                                    r == 0,
                                                    t,
                                                );
                                            });
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.add_space(7.0);
                }
                MdBlock::Image { alt, src } => {
                    ui.label(
                        RichText::new(format!(
                            "[{}] {alt}",
                            if lang == Lang::Zh { "图片" } else { "Image" }
                        ))
                        .size(13.0)
                        .color(t.text_muted),
                    )
                    .on_hover_text(src);
                }
                MdBlock::Rule => {
                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(3.0);
                }
            });
        }
    }
}

fn inline(ui: &mut egui::Ui, text: &str, size: f32, strong: bool, t: &Tokens) {
    let (mut job, links) = md_inline_layout(text, size, strong, t);
    job.wrap.max_width = ui.available_width().max(1.0);
    job.wrap.break_anywhere = true;
    for section in &mut job.sections {
        section.format.line_height = Some(size * 1.5);
        if section.format.font_id.family == egui::FontFamily::Monospace {
            section.format.font_id = FontId::proportional(size);
            section.format.color = t.text_primary;
            section.format.background = Color32::TRANSPARENT;
        }
    }
    let galley = ui.fonts(|f| f.layout_job(job));
    let response = ui.add(egui::Label::new(galley.clone()).selectable(true));
    for (link, (section, target)) in links.iter().enumerate() {
        if !(target.starts_with("https://") || target.starts_with("http://"))
            || target.chars().any(char::is_control)
        {
            continue;
        }
        for (r, row) in galley.rows.iter().enumerate() {
            let rect = row
                .glyphs
                .iter()
                .filter(|g| g.section_index == *section)
                .fold(egui::Rect::NOTHING, |a, g| a.union(g.logical_rect()));
            if !rect.is_positive() {
                continue;
            }
            if ui
                .interact(
                    rect.translate(response.rect.min.to_vec2()),
                    response.id.with(("chat-link", link, r)),
                    egui::Sense::click(),
                )
                .on_hover_text(target)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(target));
            }
        }
    }
}
