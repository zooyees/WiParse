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

/// Rendering fixture only: no hardware operations or functional regression suite.
#[test]
#[ignore = "visual capture on request"]
fn compact_page_style_previews(){
    let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/compact-ui-20261109");std::fs::create_dir_all(&root).unwrap();
    let mut cfg=AppConfig::default();cfg.log_monitor.save_live_to_disk=false;cfg.log_monitor.open_log_files.clear();cfg.apps.test_tool.data_root=root.display().to_string();cfg.apps.test_tool.plugins_dir=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-tools/plugins").display().to_string();
    let mut calculator=crate::calculator::CalculatorPanel::new();let mut serial=crate::serial_tool::SerialToolPanel::new(&cfg,Lang::Zh);let mut data=crate::data_analysis::DataAnalysisPanel::new(&cfg);let mut report=crate::test_report::TestReportPanel::new(&cfg);let mut hub=crate::test_tool::TestToolPanel::new(&cfg);hub.prepare_layout_audit();
    for page in ["calculator","serial","data","report","testing","scope","dc"]{
        let ctx=egui::Context::default();crate::fonts::install_cjk_fonts(&ctx);let tokens=crate::theme::apply_theme(&ctx,false);let mut instrument=crate::instrument_control::InstrumentControlPanel::new(&cfg);
        if matches!(page,"scope"|"dc"){instrument.prepare_compact_ui_preview(&ctx,if page=="scope"{wiparse_core::instrument::InstrumentKind::Oscilloscope}else{wiparse_core::instrument::InstrumentKind::DcSource});}
        let size=Vec2::new(1600.,1000.);let mut output=egui::FullOutput::default();
        for _ in 0..3{output=ctx.run(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),..Default::default()},|ctx|{egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tokens.canvas_bg).inner_margin(4)).show(ctx,|ui|{match page{
            "calculator"=>calculator.ui(ui,Lang::Zh,&tokens),"serial"=>serial.ui(ui,Lang::Zh,&tokens),"data"=>data.ui(ui,Lang::Zh,&tokens),"report"=>{report.ui(ui,Lang::Zh,&tokens,&root,&[],None,"default",0,None);},"testing"=>hub.ui(ui,Lang::Zh,&tokens,&[]),_=>instrument.ui(ui,Lang::Zh,&tokens),
        }});});}
        output.textures_delta.set.retain(|(id,_)|*id!=egui::TextureId::Managed(0));output.textures_delta.set.push((egui::TextureId::Managed(0),egui::epaint::ImageDelta::full(ctx.fonts(|fonts|fonts.image()),egui::TextureOptions::LINEAR)));export_frame(&ctx,output,&root,page,size);
    }
}

#[test]#[ignore="style gallery on request"]fn light_and_sidebar_style_gallery(){
 let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ui-light-review-20261110/gallery");std::fs::create_dir_all(&root).unwrap();let mut cfg=AppConfig::default();cfg.log_monitor.save_live_to_disk=false;cfg.log_monitor.open_log_files.clear();cfg.apps.test_tool.data_root=root.display().to_string();cfg.apps.test_tool.plugins_dir=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-tools/plugins").display().to_string();
 let report_file=root.join("visual-report.md");std::fs::write(&report_file,"# 适配器兼容性测试\n\n## 测试结果\n\n**测试夹具：不代表真实硬件结论**\n\n| 检查项 | 结果 |\n| --- | --- |\n| 输入电压 | 5.02 V |\n| 通信质量 | 需检查超时 |\n\n### 后续检查\n\n1. 核对原始日志与波形。\n2. 复测超时阶段。\n").unwrap();
 for light in [false,true]{for width in [640.,1100.,1600.]{for page in ["data","report","calculator","testing","serial"]{
 let ctx=egui::Context::default();crate::fonts::install_cjk_fonts(&ctx);let t=crate::theme::apply_theme(&ctx,light);let mut data=crate::data_analysis::DataAnalysisPanel::new(&cfg);data.prepare_style_audit();let mut report=crate::test_report::TestReportPanel::new(&cfg);report.open_markdown(report_file.clone(),Lang::Zh);let mut calc=crate::calculator::CalculatorPanel::new();let mut hub=crate::test_tool::TestToolPanel::new(&cfg);hub.prepare_layout_audit();let mut serial=crate::serial_tool::SerialToolPanel::new(&cfg,Lang::Zh);let size=Vec2::new(width,940.);let mut output=egui::FullOutput::default();for _ in 0..3{output=ctx.run(RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),..Default::default()},|ctx|{egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas_bg).inner_margin(4)).show(ctx,|ui|match page{"data"=>data.ui(ui,Lang::Zh,&t),"report"=>{report.ui(ui,Lang::Zh,&t,&root,&[],None,"default",0,None);},"calculator"=>calc.ui(ui,Lang::Zh,&t),"testing"=>hub.ui(ui,Lang::Zh,&t,&[]),_=>serial.ui(ui,Lang::Zh,&t)});});}output.textures_delta.set.retain(|(id,_)|*id!=egui::TextureId::Managed(0));output.textures_delta.set.push((egui::TextureId::Managed(0),egui::epaint::ImageDelta::full(ctx.fonts(|f|f.image()),egui::TextureOptions::LINEAR)));export_frame(&ctx,output,&root,&format!("{page}-{width:.0}-{}",if light{"light"}else{"dark"}),size);serial.on_exit();
 }}}
}
