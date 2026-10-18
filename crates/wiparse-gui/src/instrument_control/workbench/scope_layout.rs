//! Scope-only proportions: compact controls and a real display-shaped preview.
use super::*;

impl InstrumentControlPanel {
    pub(super) fn compact_scope_dashboard(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
    ) {
        let id = self.devices[index].id;
        egui::ScrollArea::vertical()
            .id_salt(("compact-scope", id))
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .max_height(ui.available_height().max(1.0))
            .show(ui, |ui| {
                let width = ui.available_width().min(1680.0);
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                if width >= 1100.0 {
                    let column = (width - 16.0) * 0.5;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let controls = ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                                self.console_toolbar(ui, lang, t, index);
                                ui.add_space(4.0);
                                self.scope_channel_grid(ui, lang, t, index, 2);
                                self.scope_trigger_bar(ui, lang, t, index);
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                self.scope_preview_panel(
                                    ui,
                                    lang,
                                    t,
                                    index,
                                    defer,
                                    (controls.response.rect.height() - 24.0).max(320.0),
                                );
                            },
                        );
                    });
                } else {
                    self.console_toolbar(ui, lang, t, index);
                    ui.add_space(4.0);
                    let count = self.devices[index].capabilities.channels.clamp(1, 4) as usize;
                    let columns = (((width + 12.0) / 202.0).floor() as usize).clamp(1, count);
                    self.scope_channel_grid(ui, lang, t, index, columns);
                    self.scope_trigger_bar(ui, lang, t, index);
                    // Reserve a display-shaped viewport even before an image arrives.
                    let preview_height = ((width - 24.0).max(1.0) / 1.6 + 76.0).max(260.0);
                    self.scope_preview_panel(ui, lang, t, index, defer, preview_height);
                }
            });
    }

    fn scope_preview_panel(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
        height: f32,
    ) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_CARD), t.surface_bg);
        theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(rect.shrink(10.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.label(
                    RichText::new(text(lang, "屏幕预览", "Screen preview"))
                        .size(theme::FONT_SECTION)
                        .strong(),
                );
                ui.add_space(6.0);
                self.scope_screenshot_ui(ui, lang, t, self.devices[index].id, index, defer);
            },
        );
        ui.add_space(16.0);
    }

    fn scope_channel_grid(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        columns: usize,
    ) {
        let count = self.devices[index].capabilities.channels.clamp(1, 4) as usize;
        let id = self.devices[index].id;
        let columns = columns.min(count);
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
        for start in (0..count).step_by(columns) {
            ui.columns(columns, |uis| {
                for (offset, ui) in uis.iter_mut().enumerate() {
                    let channel = start + offset + 1;
                    if channel > count {
                        break;
                    }
                    ui.push_id(("scope-card", id, channel), |ui| {
                        let width = ui.available_width();
                        Frame::NONE
                            .fill(t.surface_bg)
                            .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width((width - 20.0).max(1.0));
                                ui.spacing_mut().item_spacing.y = 5.0;
                                let mut commands = Vec::new();
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("CH{channel}"))
                                            .size(theme::FONT_SECTION)
                                            .strong(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if toggle(
                                                ui,
                                                t,
                                                &mut self.devices[index].controls.scope_channel_on
                                                    [channel - 1],
                                            )
                                            .on_hover_text(text(
                                                lang,
                                                "显示此通道",
                                                "Display this channel",
                                            ))
                                            .changed()
                                            {
                                                commands.push(ControlCommand::ScopeChannel {
                                                    channel: channel as u8,
                                                    enabled: self.devices[index]
                                                        .controls
                                                        .scope_channel_on[channel - 1],
                                                });
                                            }
                                        },
                                    );
                                });
                                commands.extend(channels::scope_channel(
                                    ui,
                                    lang,
                                    t,
                                    &mut self.devices[index],
                                    channel as u8,
                                ));
                                self.dispatch_scope_commands(id, commands);
                            });
                    });
                }
            });
        }
    }

    fn scope_trigger_bar(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let id = self.devices[index].id;
        section(
            ui,
            t,
            text(lang, "时基与触发", "Timebase & trigger"),
            "",
            |ui| {
                let columns =
                    (((ui.available_width() + 12.0) / 180.0).floor() as usize).clamp(1, 4);
                let d = &mut self.devices[index];
                for start in (0..4).step_by(columns) {
                    ui.columns(columns, |uis| {
                        for (offset, ui) in uis.iter_mut().enumerate() {
                            let field = start + offset;
                            if field >= 4 {
                                break;
                            }
                            ui.push_id(("scope-common", id, field), |ui| {
                                ui.label(
                                    RichText::new(match field {
                                        0 => text(lang, "水平时基 · s/div", "Timebase · s/div"),
                                        1 => text(lang, "触发源", "Source"),
                                        2 => text(lang, "触发电平 · V", "Level · V"),
                                        _ => text(lang, "触发边沿", "Slope"),
                                    })
                                    .size(12.0)
                                    .color(t.text_muted),
                                );
                                match field {
                                    0 => {
                                        ui.add_sized(
                                            [ui.available_width(), 28.0],
                                            egui::DragValue::new(&mut d.controls.scope_timebase)
                                                .range(1e-9..=10.0)
                                                .max_decimals(9)
                                                .speed(0.00001),
                                        );
                                    }
                                    1 => {
                                        ui.spacing_mut().interact_size.y = 28.0;
                                        egui::ComboBox::from_id_salt(("scope-trigger-source", id))
                                            .width(ui.available_width())
                                            .selected_text(&d.controls.trigger_source)
                                            .show_ui(ui, |ui| {
                                                for ch in 1..=d.capabilities.channels.clamp(1, 4) {
                                                    let label = format!("CH{ch}");
                                                    ui.selectable_value(
                                                        &mut d.controls.trigger_source,
                                                        label.clone(),
                                                        label,
                                                    );
                                                }
                                            });
                                    }
                                    2 => {
                                        ui.add_sized(
                                            [ui.available_width(), 28.0],
                                            egui::DragValue::new(&mut d.controls.trigger_level)
                                                .speed(0.01),
                                        );
                                    }
                                    _ => {
                                        ui.columns(2, |uis| {
                                            for (i, ui) in uis.iter_mut().enumerate() {
                                                let value = if i == 0 { "RISE" } else { "FALL" };
                                                let label = if i == 0 {
                                                    text(lang, "上升沿", "Rising")
                                                } else {
                                                    text(lang, "下降沿", "Falling")
                                                };
                                                if ui
                                                    .add_sized(
                                                        [ui.available_width(), 28.0],
                                                        egui::Button::new(label)
                                                            .selected(
                                                                d.controls.trigger_slope == value,
                                                            )
                                                            .wrap(),
                                                    )
                                                    .clicked()
                                                {
                                                    d.controls.trigger_slope = value.into();
                                                }
                                            }
                                        });
                                    }
                                }
                            });
                        }
                    });
                }
                ui.add_space(4.0);
                if action(ui, t, text(lang, "应用触发", "Apply trigger"), false).clicked() {
                    let commands = vec![
                        ControlCommand::ScopeTimebase(d.controls.scope_timebase),
                        ControlCommand::ScopeTrigger {
                            source: d.controls.trigger_source.clone(),
                            level: d.controls.trigger_level,
                            slope: d.controls.trigger_slope.clone(),
                        },
                    ];
                    self.dispatch_scope_commands(id, commands);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn label(output: &egui::FullOutput, value: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == value => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {value}"))
    }
    fn enclosing(output: &egui::FullOutput, point: egui::Pos2, fill: Color32) -> Rect {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill == fill && r.rect.contains(point) && r.rect.height() > 70.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("panel rectangle")
    }
    #[test]
    fn scope_cards_are_compact_and_preview_keeps_display_proportions() {
        for light in [false, true] {
            for (width, height) in [(1100.0, 1400.0), (1920.0, 1080.0)] {
                let mut panel = InstrumentControlPanel::new(&AppConfig::default());
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = crate::theme::apply_theme(&ctx, light);
                panel.handle_event(
                    &ctx,
                    connected_event(1, &open_demo_session(InstrumentKind::Oscilloscope).unwrap()),
                );
                panel.selected_kind = InstrumentKind::Oscilloscope;
                panel.selected_id = Some(1);
                panel.overview_open = false;
                for i in 0..4 {
                    panel.devices[0].controls.scope_meas_results[i] =
                        Some(format!("{}", 150000 + i * 10000));
                }
                let (tx, rx) = unbounded();
                panel.tx = tx;
                let size = egui::vec2(width, height);
                let mut output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.ui(ui, Lang::Zh, &t);
                        });
                    },
                );
                for _ in 0..2 {
                    let next = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                panel.ui(ui, Lang::Zh, &t);
                            });
                        },
                    );
                    output.textures_delta.append(next.textures_delta);
                    output.shapes = next.shapes;
                }
                let cards: Vec<_> = (1..=4)
                    .map(|ch| enclosing(&output, label(&output, &format!("CH{ch}")), t.surface_bg))
                    .collect();
                assert!(cards
                    .iter()
                    .all(|r| r.height() < 300.0 && r.right() <= width));
                let center = label(&output, "点击「屏幕截图」抓取仪器画面");
                let display = enclosing(&output, center, t.plot_bg);
                let ratio = display.width() / display.height();
                assert!(
                    (0.8..=2.4).contains(&ratio),
                    "display ratio={ratio}, {display:?}"
                );
                if width > 1500.0 {
                    let preview = enclosing(&output, center, t.surface_bg);
                    for caption in ["运行", "停止", "单次", "自动设置"] {
                        let center = label(&output, caption);
                        let button = output
                            .shapes
                            .iter()
                            .filter_map(|shape| {
                                if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                                    (rect.rect.contains(center)
                                        && (rect.rect.height() - 32.0).abs() < 1.0)
                                        .then_some(rect.rect)
                                } else {
                                    None
                                }
                            })
                            .min_by(|a, b| a.area().total_cmp(&b.area()))
                            .expect("toolbar button rectangle");
                        assert!((button.top() - preview.top()).abs() < 1.0);
                    }
                    assert!(
                        (enclosing(&output, label(&output, "时基与触发"), t.surface_bg).bottom()
                            - preview.bottom())
                        .abs()
                            < 1.0,
                        "cards={cards:?} preview={preview:?}"
                    );
                    let trigger = enclosing(&output, label(&output, "时基与触发"), t.surface_bg);
                    assert!((trigger.right() - cards[1].right()).abs() < 1.0);
                    assert_eq!(cards[0].top(), cards[1].top());
                    assert_eq!(cards[2].top(), cards[3].top());
                    assert_eq!(cards[0].left(), cards[2].left());
                }
                assert!(rx.try_recv().is_err());
                if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                    output
                        .textures_delta
                        .set
                        .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                    output.textures_delta.set.push((
                        egui::TextureId::Managed(0),
                        egui::epaint::ImageDelta::full(
                            ctx.fonts(|f| f.image()),
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                    crate::ui_audit::export_frame(
                        &ctx,
                        output,
                        &PathBuf::from(dir),
                        &format!(
                            "scope-balanced-{}-{}",
                            width as u32,
                            if light { "light" } else { "dark" }
                        ),
                        size,
                    );
                }
                panel.on_exit();
            }
        }
    }

    #[test]
    fn compact_header_switch_and_common_trigger_keep_their_command_targets() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        panel.handle_event(
            &ctx,
            connected_event(1, &open_demo_session(InstrumentKind::Oscilloscope).unwrap()),
        );
        let (tx, rx) = unbounded();
        panel.tx = tx;
        let mut time = 0.0;
        let mut frame = |panel: &mut InstrumentControlPanel, events| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600.0, 1200.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.compact_scope_dashboard(ui, Lang::En, &Tokens::dark(), 0, false);
                    });
                },
            )
        };
        let output = frame(&mut panel, vec![]);
        let card = enclosing(&output, label(&output, "CH3"), Tokens::dark().surface_bg);
        let p = egui::pos2(card.right() - 28.0, card.top() + 21.0);
        frame(&mut panel, vec![egui::Event::PointerMoved(p)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        let jobs: Vec<_> = rx.try_iter().collect();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(
            &jobs[0],
            Job::Command {
                command: ControlCommand::ScopeChannel {
                    channel: 3,
                    enabled: true
                },
                ..
            }
        ));
        assert_eq!(
            panel.devices[0].controls.scope_channel_on,
            [true, false, true, false]
        );
        let d = &mut panel.devices[0].controls;
        d.scope_timebase = 3e-6;
        d.trigger_source = "CH4".into();
        d.trigger_level = 1.25;
        d.trigger_slope = "FALL".into();
        let output = frame(&mut panel, vec![]);
        let p = label(&output, "Apply trigger");
        frame(&mut panel, vec![egui::Event::PointerMoved(p)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        let jobs: Vec<_> = rx.try_iter().collect();
        assert_eq!(jobs.len(), 2);
        assert!(
            matches!(&jobs[0],Job::Command{command:ControlCommand::ScopeTimebase(v),..} if *v==3e-6)
        );
        assert!(
            matches!(&jobs[1],Job::Command{command:ControlCommand::ScopeTrigger{source,level,slope},..} if source=="CH4" && *level==1.25 && slope=="FALL")
        );
        panel.on_exit();
    }
}
