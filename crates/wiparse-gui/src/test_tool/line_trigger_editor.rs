//! Visual presentation of the existing line-trigger JSON contract.
//! Values are only serialized after an explicit edit; unknown fields survive.
use crate::theme::Tokens;
use egui::RichText;
use serde_json::{json, Value};
use wiparse_core::i18n::Lang;

fn text<'a>(lang: Lang, zh: &'a str, en: &'a str) -> &'a str {
    if matches!(lang, Lang::Zh) {
        zh
    } else {
        en
    }
}

fn document(raw: &str) -> Result<Value, ()> {
    let value: Value = if raw.trim().is_empty() {
        json!([])
    } else {
        serde_json::from_str(raw).map_err(|_| ())?
    };
    if value.is_array() || value.get("items").is_some_and(Value::is_array) {
        Ok(value)
    } else {
        Err(())
    }
}

fn items_mut(value: &mut Value) -> &mut Vec<Value> {
    if value.is_array() {
        value.as_array_mut().unwrap()
    } else {
        value["items"].as_array_mut().unwrap()
    }
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn pattern_key(value: &Value) -> &'static str {
    if value.get("pattern").is_some_and(|v| !v.is_null()) {
        "pattern"
    } else if value.get("regex").is_some_and(|v| !v.is_null()) {
        "regex"
    } else if value.get("text").is_some_and(|v| !v.is_null()) {
        "text"
    } else {
        "pattern"
    }
}

fn pattern(value: &Value) -> String {
    string(value, pattern_key(value))
}
fn mode(value: &Value) -> String {
    value
        .get("type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("regex")
        .to_ascii_lowercase()
}
fn title(value: &Value, index: usize) -> String {
    let label = string(value, "label");
    if !label.is_empty() {
        label
    } else {
        let id = string(value, "id");
        if id.is_empty() {
            format!("T{}", index + 1)
        } else {
            id
        }
    }
}

/// Same ASCII slug and fallback used by sanitizeTriggerId in line-triggers.mjs.
fn effective_id(value: &Value, index: usize) -> String {
    let id = string(value, "id");
    let fallback = format!("T{}", index + 1);
    let source = if id.is_empty() { &fallback } else { id.trim() };
    let mut out = String::new();
    let mut invalid = false;
    for ch in source.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
            out.push(ch);
            invalid = false;
        } else if !invalid {
            out.push('_');
            invalid = true;
        }
    }
    out
}

fn predicate_valid(value: &Value) -> bool {
    value.is_object()
        && !pattern(value).is_empty()
        && matches!(mode(value).as_str(), "contains" | "regex")
}

pub(super) fn validate(raw: &str, lang: Lang) -> Result<(), String> {
    let mut doc = document(raw).map_err(|_| {
        text(
            lang,
            "触发条件格式无法读取，请展开“配置修复”检查原配置。",
            "Cannot read triggers. Open Configuration recovery to inspect the original value.",
        )
        .to_owned()
    })?;
    let mut ids = std::collections::HashSet::new();
    for (index, item) in items_mut(&mut doc).iter().enumerate() {
        if item.get("enabled") == Some(&Value::Bool(false)) {
            continue;
        }
        let id = effective_id(item, index);
        if !predicate_valid(item) || id.is_empty() {
            return Err(format!(
                "{} {}",
                title(item, index),
                text(
                    lang,
                    "：请填写匹配内容并选择有效的匹配方式。",
                    ": enter match text and a valid match mode."
                )
            ));
        }
        if !ids.insert(id) {
            return Err(text(
                lang,
                "已启用规则的标识重复，请在高级选项中修改。",
                "Enabled rules have duplicate IDs. Change the ID in Advanced options.",
            )
            .to_owned());
        }
        for key in ["unless", "except", "exclude"] {
            let Some(raw) = item.get(key) else {
                continue;
            };
            if key != "exclude" && !raw.is_null() && !raw.is_array() {
                return Err(text(
                    lang,
                    "排除条件列表格式无效，请通过配置修复检查原配置。",
                    "Invalid exclusion list. Inspect the original configuration in recovery.",
                )
                .to_owned());
            }
            let entries: Vec<&Value> = if let Some(a) = raw.as_array() {
                a.iter().collect()
            } else if key == "exclude" {
                vec![raw]
            } else {
                continue;
            };
            for entry in entries {
                if entry.is_null() || entry.as_str() == Some("") {
                    continue;
                }
                if !entry.is_string() && !predicate_valid(entry) {
                    return Err(text(
                        lang,
                        "请补全排除条件的匹配内容。",
                        "Complete the exclusion match text.",
                    )
                    .to_owned());
                }
            }
        }
    }
    if ids.is_empty() {
        return Err(text(
            lang,
            "请添加并启用至少一条触发规则。",
            "Add and enable at least one trigger rule.",
        )
        .to_owned());
    }
    Ok(())
}

fn edit_string(ui: &mut egui::Ui, value: &mut Value, key: &str, hint: &str) -> bool {
    let mut current = string(value, key);
    let changed = ui
        .add(
            egui::TextEdit::singleline(&mut current)
                .desired_width(ui.available_width())
                .hint_text(hint),
        )
        .changed();
    if changed {
        value[key] = Value::String(current);
    }
    changed
}

fn predicate_ui(ui: &mut egui::Ui, lang: Lang, value: &mut Value) -> bool {
    let mut changed = false;
    let mut selected = mode(value);
    ui.vertical(|ui| {
        ui.label(text(lang, "匹配方式", "Match mode"));
        egui::ComboBox::from_id_salt("mode")
            .width(ui.available_width().min(240.0))
            .truncate()
            .selected_text(match selected.as_str() {
                "contains" => text(lang, "包含关键词", "Contains keywords"),
                "regex" => text(lang, "正则表达式", "Regular expression"),
                _ => selected.as_str(),
            })
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(
                        &mut selected,
                        "contains".into(),
                        text(lang, "包含关键词", "Contains keywords"),
                    )
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut selected,
                        "regex".into(),
                        text(lang, "正则表达式", "Regular expression"),
                    )
                    .changed();
            });
    });
    if changed {
        value["type"] = Value::String(selected.clone());
    }
    let key = pattern_key(value);
    changed |= edit_string(
        ui,
        value,
        key,
        if selected == "contains" {
            text(
                lang,
                "例如：timeout 或 ERROR",
                "For example: timeout or ERROR",
            )
        } else {
            text(lang, "输入正则表达式", "Enter a regular expression")
        },
    );
    if selected == "contains" {
        let mut sensitive = value.get("case_sensitive") == Some(&Value::Bool(true));
        if ui
            .checkbox(&mut sensitive, text(lang, "区分大小写", "Case sensitive"))
            .changed()
        {
            value["case_sensitive"] = json!(sensitive);
            changed = true;
        }
    }
    changed
}

fn exclusion_ui(ui: &mut egui::Ui, lang: Lang, item: &mut Value) -> bool {
    let mut changed = false;
    for key in ["unless", "except", "exclude"] {
        let Some(value) = item.get_mut(key) else {
            continue;
        };
        if value.is_null() || value.as_str() == Some("") {
            continue;
        }
        ui.push_id(key, |ui| {
            if let Some(entries) = value.as_array_mut() {
                let mut remove = None;
                for (i, entry) in entries.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} {}", text(lang, "排除", "Exclude"), i + 1));
                            if ui.small_button(text(lang, "移除", "Remove")).clicked() {
                                remove = Some(i);
                            }
                        });
                        changed |= exclusion_entry(ui, lang, entry);
                    });
                }
                if let Some(i) = remove {
                    entries.remove(i);
                    changed = true;
                }
            } else {
                let mut remove = false;
                ui.horizontal(|ui| {
                    ui.label(text(lang, "排除", "Exclude"));
                    remove = ui.small_button(text(lang, "移除", "Remove")).clicked();
                });
                changed |= exclusion_entry(ui, lang, value);
                if remove {
                    *value = Value::Null;
                    changed = true;
                }
            }
        });
    }
    if ui
        .small_button(text(lang, "+ 排除条件", "+ Exclusion"))
        .clicked()
    {
        if item.get("unless").is_none_or(Value::is_null) {
            item["unless"] = json!([]);
        }
        if let Some(entries) = item["unless"].as_array_mut() {
            entries.push(json!({"type":"contains", "pattern":""}));
            changed = true;
        }
    }
    changed
}

fn exclusion_entry(ui: &mut egui::Ui, lang: Lang, entry: &mut Value) -> bool {
    if let Some(raw) = entry.as_str() {
        let mut raw = raw.to_owned();
        ui.small(text(
            lang,
            "正则排除（沿用排除标志）",
            "Regex exclusion (inherits exclusion flags)",
        ));
        if ui
            .add(egui::TextEdit::singleline(&mut raw).desired_width(ui.available_width()))
            .changed()
        {
            *entry = json!(raw);
            return true;
        }
        false
    } else if entry.is_object() {
        let mut changed = predicate_ui(ui, lang, entry);
        if mode(entry) == "regex" {
            changed |= edit_string(
                ui,
                entry,
                "flags",
                text(
                    lang,
                    "正则标志，例如 i（空缺时沿用排除标志）",
                    "Regex flags, e.g. i (inherits exclusion flags when absent)",
                ),
            );
        }
        changed
    } else {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            text(
                lang,
                "无法读取此排除条件，可移除后重新添加。",
                "Cannot read this exclusion. Remove and add it again.",
            ),
        );
        false
    }
}

fn next_rule(items: &[Value]) -> Value {
    let ids: std::collections::HashSet<_> = items
        .iter()
        .enumerate()
        .map(|(i, v)| effective_id(v, i))
        .collect();
    let mut n = 1;
    while ids.contains(&format!("T{n}")) {
        n += 1;
    }
    json!({"id": format!("T{n}"), "enabled": true, "type":"contains", "pattern":""})
}

pub(super) fn show(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    _label: &str,
    raw: &mut String,
) -> bool {
    let Ok(mut doc) = document(raw) else {
        ui.colored_label(
            tokens.warning,
            text(
                lang,
                "无法读取条件，请检查原配置。",
                "Cannot read conditions. Check the original configuration.",
            ),
        );
        let mut changed = false;
        ui.collapsing(text(lang, "配置修复", "Configuration recovery"), |ui| {
            changed = ui
                .add(
                    egui::TextEdit::multiline(raw)
                        .desired_width(ui.available_width())
                        .desired_rows(4),
                )
                .changed();
        });
        return changed;
    };
    let dialog_id = ui.id().with("condition_dialog_v2");
    let draft_id = dialog_id.with("draft");
    let mut editing = ui.data_mut(|d| d.get_temp::<usize>(dialog_id));
    let mut draft = ui
        .data_mut(|d| d.get_temp::<String>(draft_id))
        .unwrap_or_default();
    let items = items_mut(&mut doc);
    let mut changed = false;
    let mut remove = None;
    ui.label(
        RichText::new(text(
            lang,
            "收到以下内容时，自动保存截图和波形。",
            "Save a screenshot and waveform when this text arrives.",
        ))
        .color(tokens.text_muted),
    );
    ui.add_space(4.0);
    for (i, item) in items.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            // Explicit geometry avoids the parent's justified form stretching rows.
            let width = ui.available_width().max(1.0);
            let (row, _) = ui.allocate_exact_size(egui::vec2(width, 38.0), egui::Sense::hover());
            ui.painter().rect_filled(row, 6, tokens.surface_bg);
            let enabled_rect =
                egui::Rect::from_min_size(row.min + egui::vec2(6.0, 5.0), egui::vec2(24.0, 28.0));
            let delete_rect = egui::Rect::from_min_size(
                egui::pos2(row.max.x - 30.0, row.min.y + 5.0),
                egui::vec2(24.0, 28.0),
            );
            let edit_rect = egui::Rect::from_min_size(
                egui::pos2(delete_rect.min.x - 52.0, row.min.y + 5.0),
                egui::vec2(48.0, 28.0),
            );
            let content_rect = egui::Rect::from_min_max(
                egui::pos2(enabled_rect.max.x + 4.0, row.min.y + 5.0),
                egui::pos2(edit_rect.min.x - 6.0, row.max.y - 5.0),
            );
            let mut enabled = item.get("enabled") != Some(&Value::Bool(false));
            if ui
                .put(enabled_rect, egui::Checkbox::without_text(&mut enabled))
                .on_hover_text(text(lang, "启用此条件", "Enable condition"))
                .changed()
                && item.is_object()
            {
                item["enabled"] = json!(enabled);
                changed = true;
            }
            if mode(item) == "contains" && item.is_object() {
                let key = pattern_key(item);
                let mut value = pattern(item);
                if ui
                    .put(
                        content_rect,
                        egui::TextEdit::singleline(&mut value).hint_text(text(
                            lang,
                            "例如 timeout",
                            "Keyword, e.g. timeout",
                        )),
                    )
                    .changed()
                {
                    item[key] = json!(value);
                    changed = true;
                }
            } else {
                let caption = title(item, i);
                let response = ui.interact(
                    content_rect,
                    ui.id().with("condition_name"),
                    egui::Sense::click(),
                );
                ui.painter().with_clip_rect(content_rect).text(
                    content_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &caption,
                    egui::FontId::proportional(14.0),
                    if enabled {
                        tokens.text_primary
                    } else {
                        tokens.text_muted
                    },
                );
                if response.clicked() {
                    editing = Some(i);
                    draft = item.to_string();
                }
                response.on_hover_text(text(
                    lang,
                    "已保存的精确匹配，点击编辑查看条件和忽略项。",
                    "Saved exact match. Edit to inspect the condition and exclusions.",
                ));
            }
            if ui
                .put(edit_rect, egui::Button::new(text(lang, "编辑", "Edit")))
                .clicked()
            {
                editing = Some(i);
                draft = item.to_string();
            }
            if ui
                .put(delete_rect, egui::Button::new("×"))
                .on_hover_text(text(lang, "删除条件", "Delete condition"))
                .clicked()
            {
                remove = Some(i);
            }
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        items.remove(i);
        editing = None;
        changed = true;
    }
    if ui
        .button(text(lang, "+ 添加关键词", "+ Add keyword"))
        .clicked()
    {
        items.push(next_rule(items));
        changed = true;
    }
    if items.is_empty() {
        ui.weak(text(
            lang,
            "例如输入 ERROR，收到包含 ERROR 的消息时就会抓取。",
            "For example, ERROR captures messages containing ERROR.",
        ));
    }
    let mut open = editing.is_some_and(|i| i < items.len());
    let mut apply = false;
    let mut cancel = false;
    let mut movement = None;
    if let Some(i) = editing.filter(|&i| i < items.len()) {
        let mut candidate: Value =
            serde_json::from_str(&draft).unwrap_or_else(|_| items[i].clone());
        egui::Window::new(text(lang, "编辑抓取条件", "Edit capture condition"))
            .id(dialog_id)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .max_width((ui.ctx().screen_rect().width() - 32.0).max(180.0))
            .max_height((ui.ctx().screen_rect().height() - 100.0).max(200.0))
            .vscroll(true)
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                predicate_ui(ui, lang, &mut candidate);
                ui.collapsing(
                    text(
                        lang,
                        "忽略这些消息（可选）",
                        "Ignore these messages (optional)",
                    ),
                    |ui| {
                        exclusion_ui(ui, lang, &mut candidate);
                    },
                );
                ui.collapsing(
                    text(lang, "名称和高级选项", "Name and advanced options"),
                    |ui| {
                        ui.label(text(lang, "显示名称", "Display name"));
                        edit_string(ui, &mut candidate, "label", text(lang, "选填", "Optional"));
                        ui.label(text(lang, "内部标识", "Internal ID"));
                        edit_string(ui, &mut candidate, "id", "T1");
                        if mode(&candidate) == "regex" {
                            ui.label(text(lang, "正则标志", "Regex flags"));
                            edit_string(ui, &mut candidate, "flags", "i");
                        }
                        ui.label(text(lang, "忽略条件的正则标志", "Exclusion flags"));
                        edit_string(ui, &mut candidate, "exclude_flags", "");
                        ui.label(text(lang, "备注", "Note"));
                        edit_string(ui, &mut candidate, "note", "");
                    },
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(i > 0, egui::Button::new(text(lang, "上移", "Up")))
                        .clicked()
                    {
                        movement = Some(-1);
                        apply = true;
                    }
                    if ui
                        .add_enabled(
                            i + 1 < items.len(),
                            egui::Button::new(text(lang, "下移", "Down")),
                        )
                        .clicked()
                    {
                        movement = Some(1);
                        apply = true;
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    apply |= ui
                        .button(text(lang, "保存条件", "Save condition"))
                        .clicked();
                    cancel = ui.button(text(lang, "取消", "Cancel")).clicked();
                });
            });
        draft = candidate.to_string();
        if apply {
            items[i] = candidate;
            changed = true;
            open = false;
            if let Some(delta) = movement {
                let other = (i as isize + delta) as usize;
                if other < items.len() {
                    items.swap(i, other);
                }
            }
        }
    }
    if !open || cancel {
        editing = None;
    }
    ui.data_mut(|d| {
        if let Some(i) = editing {
            d.insert_temp(dialog_id, i);
            d.insert_temp(draft_id, draft);
        } else {
            d.remove::<usize>(dialog_id);
            d.remove::<String>(draft_id);
        }
    });
    if changed {
        *raw = serde_json::to_string_pretty(&doc).unwrap();
    }
    if let Err(message) = validate(raw, lang) {
        ui.colored_label(tokens.warning, message);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> String {
        let station: Value = serde_json::from_str(include_str!(
            "../../../../test-tools/plugins/scope-serial-monitor/station.json"
        ))
        .unwrap();
        station["serial_triggers"]["items"].to_string()
    }

    #[test]
    fn line_trigger_editor_validates_drafts_and_slug_collisions() {
        assert!(validate(&recipe(), Lang::Zh).is_ok());
        for invalid in [
            "",
            "[]",
            "null",
            "{}",
            "oops",
            "[null]",
            r#"[{"type":"contains","pattern":""}]"#,
            r#"[{"enabled":false,"pattern":"ok"}]"#,
            r#"[{"type":"unknown","pattern":"ok"}]"#,
            r#"[{"id":" ","pattern":"ok"}]"#,
            r#"[{"pattern":"ok","unless":[{}]}]"#,
            r#"[{"pattern":"ok","unless":"ignored legacy value"}]"#,
            r#"[{"id":"a b","pattern":"ok"},{"id":"a?b","pattern":"ok"}]"#,
        ] {
            assert!(validate(invalid, Lang::Zh).is_err(), "{invalid}");
        }
        assert!(validate(
            r#"[{"pattern":"ok"},{"id":"T1","enabled":false}]"#,
            Lang::En
        )
        .is_ok());
        assert!(validate(
            r#"[{"text":"ok","type":"contains","exclude":"skip","exclude_flags":"i"}]"#,
            Lang::En
        )
        .is_ok());
    }

    #[test]
    fn line_trigger_editor_preserves_aliases_extensions_and_envelope() {
        let mut doc = document(r#"{"items":[{"id":"A","text":"old","type":"contains","custom":{"keep":true},"except":["skip"],"exclude":"BAD"}],"reload_s":2}"#).unwrap();
        let rule = &mut items_mut(&mut doc)[0];
        let key = pattern_key(rule);
        rule[key] = json!("new");
        assert_eq!(doc["items"][0]["text"], "new");
        assert!(doc["items"][0].get("pattern").is_none());
        assert_eq!(doc["items"][0]["custom"]["keep"], true);
        assert_eq!(doc["items"][0]["except"], json!(["skip"]));
        assert_eq!(doc["items"][0]["exclude"], "BAD");
        assert_eq!(doc["reload_s"], 2);
        assert!(validate(&doc.to_string(), Lang::Zh).is_ok());
        assert_eq!(next_rule(&[json!({}), json!({"id":"T2"})])["id"], "T3");
    }

    fn frame(ctx: &egui::Context, raw: &mut String, events: Vec<egui::Event>) -> egui::FullOutput {
        let tokens = crate::theme::apply_theme(ctx, false);
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(380.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, Lang::Zh, &tokens, "串口触发条件", raw);
                });
            },
        )
    }

    fn find_text(shapes: &[egui::epaint::ClippedShape], target: &str) -> egui::Pos2 {
        fn find(shape: &egui::epaint::Shape, target: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == target => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| find(s, target)),
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|s| find(&s.shape, target))
            .unwrap_or_else(|| panic!("Missing UI label: {target}"))
    }

    fn click(ctx: &egui::Context, raw: &mut String, pos: egui::Pos2) -> egui::FullOutput {
        let _ = frame(
            ctx,
            raw,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            ctx,
            raw,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
    }

    #[test]
    fn line_trigger_editor_pointer_add_type_reorder_delete_roundtrip() {
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let mut raw = recipe();
        let original = raw.clone();
        let output = frame(&ctx, &mut raw, vec![]);
        assert_eq!(raw, original);
        let add = find_text(&output.shapes, "+ 添加关键词");
        let output = click(&ctx, &mut raw, add);
        assert_eq!(
            serde_json::from_str::<Value>(&raw)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            4
        );
        let output = frame(&ctx, &mut raw, vec![]);
        let input = find_text(&output.shapes, "例如 timeout");
        click(&ctx, &mut raw, input);
        let output = frame(&ctx, &mut raw, vec![egui::Event::Text("ERROR".into())]);
        assert!(validate(&raw, Lang::Zh).is_ok());
        let delete = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == "×" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        click(&ctx, &mut raw, delete);
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap(),
            serde_json::from_str::<Value>(&original).unwrap()
        );
    }

    #[test]
    fn line_trigger_editor_string_example_and_advanced_click() {
        let ctx = egui::Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        crate::fonts::install_cjk_fonts(&ctx);
        let mut raw=json!([
          {"id":"T1","label":"通信异常","type":"contains","pattern":"ERROR","unless":[{"type":"contains","pattern":"ignored"}],"custom":true},
          {"id":"T2","label":"通信超时","type":"contains","pattern":"timeout"}
        ]).to_string();
        let original = raw.clone();
        let output = frame(&ctx, &mut raw, vec![]);
        if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
            crate::ui_audit::export_frame(
                &ctx,
                output.clone(),
                std::path::Path::new(&dir),
                "serial-triggers-string-example",
                egui::vec2(380.0, 720.0),
            );
        }
        let edit = find_text(&output.shapes, "编辑");
        click(&ctx, &mut raw, edit);
        let output = frame(&ctx, &mut raw, vec![]);
        let case = find_text(&output.shapes, "区分大小写");
        click(&ctx, &mut raw, case);
        assert_eq!(raw, original, "Dialog draft must not apply before Save");
        let output = frame(&ctx, &mut raw, vec![]);
        let cancel = find_text(&output.shapes, "取消");
        click(&ctx, &mut raw, cancel);
        assert_eq!(raw, original, "Cancel preserves every original byte");
        let output = frame(&ctx, &mut raw, vec![]);
        let edit = find_text(&output.shapes, "编辑");
        click(&ctx, &mut raw, edit);
        let output = frame(&ctx, &mut raw, vec![]);
        let case = find_text(&output.shapes, "区分大小写");
        click(&ctx, &mut raw, case);
        let output = frame(&ctx, &mut raw, vec![]);
        let save = find_text(&output.shapes, "保存条件");
        click(&ctx, &mut raw, save);
        let result: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(result[0]["case_sensitive"], true);
        assert_eq!(result[0]["custom"], true);
        assert_eq!(
            result[0]["unless"],
            json!([{"type":"contains","pattern":"ignored"}])
        );
    }

    #[test]
    fn line_trigger_editor_visual_widths_themes_languages() {
        for lang in [Lang::Zh, Lang::En] {
            for light in [true, false] {
                for width in [260.0, 380.0, 640.0] {
                    let ctx = egui::Context::default();
                    crate::fonts::install_cjk_fonts(&ctx);
                    let tokens = crate::theme::apply_theme(&ctx, light);
                    let size = egui::vec2(width, 720.0);
                    let mut raw = recipe();
                    let original = raw.clone();
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                show(
                                    ui,
                                    lang,
                                    &tokens,
                                    text(lang, "串口触发条件", "Serial line triggers"),
                                    &mut raw,
                                );
                                assert!(
                                    ui.min_rect().right() <= width + 1.0,
                                    "overflow at {width}"
                                );
                            });
                        },
                    );
                    assert_eq!(raw, original);
                    if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                        let name = format!(
                            "serial-triggers-{}-{}-{width}",
                            text(lang, "zh", "en"),
                            if light { "light" } else { "dark" }
                        );
                        crate::ui_audit::export_frame(
                            &ctx,
                            output,
                            std::path::Path::new(&dir),
                            &name,
                            size,
                        );
                    }
                }
            }
        }
    }
}
