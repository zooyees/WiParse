//! Flat, responsive instrument dashboards. Mode values are routing contexts, not navigation.
use super::*;

impl InstrumentControlPanel {
    pub(super) fn flat_console(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
    ) {
        let kind = self.devices[index].kind;
        if matches!(
            kind,
            InstrumentKind::Oscilloscope | InstrumentKind::DcSource
        ) {
            self.channel_dashboard(ui, lang, t, index, defer, false);
            return;
        }
        if matches!(
            kind,
            InstrumentKind::ElectronicLoad | InstrumentKind::Multimeter
        ) && ui.available_width() >= 720.0
        {
            egui::ScrollArea::vertical()
                .id_salt(("measurement-dashboard", self.devices[index].id))
                .auto_shrink([false, false])
                .max_height(ui.available_height().max(1.0))
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    let column = (ui.available_width() - 16.0) * 0.5;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        let controls = ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                self.flat_card(ui, lang, t, index, 0, defer);
                                self.live_monitor(ui, lang, t, index, defer);
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(column, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                let height = controls.response.rect.height()
                                    - 12.0
                                    - ui.spacing().item_spacing.y;
                                aligned_panel(
                                    ui,
                                    t,
                                    text(lang, "测量趋势与导出", "Measurement trend & export"),
                                    height,
                                    |ui| {
                                        self.acquisition_ui_sized(
                                            ui,
                                            lang,
                                            t,
                                            self.devices[index].id,
                                            index,
                                            false,
                                            true,
                                        );
                                    },
                                );
                            },
                        );
                    });
                });
            return;
        }
        let id = self.devices[index].id;
        // Task forms and results have independent scroll positions: a long form
        // must never push incoming data or measured values out of sight.
        if is_port_device(kind) {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                for &slot in Self::task_slots(kind) {
                    let title = match kind {
                        InstrumentKind::UsbBridge => ["SPI", "I2C", "GPIO"][slot],
                        InstrumentKind::WiparseDongle => [
                            text(lang, "目标", "Target"),
                            "UART",
                            "I2C",
                            text(lang, "烧录", "Flash"),
                            text(lang, "内存", "Memory"),
                            "RTT",
                        ][slot],
                        _ => [
                            text(lang, "目标", "Target"),
                            text(lang, "烧录", "Flash"),
                            text(lang, "内存", "Memory"),
                            "RTT",
                        ][slot],
                    };
                    if ui
                        .add_sized([64.0, 28.0], egui::Button::new(title))
                        .on_hover_text(text(lang, "定位到此操作区域", "Scroll to this task"))
                        .clicked()
                    {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("instrument-task-anchor", id)), slot)
                        });
                    }
                }
            });
            ui.add_space(8.0);
        }
        let width = ui.available_width();
        let height = ui.available_height().max(1.0);
        if width >= 720.0 {
            let left = (width - 16.0) * 0.5;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            for column in 0..2 {
                let r = if column == 0 {
                    Rect::from_min_size(rect.min, egui::vec2(left, height))
                } else {
                    Rect::from_min_max(rect.min + egui::vec2(left + 16.0, 0.0), rect.max)
                };
                theme::scope_in_allocated_rect(
                    ui,
                    egui::UiBuilder::new()
                        .max_rect(r)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                    |ui| {
                        ui.set_width(r.width());
                        ui.set_clip_rect(r.intersect(ui.clip_rect()));
                        egui::ScrollArea::vertical()
                            .id_salt(("instrument-workspace-column", id, column))
                            .auto_shrink([false, false])
                            .max_height(height)
                            .min_scrolled_height(0.0)
                            .show(ui, |ui| {
                                if column == 0 {
                                    self.task_forms(ui, lang, t, index, defer);
                                } else {
                                    self.task_results(ui, lang, t, index, defer, height);
                                }
                            });
                    },
                );
            }
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("instrument-stacked", id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // On narrow windows keep results near the first task, rather than
                    // below every UART/I2C/debug form.
                    let first = if kind == InstrumentKind::WiparseDongle {
                        1
                    } else {
                        0
                    };
                    self.flat_card(ui, lang, t, index, first, defer);
                    self.task_results(ui, lang, t, index, defer, 260.0);
                    for slot in Self::task_slots(kind)
                        .iter()
                        .copied()
                        .filter(|s| *s != first)
                    {
                        self.flat_card(ui, lang, t, index, slot, defer);
                    }
                });
        }
    }

    fn task_slots(kind: InstrumentKind) -> &'static [usize] {
        match kind {
            InstrumentKind::Oscilloscope | InstrumentKind::DcSource => &[0, 1],
            InstrumentKind::DebugProbe => &[0, 1, 2, 3],
            InstrumentKind::UsbBridge => &[0, 1, 2],
            InstrumentKind::WiparseDongle => &[1, 2, 0, 3, 4, 5],
            _ => &[0],
        }
    }

    fn task_forms(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize, defer: bool) {
        for &slot in Self::task_slots(self.devices[index].kind) {
            self.flat_card(ui, lang, t, index, slot, defer);
        }
    }

    fn task_results(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
        height: f32,
    ) {
        let kind = self.devices[index].kind;
        if is_port_device(kind) {
            self.traffic_view(ui, lang, t, index, (height - 8.0).max(160.0));
        } else {
            self.live_monitor(ui, lang, t, index, defer);
            if kind == InstrumentKind::Oscilloscope {
                let id = self.devices[index].id;
                viewport(
                    ui,
                    t,
                    text(lang, "屏幕预览", "Screen preview"),
                    240.0,
                    |ui| {
                        self.scope_screenshot_ui(ui, lang, t, id, index, defer);
                    },
                );
            }
        }
    }

    fn flat_card(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        slot: usize,
        defer: bool,
    ) -> f32 {
        let kind = self.devices[index].kind;
        let id = self.devices[index].id;
        let c = &self.devices[index].controls;
        // A pending numeric edit must never migrate to a different device, channel or card.
        let channel = (c.source_focus, c.scope_channel);
        let card = ui
            .push_id(("flat-instrument-card", id, slot, channel), |ui| {
                let title = match kind {
                    InstrumentKind::Oscilloscope if slot == 0 => {
                        text(lang, "垂直通道", "Vertical channel")
                    }
                    InstrumentKind::Oscilloscope => text(lang, "时基与触发", "Timebase & trigger"),
                    InstrumentKind::DcSource if slot == 0 => {
                        text(lang, "输出设定", "Output setpoints")
                    }
                    InstrumentKind::DcSource => text(lang, "保护设置", "Protection"),
                    InstrumentKind::ElectronicLoad => text(lang, "负载设定", "Load setpoints"),
                    InstrumentKind::Multimeter => text(lang, "测量设置", "Measurement setup"),
                    InstrumentKind::UsbBridge => ["SPI", "I2C", "GPIO"][slot],
                    InstrumentKind::WiparseDongle => [
                        text(lang, "DAP · 目标控制", "DAP · Target"),
                        "UART",
                        "I2C",
                        text(lang, "固件烧录", "Firmware"),
                        text(lang, "内存访问", "Memory"),
                        "RTT",
                    ][slot],
                    _ => [
                        text(lang, "目标控制", "Target control"),
                        text(lang, "固件烧录", "Firmware"),
                        text(lang, "内存访问", "Memory"),
                        "RTT",
                    ][slot],
                };
                section(ui, t, title, "", |ui| {
                    let d = &mut self.devices[index];
                    let previous = (
                        d.controls.scope_setup_page,
                        d.controls.probe_page,
                        d.controls.bridge_tab,
                        d.controls.dongle_tab,
                    );
                    let commands = match kind {
                        InstrumentKind::Oscilloscope => {
                            d.controls.scope_setup_page = slot as u8;
                            controls::scope(ui, lang, t, d)
                        }
                        InstrumentKind::DcSource if slot == 0 => controls::source(ui, lang, t, d),
                        InstrumentKind::DcSource => controls::protection(ui, lang, t, d),
                        InstrumentKind::ElectronicLoad => controls::load(ui, lang, t, d),
                        InstrumentKind::Multimeter => controls::dmm(ui, lang, t, d),
                        InstrumentKind::DebugProbe => {
                            d.controls.probe_page = slot as u8;
                            task_form(ui, lang, t, d, ports::probe, controls::probe_actions)
                        }
                        InstrumentKind::UsbBridge => {
                            d.controls.bridge_tab = slot as u8;
                            task_form(ui, lang, t, d, ports::bridge, controls::bridge_actions)
                        }
                        InstrumentKind::WiparseDongle => {
                            d.controls.dongle_tab = if slot == 1 || slot == 2 {
                                slot as u8
                            } else {
                                0
                            };
                            d.controls.probe_page = if slot >= 3 { (slot - 2) as u8 } else { 0 };
                            task_form(ui, lang, t, d, ports::dongle, controls::dongle_actions)
                        }
                        _ => Vec::new(),
                    };
                    (
                        d.controls.scope_setup_page,
                        d.controls.probe_page,
                        d.controls.bridge_tab,
                        d.controls.dongle_tab,
                    ) = previous;
                    self.dispatch_scope_commands(id, commands);
                });
            })
            .response;
        let key = egui::Id::new(("instrument-task-anchor", id));
        if ui.ctx().data(|d| d.get_temp::<usize>(key)) == Some(slot) {
            ui.scroll_to_rect(card.rect, Some(egui::Align::Min));
            ui.ctx().data_mut(|d| d.remove::<usize>(key));
        }
        card.rect.height()
    }
}

#[cfg(test)]
mod workspace_geometry_tests {
    use super::*;
    #[test]
    fn unified_device_pages_keep_header_tabs_and_export_previews() {
        for (name, kind) in [
            ("load", InstrumentKind::ElectronicLoad),
            ("multimeter", InstrumentKind::Multimeter),
            ("probe", InstrumentKind::DebugProbe),
            ("bridge", InstrumentKind::UsbBridge),
            ("dongle", InstrumentKind::WiparseDongle),
            ("scpi", InstrumentKind::Generic),
        ] {
            for light in [false, true] {
                let mut panel = InstrumentControlPanel::new(&AppConfig::default());
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = crate::theme::apply_theme(&ctx, light);
                panel.handle_event(&ctx, connected_event(1, &open_demo_session(kind).unwrap()));
                panel.selected_kind = kind;
                panel.selected_id = Some(1);
                panel.overview_open = false;
                let (tx, rx) = unbounded();
                panel.tx = tx;
                let size = egui::vec2(1600.0, 1000.0);
                let mut output = egui::FullOutput::default();
                for _ in 0..3 {
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
                    output.pixels_per_point = next.pixels_per_point;
                }
                let titles: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(text)
                            if ["控制台", "采集记录", "设备信息"]
                                .contains(&text.galley.job.text.as_str()) =>
                        {
                            Some(text.pos.y)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(titles.len(), 3, "{name}: header tabs");
                assert!(titles
                    .iter()
                    .all(|y| *y < 60.0 && (*y - titles[0]).abs() < 1.0));
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
                        &format!("unified-{name}-{}", if light { "light" } else { "dark" }),
                        size,
                    );
                }
                panel.on_exit();
            }
        }
    }

    #[test]
    fn short_workspaces_do_not_expand_beyond_the_visible_height() {
        for kind in [InstrumentKind::Oscilloscope, InstrumentKind::WiparseDongle] {
            for height in [120.0, 160.0, 240.0, 360.0] {
                let mut panel = InstrumentControlPanel::new(&AppConfig::default());
                let ctx = egui::Context::default();
                panel.handle_event(&ctx, connected_event(1, &open_demo_session(kind).unwrap()));
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(840.0, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let bottom = ui.max_rect().bottom();
                            panel.flat_console(ui, Lang::En, &Tokens::dark(), 0, false);
                            assert!(
                                ui.min_rect().bottom() <= bottom + 1.0,
                                "{kind:?} at {height}: content extends past the visible workspace"
                            );
                        });
                    },
                );
                panel.on_exit();
            }
        }
    }

    #[test]
    fn task_anchor_scrolls_forms_without_moving_results_or_sending_commands() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        panel.handle_event(
            &ctx,
            connected_event(
                1,
                &open_demo_session(InstrumentKind::WiparseDongle).unwrap(),
            ),
        );
        let (tx, rx) = unbounded();
        panel.tx = tx;
        let mut time = 0.0;
        let mut frame = |panel: &mut InstrumentControlPanel, events| {
            time += 0.2;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(840.0, 570.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.flat_console(ui, Lang::Zh, &Tokens::dark(), 0, false);
                    });
                },
            )
        };
        let output = frame(&mut panel, vec![]);
        let find = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => {
                        Some((t.pos, t.galley.size()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let before = find(&output, "接收记录")[0].0;
        let (p, size) = find(&output, "I2C")[0];
        let p = p + size * 0.5;
        frame(&mut panel, vec![egui::Event::PointerMoved(p)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        let mut output = frame(&mut panel, vec![]);
        for _ in 0..8 {
            output = frame(&mut panel, vec![]);
        }
        assert_eq!(find(&output, "接收记录")[0].0, before);
        assert!(
            find(&output, "I2C")
                .iter()
                .any(|(p, _)| p.y > 35.0 && p.y < 120.0),
            "I2C form must be brought into view"
        );
        assert!(rx.try_recv().is_err());
        panel.on_exit();
    }

    #[test]
    fn results_remain_beside_forms_and_rendering_never_sends_commands() {
        for kind in [
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
            InstrumentKind::WiparseDongle,
        ] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            panel.handle_event(&ctx, connected_event(1, &open_demo_session(kind).unwrap()));
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let mut last = None;
            for _ in 0..3 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(840.0, 570.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.flat_console(ui, Lang::Zh, &Tokens::dark(), 0, false);
                        });
                    },
                );
                let title = if is_port_device(kind) {
                    "接收记录"
                } else {
                    "测量趋势与导出"
                };
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.job.text == title => Some(t.pos),
                        _ => None,
                    })
                    .expect("results must be visible on first screen");
                assert!(
                    pos.x > 420.0 && pos.x < 465.0 && pos.y < 84.0,
                    "{kind:?}: results shifted away from their reserved column: {pos:?}"
                );
                if let Some(previous) = last {
                    assert_eq!(pos, previous, "workspace must not drift after repaint");
                }
                last = Some(pos);
            }
            assert!(rx.try_recv().is_err(), "rendering sent a hardware command");
            panel.on_exit();
        }
    }
}
