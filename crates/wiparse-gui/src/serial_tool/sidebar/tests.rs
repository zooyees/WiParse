use super::*;

#[test]
fn actions_fit_narrow_cards_and_preserve_enabled_state() {
    for lang in [Lang::Zh, Lang::En] {
        for width in [120., 150., 174., 220., 320.] {
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let t = ui_theme::apply_theme(&ctx, false);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(width);
                    let left = ui.cursor().left();
                    let (a, b) = action_pair(
                        ui,
                        &t,
                        width,
                        &tr(lang, "btn.browse_dir"),
                        &tr(lang, "btn.open_log"),
                        false,
                    );
                    for response in [&a, &b] {
                        assert!(response.rect.left() >= left - 0.1);
                        assert!(
                            response.rect.right() <= left + width + 0.1,
                            "{lang:?} {width}: {:?}",
                            response.rect
                        );
                        assert!((response.rect.height() - ui_theme::CTRL_H).abs() < 0.1);
                    }
                    assert!(a.enabled());
                    assert!(!b.enabled());
                    if width <= 174. {
                        assert!(b.rect.top() >= a.rect.bottom());
                        assert_eq!(a.rect.width(), b.rect.width());
                    } else if width == 320. {
                        assert_eq!(a.rect.top(), b.rect.top());
                    }
                });
            });
        }
    }
}

#[test]
fn serial_sidebar_expanded_recording_controls_stay_inside_cards() {
    for lang in [Lang::Zh, Lang::En] {
        for width in [200., 240., 320.] {
            for light in [false, true] {
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = ui_theme::apply_theme(&ctx, light);
                let mut config = AppConfig::default();
                config.log_monitor.save_live_to_disk = false;
                config.log_monitor.open_log_files.clear();
                let mut panel = SerialToolPanel::new(&config, lang);
                panel.recording_settings_open = true;
                panel.browser_dir.clear();
                let size = egui::vec2(1000., 1000.);
                let mut out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let id = ui.make_persistent_id(("tool_sidebar_width", "serial_tool"));
                            ui.ctx().data_mut(|d| d.insert_temp(id, width));
                            panel.ui(ui, lang, &t);
                        });
                    },
                );
                let mut count = 0;
                for shape in &out.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if ["btn.browse_dir", "btn.open_log", "btn.refresh_browser"]
                            .iter()
                            .any(|key| text.galley.text() == tr(lang, key))
                        {
                            count += 1;
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(
                                rect.right() <= shape.clip_rect.right() + 0.5,
                                "{lang:?} {width}: {} {rect:?} {:?}",
                                text.galley.text(),
                                shape.clip_rect
                            );
                            assert!(
                                rect.right() <= width + 8.,
                                "Control expanded sidebar: {rect:?}"
                            );
                        }
                    }
                }
                assert_eq!(count, 4);
                if lang == Lang::Zh && width == 200. {
                    out.textures_delta
                        .set
                        .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                    out.textures_delta.set.push((
                        egui::TextureId::Managed(0),
                        egui::epaint::ImageDelta::full(
                            ctx.fonts(|f| f.image()),
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                    crate::ui_audit::export_frame(
                        &ctx,
                        out,
                        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../tmp/serial-sidebar-fit-ui"),
                        if light {
                            "sidebar-200-light"
                        } else {
                            "sidebar-200-dark"
                        },
                        size,
                    );
                }
            }
        }
    }
}
