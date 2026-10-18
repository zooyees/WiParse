//! All supported channels remain visible; commands carry an explicit channel identity.
use super::*;

impl InstrumentControlPanel {
    pub(super) fn channel_dashboard(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
        include_utilities: bool,
    ) {
        if self.devices[index].kind == InstrumentKind::Oscilloscope {
            self.compact_scope_dashboard(ui, lang, t, index, defer);
            return;
        }
        let id = self.devices[index].id;
        let count = self.devices[index].capabilities.channels.clamp(1, 4) as usize;
        egui::ScrollArea::vertical()
            .id_salt(("all-channel-dashboard", id))
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .max_height(ui.available_height().max(1.0))
            .show(ui, |ui| {
                if include_utilities {
                    if let Some(error) = self.devices[index].last_error.clone() {
                        ui.colored_label(t.stop_bg, error);
                        if ui.button(text(lang, "收起", "Dismiss")).clicked() {
                            self.devices[index].last_error = None;
                        }
                    }
                    if self.busy_device == Some(id) {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(&self.busy_label);
                        });
                    }
                }
                let width = ui.available_width();
                // Keep every channel visible; use two balanced columns when controls fit.
                let side_by_side =
                    include_utilities && width >= (count as f32 * 232.0 + 24.0) * 2.0;
                if side_by_side {
                    let column = (width - 16.0) * 0.5;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let channels = ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                self.source_channel_grid(ui, lang, t, index, 220.0);
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                let height = channels.response.rect.height()
                                    - 12.0
                                    - ui.spacing().item_spacing.y;
                                aligned_panel(
                                    ui,
                                    t,
                                    text(lang, "测量趋势与导出", "Measurement trend & export"),
                                    height,
                                    |ui| {
                                        self.acquisition_ui_sized(
                                            ui, lang, t, id, index, false, true,
                                        );
                                    },
                                );
                            },
                        );
                    });
                } else {
                    self.source_channel_grid(ui, lang, t, index, 250.0);
                }
                if let Some(sample) = self
                    .samples
                    .iter()
                    .filter(|s| s.resource == self.devices[index].resource)
                    .last()
                {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            text(lang, "上次采样", "Last sample"),
                            sample.timestamp.format("%H:%M:%S%.3f")
                        ))
                        .size(11.0)
                        .color(t.text_muted),
                    );
                }
                if !include_utilities {
                    section(ui, t, text(lang, "采集控制", "Acquisition"), "", |ui| {
                        self.measurement_actions(ui, lang, t, index);
                    });
                }
                if include_utilities {
                    if !side_by_side {
                        self.source_trend_panel(ui, lang, t, index);
                    }
                    if side_by_side && count >= 3 {
                        let info_width = (width - 16.0) * 0.5;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            let info = ui.allocate_ui_with_layout(
                                egui::vec2(info_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(info_width);
                                    self.device_information_sections(ui, lang, t, index);
                                },
                            );
                            let console_width = width - info_width - 16.0;
                            ui.allocate_ui_with_layout(
                                egui::vec2(console_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(console_width);
                                    let height = info.response.rect.height()
                                        - 12.0
                                        - ui.spacing().item_spacing.y;
                                    aligned_panel(
                                        ui,
                                        t,
                                        text(lang, "SCPI 控制台", "SCPI console"),
                                        height,
                                        |ui| {
                                            self.console_ui_compact(ui, lang, t, id, index);
                                        },
                                    );
                                },
                            );
                        });
                    } else {
                        self.device_details(ui, lang, t, index);
                    }
                }
            });
    }
    fn source_channel_grid(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        min_width: f32,
    ) {
        let id = self.devices[index].id;
        let count = self.devices[index].capabilities.channels.clamp(1, 4) as usize;
        let columns =
            (((ui.available_width() + 12.0) / (min_width + 12.0)).floor() as usize).clamp(1, count);
        ui.spacing_mut().item_spacing.x = 12.0;
        for start in (0..count).step_by(columns) {
            ui.columns(columns, |uis| {
                for (offset, ui) in uis.iter_mut().enumerate() {
                    let channel = start + offset + 1;
                    if channel > count {
                        break;
                    }
                    ui.push_id(("channel-controls", id, channel), |ui| {
                        section(ui, t, &format!("CH{channel}"), "", |ui| {
                            let commands = controls::source_channel(
                                ui,
                                lang,
                                t,
                                &mut self.devices[index],
                                channel as u8,
                            );
                            self.dispatch_scope_commands(id, commands);
                            ui.separator();
                            ui.label(RichText::new(text(lang, "保护设置", "Protection")).strong());
                            let commands = controls::protection_channel(
                                ui,
                                lang,
                                t,
                                &mut self.devices[index],
                                channel as u8,
                            );
                            self.dispatch_scope_commands(id, commands);
                            ui.separator();
                            ui.label(
                                RichText::new(text(lang, "实测读数", "Measured readings")).strong(),
                            );
                            for (label, key) in [
                                (text(lang, "电压", "Voltage"), "Voltage"),
                                (text(lang, "电流", "Current"), "Current"),
                                (text(lang, "功率", "Power"), "Power"),
                            ] {
                                metric(
                                    ui,
                                    t,
                                    label,
                                    self.latest.get(&(id, format!("CH{channel} {key}"))),
                                    20.0,
                                );
                            }
                        });
                    });
                }
            });
        }
    }
    fn source_trend_panel(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let id = self.devices[index].id;
        section(
            ui,
            t,
            text(lang, "测量趋势与导出", "Measurement trend & export"),
            "",
            |ui| {
                self.acquisition_ui_with_readings(ui, lang, t, id, index, false);
            },
        );
    }
}

pub(super) fn scope_channel(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
    channel: u8,
) -> Vec<ControlCommand> {
    let i = channel as usize - 1;
    let mut commands = Vec::new();
    let columns = if ui.available_width() >= 230.0 { 2 } else { 1 };
    ui.columns(columns, |uis| {
        compact_number(
            &mut uis[0],
            t,
            text(lang, "刻度 · V/div", "Scale · V/div"),
            &mut d.controls.scope_scales[i],
            1e-6..=100.0,
            0.001,
        );
        compact_number(
            &mut uis[columns - 1],
            t,
            text(lang, "位置 · div", "Position · div"),
            &mut d.controls.scope_positions[i],
            -8.0..=8.0,
            SCOPE_POS_STEP,
        );
    });
    if action(ui, t, text(lang, "应用通道", "Apply channel"), false).clicked() {
        commands.push(ControlCommand::ScopeScale {
            channel,
            volts_per_div: d.controls.scope_scales[i],
        });
        commands.push(ControlCommand::ScopePosition {
            channel,
            divisions: d.controls.scope_positions[i],
        });
    }
    ui.add_space(2.0);
    ui.separator();
    ui.label(
        RichText::new(text(lang, "测量项目", "Measurement"))
            .color(t.text_muted)
            .size(12.0),
    );
    egui::ComboBox::from_id_salt(("channel-measurement", d.id, channel))
        .width(ui.available_width())
        .selected_text(d.controls.scope_meas_types[i].label())
        .show_ui(ui, |ui| {
            for m in ScopeMeasType::all() {
                if ui
                    .selectable_value(&mut d.controls.scope_meas_types[i], *m, m.label())
                    .changed()
                {
                    d.controls.scope_meas_results[i] = None;
                }
            }
        });
    let value = format_scope_meas_display(
        d.controls.scope_meas_results[i].as_deref().unwrap_or("—"),
        d.controls.scope_meas_types[i],
    );
    ui.horizontal(|ui| {
        let value_width = (ui.available_width() - 64.0 - ui.spacing().item_spacing.x).max(1.0);
        ui.add_sized(
            [value_width, 28.0],
            egui::Label::new(RichText::new(&value).monospace().size(20.0)).truncate(),
        )
        .on_hover_text(&value);
        if ui
            .add_sized(
                [64.0, 28.0],
                egui::Button::new(text(lang, "读取", "Read")).wrap(),
            )
            .clicked()
        {
            commands.push(ControlCommand::ScopeMeasure {
                channel,
                meas_type: d.controls.scope_meas_types[i],
            });
        }
    });
    commands
}

fn compact_number(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) {
    ui.push_id(label, |ui| {
        ui.label(RichText::new(label).color(t.text_muted).size(12.0));
        ui.add_sized(
            [ui.available_width(), 28.0],
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .max_decimals(6),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        panel: &mut InstrumentControlPanel,
        ctx: &egui::Context,
        time: &mut f64,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        *time += 0.1;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 4000.0),
                )),
                time: Some(*time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let bounds = ui.max_rect();
                    panel.channel_dashboard(ui, Lang::En, &Tokens::dark(), 0, false, false);
                    assert!(
                        ui.min_rect().right() <= bounds.right() + 1.0,
                        "kind={:?} count={} width={} actual={:?} bounds={:?}",
                        panel.devices[0].kind,
                        panel.devices[0].capabilities.channels,
                        width,
                        ui.min_rect(),
                        bounds
                    );
                    assert!(ui.min_rect().bottom() <= bounds.bottom() + 1.0);
                });
            },
        )
    }

    fn positions(output: &egui::FullOutput, label: &str) -> Vec<egui::Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn all_supported_channel_cards_are_aligned_without_selection_or_commands() {
        for kind in [InstrumentKind::Oscilloscope, InstrumentKind::DcSource] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            panel.handle_event(&ctx, connected_event(1, &open_demo_session(kind).unwrap()));
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let mut time = 0.0;
            for count in [1, 2, 3, 4] {
                panel.devices[0].capabilities.channels = count;
                for width in [420.0, 840.0, 1400.0] {
                    let output = frame(&mut panel, &ctx, &mut time, width, vec![]);
                    let mut headers = Vec::new();
                    for ch in 1..=count {
                        let p: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|s| match &s.shape {
                                egui::Shape::Text(t)
                                    if t.galley.job.text == format!("CH{ch}")
                                        && t.galley.job.sections.first().is_some_and(|s| {
                                            s.format.font_id.size == theme::FONT_SECTION
                                        }) =>
                                {
                                    Some(t.pos)
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(p.len(), 1, "{kind:?} / {width} / CH{ch}");
                        headers.push(p[0]);
                    }
                    for ch in count + 1..=4 {
                        assert!(positions(&output, &format!("CH{ch}")).is_empty());
                    }
                    if width == 1400.0 && kind != InstrumentKind::Oscilloscope {
                        assert!(headers.iter().all(|p| (p.y - headers[0].y).abs() < 0.1));
                    }
                    assert_eq!(panel.devices[0].controls.source_focus, 1);
                    assert_eq!(panel.devices[0].controls.scope_channel, 1);
                    assert!(rx.try_recv().is_err(), "rendering must not send commands");
                }
            }
            panel.on_exit();
        }
    }

    #[test]
    fn source_channels_share_half_width_beside_trend() {
        for light in [false, true] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let t = crate::theme::apply_theme(&ctx, light);
            panel.handle_event(
                &ctx,
                connected_event(1, &open_demo_session(InstrumentKind::DcSource).unwrap()),
            );
            panel.selected_kind = InstrumentKind::DcSource;
            panel.selected_id = Some(1);
            panel.overview_open = false;
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let size = egui::vec2(1920.0, 1400.0);
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
            let card = |caption: &str| {
                let point = positions(&output, caption)[0];
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                            (rect.fill == t.surface_bg && rect.rect.contains(point))
                                .then_some(rect.rect)
                        } else {
                            None
                        }
                    })
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .expect("card")
            };
            let channels: Vec<_> = (1..=3).map(|ch| card(&format!("CH{ch}"))).collect();
            let trend = card("测量趋势与导出");
            for channel in &channels {
                assert!((channel.top() - trend.top()).abs() < 1.0);
                assert!(channel.right() < trend.left());
                assert!((channel.width() - channels[0].width()).abs() < 1.0);
            }
            assert!(
                (trend.bottom() - channels[0].bottom()).abs() < 1.0,
                "trend={trend:?}, channels={channels:?}"
            );
            let info = card("设备信息");
            let maintenance = card("设备维护");
            let console = card("SCPI 控制台");
            for section in [info, card("通信与采样"), maintenance] {
                assert!((section.right() - channels[2].right()).abs() < 1.0);
            }
            assert!((console.left() - trend.left()).abs() < 1.0);
            assert!(
                (console.bottom() - maintenance.bottom()).abs() < 1.0,
                "console={console:?}, maintenance={maintenance:?}"
            );
            let group_width = channels[2].right() - channels[0].left();
            assert!((group_width - trend.width()).abs() < 1.0);
            assert!(channels[0].width() < 290.0);
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
                        "source-side-by-side-{}",
                        if light { "light" } else { "dark" }
                    ),
                    size,
                );
            }
            panel.on_exit();
        }
    }

    #[test]
    fn source_dashboard_exposes_acquisition_export_and_maintenance_without_tabs() {
        for light in [false, true] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let tokens = crate::theme::apply_theme(&ctx, light);
            panel.handle_event(
                &ctx,
                connected_event(1, &open_demo_session(InstrumentKind::DcSource).unwrap()),
            );
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let size = egui::vec2(1000.0, 2400.0);
            let mut output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.industrial_workspace(ui, Lang::En, &tokens, 0);
                    });
                },
            );
            for label in [
                "CH1",
                "CH2",
                "CH3",
                "Measurement trend & export",
                "Export CSV",
                "Device information",
                "Communication & sampling",
                "Read errors",
                "Clear status",
                "Reset device",
                "SCPI console",
            ] {
                assert!(!positions(&output, label).is_empty(), "missing {label}");
            }
            assert!(
                positions(&output, "Console").is_empty(),
                "there must be no workspace tabs"
            );
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
                        "source-all-functions-{}",
                        if light { "light" } else { "dark" }
                    ),
                    size,
                );
            }
            panel.on_exit();
        }
    }

    #[test]
    fn every_channel_card_routes_apply_measure_output_and_protection_to_its_own_channel() {
        for kind in [InstrumentKind::Oscilloscope, InstrumentKind::DcSource] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            panel.handle_event(&ctx, connected_event(1, &open_demo_session(kind).unwrap()));
            panel.devices[0].capabilities.channels = 4;
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let labels: &[&str] = if kind == InstrumentKind::Oscilloscope {
                &["Apply channel", "Read"]
            } else {
                &[
                    "Apply setpoints",
                    "Output ON",
                    "Output OFF",
                    "Apply protection",
                ]
            };
            let mut time = 0.0;
            for channel in 1..=4u8 {
                for &label in labels {
                    let output = frame(&mut panel, &ctx, &mut time, 1400.0, vec![]);
                    let p = positions(&output, label)[channel as usize - 1];
                    frame(
                        &mut panel,
                        &ctx,
                        &mut time,
                        1400.0,
                        vec![egui::Event::PointerMoved(p)],
                    );
                    for pressed in [true, false] {
                        frame(
                            &mut panel,
                            &ctx,
                            &mut time,
                            1400.0,
                            vec![egui::Event::PointerButton {
                                pos: p,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            }],
                        );
                    }
                    let jobs: Vec<_> = rx.try_iter().collect();
                    assert_eq!(
                        jobs.len(),
                        if matches!(
                            label,
                            "Apply channel" | "Apply setpoints" | "Apply protection"
                        ) {
                            2
                        } else {
                            1
                        },
                        "{kind:?} CH{channel} {label}"
                    );
                    for job in jobs {
                        let Job::Command { command, .. } = job else {
                            panic!("unexpected job")
                        };
                        let (actual, action) = match command {
                            ControlCommand::ScopeScale { channel, .. }
                            | ControlCommand::ScopePosition { channel, .. } => {
                                (channel, "Apply channel")
                            }
                            ControlCommand::ScopeMeasure { channel, .. } => (channel, "Read"),
                            ControlCommand::SourceVoltage { channel, .. }
                            | ControlCommand::SourceCurrent { channel, .. } => {
                                (channel, "Apply setpoints")
                            }
                            ControlCommand::SourceOutput { channel, enabled } => {
                                (channel, if enabled { "Output ON" } else { "Output OFF" })
                            }
                            ControlCommand::SourceOvp { channel, .. }
                            | ControlCommand::SourceOcp { channel, .. } => {
                                (channel, "Apply protection")
                            }
                            _ => panic!("unexpected command"),
                        };
                        assert_eq!((actual, action), (channel, label));
                    }
                }
            }
            panel.on_exit();
        }
    }
}
