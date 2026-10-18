use super::design::*;
use super::*;

pub(super) fn scan(panel: &mut InstrumentControlPanel, lang: Lang) {
    if panel.scanning {
        return;
    }
    panel.scanning = true;
    panel.status = text(lang, "正在查找设备…", "Looking for devices…").into();
    let _ = panel.tx.send(Job::Scan {
        library: panel.visa_library.clone(),
        timeout_ms: panel.timeout_ms,
    });
}

pub(super) fn sidebar(
    panel: &mut InstrumentControlPanel,
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
) {
    Frame::NONE
        .fill(t.panel_bg)
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(text(lang, "仪器", "Instruments"))
                        .size(17.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !panel.scanning,
                            egui::Button::new(text(lang, "查找", "Discover")).stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        scan(panel, lang);
                    }
                    if panel.scanning {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(16.0);
            if nav_row(
                ui,
                t,
                None,
                text(lang, "设备总览", "All instruments"),
                &format!(
                    "{} {}",
                    panel.devices.len(),
                    text(lang, "个会话", "sessions")
                ),
                panel.show_overview(),
                false,
            )
            .clicked()
            {
                panel.select_overview();
            }
            ui.add_space(14.0);
            ui.label(
                RichText::new(text(lang, "设备", "DEVICES"))
                    .size(11.0)
                    .color(t.text_muted),
            );
            ui.add_space(4.0);
            let mut picked = None;
            let height = (ui.available_height() - 34.0).max(40.0);
            egui::ScrollArea::vertical()
                .id_salt("instrument-source-list")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut kinds = vec![
                        InstrumentKind::Oscilloscope,
                        InstrumentKind::DcSource,
                        InstrumentKind::ElectronicLoad,
                        InstrumentKind::Multimeter,
                        InstrumentKind::DebugProbe,
                        InstrumentKind::UsbBridge,
                        InstrumentKind::WiparseDongle,
                    ];
                    if panel
                        .devices
                        .iter()
                        .any(|d| d.kind == InstrumentKind::Generic)
                    {
                        kinds.push(InstrumentKind::Generic);
                    }
                    for kind in kinds {
                        let devices: Vec<_> =
                            panel.devices.iter().filter(|d| d.kind == kind).collect();
                        if devices.is_empty() {
                            if nav_row(
                                ui,
                                t,
                                Some(kind),
                                instrument_name(lang, kind),
                                text(lang, "未连接", "Not connected"),
                                !panel.show_overview() && panel.selected_kind == kind,
                                false,
                            )
                            .clicked()
                            {
                                picked = Some((kind, None));
                            }
                        } else {
                            for d in devices {
                                let demo = d.resource.starts_with("DEMO::")
                                    || d.resource.contains("serial=DEMO");
                                let detail = if demo {
                                    format!("{} · {}", d.identity.model, text(lang, "模拟", "Demo"))
                                } else {
                                    d.identity.model.clone()
                                };
                                if nav_row(
                                    ui,
                                    t,
                                    Some(kind),
                                    instrument_name(lang, kind),
                                    &detail,
                                    !panel.show_overview()
                                        && panel.selected_kind == kind
                                        && panel.selected_id == Some(d.id),
                                    !demo,
                                )
                                .clicked()
                                {
                                    picked = Some((kind, Some(d.id)));
                                }
                            }
                        }
                        ui.add_space(2.0);
                    }
                });
            if let Some((kind, id)) = picked {
                panel.select_kind(kind);
                if id.is_some() {
                    panel.selected_id = id;
                }
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("WiParse · {}", env!("CARGO_PKG_VERSION")))
                    .size(11.0)
                    .color(t.text_muted),
            );
            let status = format!(
                "{} {}",
                panel.devices.len(),
                text(lang, "个已连接会话", "connected sessions")
            );
            ui.add(
                egui::Label::new(RichText::new(status).size(11.0).color(t.text_muted)).truncate(),
            )
            .on_hover_text(&panel.status);
        });
}

pub(super) fn header(
    panel: &mut InstrumentControlPanel,
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    index: usize,
) {
    let d = &panel.devices[index];
    let id = d.id;
    let width = ui.available_width();
    let (r, _) = ui.allocate_exact_size(egui::vec2(width, 76.0), egui::Sense::hover());
    let has_tabs = d.kind != InstrumentKind::DcSource;
    let tabs_width = (width * 0.46).min(370.0);
    let tabs_left = (r.right() - 100.0 - tabs_width).max(r.left());
    let title_rect = Rect::from_min_max(
        r.min + egui::vec2(6.0, 2.0),
        egui::pos2(
            (if has_tabs {
                tabs_left - 12.0
            } else {
                r.right() - 90.0
            })
            .max(r.left() + 1.0),
            r.bottom(),
        ),
    );
    theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(title_rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(
                RichText::new(instrument_name(lang, d.kind))
                    .size(12.0)
                    .color(t.text_muted),
            );
            ui.add(
                egui::Label::new(RichText::new(&d.identity.model).size(24.0).strong()).truncate(),
            );
            let demo = d.resource.starts_with("DEMO::") || d.resource.contains("serial=DEMO");
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{}  ·  {}",
                        d.identity.manufacturer,
                        if demo {
                            text(lang, "模拟设备", "Simulated device")
                        } else {
                            text(lang, "会话已连接", "Session connected")
                        }
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                )
                .truncate(),
            )
            .on_hover_text(format!("{}\n{}", d.resource, d.identity.serial));
        },
    );
    if has_tabs {
        let key = egui::Id::new(("instrument-task", id));
        let mut tab = ui.ctx().data_mut(|d| d.get_temp::<u8>(key).unwrap_or(0));
        let old = tab;
        let rect = Rect::from_min_size(
            egui::pos2(tabs_left, r.top() + 18.0),
            egui::vec2(tabs_width, 34.0),
        );
        theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.push_id(("instrument-header-tabs", id), |ui| {
                    segments(
                        ui,
                        t,
                        "workspace-mode",
                        &mut tab,
                        &[
                            text(lang, "控制台", "Console"),
                            text(lang, "采集记录", "Capture"),
                            text(lang, "设备信息", "Device"),
                        ],
                    );
                });
            },
        );
        ui.ctx().data_mut(|d| d.insert_temp(key, tab));
        if tab != old && panel.devices[index].kind == InstrumentKind::Oscilloscope {
            panel.scope_heavy_defer_frames = 2;
        }
    }
    let actions = Rect::from_min_max(
        egui::pos2((r.right() - 84.0).max(r.left()), r.top() + 18.0),
        r.max,
    );
    theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(actions)
            .layout(egui::Layout::top_down(egui::Align::Max)),
        |ui| {
            if ui.button(text(lang, "断开", "Disconnect")).clicked() {
                let _ = panel.tx.send(Job::Disconnect(id));
            }
        },
    );
}

pub(super) fn connection(
    panel: &mut InstrumentControlPanel,
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
) {
    ui.add_space(24.0);
    ui.label(
        RichText::new(instrument_name(lang, panel.selected_kind))
            .size(24.0)
            .strong(),
    );
    ui.label(
        RichText::new(text(
            lang,
            "选择设备，建立仪表会话。",
            "Choose a device to establish a session.",
        ))
        .color(t.text_muted),
    );
    ui.add_space(24.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        section(
            ui,
            t,
            text(lang, "连接设备", "Connect an instrument"),
            "",
            |ui| {
                let slot = instrument_kind_slot(panel.selected_kind);
                property(ui, t, text(lang, "设备地址", "Resource"), |ui| {
                    field(
                        ui,
                        &mut panel.resource_inputs[slot],
                        "USB / TCPIP / probe:// …",
                    );
                });
                ui.add_space(8.0);
                action_group(ui, 2, |ui| {
                    if action(ui, t, text(lang, "连接", "Connect"), true).clicked() {
                        panel.begin_connect(panel.selected_kind, lang);
                    }
                    if action(ui, t, text(lang, "查找设备", "Discover"), false).clicked() {
                        scan(panel, lang);
                    }
                });
            },
        );
        let resources: Vec<_> = panel
            .resources
            .iter()
            .filter(|r| r.kind == Some(panel.selected_kind) || r.kind.is_none())
            .map(|r| {
                (
                    r.address.clone(),
                    r.identity
                        .as_ref()
                        .map(|i| i.model.clone())
                        .unwrap_or_else(|| short_resource(&r.address)),
                )
            })
            .collect();
        if !resources.is_empty() {
            section(ui, t, text(lang, "已发现", "Discovered"), "", |ui| {
                for (address, name) in resources {
                    if ui
                        .selectable_label(false, name)
                        .on_hover_text(&address)
                        .clicked()
                    {
                        panel.resource_inputs[instrument_kind_slot(panel.selected_kind)] = address;
                    }
                }
            });
        }
    });
}

pub(super) fn overview(
    panel: &mut InstrumentControlPanel,
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text(lang, "设备总览", "All instruments"))
            .size(24.0)
            .strong(),
    );
    ui.label(
        RichText::new(format!(
            "{} {}",
            panel.devices.len(),
            text(lang, "个设备会话", "instrument sessions")
        ))
        .color(t.text_muted),
    );
    ui.add_space(20.0);
    if panel
        .resources
        .iter()
        .any(|r| !panel.devices.iter().any(|d| d.resource == r.address))
    {
        if action(
            ui,
            t,
            text(lang, "连接已发现设备", "Connect discovered devices"),
            false,
        )
        .clicked()
        {
            let _ = panel.queue_connect_all(lang);
        }
        ui.add_space(12.0);
    }
    let mut picked = None;
    egui::ScrollArea::vertical()
        .id_salt("instrument-overview-list")
        .show(ui, |ui| {
            if panel.devices.is_empty() {
                section(
                    ui,
                    t,
                    text(lang, "开始连接", "Get connected"),
                    text(
                        lang,
                        "通过 USB 或网络连接实验室设备。",
                        "Connect your lab instruments over USB or the network.",
                    ),
                    |ui| {
                        if action(ui, t, text(lang, "查找设备", "Discover instruments"), true)
                            .clicked()
                        {
                            scan(panel, lang);
                        }
                    },
                );
            }
            let cols = if ui.available_width() >= 1050.0 {
                3
            } else if ui.available_width() >= 650.0 {
                2
            } else {
                1
            };
            for row in panel.devices.chunks(cols) {
                ui.columns(cols, |columns| {
                    for (d, ui) in row.iter().zip(columns.iter_mut()) {
                        section(
                            ui,
                            t,
                            &d.identity.model,
                            instrument_name(lang, d.kind),
                            |ui| {
                                ui.add_space(4.0);
                                let reading = panel
                                    .latest
                                    .iter()
                                    .filter(|((id, _), _)| *id == d.id)
                                    .min_by(|a, b| a.0.cmp(b.0))
                                    .map(|(_, r)| r);
                                metric(
                                    ui,
                                    t,
                                    text(lang, "最近读数", "Last reading"),
                                    reading,
                                    26.0,
                                );
                                ui.add_space(12.0);
                                if action(ui, t, text(lang, "打开控制台", "Open console"), false)
                                    .clicked()
                                {
                                    picked = Some((d.kind, d.id));
                                }
                            },
                        );
                    }
                });
            }
        });
    if let Some((kind, id)) = picked {
        panel.select_kind(kind);
        panel.selected_id = Some(id);
    }
}
