//! Native decoded-frame inspection, sharing the waveform's time axis and decoder.
use super::*;
use wiparse_core::bus_decode::BusFrame;

pub(super) fn is_data_frame(kind: BusKind, frame: &BusFrame) -> bool {
    // Retain PING in the decoder for clock recovery and reply/register association.
    !(kind == BusKind::Scp && frame.bytes.is_empty() && frame.summary == "PING")
}
pub(super) fn data_count(kind: BusKind, result: &BusDecodeResult) -> usize {
    result
        .frames
        .iter()
        .filter(|f| is_data_frame(kind, f))
        .count()
}
pub(super) fn matches_query(frame: &BusFrame, query: &str) -> bool {
    if query.is_empty() || frame.summary.to_lowercase().contains(query) {
        return true;
    }
    let hex: String = query
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    frame.bytes.windows(bytes.len()).any(|part| part == bytes)
}
fn detail_lines(kind: BusKind, frames: &[BusFrame], index: usize, zh: bool) -> Vec<String> {
    let Some(frame) = frames.get(index) else {
        return vec![];
    };
    if frame.bytes.len() > 256 {
        let raw = frame
            .bytes
            .iter()
            .take(256)
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        return vec![
            if zh {
                format!("显示范围  超长报文共{}字节，仅预览前256字节；未用截断数据作字段/CRC解释，完整数据仍保留。",frame.bytes.len())
            } else {
                format!("Preview  Long frame: {} bytes; first 256 shown. Truncated bytes are not reinterpreted or CRC-checked. Full decoded data retained.",frame.bytes.len())
            },
            format!("{}  {raw}", if zh { "原始字节" } else { "Raw" }),
        ];
    }
    explain_frame(kind, frames, index, zh)
}
#[derive(PartialEq)]
struct Key {
    generation: u64,
    lane: usize,
    address: usize,
    count: usize,
    query: String,
    errors: bool,
    kind: BusKind,
}
pub(super) struct RowCache {
    key: Key,
    rows: Vec<usize>,
}

fn left_cell(ui: &mut egui::Ui, width: f32, text: RichText, wrap: bool) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 18.),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            let label = egui::Label::new(text);
            ui.add(if wrap { label.wrap() } else { label.truncate() })
        },
    )
    .inner
}

impl WaveformAnalysisPanel {
    pub(super) fn protocol_results_panel(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
    ) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .corner_radius(6)
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing = egui::vec2(6., 4.);
                self.protocol_results_body(ui, lang, tokens);
            });
    }
    fn protocol_results_body(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let Some(result) = self.bus_result.as_ref() else {
            return;
        };
        let kind = self.bus_settings.kind;
        let width = ui.available_width();
        let key = Key {
            generation: self.bus_decode_gen,
            lane: self.active_bus_lane,
            address: result.frames.as_ptr() as usize,
            count: result.frames.len(),
            query: self.protocol_search.trim().to_lowercase(),
            errors: self.protocol_errors_only,
            kind,
        };
        if self
            .protocol_rows_cache
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            let rows: Vec<usize> = result
                .frames
                .iter()
                .enumerate()
                .filter(|(_, frame)| {
                    is_data_frame(kind, frame)
                        && (!key.errors || frame.has_error())
                        && matches_query(frame, &key.query)
                })
                .map(|(index, _)| index)
                .collect();
            if self
                .selected_bus_frame
                .is_some_and(|index| !rows.contains(&index))
            {
                self.selected_bus_frame = None;
                self.bus_frame_popup = None;
            }
            self.protocol_rows_cache = Some(RowCache { key, rows });
        }
        let rows = &self.protocol_rows_cache.as_ref().unwrap().rows;
        let total = data_count(kind, result);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(t(lang, "协议报文", "Decoded frames"))
                    .size(14.)
                    .strong(),
            )
            .on_hover_text(protocol_brief(kind, matches!(lang, Lang::Zh)));
            ui.label(
                RichText::new(format!("{} / {}", rows.len(), total))
                    .size(12.)
                    .color(tokens.text_muted),
            )
            .on_hover_text(t(
                lang,
                "显示 / 全部数据报文；SCP PING仅为时序记录，不计入报文。",
                "Shown / all data frames. SCP PING remains a timing record, excluded here.",
            ));
            ui.add(
                egui::TextEdit::singleline(&mut self.protocol_search)
                    .font(egui::FontId::proportional(13.))
                    .desired_width((width - 300.).clamp(70., 240.))
                    .char_limit(128)
                    .hint_text(t(lang, "筛选报文或 HEX…", "Filter frames or HEX…")),
            );
            if width > 480. {
                ui.checkbox(
                    &mut self.protocol_errors_only,
                    t(lang, "仅异常", "Errors only"),
                );
            } else {
                ui.menu_button(t(lang, "筛选", "Filter"), |ui| {
                    ui.checkbox(
                        &mut self.protocol_errors_only,
                        t(lang, "仅异常", "Errors only"),
                    );
                });
            }
            if width < 640.
                && ui
                    .small_button(if self.protocol_show_details {
                        t(lang, "报文", "Frames")
                    } else {
                        t(lang, "字段", "Fields")
                    })
                    .clicked()
            {
                self.protocol_show_details = !self.protocol_show_details;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text(t(lang, "收起报文列表", "Hide decoded frames"))
                    .clicked()
                {
                    self.protocol_results_open = false;
                }
            });
        });
        ui.separator();
        let area = ui.available_rect_before_wrap();
        let split = width >= 640.;
        let list_width = if split {
            (width * 0.55).min(width - 240.)
        } else {
            width
        };
        let mut selected = None;
        if split || !self.protocol_show_details {
            panel_in_rect(
                ui,
                egui::Rect::from_min_size(area.min, egui::vec2(list_width, area.height())),
                |ui| {
                    let status_w = if list_width >= 300. { 42. } else { 0. };
                    let time_w = 100.0_f32.min(list_width * 0.34);
                    let (head, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 22.),
                        egui::Sense::hover(),
                    );
                    for (x, name) in [
                        (6., t(lang, "时间", "Time")),
                        (time_w, t(lang, "报文", "Frame")),
                    ] {
                        ui.painter().text(
                            head.min + egui::vec2(x, 11.),
                            egui::Align2::LEFT_CENTER,
                            name,
                            egui::FontId::proportional(12.),
                            tokens.text_muted,
                        );
                    }
                    if status_w > 0. {
                        ui.painter().text(
                            head.right_center() - egui::vec2(4., 0.),
                            egui::Align2::RIGHT_CENTER,
                            t(lang, "状态", "Status"),
                            egui::FontId::proportional(12.),
                            tokens.text_muted,
                        );
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("native-protocol-rows")
                        .auto_shrink([false, false])
                        .show_rows(ui, 26., rows.len(), |ui, range| {
                            for row in range {
                                let index = rows[row];
                                let frame = &result.frames[index];
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 26.),
                                    egui::Sense::click(),
                                );
                                if self.selected_bus_frame == Some(index) || response.hovered() {
                                    ui.painter().rect_filled(rect, 3., tokens.accent_soft);
                                } else if row % 2 == 1 {
                                    ui.painter().rect_filled(rect, 2., tokens.panel_bg);
                                }
                                if self.selected_bus_frame == Some(index) {
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            rect.min,
                                            egui::vec2(2., rect.height()),
                                        ),
                                        1.,
                                        tokens.accent,
                                    );
                                }
                                let color = if frame.has_error() {
                                    tokens.stop_bg
                                } else {
                                    tokens.text_primary
                                };
                                ui.painter().text(
                                    rect.min + egui::vec2(6., 13.),
                                    egui::Align2::LEFT_CENTER,
                                    format_eng(frame.t_start, "s"),
                                    egui::FontId::monospace(12.),
                                    tokens.text_muted,
                                );
                                let summary = fit_text(
                                    ui,
                                    &frame.summary,
                                    egui::FontId::proportional(13.),
                                    color,
                                    (rect.width() - time_w - status_w - 10.).max(1.),
                                );
                                ui.painter().text(
                                    rect.min + egui::vec2(time_w, 13.),
                                    egui::Align2::LEFT_CENTER,
                                    summary,
                                    egui::FontId::proportional(13.),
                                    color,
                                );
                                if status_w > 0. {
                                    ui.painter().text(
                                        rect.right_center() - egui::vec2(4., 0.),
                                        egui::Align2::RIGHT_CENTER,
                                        if frame.has_error() {
                                            t(lang, "异常", "Error")
                                        } else {
                                            t(lang, "正常", "OK")
                                        },
                                        egui::FontId::proportional(12.),
                                        if frame.has_error() {
                                            tokens.stop_bg
                                        } else {
                                            tokens.text_muted
                                        },
                                    );
                                }
                                if response
                                    .on_hover_text(format!(
                                        "{}\n{:.9} s · {} · {}",
                                        frame.summary,
                                        frame.t_start,
                                        duration_text(frame.t_start, frame.t_end),
                                        integrity_text(frame.error, matches!(lang, Lang::Zh))
                                    ))
                                    .clicked()
                                {
                                    selected = Some(index);
                                }
                            }
                        });
                },
            );
        }
        if split || self.protocol_show_details {
            let left = if split {
                area.left() + list_width + 6.
            } else {
                area.left()
            };
            if split {
                ui.painter().line_segment(
                    [
                        egui::pos2(left - 3., area.top()),
                        egui::pos2(left - 3., area.bottom()),
                    ],
                    Stroke::new(1., tokens.divider),
                );
            }
            panel_in_rect(
                ui,
                egui::Rect::from_min_max(egui::pos2(left, area.top()), area.max),
                |ui| {
                    let (head, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 22.),
                        egui::Sense::hover(),
                    );
                    ui.painter().text(
                        head.left_center(),
                        egui::Align2::LEFT_CENTER,
                        t(lang, "字段详情", "Frame details"),
                        egui::FontId::proportional(12.),
                        tokens.text_muted,
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("native-protocol-fields")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let index = selected.or(self.selected_bus_frame);
                            if let Some((index, frame)) = index
                                .and_then(|index| {
                                    result.frames.get(index).map(|frame| (index, frame))
                                })
                                .filter(|(_, frame)| is_data_frame(kind, frame))
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&frame.summary).size(13.).strong(),
                                    )
                                    .wrap(),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(duration_text(frame.t_start, frame.t_end))
                                            .size(12.)
                                            .color(tokens.text_muted),
                                    );
                                    ui.label(
                                        RichText::new(integrity_text(
                                            frame.error,
                                            matches!(lang, Lang::Zh),
                                        ))
                                        .size(12.)
                                        .color(
                                            if frame.has_error() {
                                                tokens.stop_bg
                                            } else {
                                                tokens.text_muted
                                            },
                                        ),
                                    );
                                })
                                .response
                                .on_hover_text(format!(
                                    "{:.9}–{:.9} s",
                                    frame.t_start, frame.t_end
                                ));
                                ui.separator();
                                let label_w = (ui.available_width() * 0.27).clamp(58., 90.);
                                let value_w = (ui.available_width() - label_w - 8.).max(20.);
                                egui::Grid::new("native-field-grid")
                                    .num_columns(2)
                                    .min_col_width(0.)
                                    .spacing(egui::vec2(8., 5.))
                                    .show(ui, |ui| {
                                        for line in detail_lines(
                                            kind,
                                            &result.frames,
                                            index,
                                            matches!(lang, Lang::Zh),
                                        ) {
                                            let (name, value, raw) =
                                                field_parts(&line, matches!(lang, Lang::Zh));
                                            let (code, value) = split_code_value(&value);
                                            left_cell(
                                                ui,
                                                label_w,
                                                RichText::new(&name)
                                                    .size(12.)
                                                    .color(tokens.text_muted),
                                                false,
                                            )
                                            .on_hover_text(format!(
                                                "{}\n{}",
                                                name,
                                                field_note(&name, matches!(lang, Lang::Zh))
                                            ));
                                            let text = if raw {
                                                RichText::new(&value)
                                                    .font(egui::FontId::monospace(12.))
                                            } else {
                                                RichText::new(&value).size(13.)
                                            };
                                            let response = left_cell(ui, value_w, text, true);
                                            if !code.is_empty() {
                                                response.on_hover_text(format!(
                                                    "{}: {}",
                                                    t(lang, "原始编码", "Raw code"),
                                                    code
                                                ));
                                            }
                                            ui.end_row();
                                        }
                                    });
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(t(
                                            lang,
                                            "选择报文查看字段与数值",
                                            "Select a frame to inspect its fields",
                                        ))
                                        .size(13.)
                                        .color(tokens.text_muted),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                },
            );
        }
        if let Some(index) = selected {
            self.open_bus_frame(index);
            if !split {
                self.protocol_show_details = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scp_ping_is_not_data_but_other_protocol_ping_and_reset_remain_visible() {
        let ping = BusFrame {
            summary: "PING".into(),
            ..Default::default()
        };
        assert!(!is_data_frame(BusKind::Scp, &ping));
        assert!(is_data_frame(BusKind::Ufcs, &ping));
        assert!(is_data_frame(BusKind::UsbPd, &ping));
        assert!(is_data_frame(
            BusKind::Scp,
            &BusFrame {
                summary: "RESET".into(),
                ..Default::default()
            }
        ));
        assert!(is_data_frame(
            BusKind::Scp,
            &BusFrame {
                summary: "PING".into(),
                bytes: vec![0x0c],
                error: wiparse_core::bus_decode::BusFrameError::None,
                ..Default::default()
            }
        ));
    }
    #[test]
    fn dense_ping_does_not_consume_the_data_overlay_sampling_budget() {
        let frames: Vec<_> = (0..8192)
            .map(|i| BusFrame {
                t_start: i as f64,
                t_end: i as f64 + 0.5,
                summary: if i % 2 == 0 {
                    "PING".into()
                } else {
                    "data".into()
                },
                bytes: if i % 2 == 0 { vec![] } else { vec![1] },
                ..Default::default()
            })
            .collect();
        let marks = sampled_visible_indices(&frames, None, 128, Some(8191), |frame| {
            if is_data_frame(BusKind::Scp, frame) {
                (frame.t_start, frame.t_end)
            } else {
                (f64::NAN, f64::NAN)
            }
        });
        assert!(!marks.is_empty() && marks.len() <= 129);
        assert!(marks.iter().all(|i| i % 2 == 1));
        assert!(marks.contains(&8191));
    }
    #[test]
    fn hex_filter_reads_entire_frame_but_detail_preview_does_not_reinterpret_truncated_payload() {
        let mut bytes = vec![0; 10000];
        bytes.extend([0x0c, 0x7e]);
        let frame = BusFrame {
            bytes,
            summary: "long data".into(),
            ..Default::default()
        };
        assert!(matches_query(&frame, "0c 7e"));
        assert!(matches_query(&frame, "0c7e"));
        assert!(!matches_query(&frame, "AA BB"));
        let lines = detail_lines(BusKind::Scp, &[frame], 0, true);
        assert!(lines[0].contains("未用截断数据"));
        assert!(lines.iter().map(String::len).sum::<usize>() < 1600);
    }
    #[test]
    fn native_table_virtualizes_ping_filtered_rows_and_click_preserves_decoder_index() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-local-log-ui");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("native-protocol.csv");
        std::fs::write(&path, "Time(s),CH1(V)\n0,0\n0.001,5\n0.002,0\n").unwrap();
        let mut panel = WaveformAnalysisPanel::new(&AppConfig::default());
        panel.open_path(&path, Lang::Zh).unwrap();
        panel.bus_settings.kind = BusKind::Scp;
        panel.bus_settings.channels.charging_signal = Some(0);
        panel.commit_active_lane();
        let frames = (0..8192)
            .map(|i| BusFrame {
                t_start: i as f64 * 1e-7,
                t_end: (i as f64 + 0.8) * 1e-7,
                summary: if i % 2 == 0 {
                    "PING".into()
                } else {
                    format!("REQ_{i}")
                },
                bytes: if i % 2 == 0 {
                    vec![]
                } else {
                    vec![0x0c, 0x7e, 0x12]
                },
                ..Default::default()
            })
            .collect();
        let result = BusDecodeResult {
            frames,
            ..Default::default()
        };
        panel.bus_lanes[0].result = Some(result.clone());
        panel.bus_result = Some(result);
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let tokens = ui_theme::apply_theme(&ctx, false);
        ctx.style_mut(|style| style.animation_time = 0.);
        let mut draw = |events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 720.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(Frame::NONE.inner_margin(Margin::same(2)))
                        .show(ctx, |ui| panel.ui(ui, Lang::Zh, &tokens));
                },
            )
        };
        let _ = draw(vec![]);
        let _ = draw(vec![]);
        let out = draw(vec![]);
        let row_texts: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.pos.y > 500. && text.galley.job.text.starts_with("REQ_") =>
                {
                    Some(text)
                }
                _ => None,
            })
            .collect();
        assert!(
            !row_texts.is_empty() && row_texts.len() < 20,
            "unbounded rows: {}",
            row_texts.len()
        );
        assert!(!out.shapes.iter().any(
            |shape| matches!(&shape.shape,egui::Shape::Text(text)if text.galley.job.text=="PING")
        ));
        let text = row_texts
            .iter()
            .find(|text| text.galley.job.text == "REQ_1")
            .unwrap();
        let pos = text.pos + text.galley.rect.size() * 0.5;
        let _ = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let _ = draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let mut output = draw(vec![]);
        drop(draw);
        assert_eq!(panel.selected_bus_frame, Some(1));
        assert_eq!(panel.protocol_rows_cache.as_ref().unwrap().rows.len(), 4096);
        assert!(panel.last_x_range.is_some_and(|(a, b)| b - a < 0.001));
        // CRC is below the initial field viewport; verify real scrolling reaches it.
        for step in 0..24 {
            let events = if step == 0 {
                vec![
                    egui::Event::PointerMoved(egui::pos2(740., 675.)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -500.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 720.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(Frame::NONE.inner_margin(Margin::same(2)))
                        .show(ctx, |ui| panel.ui(ui, Lang::Zh, &tokens));
                },
            );
        }
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text)if text.galley.job.text.contains("CRC-8"))));
        output
            .textures_delta
            .set
            .retain(|(id, _)| *id != egui::TextureId::Managed(0));
        output.textures_delta.set.push((
            egui::TextureId::Managed(0),
            egui::epaint::ImageDelta::full(
                ctx.fonts(|fonts| fonts.image()),
                egui::TextureOptions::LINEAR,
            ),
        ));
        crate::ui_audit::export_frame(
            &ctx,
            output,
            &dir,
            "native-scp-protocol",
            egui::vec2(1100., 720.),
        );
        panel.protocol_errors_only = true;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 720.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel.ui(ui, Lang::Zh, &tokens));
            },
        );
        assert!(panel.protocol_rows_cache.as_ref().unwrap().rows.is_empty());
        assert_eq!(panel.selected_bus_frame, None);
    }
    #[test]
    #[ignore = "Set WIPARSE_SCP_CAPTURE to the reference Normal.isf capture"]
    fn native_scp_reference_capture_shows_60_data_frames_without_ping() {
        let path = std::env::var("WIPARSE_SCP_CAPTURE").unwrap();
        let mut panel = WaveformAnalysisPanel::new(&AppConfig::default());
        panel.open_path(Path::new(&path), Lang::Zh).unwrap();
        let channel = panel
            .waves
            .iter()
            .position(|wave| wave.trace.channel == "CH2")
            .unwrap();
        panel.bus_settings.kind = BusKind::Scp;
        panel.bus_settings.channels.charging_signal = Some(channel);
        panel.commit_active_lane();
        let result = decode_bus(
            &panel
                .waves
                .iter()
                .map(|wave| wave.trace.clone())
                .collect::<Vec<_>>(),
            &panel.bus_settings,
        );
        assert_eq!(data_count(BusKind::Scp, &result), 60);
        assert_eq!(
            result
                .frames
                .iter()
                .filter(|frame| !is_data_frame(BusKind::Scp, frame))
                .count(),
            218
        );
        let first = result
            .frames
            .iter()
            .position(|frame| is_data_frame(BusKind::Scp, frame))
            .unwrap();
        panel.bus_lanes[0].result = Some(result.clone());
        panel.bus_result = Some(result);
        panel.open_bus_frame(first);
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let tokens = ui_theme::apply_theme(&ctx, false);
        let mut output = egui::FullOutput::default();
        for _ in 0..3 {
            output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600., 1000.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(Frame::NONE.inner_margin(Margin::same(2)))
                        .show(ctx, |ui| panel.ui(ui, Lang::Zh, &tokens));
                },
            );
        }
        assert_eq!(panel.protocol_rows_cache.as_ref().unwrap().rows.len(), 60);
        assert!(!output.shapes.iter().any(
            |shape| matches!(&shape.shape,egui::Shape::Text(text)if text.galley.job.text=="PING")
        ));
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-local-log-ui");
        std::fs::create_dir_all(&dir).unwrap();
        output
            .textures_delta
            .set
            .retain(|(id, _)| *id != egui::TextureId::Managed(0));
        output.textures_delta.set.push((
            egui::TextureId::Managed(0),
            egui::epaint::ImageDelta::full(
                ctx.fonts(|fonts| fonts.image()),
                egui::TextureOptions::LINEAR,
            ),
        ));
        crate::ui_audit::export_frame(
            &ctx,
            output,
            &dir,
            "native-scp-reference",
            egui::vec2(1600., 1000.),
        );
    }
}
