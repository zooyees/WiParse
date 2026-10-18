use super::design::*;
use super::*;

pub(super) fn probe(ui: &mut egui::Ui, lang: Lang, t: &Tokens, d: &mut DeviceUi) {
    match d.controls.probe_page {
        1 => {
            property(ui, t, text(lang, "目标芯片", "Target"), |ui| {
                field(ui, &mut d.controls.probe_chip, "STM32F103C8");
            });
            property(ui, t, text(lang, "固件文件", "Firmware"), |ui| {
                field(ui, &mut d.controls.probe_flash_path, "HEX / BIN / ELF");
            });
            property(ui, t, text(lang, "写后校验", "Verify"), |ui| {
                toggle(ui, t, &mut d.controls.probe_flash_verify);
            });
            ui.add_space(12.0);
            if action(ui, t, text(lang, "选择文件…", "Choose file…"), false).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Firmware", &["hex", "ihex", "bin", "elf"])
                    .pick_file()
                {
                    d.controls.probe_flash_path = path.display().to_string();
                }
            }
        }
        2 => {
            property(ui, t, text(lang, "起始地址", "Address"), |ui| {
                field(ui, &mut d.controls.probe_mem_addr, "08000000");
            });
            property(ui, t, text(lang, "读取长度", "Read length"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.probe_mem_len)
                        .range(1..=4096)
                        .suffix(" B"),
                );
            });
            property(ui, t, text(lang, "写入数据", "Write data"), |ui| {
                field(ui, &mut d.controls.probe_mem_write_hex, "DE AD BE EF");
            });
        }
        3 => {
            property(ui, t, text(lang, "上行通道", "Up channel"), |ui| {
                ui.add(egui::DragValue::new(&mut d.controls.probe_rtt_channel).range(0..=15));
            });
        }
        _ => {
            property(ui, t, text(lang, "目标芯片", "Target"), |ui| {
                field(ui, &mut d.controls.probe_chip, "STM32F103C8");
            });
            property(ui, t, text(lang, "SWD 速率", "SWD speed"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.probe_speed_khz)
                        .range(10..=50000)
                        .speed(100.0)
                        .suffix(" kHz"),
                );
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(text(
                    lang,
                    "芯片留空时仅连接调试端口。",
                    "Leave target empty for a debug-port-only connection.",
                ))
                .color(t.text_muted)
                .size(11.0),
            );
        }
    }
}

pub(super) fn bridge(ui: &mut egui::Ui, lang: Lang, t: &Tokens, d: &mut DeviceUi) {
    match d.controls.bridge_tab {
        2 => {
            property(ui, t, text(lang, "引脚", "Pin"), |ui| {
                ui.add(egui::DragValue::new(&mut d.controls.bridge_gpio_pin).range(0..=3));
            });
            property(ui, t, text(lang, "输出模式", "Output mode"), |ui| {
                toggle(ui, t, &mut d.controls.bridge_gpio_out);
            });
            property(ui, t, text(lang, "高电平", "High level"), |ui| {
                toggle(ui, t, &mut d.controls.bridge_gpio_value);
            });
        }
        mode => {
            if mode == 1 {
                property(ui, t, text(lang, "7 位地址", "7-bit address"), |ui| {
                    field(ui, &mut d.controls.bridge_i2c_addr, "50");
                });
                property(ui, t, text(lang, "时钟频率", "Clock"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut d.controls.bridge_i2c_hz)
                            .range(60000..=3400000)
                            .speed(1000.0)
                            .suffix(" Hz"),
                    );
                });
            } else {
                property(ui, t, text(lang, "SPI 模式", "SPI mode"), |ui| {
                    ui.add(egui::DragValue::new(&mut d.controls.bridge_spi_mode).range(0..=3));
                });
                property(ui, t, text(lang, "时钟频率", "Clock"), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut d.controls.bridge_spi_hz)
                            .range(60000..=30000000)
                            .speed(1000.0)
                            .suffix(" Hz"),
                    );
                });
                property(ui, t, text(lang, "片选", "Chip select"), |ui| {
                    ui.add(egui::DragValue::new(&mut d.controls.bridge_spi_cs).range(0..=3));
                });
            }
            property(ui, t, text(lang, "发送 HEX", "Transmit HEX"), |ui| {
                field(ui, &mut d.controls.bridge_write_hex, "AA 55");
            });
            property(ui, t, text(lang, "读取长度", "Read length"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.bridge_read_len)
                        .range(0..=256)
                        .suffix(" B"),
                );
            });
        }
    }
}

pub(super) fn dongle(ui: &mut egui::Ui, lang: Lang, t: &Tokens, d: &mut DeviceUi) {
    if d.controls.dongle_tab != 0 {
        property(ui, t, text(lang, "网络主机", "Host"), |ui| {
            field(ui, &mut d.controls.dongle_host, "dap.local");
        });
        property(ui, t, text(lang, "服务端口", "Service port"), |ui| {
            let port = if d.controls.dongle_tab == 1 {
                &mut d.controls.dongle_uart_port
            } else {
                &mut d.controls.dongle_i2c_port
            };
            ui.add(egui::DragValue::new(port).range(1..=65535));
        });
        ui.add_space(8.0);
    }
    match d.controls.dongle_tab {
        1 => {
            property(ui, t, text(lang, "波特率", "Baud rate"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.dongle_uart_baud)
                        .range(dongle::UART_BAUD_MIN..=dongle::UART_BAUD_MAX)
                        .speed(100.0),
                );
            });
            property(
                ui,
                t,
                text(lang, "串口（可选）", "COM (optional)"),
                |ui| {
                    field(ui, &mut d.controls.dongle_uart_com, "COM5");
                },
            );
            property(ui, t, text(lang, "HEX 模式", "HEX mode"), |ui| {
                toggle(ui, t, &mut d.controls.dongle_uart_hex);
            });
            property(ui, t, text(lang, "持续接收", "Continuous RX"), |ui| {
                toggle(ui, t, &mut d.controls.dongle_uart_listen);
            });
            property(ui, t, text(lang, "发送数据", "Transmit"), |ui| {
                field(ui, &mut d.controls.dongle_uart_tx, "AA 55");
            });
        }
        2 => {
            property(ui, t, text(lang, "7 位地址", "7-bit address"), |ui| {
                field(ui, &mut d.controls.dongle_i2c_addr, "50");
            });
            property(ui, t, text(lang, "总线速率", "Bus speed"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.dongle_i2c_hz)
                        .range(10000..=1000000)
                        .speed(1000.0)
                        .suffix(" Hz"),
                );
            });
            property(ui, t, text(lang, "超时", "Timeout"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.dongle_i2c_timeout_ms)
                        .range(1..=1000)
                        .suffix(" ms"),
                );
            });
            property(ui, t, text(lang, "发送 HEX", "Transmit HEX"), |ui| {
                field(ui, &mut d.controls.dongle_i2c_write_hex, "AA 55");
            });
            property(ui, t, text(lang, "读取长度", "Read length"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.controls.dongle_i2c_read_len)
                        .range(1..=256)
                        .suffix(" B"),
                );
            });
        }
        _ => probe(ui, lang, t, d),
    }
}

pub(super) fn network(ui: &mut egui::Ui, lang: Lang, t: &Tokens, d: &mut DeviceUi) {
    property(ui, t, text(lang, "网络主机", "Host"), |ui| {
        field(ui, &mut d.controls.dongle_host, "dap.local");
    });
    property(ui, t, text(lang, "UART 端口", "UART port"), |ui| {
        ui.add(egui::DragValue::new(&mut d.controls.dongle_uart_port).range(1..=65535));
    });
    property(ui, t, text(lang, "I2C 端口", "I2C port"), |ui| {
        ui.add(egui::DragValue::new(&mut d.controls.dongle_i2c_port).range(1..=65535));
    });
}
