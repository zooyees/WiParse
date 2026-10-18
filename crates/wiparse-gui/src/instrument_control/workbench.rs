//! Instrument consoles: a compact source list, aligned property groups and prominent live readings.
use super::*;
mod channels;
pub(super) mod controls;
mod design;
mod flat;
mod navigation;
mod ports;
mod scope_layout;
use design::*;

impl InstrumentControlPanel {
    pub(super) fn apple_sidebar(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        navigation::sidebar(self, ui, lang, t);
    }
    pub(super) fn detail_header(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
    ) {
        navigation::header(self, ui, lang, t, index);
    }
    pub(super) fn connection_page(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        navigation::connection(self, ui, lang, t);
    }
    pub(super) fn fleet_overview(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        navigation::overview(self, ui, lang, t);
    }

    pub(super) fn industrial_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
    ) {
        let id = self.devices[index].id;
        ui.push_id(("instrument-device-workspace", id), |ui| {
            self.device_workspace_body(ui, lang, t, index);
        });
    }

    fn device_workspace_body(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let id = self.devices[index].id;
        let kind = self.devices[index].kind;
        // Power supply operations, trends and device utilities share one scrollable page.
        if kind == InstrumentKind::DcSource {
            self.channel_dashboard(ui, lang, t, index, false, true);
            return;
        }
        let key = egui::Id::new(("instrument-task", id));
        let tab = ui.ctx().data_mut(|d| d.get_temp::<u8>(key).unwrap_or(0));
        let defer = self.scope_heavy_defer_frames > 0;
        if kind == InstrumentKind::Oscilloscope && defer {
            self.scope_heavy_defer_frames = self.scope_heavy_defer_frames.saturating_sub(1);
            ui.ctx().request_repaint();
        }
        ui.add_space(16.0);
        if let Some(error) = self.devices[index].last_error.clone() {
            Frame::NONE
                .fill(t.stop_bg.linear_multiply(0.08))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(text(lang, "上次操作未完成", "Last operation failed"))
                            .strong()
                            .color(t.stop_bg),
                    );
                    ui.add(egui::Label::new(&error).truncate())
                        .on_hover_text(&error);
                    if ui.small_button(text(lang, "收起", "Dismiss")).clicked() {
                        self.devices[index].last_error = None;
                    }
                });
            ui.add_space(12.0);
        }
        if self.busy_device == Some(id) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add(egui::Label::new(&self.busy_label).truncate());
            });
        }
        if tab == 0 && kind != InstrumentKind::Generic {
            self.flat_console(ui, lang, t, index, defer);
            return;
        }
        egui::ScrollArea::vertical().id_salt(("console-content",id,tab)).auto_shrink([false,false]).show(ui,|ui|{
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing=egui::vec2(8.0,8.0);
            match tab{
                0 if kind==InstrumentKind::Generic=>{viewport(ui,t,text(lang,"SCPI 控制台","SCPI console"),430.0,|ui|{self.console_ui_compact(ui,lang,t,id,index);});}
                0=>{ self.flat_console(ui,lang,t,index,defer); }
                1 if kind==InstrumentKind::Oscilloscope=>{
                    split(ui,|ui,col|{viewport(ui,t,if col==0{text(lang,"屏幕截图","Screen capture")}else{text(lang,"波形数据","Waveform samples")},400.0,|ui|{
                        if col==0{self.scope_screenshot_ui(ui,lang,t,id,index,defer);}else{self.scope_waveform_data_ui(ui,lang,t,id,index,defer);}
                    });});
                }
                1 if is_port_device(kind)=>{self.traffic_view(ui,lang,t,index,440.0);}
                1 if kind==InstrumentKind::Generic=>{section(ui,t,text(lang,"采集能力","Measurement capability"),"",|ui|{ui.label(text(lang,"设备未声明测量能力，请使用控制台查询。","This device advertises no measurement capability. Use the console to query it."));});}
                1=>{section(ui,t,text(lang,"测量趋势","Measurement trend"),"",|ui|{self.acquisition_ui(ui,lang,t,id,index);});}
                _=>{self.device_details(ui,lang,t,index);}
            }
        });
    }

    fn console_toolbar(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let d = &mut self.devices[index];
        let id = d.id;
        match d.kind {
            InstrumentKind::Oscilloscope => {
                let mut commands = Vec::new();
                ui.columns(4, |columns| {
                    for (column, (label, command, primary)) in columns.iter_mut().zip([
                        (text(lang, "运行", "Run"), ControlCommand::ScopeRun, true),
                        (text(lang, "停止", "Stop"), ControlCommand::ScopeStop, false),
                        (
                            text(lang, "单次", "Single"),
                            ControlCommand::ScopeSingle,
                            false,
                        ),
                        (
                            text(lang, "自动设置", "Autoset"),
                            ControlCommand::ScopeAutoset,
                            false,
                        ),
                    ]) {
                        let button = if primary {
                            egui::Button::new(RichText::new(label).color(t.accent_text))
                                .fill(t.accent)
                        } else {
                            egui::Button::new(label)
                        };
                        if column
                            .add_sized([column.available_width(), 32.0], button.wrap())
                            .clicked()
                        {
                            commands.push(command);
                        }
                    }
                });

                self.dispatch_scope_commands(id, commands);
            }
            _ => {}
        }
    }

    fn measurement_actions(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let id = self.devices[index].id;
        action_group(ui, 3, |ui| {
            if action(ui, t, text(lang, "读取", "Read"), true).clicked() {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
            let acquiring = self.devices[index].acquiring;
            if !acquiring {
                if action(ui, t, text(lang, "连续采样", "Continuous"), false).clicked() {
                    self.devices[index].acquiring = true;
                    self.devices[index].paused = false;
                    self.last_sample =
                        Instant::now() - Duration::from_millis(self.sample_interval_ms);
                }
            } else {
                if action(
                    ui,
                    t,
                    if self.devices[index].paused {
                        text(lang, "继续", "Resume")
                    } else {
                        text(lang, "暂停", "Pause")
                    },
                    false,
                )
                .clicked()
                {
                    self.devices[index].paused = !self.devices[index].paused;
                }
                if action(ui, t, text(lang, "停止", "Stop"), false).clicked() {
                    self.devices[index].acquiring = false;
                    self.devices[index].paused = false;
                }
            }
        });
    }

    fn live_monitor(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        defer: bool,
    ) {
        let d = &self.devices[index];
        let id = d.id;
        let kind = d.kind;
        if is_port_device(kind) {
            self.traffic_view(ui, lang, t, index, 360.0);
            return;
        }
        if kind == InstrumentKind::Generic {
            viewport(
                ui,
                t,
                text(lang, "SCPI 控制台", "SCPI console"),
                400.0,
                |ui| {
                    self.console_ui_compact(ui, lang, t, id, index);
                },
            );
            return;
        }
        section(
            ui,
            t,
            text(lang, "实测读数", "Measured readings"),
            "",
            |ui| {
                if kind == InstrumentKind::Oscilloscope {
                    let i = self.devices[index].controls.scope_channel.clamp(1, 4) as usize - 1;
                    let raw = self.devices[index].controls.scope_meas_results[i]
                        .as_deref()
                        .unwrap_or("—");
                    ui.add(
                        egui::Label::new(
                            RichText::new(format_scope_meas_display(
                                raw,
                                self.devices[index].controls.scope_meas_types[i],
                            ))
                            .monospace()
                            .size(26.0),
                        )
                        .wrap(),
                    );
                    ui.add_space(16.0);
                    property(ui, t, text(lang, "测量项目", "Measurement"), |ui| {
                        egui::ComboBox::from_id_salt(("scope-reading", id))
                            .width(ui.available_width())
                            .selected_text(self.devices[index].controls.scope_meas_types[i].label())
                            .show_ui(ui, |ui| {
                                for m in ScopeMeasType::all() {
                                    ui.selectable_value(
                                        &mut self.devices[index].controls.scope_meas_types[i],
                                        *m,
                                        m.label(),
                                    );
                                }
                            });
                    });
                    ui.add_space(12.0);
                    if action(ui, t, text(lang, "读取测量", "Read measurement"), true).clicked()
                    {
                        let ch = self.devices[index].controls.scope_channel;
                        let m = self.devices[index].controls.scope_meas_types[i];
                        self.dispatch_scope_commands(
                            id,
                            vec![ControlCommand::ScopeMeasure {
                                channel: ch,
                                meas_type: m,
                            }],
                        );
                    }
                } else if kind == InstrumentKind::Multimeter {
                    let function = self.devices[index].controls.dmm_function;
                    metric(
                        ui,
                        t,
                        function.label(),
                        self.latest.get(&(id, function.scpi().into())),
                        28.0,
                    );
                    ui.add_space(20.0);
                    self.measurement_actions(ui, lang, t, index);
                } else {
                    let prefix = if kind == InstrumentKind::DcSource {
                        format!("CH{} ", self.devices[index].controls.source_focus)
                    } else {
                        String::new()
                    };
                    metric(
                        ui,
                        t,
                        text(lang, "电压", "Voltage"),
                        self.latest.get(&(id, format!("{prefix}Voltage"))),
                        28.0,
                    );
                    ui.add_space(16.0);
                    ui.columns(2, |cols| {
                        metric(
                            &mut cols[0],
                            t,
                            text(lang, "电流", "Current"),
                            self.latest.get(&(id, format!("{prefix}Current"))),
                            22.0,
                        );
                        metric(
                            &mut cols[1],
                            t,
                            text(lang, "功率", "Power"),
                            self.latest.get(&(id, format!("{prefix}Power"))),
                            22.0,
                        );
                    });
                    ui.add_space(20.0);
                    self.measurement_actions(ui, lang, t, index);
                }
                ui.add_space(14.0);
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
                        .color(t.text_muted)
                        .size(11.0),
                    );
                } else if match kind {
                    InstrumentKind::Oscilloscope => self.devices[index].controls.scope_meas_results
                        [self.devices[index].controls.scope_channel.clamp(1, 4) as usize - 1]
                        .is_some(),
                    InstrumentKind::Multimeter => self.latest.contains_key(&(
                        id,
                        self.devices[index].controls.dmm_function.scpi().into(),
                    )),
                    _ => self.latest.keys().any(|(device, _)| *device == id),
                } {
                    ui.label(
                        RichText::new(text(
                            lang,
                            "最近一次读取值 · 点击读取刷新",
                            "Last reading · Read to refresh",
                        ))
                        .color(t.text_muted)
                        .size(11.0),
                    );
                } else {
                    ui.label(
                        RichText::new(text(
                            lang,
                            "点击读取以获得实测值。",
                            "Measure once to display an actual reading.",
                        ))
                        .color(t.text_muted)
                        .size(11.0),
                    );
                }
            },
        );
    }

    fn traffic_view(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
        height: f32,
    ) {
        viewport(
            ui,
            t,
            text(lang, "接收记录", "Received data"),
            height,
            |ui| {
                self.device_io_log_ui(ui, lang, t, index);
            },
        );
    }

    fn maintenance_actions(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, id: u64) {
        let mut commands = Vec::new();
        action_group(ui, 3, |ui| {
            for (label, command) in [
                (
                    text(lang, "读取错误", "Read errors"),
                    ControlCommand::RawQuery("SYST:ERR?".into()),
                ),
                (
                    text(lang, "清除状态", "Clear status"),
                    ControlCommand::Clear,
                ),
                (
                    text(lang, "复位设备", "Reset device"),
                    ControlCommand::Reset,
                ),
            ] {
                if action(ui, t, label, false).clicked() {
                    commands.push(command);
                }
            }
        });
        self.dispatch_scope_commands(id, commands);
    }

    fn device_information_sections(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        index: usize,
    ) {
        let id = self.devices[index].id;
        let kind = self.devices[index].kind;
        section(
            ui,
            t,
            text(lang, "设备信息", "Device information"),
            "",
            |ui| {
                let d = &self.devices[index];
                for (label, value) in [
                    (
                        text(lang, "制造商", "Manufacturer"),
                        &d.identity.manufacturer,
                    ),
                    (text(lang, "型号", "Model"), &d.identity.model),
                    (text(lang, "序列号", "Serial number"), &d.identity.serial),
                    (text(lang, "固件", "Firmware"), &d.identity.firmware),
                    (text(lang, "资源地址", "Resource"), &d.resource),
                ] {
                    property(ui, t, label, |ui| {
                        ui.add(egui::Label::new(value).truncate())
                            .on_hover_text(value);
                    });
                }
            },
        );
        section(
            ui,
            t,
            text(lang, "通信与采样", "Communication & sampling"),
            "",
            |ui| {
                property(ui, t, text(lang, "超时", "Timeout"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.timeout_ms)
                            .range(100..=120000)
                            .suffix(" ms"),
                    );
                });
                property(ui, t, text(lang, "采样周期", "Sample interval"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.sample_interval_ms)
                            .range(100..=60000)
                            .suffix(" ms"),
                    );
                });
                if kind == InstrumentKind::WiparseDongle {
                    ports::network(ui, lang, t, &mut self.devices[index]);
                    ui.add_space(12.0);
                    if action(ui, t, text(lang, "读取链路信息", "Read link info"), false).clicked()
                    {
                        let d = &self.devices[index];
                        let commands = vec![
                            ControlCommand::DongleSetLink {
                                host: d.controls.dongle_host.clone(),
                                uart_port: d.controls.dongle_uart_port,
                                i2c_port: d.controls.dongle_i2c_port,
                            },
                            ControlCommand::DongleInfo,
                        ];
                        self.dispatch_scope_commands(id, commands);
                    }
                }
            },
        );
        if !is_port_device(kind) && kind != InstrumentKind::Generic {
            if kind == InstrumentKind::DcSource {
                section(ui, t, text(lang, "设备维护", "Maintenance"), "", |ui| {
                    self.maintenance_actions(ui, lang, t, id);
                });
            } else {
                egui::CollapsingHeader::new(text(lang, "设备维护", "Maintenance"))
                    .id_salt(("device-maintenance", id))
                    .show(ui, |ui| {
                        self.maintenance_actions(ui, lang, t, id);
                    });
            }
        }
    }

    fn device_details(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, index: usize) {
        let id = self.devices[index].id;
        let kind = self.devices[index].kind;
        split(ui, |ui, col| {
            if col == 0 {
                self.device_information_sections(ui, lang, t, index);
            } else if is_port_device(kind) {
                self.traffic_view(ui, lang, t, index, 430.0);
            } else {
                viewport(
                    ui,
                    t,
                    text(lang, "SCPI 控制台", "SCPI console"),
                    430.0,
                    |ui| {
                        self.console_ui_compact(ui, lang, t, id, index);
                    },
                );
            }
        });
    }
}

fn is_port_device(kind: InstrumentKind) -> bool {
    matches!(
        kind,
        InstrumentKind::DebugProbe | InstrumentKind::UsbBridge | InstrumentKind::WiparseDongle
    )
}
fn viewport(
    ui: &mut egui::Ui,
    t: &Tokens,
    title: &str,
    height: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    design::aligned_panel(ui, t, title, height, add);
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_readouts_are_complete_sorted_and_single_line() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        panel.handle_event(
            &ctx,
            connected_event(1, &open_demo_session(InstrumentKind::DcSource).unwrap()),
        );
        let mut labels = Vec::new();
        for ch in 1..=3 {
            for (name, unit) in [("Voltage", "V"), ("Current", "A"), ("Power", "W")] {
                let channel = format!("CH{ch} {name}");
                panel.latest.insert(
                    (1, channel.clone()),
                    Reading {
                        channel: channel.clone(),
                        value: 9.0,
                        unit: unit.into(),
                    },
                );
                labels.push(channel);
            }
        }
        let t = Tokens::dark();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| panel.acquisition_ui(ui, Lang::Zh, &t, 1, 0));
            },
        );
        let mut previous = (0.0, 0.0);
        let mut positions = Vec::new();
        for label in labels {
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos),
                    _ => None,
                })
                .expect("every channel reading must be rendered");
            assert!((pos.y, pos.x) > previous, "reading order must be stable");
            previous = (pos.y, pos.x);
            positions.push(pos);
        }
        assert!(
            (positions[0].y - positions[2].y).abs() < 1.0,
            "each channel's voltage/current/power must share a row"
        );
        assert!(positions[3].y > positions[2].y && (positions[3].x - positions[0].x).abs() < 1.0);
        let values: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text.starts_with("9.0000 ") => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(values.len(), 9);
        assert!(
            values.iter().all(|t| t.galley.rows.len() == 1),
            "numbers must not split across lines"
        );
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == t.accent && (r.rect.width()-104.0).abs()<1.0)), "start button must have toolbar size");
        panel.on_exit();
    }

    #[test]
    fn clearing_one_instrument_preserves_other_instrument_history() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        panel.samples.push(Sample::value("A", "Voltage", 5.0, "V"));
        panel.samples.push(Sample::value("B", "Voltage", 9.0, "V"));
        panel.samples.push(Sample::value("A", "Current", 1.0, "A"));
        panel.samples.push(Sample::value("B", "Current", 2.0, "A"));
        for id in [1, 2] {
            panel.latest.insert(
                (id, "Voltage".into()),
                Reading {
                    channel: "Voltage".into(),
                    value: 5.0,
                    unit: "V".into(),
                },
            );
        }
        panel.clear_device_samples(1, "A");
        let remaining: Vec<_> = panel.samples.iter().collect();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().all(|s| s.resource == "B"));
        assert_eq!(remaining[0].value, Some(9.0));
        assert_eq!(remaining[1].value, Some(2.0));
        assert!(!panel.latest.contains_key(&(1, "Voltage".into())));
        assert!(panel.latest.contains_key(&(2, "Voltage".into())));
        panel.on_exit();
    }

    #[test]
    fn pending_numeric_edit_is_not_committed_into_a_different_channel() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let session = open_demo_session(InstrumentKind::DcSource).unwrap();
        panel.handle_event(&ctx, connected_event(1, &session));
        panel.devices[0].controls.source_voltages[0] = 5.0;
        panel.devices[0].controls.source_voltages[1] = 7.0;
        let mut time = 0.0;
        let mut frame = |panel: &mut InstrumentControlPanel, events| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 760.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.industrial_workspace(ui, Lang::Zh, &Tokens::dark(), 0)
                    });
                },
            )
        };
        let output = frame(&mut panel, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == "5.00 V" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("voltage editor");
        frame(&mut panel, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        let editor = ctx.memory(|m| m.focused()).expect("numeric editor focused");
        // egui retains this pending edit until focus loss or confirmation.
        ctx.data_mut(|d| d.insert_temp(editor, "12.34".to_string()));
        panel.devices[0].controls.source_focus = 2;
        frame(
            &mut panel,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        frame(&mut panel, vec![]);
        assert_eq!(panel.devices[0].controls.source_voltages[1], 7.0);
        panel.on_exit();
    }

    #[test]
    fn dongle_i2c_buttons_apply_the_visible_endpoint_before_the_transaction() {
        for label in ["写后读", "读取 I2C", "写入 I2C"] {
            let mut panel = InstrumentControlPanel::new(&AppConfig::default());
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let session = open_demo_session(InstrumentKind::WiparseDongle).unwrap();
            panel.handle_event(&ctx, connected_event(1, &session));
            let d = &mut panel.devices[0];
            d.controls.dongle_tab = 2;
            d.controls.dongle_host = "192.0.2.42".into();
            d.controls.dongle_i2c_port = 4444;
            let (tx, rx) = unbounded();
            panel.tx = tx;
            let mut time = 0.0;
            let mut frame = |panel: &mut InstrumentControlPanel, events| {
                time += 0.1;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 1000.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.industrial_workspace(ui, Lang::Zh, &Tokens::dark(), 0);
                        });
                    },
                )
            };
            let output = frame(&mut panel, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("transaction button must be visible");
            frame(&mut panel, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(
                    &mut panel,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    }],
                );
            }
            let jobs: Vec<_> = rx.try_iter().collect();
            assert_eq!(jobs.len(), 2);
            assert!(
                matches!(&jobs[0], Job::Command { command: ControlCommand::DongleSetLink { host, i2c_port: 4444, .. }, .. } if host == "192.0.2.42")
            );
            assert!(matches!(
                &jobs[1],
                Job::Command {
                    command: ControlCommand::DongleI2cWriteRead { .. }
                        | ControlCommand::DongleI2cRead { .. }
                        | ControlCommand::DongleI2cWrite { .. },
                    ..
                }
            ));
            panel.on_exit();
        }
    }

    #[test]
    fn normal_mode_overview_is_available_without_simulated_devices() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        assert!(!panel.debug_mode);
        let reply = panel.api_select(&serde_json::json!({"overview":true}));
        assert!(reply.ok);
        assert!(panel.show_overview());
        assert!(
            panel.devices.is_empty(),
            "opening the real device overview must not create simulations"
        );
        panel.on_exit();
    }

    #[test]
    fn disconnected_instruments_render_in_both_themes_without_connecting() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        for lang in [Lang::Zh, Lang::En] {
            for light in [false, true] {
                for width in [640.0, 1100.0, 1920.0] {
                    for kind in [
                        InstrumentKind::Oscilloscope,
                        InstrumentKind::DcSource,
                        InstrumentKind::ElectronicLoad,
                        InstrumentKind::Multimeter,
                        InstrumentKind::DebugProbe,
                        InstrumentKind::UsbBridge,
                        InstrumentKind::WiparseDongle,
                        InstrumentKind::Generic,
                    ] {
                        panel.select_kind(kind);
                        let ctx = egui::Context::default();
                        crate::fonts::install_cjk_fonts(&ctx);
                        let t = crate::theme::apply_theme(&ctx, light);
                        let size = egui::vec2(width, 640.0);
                        let output = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default().show(ctx, |ui| {
                                    panel.ui(ui, lang, &t);
                                });
                            },
                        );
                        let captions = match lang {
                            Lang::Zh => ["连接", "查找设备"],
                            _ => ["Connect", "Discover"],
                        };
                        let mut buttons = Vec::new();
                        for caption in captions {
                            let label = output
                                .shapes
                                .iter()
                                .rev()
                                .find_map(|shape| match &shape.shape {
                                    egui::Shape::Text(text) if text.galley.job.text == caption => {
                                        Some(text)
                                    }
                                    _ => None,
                                })
                                .expect("connection action caption");
                            assert_eq!(
                                label.galley.rows.len(),
                                1,
                                "{kind:?} {width}: {caption} wrapped vertically"
                            );
                            let center = label.pos + label.galley.size() * 0.5;
                            let rect = output.shapes.iter().rev().find_map(|shape| match &shape.shape {
                                egui::Shape::Rect(button) if button.rect.contains(center) && button.rect.height() >= 28.0 && button.rect.height() <= 44.0 => Some(button.rect),
                                _ => None,
                            }).unwrap_or_else(|| panic!("connection action background: {kind:?} {lang:?} {width} {caption}"));
                            assert!(rect.width() >= 112.0, "button was squeezed: {rect:?}");
                            buttons.push(rect);
                        }
                        assert!((buttons[0].width() - buttons[1].width()).abs() < 1.0);
                        assert!((buttons[0].top() - buttons[1].top()).abs() < 1.0);
                        assert!((buttons[1].left() - buttons[0].right() - 8.0).abs() < 1.0);
                        for primitive in
                            ctx.tessellate(output.shapes.clone(), output.pixels_per_point)
                        {
                            assert!(primitive.clip_rect.is_finite());
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                assert!(mesh.is_valid());
                            }
                        }
                        if kind == InstrumentKind::DcSource
                            && !light
                            && matches!(lang, Lang::Zh)
                            && width <= 1100.0
                        {
                            if let Some(dir) = std::env::var_os("WIPARSE_UI_AUDIT_OUTPUT") {
                                crate::ui_audit::export_frame(
                                    &ctx,
                                    output,
                                    &std::path::PathBuf::from(dir),
                                    &format!("connection-{width:.0}"),
                                    size,
                                );
                            }
                        }
                        assert!(panel.devices.is_empty());
                        assert!(!panel.scanning);
                    }
                }
            }
        }
        panel.on_exit();
    }

    #[test]
    fn source_channel_cards_apply_setpoints_without_selecting_a_channel() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let session = open_demo_session(InstrumentKind::DcSource).unwrap();
        panel.handle_event(&ctx, connected_event(1, &session));
        let (tx, rx) = unbounded();
        panel.tx = tx;
        panel.devices[0].controls.source_voltages[2] = 18.0;
        panel.devices[0].controls.source_currents[2] = 0.8;
        panel.latest.insert(
            (1, "CH3 Voltage".into()),
            Reading {
                channel: "CH3 Voltage".into(),
                value: 18.1234,
                unit: "V".into(),
            },
        );
        let mut time = 0.0;
        let mut frame = |panel: &mut InstrumentControlPanel, events| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 760.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.industrial_workspace(ui, Lang::Zh, &Tokens::dark(), 0);
                    });
                },
            )
        };
        let output = frame(&mut panel, vec![]);
        let find = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        (t.galley.job.text == label).then_some(t.pos + t.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let pos = find(&output, "CH3");
        frame(&mut panel, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        assert_eq!(panel.devices[0].controls.source_focus, 1);
        assert!(
            rx.try_iter().next().is_none(),
            "channel navigation must not control the instrument"
        );
        let output = frame(&mut panel, vec![]);
        assert!(output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="18.1234")));
        let pos = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == "应用设定" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .last()
            .expect("CH3 apply button");
        frame(&mut panel, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                &mut panel,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        let jobs: Vec<_> = rx.try_iter().collect();
        assert_eq!(jobs.len(), 2);
        assert!(
            matches!(&jobs[0],Job::Command{command:ControlCommand::SourceVoltage{channel:3,value},..} if *value==18.0)
        );
        assert!(
            matches!(&jobs[1],Job::Command{command:ControlCommand::SourceCurrent{channel:3,value},..} if *value==0.8)
        );
        panel.on_exit();
    }

    #[test]
    fn a_device_failure_does_not_contaminate_other_devices() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        for id in [1, 2] {
            let session = open_demo_session(InstrumentKind::DcSource).unwrap();
            panel.handle_event(&ctx, connected_event(id, &session));
        }
        panel.handle_event(
            &ctx,
            Event::Error {
                id: Some(1),
                job_id: None,
                message: "SIM timeout".into(),
            },
        );
        assert_eq!(panel.devices[0].last_error.as_deref(), Some("SIM timeout"));
        assert!(panel.devices[1].last_error.is_none());
        assert!(panel.logs.back().unwrap().contains("#1 ERROR SIM timeout"));
        panel.on_exit();
    }

    #[test]
    fn dmm_function_change_does_not_display_an_old_voltage_as_resistance() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let session = open_demo_session(InstrumentKind::Multimeter).unwrap();
        panel.handle_event(&ctx, connected_event(1, &session));
        panel.latest.insert(
            (1, MeasureFunction::all()[0].scpi().into()),
            Reading {
                channel: MeasureFunction::all()[0].scpi().into(),
                value: 9.123456,
                unit: "V".into(),
            },
        );
        panel.devices[0].controls.dmm_function = *MeasureFunction::all()
            .iter()
            .find(|f| f.scpi() == "RES")
            .unwrap();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 400.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panel.live_monitor(ui, Lang::En, &Tokens::dark(), 0, false);
                });
            },
        );
        assert!(!output.shapes.iter().any(
            |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains("9.123456"))
        ));
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains("Measure once"))));
        panel.on_exit();
    }
}
