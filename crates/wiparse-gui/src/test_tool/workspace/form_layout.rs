//! Declarative form presentation. These helpers never supply business defaults on paint.
use super::*;

pub(super) fn ui_spec<'a>(layout: &'a Value, section: &str, dot: &str) -> &'a Value {
    if let Some(v) = layout[section].get(dot) {
        return v;
    }
    let wildcard = dot
        .split('.')
        .map(|p| if p.parse::<usize>().is_ok() { "*" } else { p })
        .collect::<Vec<_>>()
        .join(".");
    &layout[section][&wildcard]
}

pub(super) fn caption<'a>(
    schema: &'a Value,
    spec: &'a Value,
    fallback: &'a str,
    zh: bool,
) -> &'a str {
    (if zh {
        spec["label_zh"].as_str().or(schema["x-title-zh"].as_str())
    } else {
        None
    })
    .or(spec["label"].as_str())
    .or(schema["title"].as_str())
    .unwrap_or(fallback)
}

pub(super) fn enum_caption(schema: &Value, spec: &Value, value: &Value, zh: bool) -> String {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    (if zh {
        spec["enum_labels_zh"][&raw]
            .as_str()
            .or(schema["x-enum-labels-zh"][&raw].as_str())
    } else {
        None
    })
    .or(spec["enum_labels"][&raw].as_str())
    .or(schema["x-enum-labels"][&raw].as_str())
    .unwrap_or(&raw)
    .to_owned()
}

fn field_dot(pointer: &str, key: &str) -> String {
    format!(
        "{}.{}",
        pointer.trim_start_matches('/').replace('/', "."),
        key
    )
    .trim_start_matches('.')
    .to_owned()
}

pub(super) fn object_fields(
    ui: &mut egui::Ui,
    props: &serde_json::Map<String, Value>,
    schema: &Value,
    docs: &Value,
    layout: &Value,
    value: &mut Value,
    pointer: &str,
    lang: Lang,
    errors: &Value,
    focus: Option<&str>,
    depth: usize,
    grouped: &HashSet<String>,
) {
    let zh = matches!(lang, Lang::Zh);
    let mut keys: Vec<_> = props
        .keys()
        .filter(|k| !grouped.contains(*k))
        .cloned()
        .collect();
    keys.sort_by(|a, b| {
        let order = |k: &str| {
            ui_spec(layout, "fields", &field_dot(pointer, k))["order"]
                .as_i64()
                .unwrap_or(i64::MAX)
        };
        order(a).cmp(&order(b)).then_with(|| a.cmp(b))
    });
    let required = |k: &str| {
        schema["required"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == k))
    };
    let mut common = Vec::new();
    let mut advanced = Vec::new();
    let mut missing = Vec::new();
    for k in keys {
        let dot = field_dot(pointer, &k);
        let spec = ui_spec(layout, "fields", &dot);
        let s = resolved(&props[&k], docs);
        if !visible(layout, &dot, &layout["_root"]) {
            continue;
        }
        if spec["hidden"] == true || s.get("const").is_some() {
            // Hidden values are still validated. Do not silently repair malformed constants.
            if s.get("const")
                .is_some_and(|c| value.get(&k).is_some_and(|v| v != c))
            {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    if zh {
                        "固定参数不合法，请查看校验结果并修复配置"
                    } else {
                        "Invalid constant; review configuration diagnostics"
                    },
                );
            }
            continue;
        }
        if value.get(&k).is_none() {
            missing.push(k);
        } else if spec["advanced"] == true {
            advanced.push(k);
        } else {
            common.push(k);
        }
    }
    let render = |ui: &mut egui::Ui, k: &str, value: &mut Value| {
        let path = format!("{pointer}/{}", k.replace('~', "~0").replace('/', "~1"));
        let s = resolved(&props[k], docs);
        let spec = ui_spec(layout, "fields", &field_dot(pointer, k));
        let complex = s["type"] == "object"
            || s["properties"].is_object()
            || s["oneOf"].is_array()
            || s["anyOf"].is_array();
        if complex {
            ui.label(egui::RichText::new(caption(&s, spec, k, zh)).strong());
        }
        form(
            ui,
            &props[k],
            docs,
            layout,
            &mut value[k],
            &path,
            lang,
            errors,
            focus,
            depth + 1,
        );
        ui.add_space(8.0);
    };
    let compact = |k: &str| {
        let s = resolved(&props[k], docs);
        let spec = ui_spec(layout, "fields", &field_dot(pointer, k));
        matches!(spec["width"].as_str(), Some("short" | "normal"))
            && !matches!(s["type"].as_str(), Some("array" | "object"))
            && !s["oneOf"].is_array()
            && !s["anyOf"].is_array()
    };
    let mut index = 0;
    while index < common.len() {
        if ui.available_width() >= 560.0
            && index + 1 < common.len()
            && compact(&common[index])
            && compact(&common[index + 1])
        {
            ui.columns(2, |columns| {
                render(&mut columns[0], &common[index], value);
                render(&mut columns[1], &common[index + 1], value);
            });
            index += 2;
        } else {
            render(ui, &common[index], value);
            index += 1;
        }
    }
    if missing.iter().any(|k| required(k)) {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            if zh {
                "缺少必需字段，请在高级设置中添加后保存"
            } else {
                "Required fields are missing; add them in Advanced settings before saving"
            },
        );
    }
    if !advanced.is_empty() || !missing.is_empty() {
        let open = focus.is_some_and(|p| {
            advanced
                .iter()
                .any(|k| p == format!("{pointer}/{k}") || p.starts_with(&format!("{pointer}/{k}/")))
        });
        egui::CollapsingHeader::new(if zh {
            "高级设置"
        } else {
            "Advanced settings"
        })
        .id_salt((pointer, "advanced"))
        .open(if open { Some(true) } else { None })
        .show(ui, |ui| {
            for k in &advanced {
                render(ui, k, value);
            }
            if !missing.is_empty() {
                ui.menu_button(
                    if zh {
                        "添加字段…"
                    } else {
                        "Add field…"
                    },
                    |ui| {
                        for k in &missing {
                            let s = resolved(&props[k], docs);
                            let spec = ui_spec(layout, "fields", &field_dot(pointer, k));
                            let label = caption(&s, spec, k, zh);
                            let label = if required(k) {
                                format!("{label} {}", if zh { "（必需）" } else { "(required)" })
                            } else {
                                label.to_owned()
                            };
                            if ui.button(label).clicked() {
                                value[k] = field_defaults(&props[k], docs, depth + 1);
                                ui.close_menu();
                            }
                        }
                    },
                );
            }
            let removable: Vec<_> = common
                .iter()
                .chain(advanced.iter())
                .filter(|k| !required(k))
                .collect();
            if !removable.is_empty() {
                ui.menu_button(
                    if zh {
                        "移除可选字段…"
                    } else {
                        "Remove optional field…"
                    },
                    |ui| {
                        for k in removable {
                            let s = resolved(&props[k], docs);
                            let spec = ui_spec(layout, "fields", &field_dot(pointer, k));
                            if ui.button(caption(&s, spec, k, zh)).clicked() {
                                value.as_object_mut().unwrap().remove(k);
                                ui.close_menu();
                            }
                        }
                    },
                );
            }
        });
    }
}

pub(super) fn item_summary(
    item: &Value,
    item_schema: &Value,
    docs: &Value,
    widget: &Value,
    layout: &Value,
    pointer: &str,
    zh: bool,
    index: usize,
) -> String {
    let s = resolved(item_schema, docs);
    let branch = s["oneOf"]
        .as_array()
        .or_else(|| s["anyOf"].as_array())
        .and_then(|variants| {
            variants.iter().find(|s| {
                s["properties"].as_object().is_some_and(|props| {
                    props
                        .iter()
                        .all(|(k, v)| v.get("const").is_none_or(|c| item.get(k) == Some(c)))
                })
            })
        });
    let name = item["name"]
        .as_str()
        .or(item["label"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| localized_title(branch.unwrap_or(&s), zh));
    let mut parts = vec![if name.is_empty() {
        format!("{} {}", if zh { "项目" } else { "Item" }, index + 1)
    } else {
        name.to_owned()
    }];
    for field in widget["summary_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if field == "type" && branch.is_some() {
            continue;
        }
        if let Some(v) = item.pointer(&format!("/{}", field.replace('.', "/"))) {
            if v.is_string() || v.is_number() || v.is_boolean() {
                let field_schema = branch
                    .and_then(|b| b["properties"].get(field))
                    .unwrap_or(&Value::Null);
                let spec = ui_spec(
                    layout,
                    "fields",
                    &format!(
                        "{}.{}.{}",
                        pointer.trim_start_matches('/').replace('/', "."),
                        index,
                        field
                    ),
                );
                let label = caption(field_schema, spec, field, zh);
                let display = enum_caption(field_schema, spec, v, zh);
                parts.push(if field == "unit" {
                    display
                } else {
                    format!("{label} {display}")
                });
            }
        }
    }
    if item["required"] == true {
        parts.push(if zh { "必需" } else { "Required" }.into());
    }
    let result = parts.join(" · ");
    if result.chars().count() > 100 {
        result.chars().take(97).chain("…".chars()).collect()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_metadata_and_paint_preserve_invalid_and_absent_values() {
        let layout = json!({"fields":{"rules.*.count":{"width":"short","label_zh":"次数"},"rules.0.count":{"label_zh":"精确次数"}},"_root":{}});
        assert_eq!(
            ui_spec(&layout, "fields", "rules.0.count")["label_zh"],
            "精确次数"
        );
        assert_eq!(
            ui_spec(&layout, "fields", "rules.2.count")["label_zh"],
            "次数"
        );
        let schema = json!({"type":"object","properties":{"constant":{"const":"fixed"},"missing":{"type":"string","default":"must-not-appear"},"required":{"type":"string"},"present":{"type":"string","title":"Present"}},"required":["required"]});
        let mut value = json!({"constant":"invalid","present":"kept"});
        let before = value.clone();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                form(
                    ui,
                    &schema,
                    &json!({}),
                    &layout,
                    &mut value,
                    "",
                    Lang::En,
                    &json!([]),
                    None,
                    0,
                )
            });
        });
        assert_eq!(
            value, before,
            "Painting must not repair constants or insert missing defaults"
        );
    }

    #[test]
    fn rule_summary_uses_business_type_and_bounds_without_raw_discriminator() {
        let schema = json!({"oneOf":[{"title":"Range","x-title-zh":"数值范围","properties":{"type":{"const":"numeric_range"}}}]});
        let item =
            json!({"type":"numeric_range","minimum":1,"maximum":2,"unit":"Hz","required":true});
        let summary = item_summary(
            &item,
            &schema,
            &json!({}),
            &json!({"summary_fields":["type","minimum","maximum","unit"]}),
            &json!({}),
            "/rules",
            true,
            0,
        );
        assert_eq!(summary, "数值范围 · minimum 1 · maximum 2 · Hz · 必需");
        assert!(!summary.contains("numeric_range"));
    }

    #[test]
    #[ignore = "requires the current real plugin UI fixture"]
    fn real_rule_editor_creates_and_switches_types_with_pointer_events() {
        let fixture: Value = serde_json::from_slice(
            &fs::read(std::env::var("WIPARSE_FPGA_UI_FIXTURE").unwrap()).unwrap(),
        )
        .unwrap();
        let schema = schema_at(
            &fixture["workspace"]["schema"],
            &fixture["workspace"]["documents"],
            "verdict.rules",
        )
        .unwrap();
        let docs = &fixture["workspace"]["documents"];
        let mut layout = fixture["workspace"]["ui"].clone();
        layout["_root"] = fixture["record"]["value"].clone();
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let mut value = json!([]);
        fn frame(
            ctx: &egui::Context,
            schema: &Value,
            docs: &Value,
            layout: &Value,
            value: &mut Value,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 1400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        form(
                            ui,
                            schema,
                            docs,
                            layout,
                            value,
                            "/verdict/rules",
                            Lang::Zh,
                            &json!([]),
                            None,
                            0,
                        )
                    });
                },
            )
        }
        fn position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
            output
                .shapes
                .iter()
                .find_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        (t.galley.job.text == label).then_some(t.pos + t.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Missing control: {label}"))
        }
        let click = |value: &mut Value, point: egui::Pos2| {
            for pressed in [true, false] {
                let _ = frame(
                    &ctx,
                    &schema,
                    docs,
                    &layout,
                    value,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        let output = frame(&ctx, &schema, docs, &layout, &mut value, vec![]);
        click(&mut value, position(&output, "+ 添加"));
        assert_eq!(value.as_array().unwrap().len(), 1);
        assert_eq!(value[0]["type"], "log_contains");
        let original_id = value[0]["id"].clone();
        let item_schema = resolved(&schema["items"], docs);
        let branch_title = |kind: &str| {
            localized_title(
                item_schema["oneOf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|b| b["properties"]["type"]["const"] == kind)
                    .unwrap(),
                true,
            )
            .to_owned()
        };
        for (old_kind, kind) in [
            ("log_contains", "waveform_metric_range"),
            ("waveform_metric_range", "all"),
        ] {
            let current = branch_title(old_kind);
            let target = branch_title(kind);
            let output = frame(&ctx, &schema, docs, &layout, &mut value, vec![]);
            click(&mut value, position(&output, &current));
            let output = frame(&ctx, &schema, docs, &layout, &mut value, vec![]);
            click(&mut value, position(&output, &target));
            assert_eq!(value[0]["type"], kind);
            assert_eq!(value[0]["id"], original_id);
            assert!(
                value[0].get("text").is_none(),
                "Inapplicable branch field remains"
            );
        }
        let before = value[0]["rules"].as_array().unwrap().len();
        let output = frame(&ctx, &schema, docs, &layout, &mut value, vec![]);
        click(&mut value, position(&output, "+ 添加"));
        assert_eq!(
            value[0]["rules"].as_array().unwrap().len(),
            before + 1,
            "Nested combination must support typed child creation"
        );
    }
}
