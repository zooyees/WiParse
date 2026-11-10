use super::*;

fn fixture(count: usize) -> TestToolPanel {
    let mut p = super::super::tests::fixture(count);
    let previous = std::mem::take(&mut p.workspace);
    p.select_plugin("fpga-firmware-test".into());
    p.workspace = previous;
    let plugin = p.selected_plugin().unwrap().clone();
    p.workspace.key = format!(
        "{}:{}:{}",
        plugin.id,
        p.effective_project_id(),
        p.hub_report_name(&plugin)
    );
    p.workspace.parameter_fingerprint = serde_json::to_string(
        &p.param_values
            .iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
    )
    .unwrap();
    let schema: Value = serde_json::from_slice(
        &fs::read(plugin.dir.join("schemas/fpga-test-case.schema.json")).unwrap(),
    )
    .unwrap();
    let layout: Value = serde_json::from_slice(
        &fs::read(plugin.dir.join("schemas/fpga-test-case.ui.json")).unwrap(),
    )
    .unwrap();
    p.workspace.snapshot["schema"] = schema.clone();
    p.workspace.snapshot["documents"] = json!({"":schema});
    p.workspace.snapshot["ui"] = layout;
    p.workspace.snapshot["project_files"] = json!({"active":"fpga-demo","items":[{"id":"fpga-demo","name":"FPGA 示例工程 / Demo project"}]});
    p.workspace.snapshot["project_config"] = json!({"saved":true});
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../recipes/fpga-firmware-test/cases");
    let mut files: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let cases: Vec<Value> = files
        .iter()
        .map(|p| serde_json::from_slice(&fs::read(p).unwrap()).unwrap())
        .collect();
    assert_eq!(cases.len(), 20);
    let rows:Vec<Value>=(0..count).map(|n| {
        let case=&cases[n%cases.len()];
        let id=if n<20 {case["id"].as_str().unwrap().to_owned()}else{format!("case_{n:03}")};
        json!({"id":id,"name":case["name"],"revision":2,"enabled":true,"order":n,"tags":case["tags"],"status":"valid","sha256":"abc"})
    }).collect();
    let matrix:Vec<Value>=rows.iter().map(|r|json!({"id":r["id"],"name":r["name"],"selected":true,"status":"pending","config_sha256":"abc"})).collect();
    p.workspace.snapshot["items"] = json!(rows);
    p.workspace.snapshot["matrix"]["items"] = json!(matrix);
    p.workspace.selected = rows
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect();
    if count > 0 {
        p.workspace.current = rows[0]["id"].as_str().unwrap().to_owned();
        p.workspace.document = cases[0].clone();
        p.workspace.original = p.workspace.document.clone();
        p.workspace.last_form_value = p.workspace.document.clone();
    }
    p
}

fn render(
    p: &mut TestToolPanel,
    ctx: &egui::Context,
    width: f32,
    lang: Lang,
    light: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let tokens = ui_theme::apply_theme(ctx, light);
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| p.paint_workspace(ui, lang, &tokens));
        },
    )
}

fn text_shapes(output: &egui::FullOutput) -> Vec<(&str, egui::Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text(),
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn fpga_chrome_counts_widths_themes_and_languages() {
    for count in [5, 20, 500] {
        for lang in [Lang::Zh, Lang::En] {
            for light in [false, true] {
                for width in [640.0, 1100.0, 1440.0, 1920.0] {
                    let mut p = fixture(count);
                    let ctx = egui::Context::default();
                    crate::fonts::install_cjk_fonts(&ctx);
                    let value = p.workspace.document.clone();
                    let output = render(&mut p, &ctx, width, lang, light, vec![]);
                    assert_eq!(p.workspace.snapshot["total"], count);
                    assert_eq!(
                        p.workspace.snapshot["items"].as_array().unwrap().len(),
                        count
                    );
                    assert_eq!(
                        p.workspace.document, value,
                        "paint must not edit configurations"
                    );
                    assert!(
                        p.workspace.rx.is_none(),
                        "unchanged paint must not launch IO"
                    );
                    let labels = text_shapes(&output);
                    let project_label = "FPGA 示例工程 / Demo project";
                    assert_eq!(
                        labels
                            .iter()
                            .filter(|(label, _)| *label == project_label)
                            .count(),
                        1,
                        "project name duplicated"
                    );
                    assert!(!labels
                        .iter()
                        .any(|(label, _)| *label == "Batch tags, comma-separated"));
                    let cards = ctx.data_mut(|d| {
                        d.get_temp::<Vec<(egui::Rect, egui::Rect, egui::Rect)>>(egui::Id::new(
                            "config_card_audit",
                        ))
                    });
                    if width >= 1440.0 {
                        let cards = cards.unwrap();
                        assert!(!cards.is_empty());
                        assert!(
                            cards[0].0.top() < 800.0,
                            "cards must be visible below the command band"
                        );
                        assert!(cards.len() < 25, "list must stay virtualized");
                        for (row, check, edit) in &cards {
                            assert!(
                                row.top() < 1020.0,
                                "only the viewport and its overscan rows should be rendered"
                            );
                            assert!(
                                (row.width() - cards[0].0.width()).abs() < 0.01,
                                "card widths differ"
                            );
                            assert!(!check.intersects(*edit));
                            assert!(row.contains_rect(*check) && row.contains_rect(*edit));
                            assert!(row.right() <= width, "card overflow");
                        }
                        for pair in cards.windows(2) {
                            assert!(
                                ((pair[1].0.top() - pair[0].0.top()) - (CONFIG_ROW_H + crate::theme::SPACE_SM)).abs()
                                    < 0.1,
                                "rendered row pitch must match virtual scrolling: {:?} {:?}",pair[0].0,pair[1].0
                            );
                        }
                    }
                    for (label, rect) in &labels {
                        if matches!(
                            *label,
                            "Execution" | "执行控制" | "▶ Start test" | "▶ 启动测试"
                        ) {
                            assert!(rect.bottom() < 180.0, "command band consumed the page");
                            assert!(
                                rect.right() <= width + 1.0,
                                "command overflow {width}: {label}"
                            );
                        }
                    }
                    if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                        if count == 20
                            || (count == 500 && width == 1440.0)
                            || (count == 5 && width == 1440.0)
                        {
                            let name = format!(
                                "fpga-chrome-{count}-{}-{}-{width}",
                                if matches!(lang, Lang::Zh) { "zh" } else { "en" },
                                if light { "light" } else { "dark" }
                            );
                            crate::ui_audit::export_frame(
                                &ctx,
                                output,
                                Path::new(&dir),
                                &name,
                                egui::vec2(width, 900.0),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fpga_chrome_pointer_checkbox_and_editor_are_independent() {
    let mut p = fixture(20);
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    p.workspace.selected.clear();
    // Dirty state retains a navigation intent instead of starting a config worker.
    p.workspace.document["name"] = json!("Edited name");
    let _ = render(&mut p, &ctx, 1440.0, Lang::En, false, vec![]);
    let cards = ctx
        .data_mut(|d| {
            d.get_temp::<Vec<(egui::Rect, egui::Rect, egui::Rect)>>(egui::Id::new(
                "config_card_audit",
            ))
        })
        .unwrap();
    let id = p.workspace.snapshot["items"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let current = p.workspace.current.clone();
    for (pos, check) in [(cards[1].1.center(), true), (cards[1].2.center(), false)] {
        for pressed in [true, false] {
            let _ = render(
                &mut p,
                &ctx,
                1440.0,
                Lang::En,
                false,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert_eq!(p.workspace.selected.len(), 1);
        assert!(p.workspace.selected.contains(&id));
        assert_eq!(p.workspace.current, current);
        if check {
            assert!(p.workspace.pending.is_none());
        } else {
            assert_eq!(
                p.workspace.pending.as_ref().unwrap(),
                &("config.get".to_owned(), json!({"config_id":id}))
            );
        }
    }
    assert!(p.workspace.rx.is_none());
}

#[test]
fn fpga_chrome_pause_resume_slot_and_dirty_execution_gate() {
    let mut p = fixture(5);
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    for (state, shown, hidden) in [
        ("running", "Ⅱ Pause", "▶ Continue"),
        ("paused", "▶ Continue", "Ⅱ Pause"),
        ("pause_requested", "▶ Continue", "Ⅱ Pause"),
    ] {
        p.workspace.snapshot["matrix"]["control"] = json!({"state":state});
        let output = render(&mut p, &ctx, 1440.0, Lang::En, false, vec![]);
        let labels = text_shapes(&output);
        assert!(labels.iter().any(|(label, _)| *label == shown));
        assert!(!labels.iter().any(|(label, _)| *label == hidden));
    }
    p.workspace.document["name"] = json!("Unsaved edit");
    assert!(p.workspace_run_gate(false).is_err());
}
