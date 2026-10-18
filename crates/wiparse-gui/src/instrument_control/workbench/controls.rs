use super::design::*;

pub(crate) fn bridge_actions(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    action_group(ui, 2, |ui| {
        match d.controls.bridge_tab {
            1 => {
                if action(ui, t, text(lang, "执行 I2C 事务", "Transfer I2C"), true).clicked() {
                    out.push(ControlCommand::BridgeI2c {
                        addr: d.controls.bridge_i2c_addr.clone(),
                        write_hex: d.controls.bridge_write_hex.clone(),
                        read_len: d.controls.bridge_read_len,
                        clock_hz: d.controls.bridge_i2c_hz,
                    });
                }
            }
            2 => {
                if action(ui, t, text(lang, "应用 GPIO", "Apply GPIO"), true).clicked() {
                    out.push(ControlCommand::BridgeGpio {
                        pin: d.controls.bridge_gpio_pin,
                        dir: Some(d.controls.bridge_gpio_out),
                        value: Some(d.controls.bridge_gpio_value),
                    });
                }
                if action(ui, t, text(lang, "读取引脚", "Read pins"), false).clicked() {
                    out.push(ControlCommand::BridgeGpio {
                        pin: 255,
                        dir: None,
                        value: None,
                    });
                }
            }
            _ => {
                if action(ui, t, text(lang, "执行 SPI 事务", "Transfer SPI"), true).clicked() {
                    out.push(ControlCommand::BridgeSpi {
                        mode: d.controls.bridge_spi_mode,
                        clock_hz: d.controls.bridge_spi_hz,
                        cs: d.controls.bridge_spi_cs,
                        write_hex: d.controls.bridge_write_hex.clone(),
                        read_len: d.controls.bridge_read_len,
                    });
                }
            }
        }
        if action(ui, t, text(lang, "芯片信息", "Chip info"), false).clicked() {
            out.push(ControlCommand::BridgeInfo);
        }
    });
    out
}

pub(crate) fn dongle_actions(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    if d.controls.dongle_tab == 0 {
        return probe_actions(ui, lang, t, d);
    }
    let mut out = Vec::new();
    if d.controls.dongle_tab == 1 {
        action_group(ui, 2, |ui| {
            if action(ui, t, text(lang, "打开串口", "Open UART"), true).clicked() {
                out.push(ControlCommand::DongleSetLink {
                    host: d.controls.dongle_host.clone(),
                    uart_port: d.controls.dongle_uart_port,
                    i2c_port: d.controls.dongle_i2c_port,
                });
                out.push(ControlCommand::DongleUartOpen {
                    host: d.controls.dongle_host.clone(),
                    port: d.controls.dongle_uart_port,
                    baud: d.controls.dongle_uart_baud,
                    com: d.controls.dongle_uart_com.clone(),
                    send_baud_cmd: true,
                });
            }
            if action(ui, t, text(lang, "发送 UART", "Send UART"), false).clicked() {
                out.push(ControlCommand::DongleUartWrite {
                    data: d.controls.dongle_uart_tx.clone(),
                    hex: d.controls.dongle_uart_hex,
                });
            }
            if action(ui, t, text(lang, "读取 UART", "Read UART"), false).clicked() {
                out.push(ControlCommand::DongleUartRead);
            }
            if action(ui, t, text(lang, "关闭", "Close"), false).clicked() {
                d.controls.dongle_uart_listen = false;
                out.push(ControlCommand::DongleUartClose);
            }
        });
    } else {
        action_group(ui, 2, |ui| {
            if action(ui, t, text(lang, "写后读", "Write / read"), true).clicked() {
                out.push(ControlCommand::DongleI2cWriteRead {
                    addr: d.controls.dongle_i2c_addr.clone(),
                    data_hex: d.controls.dongle_i2c_write_hex.clone(),
                    len: d.controls.dongle_i2c_read_len,
                });
            }
            if action(ui, t, text(lang, "读取 I2C", "Read I2C"), false).clicked() {
                out.push(ControlCommand::DongleI2cRead {
                    addr: d.controls.dongle_i2c_addr.clone(),
                    len: d.controls.dongle_i2c_read_len,
                });
            }
            if action(ui, t, text(lang, "写入 I2C", "Write I2C"), false).clicked() {
                out.push(ControlCommand::DongleI2cWrite {
                    addr: d.controls.dongle_i2c_addr.clone(),
                    data_hex: d.controls.dongle_i2c_write_hex.clone(),
                });
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new(text(lang, "总线操作", "Bus tools")).strong());
        ui.scope(|ui| {
            action_group(ui, 2, |ui| {
                if action(ui, t, text(lang, "应用总线设置", "Apply bus setup"), false).clicked()
                {
                    out.push(ControlCommand::DongleI2cSpeed {
                        hz: Some(d.controls.dongle_i2c_hz),
                    });
                    out.push(ControlCommand::DongleI2cTimeout {
                        ms: Some(d.controls.dongle_i2c_timeout_ms),
                    });
                }
                if action(ui, t, text(lang, "扫描地址", "Scan addresses"), false).clicked() {
                    out.push(ControlCommand::DongleI2cScan);
                }
                if action(ui, t, "Ping", false).clicked() {
                    out.push(ControlCommand::DongleI2cPing {
                        addr: d.controls.dongle_i2c_addr.clone(),
                    });
                }
                if action(ui, t, text(lang, "恢复总线", "Recover bus"), false).clicked() {
                    out.push(ControlCommand::DongleI2cRecover);
                }
                if action(ui, t, text(lang, "帮助", "Help"), false).clicked() {
                    out.push(ControlCommand::DongleI2cLine {
                        line: "HELP".into(),
                    });
                }
            });
        });
    }
    // Every bus operation must use the endpoint edited beside its parameters,
    // not a stale endpoint last applied by Scan or the Device information page.
    if !out.is_empty() && d.controls.dongle_tab == 2 {
        out.insert(
            0,
            ControlCommand::DongleSetLink {
                host: d.controls.dongle_host.clone(),
                uart_port: d.controls.dongle_uart_port,
                i2c_port: d.controls.dongle_i2c_port,
            },
        );
    }
    out
}
use super::*;

pub(crate) fn probe_actions(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    action_group(ui, 2, |ui| match d.controls.probe_page {
        1 => {
            if action_enabled(
                ui,
                t,
                text(lang, "烧录固件", "Flash firmware"),
                true,
                !d.controls.probe_chip.trim().is_empty()
                    && !d.controls.probe_flash_path.trim().is_empty(),
            )
            .clicked()
            {
                out.push(ControlCommand::ProbeFlash {
                    path: d.controls.probe_flash_path.clone(),
                    verify: d.controls.probe_flash_verify,
                    base_address: None,
                });
            }
        }
        2 => {
            if action(ui, t, text(lang, "读取内存", "Read memory"), true).clicked() {
                out.push(ControlCommand::ProbeMemRead {
                    address: d.controls.probe_mem_addr.clone(),
                    len: d.controls.probe_mem_len,
                });
            }
            if action_enabled(
                ui,
                t,
                text(lang, "写入内存", "Write memory"),
                false,
                !d.controls.probe_mem_write_hex.trim().is_empty(),
            )
            .clicked()
            {
                out.push(ControlCommand::ProbeMemWrite {
                    address: d.controls.probe_mem_addr.clone(),
                    data_hex: d.controls.probe_mem_write_hex.clone(),
                });
            }
        }
        3 => {
            let old = d.controls.probe_rtt_on;
            if action(
                ui,
                t,
                if old {
                    text(lang, "停止接收", "Stop RTT")
                } else {
                    text(lang, "开始接收", "Start RTT")
                },
                !old,
            )
            .clicked()
            {
                d.controls.probe_rtt_on = !old;
                out.push(if old {
                    ControlCommand::ProbeRttStop
                } else {
                    ControlCommand::ProbeRttStart {
                        up_channel: d.controls.probe_rtt_channel,
                    }
                });
            }
        }
        _ => {
            if action(ui, t, text(lang, "连接目标", "Attach"), true).clicked() {
                out.push(ControlCommand::ProbeAttach {
                    target: d.controls.probe_chip.clone(),
                });
            }
            for (label, c) in [
                (text(lang, "暂停", "Halt"), ControlCommand::ProbeHalt),
                (text(lang, "运行", "Run"), ControlCommand::ProbeRun),
                (text(lang, "状态", "Status"), ControlCommand::ProbeStatus),
                (text(lang, "寄存器", "Registers"), ControlCommand::ProbeRegs),
                (
                    text(lang, "应用速率", "Apply speed"),
                    ControlCommand::ProbeSpeed {
                        khz: d.controls.probe_speed_khz,
                    },
                ),
            ] {
                if action(ui, t, label, false).clicked() {
                    out.push(c);
                }
            }
        }
    });
    if d.controls.probe_page <= 1 {
        ui.add_space(12.0);
        egui::CollapsingHeader::new(text(lang, "复位与擦除", "Reset & erase"))
            .id_salt(("probe-maintenance", d.id, d.controls.probe_page))
            .show(ui, |ui| {
                action_group(ui, 2, |ui| {
                    for (label, c) in [
                        (
                            text(lang, "软件复位", "Reset"),
                            ControlCommand::ProbeReset { hardware: false },
                        ),
                        (
                            text(lang, "硬件复位", "Hardware reset"),
                            ControlCommand::ProbeReset { hardware: true },
                        ),
                    ] {
                        if action(ui, t, label, false).clicked() {
                            out.push(c);
                        }
                    }
                    let mut danger = *t;
                    danger.text_primary = t.stop_bg;
                    if action_enabled(
                        ui,
                        &danger,
                        text(lang, "擦除芯片", "Erase chip"),
                        false,
                        !d.controls.probe_chip.trim().is_empty(),
                    )
                    .clicked()
                    {
                        out.push(ControlCommand::ProbeErase);
                    }
                });
            });
    }
    out
}

pub(crate) fn source(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    source_channel(ui, lang, t, d, d.controls.source_focus)
}

pub(super) fn source_channel(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
    channel: u8,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let ch = channel.clamp(1, d.capabilities.channels.clamp(1, 4));
    let i = ch as usize - 1;
    property(ui, t, text(lang, "设定电压", "Voltage"), |ui| {
        ui.add(
            egui::DragValue::new(&mut d.controls.source_voltages[i])
                .range(0.0..=60.0)
                .speed(0.01)
                .suffix(" V"),
        );
    });
    property(ui, t, text(lang, "电流上限", "Current limit"), |ui| {
        ui.add(
            egui::DragValue::new(&mut d.controls.source_currents[i])
                .range(0.0..=20.0)
                .speed(0.01)
                .suffix(" A"),
        );
    });
    ui.add_space(8.0);
    if action(ui, t, text(lang, "应用设定", "Apply setpoints"), true).clicked() {
        out.push(ControlCommand::SourceVoltage {
            channel: ch,
            value: d.controls.source_voltages[i],
        });
        out.push(ControlCommand::SourceCurrent {
            channel: ch,
            value: d.controls.source_currents[i],
        });
    }
    ui.add_space(12.0);
    ui.label(RichText::new(text(lang, "输出控制", "Output")).strong());
    ui.add_space(8.0);
    action_group(ui, 2, |ui| {
        for (enabled, label) in [
            (true, text(lang, "开启输出", "Output ON")),
            (false, text(lang, "关闭输出", "Output OFF")),
        ] {
            if action(ui, t, label, false).clicked() {
                d.controls.source_outputs[i] = enabled;
                out.push(ControlCommand::SourceOutput {
                    channel: ch,
                    enabled,
                });
            }
        }
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(text(
            lang,
            "输出指令与实测读数分别显示。",
            "Output commands are separate from measured readings.",
        ))
        .size(11.0)
        .color(t.text_muted),
    );
    out
}

pub(crate) fn protection(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    protection_channel(ui, lang, t, d, d.controls.source_focus)
}

pub(super) fn protection_channel(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
    channel: u8,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    if !d.capabilities.source_protection {
        ui.label(
            RichText::new(text(
                lang,
                "设备未声明保护设置能力。",
                "Protection setup is not advertised by this device.",
            ))
            .color(t.text_muted),
        );
        return out;
    }
    let ch = channel.clamp(1, d.capabilities.channels.clamp(1, 4));
    let i = ch as usize - 1;
    property(ui, t, text(lang, "过压保护", "Overvoltage"), |ui| {
        ui.add(
            egui::DragValue::new(&mut d.controls.source_ovps[i])
                .range(0.0..=60.0)
                .speed(0.01)
                .suffix(" V"),
        );
    });
    property(ui, t, text(lang, "过流保护", "Overcurrent"), |ui| {
        ui.add(
            egui::DragValue::new(&mut d.controls.source_ocps[i])
                .range(0.0..=20.0)
                .speed(0.01)
                .suffix(" A"),
        );
    });
    ui.add_space(12.0);
    if action(ui, t, text(lang, "应用保护", "Apply protection"), false).clicked() {
        out.push(ControlCommand::SourceOvp {
            channel: ch,
            value: d.controls.source_ovps[i],
        });
        out.push(ControlCommand::SourceOcp {
            channel: ch,
            value: d.controls.source_ocps[i],
        });
    }
    out
}

pub(crate) fn load(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let modes = d.capabilities.load_modes.clone();
    let labels: Vec<_> = modes.iter().map(String::as_str).collect();
    let mut mode = modes
        .iter()
        .position(|m| *m == d.controls.load_mode)
        .unwrap_or(0) as u8;
    if !labels.is_empty() {
        segments(ui, t, ("load-mode", d.id), &mut mode, &labels);
        d.controls.load_mode = modes[mode as usize].clone();
    }
    ui.add_space(8.0);
    let unit = match d.controls.load_mode.as_str() {
        "CV" => " V",
        "CR" => " Ω",
        "CP" => " W",
        _ => " A",
    };
    property(ui, t, text(lang, "目标值", "Target level"), |ui| {
        ui.add(
            egui::DragValue::new(&mut d.controls.load_level)
                .speed(0.01)
                .suffix(unit),
        );
    });
    ui.add_space(8.0);
    if action(ui, t, text(lang, "应用设定", "Apply setpoints"), true).clicked() {
        out.push(ControlCommand::LoadMode(d.controls.load_mode.clone()));
        out.push(ControlCommand::LoadLevel {
            mode: d.controls.load_mode.clone(),
            value: d.controls.load_level,
        });
    }
    ui.add_space(12.0);
    ui.label(RichText::new(text(lang, "负载输入", "Load input")).strong());
    ui.add_space(8.0);
    action_group(ui, 2, |ui| {
        for (enabled, label) in [
            (true, text(lang, "开启输入", "Input ON")),
            (false, text(lang, "关闭输入", "Input OFF")),
        ] {
            if action(ui, t, label, false).clicked() {
                d.controls.load_input = enabled;
                out.push(ControlCommand::LoadInput(enabled));
            }
        }
    });
    out
}

pub(crate) fn dmm(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    property(ui, t, text(lang, "测量功能", "Function"), |ui| {
        egui::ComboBox::from_id_salt(("meter-function", d.id))
            .width(ui.available_width())
            .selected_text(d.controls.dmm_function.label())
            .show_ui(ui, |ui| {
                for function in MeasureFunction::all().iter().filter(|f| {
                    d.capabilities
                        .measure_functions
                        .iter()
                        .any(|s| s == f.scpi())
                }) {
                    ui.selectable_value(&mut d.controls.dmm_function, *function, function.label());
                }
            });
    });
    property(ui, t, text(lang, "自动量程", "Auto range"), |ui| {
        ui.add_enabled_ui(d.capabilities.range_control, |ui| {
            toggle(ui, t, &mut d.controls.dmm_autorange);
        });
    });
    property(ui, t, text(lang, "手动量程", "Manual range"), |ui| {
        ui.add_enabled(
            d.capabilities.range_control && !d.controls.dmm_autorange,
            egui::DragValue::new(&mut d.controls.dmm_range)
                .speed(0.1)
                .suffix(format!(" {}", d.controls.dmm_function.unit())),
        );
    });
    property(ui, t, "NPLC", |ui| {
        ui.add_enabled(
            d.capabilities.nplc_control,
            egui::DragValue::new(&mut d.controls.dmm_nplc)
                .range(0.001..=100.0)
                .speed(0.01),
        );
    });
    ui.add_space(8.0);
    if action(ui, t, text(lang, "应用配置", "Apply setup"), true).clicked() {
        out.push(ControlCommand::DmmFunction(d.controls.dmm_function));
        if d.capabilities.range_control {
            out.push(ControlCommand::DmmAutoRange {
                function: d.controls.dmm_function,
                enabled: d.controls.dmm_autorange,
            });
            if !d.controls.dmm_autorange {
                out.push(ControlCommand::DmmRange {
                    function: d.controls.dmm_function,
                    value: d.controls.dmm_range,
                });
            }
        }
        if d.capabilities.nplc_control {
            out.push(ControlCommand::DmmNplc {
                function: d.controls.dmm_function,
                value: d.controls.dmm_nplc,
            });
        }
    }
    out
}

pub(crate) fn scope(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    d: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let channels = d.capabilities.channels.clamp(1, 4);
    let ch = d.controls.scope_channel.clamp(1, channels);
    let i = ch as usize - 1;
    if d.controls.scope_setup_page == 0 {
        property(ui, t, text(lang, "显示通道", "Channel display"), |ui| {
            if toggle(ui, t, &mut d.controls.scope_channel_on[i]).changed() {
                out.push(ControlCommand::ScopeChannel {
                    channel: ch,
                    enabled: d.controls.scope_channel_on[i],
                });
            }
        });
        property(ui, t, text(lang, "垂直刻度", "Scale"), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.controls.scope_scales[i])
                    .range(1e-6..=100.0)
                    .max_decimals(6)
                    .speed(0.001)
                    .suffix(" V/div"),
            );
        });
        property(ui, t, text(lang, "垂直位置", "Position"), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.controls.scope_positions[i])
                    .range(-8.0..=8.0)
                    .speed(SCOPE_POS_STEP)
                    .suffix(" div"),
            );
        });
        ui.add_space(12.0);
        if action(ui, t, text(lang, "应用通道", "Apply channel"), true).clicked() {
            out.push(ControlCommand::ScopeScale {
                channel: ch,
                volts_per_div: d.controls.scope_scales[i],
            });
            out.push(ControlCommand::ScopePosition {
                channel: ch,
                divisions: d.controls.scope_positions[i],
            });
        }
    } else {
        property(ui, t, text(lang, "水平时基", "Timebase"), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.controls.scope_timebase)
                    .range(1e-9..=10.0)
                    .max_decimals(9)
                    .speed(0.00001)
                    .suffix(" s/div"),
            );
        });
        property(ui, t, text(lang, "触发源", "Source"), |ui| {
            egui::ComboBox::from_id_salt(("trigger-source", d.id))
                .width(ui.available_width())
                .selected_text(&d.controls.trigger_source)
                .show_ui(ui, |ui| {
                    for ch in 1..=channels {
                        let label = format!("CH{ch}");
                        ui.selectable_value(&mut d.controls.trigger_source, label.clone(), label);
                    }
                });
        });
        property(ui, t, text(lang, "触发电平", "Level"), |ui| {
            ui.add(
                egui::DragValue::new(&mut d.controls.trigger_level)
                    .speed(0.01)
                    .suffix(" V"),
            );
        });
        property(ui, t, text(lang, "触发边沿", "Slope"), |ui| {
            let mut slope = u8::from(d.controls.trigger_slope == "FALL");
            segments(
                ui,
                t,
                ("slope", d.id),
                &mut slope,
                &[
                    text(lang, "上升沿", "Rising"),
                    text(lang, "下降沿", "Falling"),
                ],
            );
            d.controls.trigger_slope = if slope == 0 { "RISE" } else { "FALL" }.into();
        });
        ui.add_space(12.0);
        if action(ui, t, text(lang, "应用触发", "Apply trigger"), false).clicked() {
            out.push(ControlCommand::ScopeTimebase(d.controls.scope_timebase));
            out.push(ControlCommand::ScopeTrigger {
                source: d.controls.trigger_source.clone(),
                level: d.controls.trigger_level,
                slope: d.controls.trigger_slope.clone(),
            });
        }
    }
    out
}
