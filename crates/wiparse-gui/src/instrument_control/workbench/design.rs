//! Instrument-local composition, using the application's shared palette and type scale.
use super::*;

pub(super) fn section(
    ui: &mut egui::Ui,
    t: &Tokens,
    title: &str,
    subtitle: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    Frame::NONE
        .fill(t.surface_bg)
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width((width - 24.0).max(1.0));
            if !title.is_empty() {
                ui.label(
                    RichText::new(title)
                        .size(theme::FONT_SECTION)
                        .strong()
                        .color(t.text_primary),
                );
                if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).size(12.0).color(t.text_muted));
                }
                ui.add_space(12.0);
            }
            body(ui);
        });
    ui.add_space(12.0);
}

/// Every label, value and accessory follows the same row alignment.
pub(super) fn property<R>(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    edit: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 34.0), egui::Sense::hover());
    let label_w = (width * 0.38).clamp(88.0, 124.0).min(width * 0.48);
    let label_rect = Rect::from_min_max(rect.min, egui::pos2(rect.left() + label_w, rect.bottom()));
    let value_rect = Rect::from_min_max(egui::pos2(label_rect.right() + 8.0, rect.top()), rect.max);
    theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(label_rect.intersect(ui.clip_rect()));
            ui.add(egui::Label::new(RichText::new(label).color(t.text_muted)).truncate())
                .on_hover_text(label);
        },
    );
    let mut result = None;
    ui.push_id(label, |ui| {
        theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(value_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(value_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().interact_size = egui::vec2(value_rect.width().max(1.0), 28.0);
                ui.spacing_mut().text_edit_width = value_rect.width().max(1.0);
                ui.spacing_mut().slider_width = value_rect.width().max(20.0);
                result = Some(edit(ui));
            },
        );
    });
    ui.painter().line_segment(
        [egui::pos2(rect.left(), rect.bottom()), rect.right_bottom()],
        Stroke::new(0.5_f32, t.divider),
    );
    result.unwrap()
}

pub(super) fn field(ui: &mut egui::Ui, value: &mut String, hint: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 28.0],
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, 28.0))
            .hint_text(hint),
    )
}

pub(super) fn toggle(ui: &mut egui::Ui, t: &Tokens, value: &mut bool) -> egui::Response {
    let (r, mut response) = ui.allocate_exact_size(egui::vec2(36.0, 22.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let x = ui.ctx().animate_bool(response.id, *value);
    ui.painter().rect_filled(
        r,
        CornerRadius::same(11),
        if *value { t.accent } else { t.button_hover },
    );
    ui.painter().circle_filled(
        egui::pos2(r.left() + 11.0 + x * 14.0, r.center().y),
        8.0,
        Color32::WHITE,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            r.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(2.0_f32, t.focus_ring),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *value,
            "toggle",
        )
    });
    response
}

pub(super) fn segments(
    ui: &mut egui::Ui,
    t: &Tokens,
    id: impl std::hash::Hash,
    selected: &mut u8,
    labels: &[&str],
) {
    ui.push_id(id, |ui| {
        let width = ui.available_width();
        Frame::NONE
            .fill(t.input_bg)
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(3))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let item_w = ((width - 6.0 - 2.0 * labels.len().saturating_sub(1) as f32)
                    / labels.len().max(1) as f32)
                    .max(1.0);
                ui.horizontal(|ui| {
                    for (i, label) in labels.iter().enumerate() {
                        let active = *selected == i as u8;
                        let response = ui.add_sized(
                            [item_w, 28.0],
                            egui::Button::new(RichText::new(*label).color(if active {
                                t.text_primary
                            } else {
                                t.text_muted
                            }))
                            .fill(if active {
                                t.button_hover
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(5))
                            .wrap(),
                        );
                        if response.clicked() {
                            *selected = i as u8;
                        }
                    }
                });
            });
    });
}

pub(super) fn action(ui: &mut egui::Ui, t: &Tokens, label: &str, primary: bool) -> egui::Response {
    action_enabled(ui, t, label, primary, true)
}

pub(super) fn action_enabled(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    primary: bool,
    enabled: bool,
) -> egui::Response {
    let width = if matches!(
        ui.layout().main_dir(),
        egui::Direction::TopDown | egui::Direction::BottomUp
    ) {
        ui.available_width().min(160.0)
    } else {
        ui.spacing().interact_size.x
    }
    .max(1.0);
    ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        ui.add_sized(
            [width, 32.0],
            egui::Button::new(RichText::new(label).color(if primary {
                t.accent_text
            } else {
                t.text_primary
            }))
            .fill(if primary { t.accent } else { t.button_bg })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .wrap(),
        )
    })
}

/// Reserve equal columns before drawing; do not wrap based on translated label length.
pub(super) fn action_group<R>(
    ui: &mut egui::Ui,
    columns: usize,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width().max(1.0);
    let columns = columns
        .min(((width + 8.0) / 112.0).floor().max(1.0) as usize)
        .max(1);
    let width = width.min(columns as f32 * 160.0 + 8.0 * (columns - 1) as f32);
    let cell = (width - 8.0 * (columns - 1) as f32) / columns as f32;
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            ui.spacing_mut().interact_size = egui::vec2(cell, 32.0);
            ui.horizontal_wrapped(add).inner
        },
    )
    .inner
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn parameter_editors_share_columns_and_actions_form_equal_rows() {
        for width in [240.0, 340.0, 460.0] {
            let ctx = egui::Context::default();
            let mut number = 12.0;
            let mut value = "COM5".to_string();
            let mut fields = Vec::new();
            let mut buttons = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width + 16.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(width);
                        let t = Tokens::dark();
                        fields.push(
                            property(ui, &t, "Voltage", |ui| {
                                ui.add(egui::DragValue::new(&mut number))
                            })
                            .interact_rect,
                        );
                        fields.push(
                            property(ui, &t, "Port", |ui| field(ui, &mut value, "COM"))
                                .interact_rect,
                        );
                        action_group(ui, 2, |ui| {
                            for name in ["Open", "Send", "Read", "Close"] {
                                buttons.push(action(ui, &t, name, false).rect);
                            }
                        });
                    });
                },
            );
            assert!(
                (fields[0].left() - fields[1].left()).abs() < 1.0,
                "field columns: {fields:?}"
            );
            assert!(
                (fields[0].right() - fields[1].right()).abs() < 1.0,
                "numeric and text fields must have equal widths: {fields:?}"
            );
            assert!(fields.iter().all(|r| r.height() >= 28.0));
            assert!((buttons[0].width() - buttons[1].width()).abs() < 1.0);
            assert!((buttons[0].top() - buttons[1].top()).abs() < 1.0);
            assert!(
                (buttons[0].left() - buttons[2].left()).abs() < 1.0,
                "second action row must align: {buttons:?}"
            );
            assert!(buttons[2].top() >= buttons[0].bottom());
        }
    }
}

pub(super) fn metric(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    value: Option<&Reading>,
    size: f32,
) {
    ui.label(RichText::new(label).color(t.text_muted).size(12.0));
    ui.horizontal(|ui| {
        let number = value
            .filter(|r| r.value.is_finite())
            .map(|r| format!("{:.4}", r.value))
            .unwrap_or_else(|| "—".into());
        ui.add(
            egui::Label::new(
                RichText::new(number)
                    .monospace()
                    .size(size)
                    .color(t.text_primary),
            )
            .truncate(),
        );
        if let Some(value) = value {
            ui.label(RichText::new(&value.unit).size(14.0).color(t.text_muted));
        }
    });
}

pub(super) fn split(ui: &mut egui::Ui, mut content: impl FnMut(&mut egui::Ui, usize)) {
    let width = ui.available_width();
    if width >= 720.0 {
        let left = (width - 16.0) * 0.5;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for (i, width) in [left, (width - left - 16.0).max(1.0)]
                .into_iter()
                .enumerate()
            {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        content(ui, i);
                    },
                );
            }
        });
    } else {
        content(ui, 0);
        content(ui, 1);
    }
}

pub(super) fn nav_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    kind: Option<InstrumentKind>,
    title: &str,
    detail: &str,
    selected: bool,
    connected: bool,
) -> egui::Response {
    let (r, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            r,
            CornerRadius::same(7),
            if selected { t.accent_soft } else { t.button_bg },
        );
    }
    let icon = Rect::from_center_size(
        egui::pos2(r.left() + 22.0, r.center().y),
        egui::vec2(22.0, 20.0),
    );
    let color = if selected { t.accent } else { t.text_muted };
    ui.painter().rect_stroke(
        icon,
        CornerRadius::same(4),
        Stroke::new(1.2_f32, color),
        egui::StrokeKind::Inside,
    );
    let points = match kind {
        Some(InstrumentKind::Oscilloscope) => vec![
            (-7.0, 2.0),
            (-4.0, 2.0),
            (-2.0, -4.0),
            (1.0, 4.0),
            (4.0, -2.0),
            (7.0, -2.0),
        ],
        Some(InstrumentKind::DcSource) => vec![(-6.0, 0.0), (6.0, 0.0)],
        _ => vec![(-6.0, -2.0), (6.0, -2.0), (6.0, 2.0), (-6.0, 2.0)],
    };
    ui.painter().add(egui::Shape::line(
        points
            .into_iter()
            .map(|(x, y)| icon.center() + egui::vec2(x, y))
            .collect(),
        Stroke::new(1.2_f32, color),
    ));
    let rect = Rect::from_min_max(
        egui::pos2(r.left() + 42.0, r.top() + 7.0),
        egui::pos2(r.right() - 18.0, r.bottom() - 6.0),
    );
    theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(
                egui::Label::new(RichText::new(title).strong().color(t.text_primary)).truncate(),
            );
            ui.add(
                egui::Label::new(RichText::new(detail).size(11.0).color(t.text_muted)).truncate(),
            );
        },
    );
    if connected {
        ui.painter()
            .circle_filled(egui::pos2(r.right() - 10.0, r.center().y), 2.5, t.success);
    }
    response
}

/// A predictable left-to-right operation: parameters, then explicit actions.
/// Both columns start at the same baseline; short forms retain natural height.
pub(super) fn task_form(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
    parameters: fn(&mut egui::Ui, Lang, &Tokens, &mut DeviceUi),
    actions: fn(&mut egui::Ui, Lang, &Tokens, &mut DeviceUi) -> Vec<ControlCommand>,
) -> Vec<ControlCommand> {
    let width = ui.available_width();
    if width < 460.0 {
        parameters(ui, lang, t, d);
        ui.add_space(12.0);
        return actions(ui, lang, t, d);
    }
    let mut out = Vec::new();
    let parameter_width = width - 240.0;
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.allocate_ui_with_layout(
            egui::vec2(parameter_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(parameter_width);
                parameters(ui, lang, t, d);
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(224.0, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(224.0);
                out = actions(ui, lang, t, d);
            },
        );
    });
    #[cfg(test)]
    assert!(
        row.response.rect.width() <= width + 1.0,
        "task form overflow {:?} probe={} dongle={} bridge={}: {} > {}",
        d.kind,
        d.controls.probe_page,
        d.controls.dongle_tab,
        d.controls.bridge_tab,
        row.response.rect.width(),
        width
    );
    let _ = row;
    out
}

// Fixed outer bounds let the plot/log fill its peer's measured height.
pub(super) fn aligned_panel(
    ui: &mut egui::Ui,
    t: &Tokens,
    title: &str,
    height: f32,
    body: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(120.0)),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_CARD), t.surface_bg);
    theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect.shrink(12.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.label(
                RichText::new(title)
                    .size(theme::FONT_SECTION)
                    .strong()
                    .color(t.text_primary),
            );
            ui.add_space(12.0);
            body(ui);
        },
    );
    ui.add_space(12.0);
}
