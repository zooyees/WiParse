//! Headless render coverage: no instrument connections or plugin execution.
use egui::{Pos2, RawInput, Rect, Vec2};
use wiparse_core::{config::AppConfig, i18n::Lang};

#[test]
fn main_pages_render_in_both_languages_themes_and_display_scales() {
    let mut cfg = AppConfig::default();
    let root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ui-audit-projects");
    cfg.apps.test_tool.data_root = root.display().to_string();
    cfg.apps.test_tool.plugins_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-tools/plugins")
        .display()
        .to_string();
    let mut calculator = crate::calculator::CalculatorPanel::new();
    let mut data = crate::data_analysis::DataAnalysisPanel::new(&cfg);
    let mut wave = crate::waveform_analysis::WaveformAnalysisPanel::new(&cfg);
    let mut serial = crate::serial_tool::SerialToolPanel::new(&cfg, Lang::Zh);
    let mut instrument = crate::instrument_control::InstrumentControlPanel::new(&cfg);
    let mut report = crate::test_report::TestReportPanel::new(&cfg);
    let mut plugins = crate::test_tool::TestToolPanel::new(&cfg);
    plugins.prepare_layout_audit();
    for lang in [Lang::Zh, Lang::En] {
        for light in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                for size in [
                    Vec2::new(640.0, 360.0),
                    Vec2::new(1100.0, 720.0),
                    Vec2::new(1920.0, 1080.0),
                ] {
                    for page in 0..7 {
                        let ctx = egui::Context::default();
                        crate::fonts::install_cjk_fonts(&ctx);
                        ctx.set_pixels_per_point(scale);
                        let tokens = crate::theme::apply_theme(&ctx, light);
                        let output = ctx.run(
                            RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default().show(ctx, |ui| match page {
                                    0 => calculator.ui(ui, lang, &tokens),
                                    1 => data.ui(ui, lang, &tokens),
                                    2 => wave.ui(ui, lang, &tokens),
                                    3 => serial.ui(ui, lang, &tokens),
                                    4 => instrument.ui(ui, lang, &tokens),
                                    5 => {
                                        report.ui(
                                            ui,
                                            lang,
                                            &tokens,
                                            &root,
                                            &[],
                                            None,
                                            "default",
                                            0,
                                            None,
                                        );
                                    }
                                    6 => plugins.ui(ui, lang, &tokens, &[]),
                                    _ => unreachable!(),
                                });
                            },
                        );
                        if matches!(lang, Lang::Zh) && !light && scale == 1.0 && size.x == 1100.0 {
                            if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                                export_frame(
                                    &ctx,
                                    output.clone(),
                                    &std::path::PathBuf::from(dir),
                                    &format!("page-{page}"),
                                    size,
                                );
                            }
                        }
                        assert!(!output.shapes.is_empty(), "empty page {page}");
                        for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                            assert!(
                                primitive.clip_rect.is_finite(),
                                "invalid clip on page {page}"
                            );
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                assert!(mesh.is_valid());
                                assert!(
                                    mesh.vertices.iter().all(|v| v.pos.is_finite()),
                                    "invalid geometry on page {page}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn export_frame(
    ctx: &egui::Context,
    output: egui::FullOutput,
    dir: &std::path::Path,
    name: &str,
    size: Vec2,
) {
    std::fs::create_dir_all(dir).unwrap();
    let mut textures = Vec::new();
    for (id, delta) in output.textures_delta.set {
        assert!(delta.pos.is_none());
        let [width, height] = delta.image.size();
        let pixels: Vec<_> = match &delta.image {
            egui::ImageData::Color(image) => image.pixels.clone(),
            egui::ImageData::Font(image) => image.srgba_pixels(None).collect(),
        };
        let bytes: Vec<u8> = pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
        let filename = format!("{name}-texture-{id:?}.png");
        image::save_buffer(
            dir.join(&filename),
            &bytes,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
        textures.push(serde_json::json!({"id": format!("{id:?}"), "file": filename}));
    }
    let meshes: Vec<_> = ctx.tessellate(output.shapes, output.pixels_per_point).into_iter()
        .filter_map(|primitive| match primitive.primitive {
            egui::epaint::Primitive::Mesh(mesh) => Some(serde_json::json!({
                "clip": [primitive.clip_rect.left(), primitive.clip_rect.top(), primitive.clip_rect.right(), primitive.clip_rect.bottom()],
                "texture": format!("{:?}", mesh.texture_id),
                "indices": mesh.indices,
                "vertices": mesh.vertices.iter().map(|v| [v.pos.x, v.pos.y, v.uv.x, v.uv.y,
                    v.color.r() as f32, v.color.g() as f32, v.color.b() as f32, v.color.a() as f32]).collect::<Vec<_>>(),
            })),
            _ => None,
        }).collect();
    let value =
        serde_json::json!({"size": [size.x, size.y], "textures": textures, "meshes": meshes});
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

#[test]
fn serial_and_wave_workbench_previews() {
    let cfg = AppConfig::default();
    let fixture_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-wave-fixture");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let wave_path = fixture_dir.join("PWM-demo.csv");
    let mut csv = String::from("Time(s),CH1(V)\n");
    for i in 0..2000 { csv.push_str(&format!("{},{}\n", i as f64 * 1e-7, if i % 100 < 45 { 5.0 } else { 0.0 })); }
    std::fs::write(&wave_path, csv).unwrap();
    for light in [false, true] {
        for (width, height) in [(1100.0, 720.0), (1600.0, 1000.0)] {
            for page in ["serial", "wave"] {
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let tokens = crate::theme::apply_theme(&ctx, light);
                let mut serial = crate::serial_tool::SerialToolPanel::new(&cfg, Lang::Zh);
                let mut wave = crate::waveform_analysis::WaveformAnalysisPanel::new(&cfg);
                if page == "wave" { wave.open_path(&wave_path, Lang::Zh).unwrap(); }
                let size = Vec2::new(width, height);
                let mut output = egui::FullOutput::default();
                for _ in 0..3 {
                    let next = ctx.run(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() }, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if page == "serial" { serial.ui(ui, Lang::Zh, &tokens); } else { wave.ui(ui, Lang::Zh, &tokens); }
                        });
                    });
                    output.textures_delta.append(next.textures_delta);
                    output.shapes = next.shapes;
                    output.pixels_per_point = next.pixels_per_point;
                }
                let title = if page == "serial" { "串口工具" } else { "波形分析" };
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == title)), "redundant page title");
                if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                    output.textures_delta.set.retain(|(id, _)| *id != egui::TextureId::Managed(0));
                    output.textures_delta.set.push((egui::TextureId::Managed(0), egui::epaint::ImageDelta::full(ctx.fonts(|f| f.image()), egui::TextureOptions::LINEAR)));
                    export_frame(&ctx, output, &std::path::PathBuf::from(dir), &format!("{page}-workbench-{}-{}", width as u32, if light { "light" } else { "dark" }), size);
                }
                serial.on_exit();
            }
        }
    }
}
