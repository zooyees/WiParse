//! Display only series supplied by the local engine; never model-generated coordinates.
use super::*;
use egui_plot::{Line, Plot, PlotPoints};
pub(super) fn show(ui: &mut egui::Ui, data: &serde_json::Value, lang: Lang, t: &Tokens) {
    let Some(fields) = data["fields"].as_array() else {
        return;
    };
    let Some(sources) = data["sources"].as_array() else {
        return;
    };
    if fields.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(label(lang, "日志实测趋势", "Extracted log trend")).strong());
    let id = ui.id().with("timeline-field");
    let default = fields.iter().position(|f| f["unit"] == "W").unwrap_or(0);
    let mut selected = ui
        .data_mut(|d| d.get_temp::<usize>(id))
        .unwrap_or(default)
        .min(fields.len() - 1);
    egui::ComboBox::from_id_salt(id)
        .width((ui.available_width() - 12.0).clamp(40.0, 260.0))
        .selected_text(format!(
            "{} · {}",
            fields[selected]["name"].as_str().unwrap_or("value"),
            fields[selected]["unit"].as_str().unwrap_or("")
        ))
        .show_ui(ui, |ui| {
            for (i, f) in fields.iter().enumerate() {
                ui.selectable_value(
                    &mut selected,
                    i,
                    format!(
                        "{} · {}",
                        f["name"].as_str().unwrap_or("value"),
                        f["unit"].as_str().unwrap_or("")
                    ),
                );
            }
        });
    ui.data_mut(|d| d.insert_temp(id, selected));
    for source in sources {
        let start = source["first_time_s"].as_f64().unwrap_or(0.0);
        if let Some(duration)=source["observed_duration_s"].as_f64() {
            ui.small(format!("{} {:.3} s · {} {}–{}",label(lang,"观测跨度","Observed span"),duration,
                label(lang,"行","rows"),source["first_line"],source["last_line"]));
        }
        let mut segments: Vec<Vec<[f64; 2]>> = vec![vec![]];
        for point in source["points"].as_array().into_iter().flatten() {
            if let Some((x, y)) = point[0].as_f64().zip(point[2][selected].as_f64()) {
                segments.last_mut().unwrap().push([x - start, y]);
            } else if !segments.last().unwrap().is_empty() {
                segments.push(vec![]);
            }
        }
        if segments.iter().any(|s| !s.is_empty()) {
            Plot::new(
                ui.id()
                    .with(("series", source["source_id"].as_str().unwrap_or(""))),
            )
            .height(158.0)
            .width(ui.available_width())
            .allow_scroll(false)
            .show(ui, |plot| {
                for segment in segments {
                    if !segment.is_empty() {
                        plot.line(
                            Line::new(PlotPoints::from(segment))
                                .color(t.accent)
                                .name(fields[selected]["unit"].as_str().unwrap_or("")),
                        );
                    }
                }
            });
        } else {
            ui.label(label(
                lang,
                "无有效带时间数值，请检查字段规则或源数据。",
                "No timestamped values; check field rules or source data.",
            ));
        }
        let stats = &source["statistics"][selected];
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "n={} · min {} · max {} · mean {}",
                    stats["count"], stats["min"], stats["max"], stats["mean"]
                ))
                .size(12.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        if source["missing_or_invalid_timestamp_rows"]
            .as_u64()
            .unwrap_or(0)
            > 0
            || source["backward_timestamp_rows_skipped"]
                .as_u64()
                .unwrap_or(0)
                > 0
        {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(if let Some(parsed)=source["parsed_timestamp_rows"].as_u64(){
                        format!("{} {} · {} {} · {} {} · {} {}",label(lang,"时间已识别","Parsed time"),parsed,
                            label(lang,"未匹配时间","No time match"),source["timestamp_selector_unmatched_rows"],
                            label(lang,"格式无效","Invalid time"),source["invalid_timestamp_value_rows"],
                            label(lang,"倒退跳过","Backward"),source["backward_timestamp_rows_skipped"])
                    }else{format!("{} {} · {} {}",label(lang,"未识别时间的行数","Rows without parsed time"),
                        source["missing_or_invalid_timestamp_rows"],label(lang,"倒退跳过","Backward"),source["backward_timestamp_rows_skipped"])})
                    .size(12.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
        }
        if data["cycle_requested"] != false && source["observed_duration_s"].is_number() && (source["end_marker_confirmed"] != true || source["start_marker_confirmed"] != true) {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(label(
                        lang,
                        "周期起止未全部确认；观测跨度不等于完整充电时长。",
                        "Cycle boundaries unconfirmed; observed span is not complete charging duration.",
                    ))
                    .size(12.0)
                    .color(t.warning),
                )
                .wrap(),
            );
        }
    }
    ui.add(
        egui::Label::new(
            egui::RichText::new(label(
                lang,
                "横轴：相对秒；曲线为有界分桶均值，统计使用全部有效样本。",
                "X: relative seconds; bounded bucket means, statistics use all valid samples.",
            ))
            .size(11.0)
            .color(t.text_muted),
        )
        .wrap(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_plot_and_quality_text_fit_narrow_light_and_dark_panes() {
        let data = json!({"fields":[{"name":"power_W","unit":"W"}],"sources":[{"source_id":"synthetic","first_time_s":0,"first_line":1,"last_line":3,"observed_duration_s":2.0,"end_marker_confirmed":false,"start_marker_confirmed":true,"missing_or_invalid_timestamp_rows":1,"backward_timestamp_rows_skipped":0,"statistics":[{"count":3,"min":0,"max":10,"mean":5}],"points":[[0,1,[0]],[1,2,[10]],[2,3,[5]]]}]});
        for width in [220.0, 420.0] {
            for light in [false, true] {
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = ui_theme::apply_theme(&ctx, light);
                let size = egui::vec2(width, 700.0);
                let mut frame = None;
                for _ in 0..2 {
                    frame = Some(ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                show(ui, &data, Lang::Zh, &t);
                                assert!(ui.min_rect().right() <= width + 0.5, "plot overflow");
                            });
                        },
                    ));
                }
                let mut frame = frame.unwrap();
                assert!(frame.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("周期起止"))));
                frame
                    .textures_delta
                    .set
                    .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                frame.textures_delta.set.push((
                    egui::TextureId::Managed(0),
                    egui::epaint::ImageDelta::full(
                        ctx.fonts(|f| f.image()),
                        egui::TextureOptions::LINEAR,
                    ),
                ));
                crate::ui_audit::export_frame(
                    &ctx,
                    frame,
                    &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tmp/serial-agent-workflow-20261110/ui"),
                    &format!(
                        "timeline-{}-{}",
                        width,
                        if light { "light" } else { "dark" }
                    ),
                    size,
                );
            }
        }
    }
}
