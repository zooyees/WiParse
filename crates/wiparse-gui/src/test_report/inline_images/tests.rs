use super::*;

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "wiparse-inline-images-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let shot = image::RgbImage::from_fn(320, 180, |x, y| {
        if y.abs_diff(90 + ((x / 20) % 2) * 25) < 3 {
            image::Rgb([30, 220, 100])
        } else if x % 40 == 0 || y % 30 == 0 {
            image::Rgb([45, 50, 60])
        } else {
            image::Rgb([12, 16, 22])
        }
    });
    shot.save(root.join("shot.png")).unwrap();
    fs::write(root.join("wave.isf"), b"fixture").unwrap();
    root
}

#[test]
fn report_inline_images_upgrade_legacy_links_without_changing_wave_or_code() {
    let source = "# Report\n\n| Item | Evidence |\n|---|---|\n| Shot | [截图](../shots/shot%20%281%29.png) |\n| Wave | [源文件](../waves/wave.isf) |\n\nSee [second](second.jpg).\n\n`[example](code.png)`\n\n~~~\n[log](never.png)\n~~~\n";
    let doc = parse_report_md(source);
    let images: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|block| match block {
            MdBlock::Image { src, .. } => Some(src.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(images, vec!["../shots/shot%20%281%29.png", "second.jpg"]);
    let table = doc
        .blocks
        .iter()
        .find_map(|block| match block {
            MdBlock::Table { rows } => Some(rows),
            _ => None,
        })
        .unwrap();
    assert_eq!(table[1][1], "截图");
    assert_eq!(table[2][1], "[源文件](../waves/wave.isf)");
    assert!(doc
        .blocks
        .iter()
        .any(|block| matches!(block, MdBlock::Code { body, .. } if body.contains("never.png"))));
    assert!(doc.blocks.iter().any(|block| matches!(block, MdBlock::Paragraph { text } if text.contains("`[example](code.png)`"))));
    // AI text rendering retains its independent, file-free Markdown path.
    assert!(!parse_md("[shot](shot.png)")
        .blocks
        .iter()
        .any(|block| matches!(block, MdBlock::Image { .. })));
}

#[test]
fn report_inline_images_prefer_explicit_images_and_keep_security_jail() {
    let doc = parse_report_md("[old](./shot.png)\n\n![截图](shot.png)\n\n[other](shot.png)\n\n[web](https://example.com/remote.png)");
    assert_eq!(
        doc.blocks
            .iter()
            .filter(|block| matches!(block, MdBlock::Image { .. }))
            .count(),
        1
    );
    let root = fixture();
    assert!(resolve_md_asset(Some(&root), "../outside.png").is_none());
    assert!(resolve_md_asset(Some(&root), "https://example.com/remote.png").is_none());
    let missing = parse_report_md("[截图](missing.png)");
    assert!(matches!(&missing.blocks[1], MdBlock::Image { src, .. } if src=="missing.png"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn report_inline_images_render_real_texture_and_click_does_not_navigate() {
    let root = fixture();
    for (width, light) in [(640.0, false), (1100.0, true), (1920.0, false)] {
        for old in [false, true] {
            let doc = parse_report_md(if old {
                "# 测试报告\n\n| 项目 | 数据 |\n|---|---|\n| 截图 | [shot.png](shot.png) |\n| 波形 | [源文件](wave.isf) |"
            } else {
                "# 测试报告\n\n## 波形截图\n\n![波形截图](shot.png)\n\n[波形源文件](wave.isf)"
            });
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let tokens = crate::theme::apply_theme(&ctx, light);
            let mut textures = HashMap::new();
            let mut open = None;
            let mut tick = 0.0;
            let mut frame = |events| {
                tick += 0.1;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 720.0),
                        )),
                        time: Some(tick),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            paint_md_doc(
                                ui,
                                &doc,
                                &tokens,
                                Some(&root),
                                &mut textures,
                                &mut open,
                                "",
                            );
                        });
                    },
                )
            };
            let output = frame(vec![]);
            let mesh = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
            let image_rect = mesh
                .iter()
                .filter_map(|primitive| match &primitive.primitive {
                    egui::epaint::Primitive::Mesh(mesh)
                        if mesh.texture_id != egui::TextureId::Managed(0) =>
                    {
                        Some(mesh.vertices.iter().fold(egui::Rect::NOTHING, |rect, v| {
                            rect.union(egui::Rect::from_min_max(v.pos, v.pos))
                        }))
                    }
                    _ => None,
                })
                .next()
                .expect("screenshot texture must be drawn inline, not just a link label");
            assert!(image_rect.is_positive());
            let point = image_rect.center();
            frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let mut settled = frame(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            drop(frame);
            settled.textures_delta.append(output.textures_delta);
            assert!(
                open.is_none(),
                "clicking the inline screenshot must not leave the report"
            );
            assert_eq!(textures.len(), 1);
            if let Ok(out) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                settled.textures_delta.set.retain(|(id, _)| *id != egui::TextureId::Managed(0));
                settled.textures_delta.set.push((egui::TextureId::Managed(0), egui::epaint::ImageDelta::full(ctx.fonts(|fonts| fonts.image()), egui::TextureOptions::LINEAR)));
                crate::ui_audit::export_frame(
                    &ctx,
                    settled,
                    &PathBuf::from(out),
                    &format!(
                        "inline-image-{}-{width:.0}-{}",
                        if old { "legacy" } else { "new" },
                        if light { "light" } else { "dark" }
                    ),
                    egui::vec2(width, 720.0),
                );
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn report_inline_images_long_reports_do_not_decode_every_screenshot_at_once() {
    let root = fixture();
    let mut text = String::new();
    for index in 0..40 {
        fs::copy(
            root.join("shot.png"),
            root.join(format!("shot-{index}.png")),
        )
        .unwrap();
        text.push_str(&format!("![shot {index}](shot-{index}.png)\n\n"));
    }
    let ctx = egui::Context::default();
    let doc = parse_report_md(&text);
    let mut textures = HashMap::new();
    let mut open = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 360.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    paint_md_doc(
                        ui,
                        &doc,
                        &Tokens::dark(),
                        Some(&root),
                        &mut textures,
                        &mut open,
                        "",
                    );
                });
            });
        },
    );
    assert!(!textures.is_empty());
    assert!(
        textures.len() < 5,
        "off-screen screenshots should not be decoded"
    );
    for index in 3..25 {
        let path = root
            .join(format!("shot-{index}.png"))
            .canonicalize()
            .unwrap();
        textures.insert(path.clone(), load_md_texture(&ctx, &path));
    }
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 360.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    paint_md_doc(
                        ui,
                        &doc,
                        &Tokens::dark(),
                        Some(&root),
                        &mut textures,
                        &mut open,
                        "",
                    );
                });
            });
        },
    );
    assert!(
        textures.len() <= 16,
        "off-screen textures must not grow without bound"
    );
    assert!(textures.contains_key(&root.join("shot-0.png").canonicalize().unwrap()));
    fs::remove_dir_all(root).unwrap();
}
