use super::*;
fn display_value(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

impl Workspace {
    pub(super) fn merge_drafts(&mut self) {
        if let Some(rows) = self.snapshot["items"].as_array_mut() {
            for (id, row) in &self.drafts {
                if !rows.iter().any(|r| r["id"] == *id) {
                    rows.push(row.clone());
                }
            }
            self.snapshot["total"] = json!(rows.len());
        }
        self.list_epoch = self.list_epoch.wrapping_add(1);
    }
}

/// The repository is the primary relation. A run is only an overlay by stable ID.
fn project_rows(snapshot: &Value, dirty_id: Option<&str>, shared_dirty: bool) -> Vec<Value> {
    let history: std::collections::HashMap<_, _> = snapshot["matrix"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["id"].as_str().map(|id| (id, r)))
        .collect();
    let shared_ok = !snapshot["project_config"].is_object()
        || (snapshot["project_config"]["saved"] == true
            && snapshot["project_config"]["sha256"]
                .as_str()
                .is_some_and(|sha| {
                    Some(sha) == snapshot["matrix"]["project_config"]["sha256"].as_str()
                }));
    let mut rows: Vec<_> = snapshot["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let id = item["id"].as_str().unwrap_or("");
            let old = history.get(id);
            let matched = old.is_some_and(|r| {
                r["config_sha256"]
                    .as_str()
                    .zip(item["sha256"].as_str())
                    .is_some_and(|(a, b)| a == b && !a.is_empty())
            }) && shared_ok
                && !shared_dirty
                && dirty_id != Some(id)
                && item["draft"] != true;
            let mut row = if matched {
                (*old.unwrap()).clone()
            } else {
                json!({"id":id,"status":"pending","phase":"","elapsed_s":null,"attempt":null})
            };
            for key in [
                "name", "enabled", "draft", "order", "tags", "revision", "sha256",
            ] {
                row[key] = item[key].clone();
            }
            if matched
                && old.is_some_and(|r| {
                    r["selected"] == false
                        && r["status"] == "skipped"
                        && !r["result_ref"].is_string()
                })
            {
                row["status"] = json!("pending");
                row["phase"] = json!("");
                row["not_selected"] = json!(true);
            }
            row["historical"] = old.map_or(Value::Null, |r| (*r).clone());
            row["stale"] = json!(old.is_some() && !matched);
            row
        })
        .collect();
    rows.sort_by_key(|r| r["order"].as_i64().unwrap_or(0));
    rows
}

impl TestToolPanel {
    pub fn take_workspace_result(&mut self) -> Option<PathBuf> {
        let reference = self.workspace.result_ref.take()?;
        // UI supplies project-relative references; the report tool revalidates them.
        Some(
            PathBuf::from(self.data_root_resolved())
                .join("projects")
                .join(self.effective_project_id())
                .join(reference),
        )
    }
    pub(super) fn project_save_params(&self) -> Value {
        json!({"value":self.workspace.project_document,"expected_revision":self.workspace.snapshot["project_config"]["revision"],"expected_sha256":self.workspace.snapshot["project_config"]["sha256"]})
    }
    pub(super) fn paint_project_settings(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let p = self.workspace.snapshot["project_config"].clone();
        ui.heading(if zh {
            "工程设置"
        } else {
            "Project settings"
        });
        ui.weak(if zh{"此工程的 DUT 约束与仪器绑定；本轮序列号、操作员请在本轮参数录入。"}else{"DUT constraints and instrument bindings for this project. Enter actual serial and operator under Run parameters."});
        if !self.workspace.current.is_empty() && self.workspace.document != self.workspace.original
        {
            ui.label(if zh {
                "请先保存或放弃测试项修改。"
            } else {
                "Save or discard item changes first."
            });
            return;
        }
        let busy = self.workspace.rx.is_some()
            || self.job.is_some()
            || self.workspace.snapshot["locked"].is_object();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "revision {} · {}",
                p["revision"],
                if p["saved"] == true {
                    if zh {
                        "已保存"
                    } else {
                        "Saved"
                    }
                } else {
                    if zh {
                        "尚未保存"
                    } else {
                        "Not saved"
                    }
                }
            ));
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if zh {
                        "保存工程设置"
                    } else {
                        "Save project settings"
                    }),
                )
                .clicked()
            {
                self.workspace_send("project.save", self.project_save_params());
            }
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if zh {
                        "放弃修改"
                    } else {
                        "Discard changes"
                    }),
                )
                .clicked()
            {
                self.workspace.project_document = self.workspace.project_original.clone();
            }
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if zh {
                        "旧配置迁移预览"
                    } else {
                        "Preview legacy migration"
                    }),
                )
                .clicked()
            {
                self.workspace_send("project.migration.preview", json!({}));
            }
        });
        let plan = self.workspace.project_migration.clone();
        if plan.is_object() {
            let conflicts = plan["conflicts"].as_array().map_or(0, Vec::len);
            ui.label(format!(
                "{}: {conflicts}",
                if zh {
                    "迁移冲突"
                } else {
                    "Migration conflicts"
                }
            ));
            if conflicts > 0 {
                ui.colored_label(
                    tokens.warning,
                    if zh {
                        "存在不同绑定，须明确处理；不会用首项覆盖。"
                    } else {
                        "Bindings differ; explicit resolution required. No first-item overwrite."
                    },
                );
            }
            ui.collapsing(
                if zh {
                    "迁移计划与冲突明细"
                } else {
                    "Migration plan and conflicts"
                },
                |ui| {
                    ui.add(
                        egui::Label::new(serde_json::to_string_pretty(&plan).unwrap_or_default())
                            .wrap(),
                    );
                },
            );
            if ui
                .add_enabled(
                    !busy && plan["ok"] == true && conflicts == 0,
                    egui::Button::new(if zh {
                        "备份并提交迁移"
                    } else {
                        "Back up and commit migration"
                    }),
                )
                .clicked()
            {
                self.workspace_send(
                    "project.migration.commit",
                    json!({"plan_sha256":plan["plan_sha256"]}),
                );
            }
        }
        let mut layout = p["ui"].clone();
        layout["_root"] = self.workspace.project_document.clone();
        layout["_project_config_root"] = self.workspace.project_document.clone();
        layout["_busy"] = json!(busy);
        layout["_serial_ports"] = json!(self
            .serial_ports
            .iter()
            .map(|p| p.device.clone())
            .collect::<Vec<_>>());
        layout["_instruments"]=json!(self.live_devices.iter().map(|d|json!({"value":d.resource,"resource":d.resource,"serial":d.serial,"kind":d.kind,"model":d.model,"label":d.label()})).collect::<Vec<_>>());
        egui::ScrollArea::vertical()
            .id_salt("project_settings")
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    form(
                        ui,
                        &p["schema"],
                        &p["documents"],
                        &layout,
                        &mut self.workspace.project_document,
                        "",
                        lang,
                        &self.workspace.errors,
                        None,
                        0,
                    );
                });
            });
        if self.workspace.project_document != self.workspace.project_original {
            self.workspace.snapshot["preflight"]["valid"] = json!(false);
            self.workspace.api_binding = None;
        }
    }
    fn send_user_file(&mut self, method: &str, file: std::path::PathBuf) {
        let extra = file.parent().map(|dir| dir.display().to_string());
        let added = extra.as_ref().is_some_and(|root| {
            if self.approved_config_roots.iter().any(|have| have == root) {
                false
            } else {
                self.approved_config_roots.push(root.clone());
                true
            }
        });
        self.workspace_send(method, json!({"file": file.display().to_string()}));
        if added {
            self.approved_config_roots.pop();
        }
    }
    pub(super) fn paint_project_file_bar(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let files = self.workspace.snapshot["project_files"].clone();
        if !files.is_object() {
            return;
        }
        let zh = matches!(lang, Lang::Zh);
        let items = files["items"].as_array().cloned().unwrap_or_default();
        let active = files["active"].as_str().unwrap_or("").to_owned();
        let busy = self.workspace.rx.is_some() || self.job.is_some();
        let dirty = self.workspace.dirty();
        ui.label(egui::RichText::new(if zh {"测试项目"}else{"Test project"}).small().color(ui.visuals().weak_text_color()));
        let selected=items.iter().find(|item|item["id"].as_str()==Some(active.as_str()));
        let label=selected.and_then(|item|item["name"].as_str()).unwrap_or(if active.is_empty(){if zh {"未选择"}else{"None"}}else{&active}).to_owned();
        ui.horizontal_wrapped(|ui| {
            let width=(ui.available_width()-76.0).clamp(32.0,220.0);
            ui.add_enabled_ui(!busy&&!dirty,|ui| {
                egui::ComboBox::from_id_salt("project_file").width(width).truncate().selected_text(label)
                    .show_ui(ui,|ui| {
                        for item in &items {
                            let id=item["id"].as_str().unwrap_or(""); let name=item["name"].as_str().unwrap_or(id);
                            if ui.selectable_label(id==active,name).clicked() && id!=active {
                                self.workspace_send("project.file.activate",json!({"id":id}));
                            }
                        }
                    });
            });
            ui.menu_button(if zh {"项目文件"}else{"Files"},|ui| {
                if ui.add_enabled(!busy&&!dirty,egui::Button::new(if zh {"导入项目配置…"}else{"Import project…"})).clicked() {
                    ui.close_menu();
                    if let Some(file)=rfd::FileDialog::new().add_filter("JSON",&["json"]).pick_file() {self.send_user_file("project.file.import",file);}
                }
                if ui.add_enabled(!busy&&!dirty&&!active.is_empty(),egui::Button::new(if zh {"导出项目配置…"}else{"Export project…"})).clicked() {
                    ui.close_menu();
                    if let Some(file)=rfd::FileDialog::new().add_filter("JSON",&["json"]).save_file() {self.send_user_file("project.file.export",file);}
                }
            });
        });
    }
    pub(super) fn paint_project_matrix(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let zh = matches!(lang, Lang::Zh);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
            let filter_w = (ui.available_width() * 0.34).clamp(160.0, 280.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.workspace.query)
                    .desired_width(filter_w)
                    .hint_text(if zh {
                        "筛选名称、ID、标签"
                    } else {
                        "Filter name, ID, tags"
                    }),
            );
            egui::ComboBox::from_id_salt("matrix_status")
                .selected_text(matrix_status_label(
                    if self.workspace.status_filter.is_empty() {
                        "all"
                    } else {
                        &self.workspace.status_filter
                    },
                    zh,
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.workspace.status_filter,
                        String::new(),
                        matrix_status_label("all", zh),
                    );
                    for s in [
                        "pending",
                        "running",
                        "pass",
                        "fail",
                        "error",
                        "aborted",
                        "skipped",
                        "inconclusive",
                    ] {
                        ui.selectable_value(
                            &mut self.workspace.status_filter,
                            s.into(),
                            matrix_status_label(s, zh),
                        );
                    }
                });
            if ui
                .button(if zh { "清除筛选" } else { "Clear filters" })
                .clicked()
            {
                self.workspace.query.clear();
                self.workspace.status_filter.clear();
            }
            ui.checkbox(
                &mut self.workspace.follow,
                if zh {
                    "跟随运行项"
                } else {
                    "Follow active"
                },
            );
        });
        let m = &self.workspace.snapshot["matrix"];
        if let Some(error) = m["error"].as_str() {
            ui.colored_label(crate::theme::error_color(ui), error);
        }
        if m["available"] == true {
            ui.weak(if self.job.is_none() || self.preflight_only {
                if zh {
                    "最近一次运行快照"
                } else {
                    "Latest run snapshot"
                }
            } else {
                if zh {
                    "本轮运行快照"
                } else {
                    "Current run snapshot"
                }
            });
        }
        let key = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            self.workspace.list_epoch,
            m["run_id"],
            m["revision"],
            self.workspace.query,
            self.workspace.status_filter,
            self.workspace.dirty(),
            self.workspace.current,
            self.workspace.snapshot["project_config"]["sha256"]
        );
        if key != self.workspace.matrix_cache_key {
            let query = self.workspace.query.to_lowercase();
            let rows = project_rows(
                &self.workspace.snapshot,
                (self.workspace.document != self.workspace.original)
                    .then_some(self.workspace.current.as_str()),
                self.workspace.project_document != self.workspace.project_original,
            );
            self.workspace.matrix_cache = std::sync::Arc::new(
                rows.into_iter()
                    .filter(|r| {
                        (self.workspace.status_filter.is_empty()
                            || r["status"] == self.workspace.status_filter)
                            && format!("{} {} {}", r["id"], r["name"], r["tags"])
                                .to_lowercase()
                                .contains(&query)
                    })
                    .collect(),
            );
            self.workspace.matrix_cache_key = key;
        }
        let rows = self.workspace.matrix_cache.clone();
        let total = self.workspace.snapshot["items"]
            .as_array()
            .map_or(0, Vec::len);
        let drafts = self.workspace.drafts.len();
        ui.label(if zh {
            format!(
                "可见 {} / 工程总数 {total} · 已保存 {} · 草稿 {drafts}",
                rows.len(),
                total.saturating_sub(drafts)
            )
        } else {
            format!(
                "Visible {} / Project {total} · Saved {} · Drafts {drafts}",
                rows.len(),
                total.saturating_sub(drafts)
            )
        });
        let stale = rows.iter().filter(|r| r["stale"] == true).count();
        if stale > 0 {
            ui.colored_label(egui::Color32::YELLOW,if zh{format!("{stale} 项旧结论不代表当前配置；当前待测试，历史保留。")}else{format!("{stale} results do not certify current content. Current items are pending; history is retained.")});
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
            for (mode, cn, en) in [
                ("all", "选择可见项", "Select visible"),
                ("clear", "清除选择", "Clear selection"),
                ("fail", "选择失败项", "Select failures"),
            ] {
                if ui
                    .add_enabled(
                        self.job.is_none(),
                        egui::Button::new(if zh { cn } else { en }),
                    )
                    .clicked()
                {
                    self.workspace.selected.clear();
                    if mode != "clear" {
                        for r in rows.iter().filter(|r| {
                            r["draft"] != true
                                && r["enabled"] != false
                                && (mode == "all"
                                    || r["status"] == "fail"
                                    || r["status"] == "error")
                        }) {
                            self.workspace
                                .selected
                                .insert(r["id"].as_str().unwrap_or("").into());
                        }
                    }
                }
            }
        });
        let wide = ui.available_width() >= 860.0;
        if wide {
            let h = ui.available_height().max(180.0);
            let gap = 12.0;
            let table_w = matrix_table_width().min((ui.available_width() - gap - 280.0).max(420.0));
            let detail_w = (ui.available_width() - gap - table_w).max(280.0);
            let left = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(table_w, h));
            let right = egui::Rect::from_min_max(
                egui::pos2(left.right() + gap, left.top()),
                egui::pos2(left.right() + gap + detail_w, left.bottom()),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left), |ui| {
                self.matrix_rows_ui(ui, lang, &rows)
            });
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right), |ui| {
                self.matrix_details_ui(ui, lang, &rows)
            });
        } else {
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() * 0.55).max(140.0))
                .id_salt("matrix_narrow")
                .show(ui, |ui| self.matrix_rows_ui(ui, lang, &rows));
            ui.separator();
            self.matrix_details_ui(ui, lang, &rows);
        }
    }
    fn matrix_rows_ui(&mut self, ui: &mut egui::Ui, lang: Lang, rows: &[Value]) {
        let zh = matches!(lang, Lang::Zh);
        ui.style_mut().spacing.scroll.floating = false;
        let scroll_style = ui.spacing().scroll;
        let bar_w = scroll_style.bar_width
            + scroll_style.bar_inner_margin
            + scroll_style.bar_outer_margin
            + scroll_style.floating_width
            + 4.0;
        let height = ui.available_height().max(120.0);
        let (area, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        let content_w = (area.width() - bar_w).max(1.0);
        let widths = matrix_column_widths(content_w);
        let labels = [
            if zh { "选" } else { "✓" },
            "#",
            if zh { "测试名称" } else { "Test name" },
            if zh { "状态" } else { "Status" },
            if zh { "阶段" } else { "Phase" },
            if zh { "耗时" } else { "Elapsed" },
        ];
        let header = matrix_cell_rects(area.min.x, area.min.y, 22.0, widths);
        let weak = ui.visuals().weak_text_color();
        for (rect, label) in header.iter().zip(labels) {
            ui.painter().text(
                egui::pos2(rect.left() + 4.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(ui_theme::FONT_CAPTION),
                weak,
            );
        }
        let body = egui::Rect::from_min_max(
            egui::pos2(area.min.x, area.min.y + 22.0),
            area.max,
        );
        let active = self.workspace.snapshot["matrix"]["active_item_id"]
            .as_str()
            .and_then(|id| rows.iter().position(|r| r["id"] == id));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body);
            ui.style_mut().spacing.scroll.floating = false;
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("test_matrix")
                .auto_shrink([false, false]);
            if ui.input(|i| i.raw_scroll_delta.y != 0.0) && ui.rect_contains_pointer(body) {
                self.workspace.follow = false;
            }
            if self.workspace.follow {
                if let Some(i) = active {
                    scroll = scroll.vertical_scroll_offset(i as f32 * 30.0);
                }
            }
            scroll.show_rows(ui, 30.0, rows.len(), |ui, range| {
                ui.set_width(content_w);
                ui.set_clip_rect(egui::Rect::from_min_max(
                    body.min,
                    egui::pos2(body.min.x + content_w, body.max.y),
                ));
                for i in range {
                    let r = &rows[i];
                    let id = r["id"].as_str().unwrap_or("");
                    let (row_rect, _) = ui.allocate_exact_size(
                        egui::vec2(content_w, 30.0),
                        egui::Sense::hover(),
                    );
                    let cells = matrix_cell_rects(row_rect.min.x, row_rect.min.y + 1.0, 28.0, widths);
                    ui.push_id(id, |ui| {
                        let mut checked = self.workspace.selected.contains(id);
                        let enabled =
                            self.job.is_none() && r["draft"] != true && r["enabled"] != false;
                        if ui
                            .put(
                                cells[0],
                                egui::Checkbox::without_text(&mut checked),
                            )
                            .changed()
                            && enabled
                        {
                            if checked {
                                self.workspace.selected.insert(id.into());
                            } else {
                                self.workspace.selected.remove(id);
                            }
                        }
                        ui.put(cells[1], egui::Label::new(format!("{}", i + 1)));
                        if matrix_name_cell(
                            ui,
                            cells[2],
                            r["name"].as_str().unwrap_or(id),
                            self.workspace.matrix_current == id,
                        )
                        .clicked()
                        {
                            self.workspace.matrix_current = id.into();
                            self.workspace.result_details = Value::Null;
                            self.workspace.result_key.clear();
                        }
                        let state = if r["draft"] == true {
                            if zh { "未保存" } else { "Draft" }
                        } else if r["enabled"] == false {
                            if zh { "已禁用" } else { "Disabled" }
                        } else {
                            matrix_status_label(r["status"].as_str().unwrap_or("pending"), zh)
                        };
                        ui.put(cells[3], egui::Label::new(state));
                        ui.put(
                            cells[4],
                            egui::Label::new(r["phase"].as_str().unwrap_or("—")),
                        );
                        ui.put(
                            cells[5],
                            egui::Label::new(
                                r["elapsed_s"]
                                    .as_f64()
                                    .map(|n| format!("{n:.1}s"))
                                    .unwrap_or_else(|| "—".into()),
                            ),
                        );
                    });
                }
            });
        });
    }
    fn matrix_details_ui(&mut self, ui: &mut egui::Ui, lang: Lang, rows: &[Value]) {
        let zh = matches!(lang, Lang::Zh);
        if self.workspace.matrix_current.is_empty() {
            if let Some(r) = rows.first() {
                self.workspace.matrix_current = r["id"].as_str().unwrap_or("").into();
            }
        }
        let Some(row) = rows
            .iter()
            .find(|r| r["id"] == self.workspace.matrix_current)
            .cloned()
        else {
            ui.weak(if zh {
                "选择测试项查看详情"
            } else {
                "Select an item to view details"
            });
            return;
        };
        let historical = &row["historical"];
        let run = self.workspace.snapshot["matrix"]["run_id"].clone();
        let request_key = format!("{}:{}:{}", row["id"], run, historical["attempt"]);
        if historical["result_ref"].is_string()
            && self.workspace.result_key != request_key
            && self.workspace.rx.is_none()
        {
            self.workspace.result_key = request_key;
            self.workspace.result_details = Value::Null;
            self.workspace_send(
                "result.get",
                json!({"config_id":row["id"],"run_id":run,"attempt":historical["attempt"]}),
            );
        }
        egui::ScrollArea::vertical().id_salt("matrix_detail").show(ui,|ui|{
            ui.heading(row["name"].as_str().unwrap_or(""));ui.weak(row["id"].as_str().unwrap_or(""));
            ui.label(matrix_status_label(row["status"].as_str().unwrap_or("pending"),zh));
            if row["stale"]==true{ui.colored_label(egui::Color32::YELLOW,if zh{"历史结果已过期：用例或工程共享配置已变化。"}else{"Historical result is stale: item or shared configuration changed."});}
            if historical.is_object(){ui.label(format!("{}: {} · #{}",if zh{"结果来源"}else{"Result source"},display_value(&run),display_value(&historical["attempt"])));ui.label(format!("{} · {}",matrix_status_label(historical["status"].as_str().unwrap_or(""),zh),historical["phase"].as_str().unwrap_or("")));for k in ["message","error_code"]{if let Some(t)=historical[k].as_str().filter(|t|!t.is_empty()){ui.add(egui::Label::new(t).wrap());}}}
            else{ui.label(if zh{"尚未测试。保存并预检后执行；采集计划见用例配置。"}else{"Not tested. Save and preflight to execute; see configuration for the capture plan."});}
            let details=self.workspace.result_details.clone();
            if let Some(error)=details["error"].as_str(){ui.colored_label(crate::theme::error_color(ui),error);}
            if details["available"]!=true {if let Some(reference)=details["result_ref"].as_str(){if ui.link(if zh{"查看未核验的历史原文件"}else{"Inspect unverified historical file"}).clicked(){self.workspace.result_ref=Some(reference.into());}}}
            if details["available"]==true{
                let value=&details["value"];
                for (key,cn,en) in [("execution_status","执行状态","Execution"),("product_verdict","产品结论","Product verdict"),("verdict","结论","Verdict"),("started_at","开始时间","Started"),("completed_at","完成时间","Completed"),("error_code","错误代码","Error"),("message","说明","Message")]{if !value[key].is_null(){ui.add(egui::Label::new(format!("{}：{}",if zh{cn}else{en},display_value(&value[key]))).wrap());}}
                let rules=value["rules"]["results"].as_array().or_else(||value["rules"].as_array()).or_else(||value["rule_results"].as_array()).or_else(||value["results"].as_array());
                if let Some(rules)=rules{ui.separator();ui.strong(if zh{"判定规则 · 实际值 / 限值"}else{"Rules · measured / limits"});for r in rules.iter().take(500){ui.add_space(5.0);ui.label(format!("{} · {}",display_value(&r["id"]).replace('_'," "),display_value(&r["verdict"])));let unit=r["unit"].as_str().unwrap_or("");let bounds=if r["minimum"].is_null()&&r["maximum"].is_null(){String::new()}else{format!(" · {} {} – {}",if zh{"限值"}else{"Limits"},display_value(&r["minimum"]),display_value(&r["maximum"]))};ui.weak(format!("{} {} {unit}{bounds}",if zh{"实际"}else{"Measured"},display_value(&r["measured"])));}}
                if let Some(reference)=details["result_ref"].as_str(){if ui.link(if zh{"打开本项结果与证据"}else{"Open item result and evidence"}).clicked(){self.workspace.result_ref=Some(reference.into());}}
                for entry in details["evidence"].as_array().into_iter().flatten(){let label=entry["label"].as_str().unwrap_or("");if entry["available"]==true{if ui.link(label).clicked(){if let Some(reference)=entry["reference"].as_str(){self.workspace.result_ref=Some(reference.into());}}}else{ui.colored_label(egui::Color32::YELLOW,format!("{label} · {}",if zh{"证据缺失或路径无效"}else{"Missing evidence or invalid path"}));}}
                ui.collapsing(if zh{"完整结果"}else{"Full result"},|ui|{ui.add(egui::Label::new(serde_json::to_string_pretty(value).unwrap_or_default()).wrap());});
            }
            let m = self.workspace.snapshot["matrix"].clone();
            ui.collapsing(if zh { "运行来源" } else { "Run provenance" }, |ui| {
                for key in ["run_id", "updated_at", "preflight_revision", "project_config"] {
                    ui.add(egui::Label::new(format!("{key}: {}", m[key])).wrap());
                }
            });
        });
    }
}

fn matrix_cell_rects(x: f32, y: f32, h: f32, widths: [f32; 6]) -> [egui::Rect; 6] {
    let gap = 8.0;
    let mut cursor = x;
    let mut rects = [egui::Rect::NOTHING; 6];
    for (i, width) in widths.into_iter().enumerate() {
        rects[i] = egui::Rect::from_min_size(egui::pos2(cursor, y), egui::vec2(width, h));
        cursor += width + gap;
    }
    rects
}

fn matrix_name_cell(ui: &mut egui::Ui, rect: egui::Rect, name: &str, selected: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with("matrix_name"), egui::Sense::click());
    let visuals = ui.visuals();
    let fill = if selected {
        Some(visuals.selection.bg_fill)
    } else if resp.hovered() {
        Some(visuals.widgets.hovered.weak_bg_fill)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), fill);
    }
    let color = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        name,
        0.0,
        egui::text::TextFormat {
            font_id: egui::FontId::proportional(ui_theme::FONT_BODY),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = (rect.width() - 10.0).max(8.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    let pos = egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
    resp
}

fn matrix_table_width() -> f32 {
    let widths = matrix_column_widths(10_000.0);
    widths.iter().sum::<f32>() + 8.0 * 5.0 + 22.0
}

fn matrix_column_widths(available: f32) -> [f32; 6] {
    let gaps = 8.0 * 5.0;
    let mut widths = [28.0, 36.0, 280.0, 84.0, 96.0, 64.0];
    let room = (available - gaps).max(0.0);
    let sum = widths.iter().sum::<f32>();
    if sum > room && sum > 0.0 {
        let scale = room / sum;
        for width in &mut widths {
            *width *= scale;
        }
    }
    widths
}

#[test]
fn repository_drives_unrun_subset_stale_and_shared_results() {
    for count in [0, 5, 20, 500] {
        let mut snapshot = json!({"items":(0..count).map(|i|json!({"id":format!("c{i}"),"name":format!("Case {i}"),"enabled":true,"sha256":"current"})).collect::<Vec<_>>(),"matrix":{"available":false}});
        assert_eq!(project_rows(&snapshot, None, false).len(), count);
        if count > 0 {
            snapshot["matrix"] = json!({"available":true,"items":[{"id":"c0","status":"pass","config_sha256":"current"},{"id":"removed","status":"fail","config_sha256":"current"}]});
            let r = project_rows(&snapshot, None, false);
            assert_eq!(r.len(), count);
            assert_eq!(r[0]["status"], "pass");
            if count > 1 {
                assert_eq!(r[1]["status"], "pending");
            }
            snapshot["project_config"] = json!({"saved":true,"sha256":"newshared"});
            let r = project_rows(&snapshot, None, false);
            assert_eq!(r[0]["status"], "pending");
            assert_eq!(r[0]["historical"]["status"], "pass");
        }
    }
}

#[cfg(test)]
fn bind_reply(panel: &mut TestToolPanel, method: &str) {
    let plugin=panel.selected_plugin().unwrap();
    let context=panel.workspace_request(plugin,method,Value::Null,true)["context"].clone();
    let config=matches!(method,"config.get"|"config.create"|"config.duplicate"|"config.save"|"config.validate");
    panel.workspace.request_guard=Some(WorkspaceRequestGuard{context,current:panel.workspace.current.clone(),
        document:config.then(||panel.workspace.document.clone()),
        project_document:method.starts_with("project.").then(||panel.workspace.project_document.clone()),control_run:None,
        matrix_host_run:(method=="matrix.get").then(||panel.agent_job_key())});
}
#[cfg(test)]
fn deliver(panel: &mut TestToolPanel, method: &str, data: Value) {
    let (tx, rx) = mpsc::channel();
    tx.send((method.into(), Ok(json!({"ok":true,"data":data}))))
        .unwrap();
    panel.workspace.rx = Some(rx);
    bind_reply(panel,method);
    panel.workspace_poll(&egui::Context::default());
}
#[test]
fn stale_config_reply_does_not_overwrite_new_editor_contents() {
    let mut panel=super::tests::fixture(1);
    bind_reply(&mut panel,"config.save");
    let (tx,rx)=mpsc::channel();
    tx.send(("config.save".into(),Ok(json!({"ok":true,"data":{"valid":true,"value":{"name":"old"}}})))).unwrap();
    panel.workspace.rx=Some(rx);
    panel.workspace.document["name"]=json!("new edit");
    panel.workspace_poll(&egui::Context::default());
    assert_eq!(panel.workspace.document["name"],"new edit");
    assert!(panel.workspace.error.contains("CONFIG.STALE_REPLY"));
    assert_eq!(panel.workspace.snapshot["preflight"]["valid"],false);
}
#[test]
fn workspace_publication_changes_for_each_edit_even_while_already_dirty() {
    let mut panel=super::tests::fixture(1);
    panel.ai_summary_dirty=true;panel.publish_ai_summary();
    let clean=panel.agent_identity();
    panel.workspace.document["name"]=json!("first");
    panel.ai_summary_dirty=true;panel.publish_ai_summary();
    let first=panel.agent_identity();
    panel.workspace.document["name"]=json!("second");
    panel.ai_summary_dirty=true;panel.publish_ai_summary();
    let second=panel.agent_identity();
    assert_ne!(clean,first);assert_ne!(first,second);
    panel.ai_summary_dirty=true;panel.publish_ai_summary();
    assert_eq!(second,panel.agent_identity());
}
#[test]
fn external_workspace_write_invalidates_binding_and_blocks_preflight_until_refresh() {
    let mut panel=super::tests::fixture(1);
    panel.ai_summary_dirty=true;panel.publish_ai_summary();
    let before=panel.agent_identity();
    panel.hub_api_mutations.fetch_add(1,Ordering::AcqRel);
    panel.hub_api_epoch.fetch_add(1,Ordering::AcqRel);
    assert_ne!(before,panel.agent_identity());
    assert!(panel.workspace_run_gate(true).is_err());
    panel.hub_api_mutations.fetch_sub(1,Ordering::AcqRel);
    assert!(panel.workspace_run_gate(true).is_err());
    panel.workspace.api_epoch=panel.hub_api_epoch.load(Ordering::Acquire);
    assert!(!panel.workspace_external_update_pending());
}
#[test]
fn drafts_are_immediate_survive_reload_and_failures_then_save_or_discard() {
    let mut p = super::tests::fixture(5);
    deliver(
        &mut p,
        "config.create",
        json!({"config_id":"new_case","value":{"id":"new_case","name":"New item","enabled":true},"draft":true,"revision":0,"sha256":""}),
    );
    assert_eq!(p.workspace.snapshot["items"].as_array().unwrap().len(), 6);
    assert_eq!(p.workspace.current, "new_case");
    assert!(p.workspace.dirty());
    assert!(!p.workspace.selected.contains("new_case"));
    let mut old = p.workspace.snapshot.clone();
    old["items"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["id"] != "new_case");
    deliver(&mut p, "workspace.open", old);
    assert_eq!(
        p.workspace.snapshot["items"].as_array().unwrap().len(),
        6,
        "late repository refresh retains local draft"
    );
    let (tx, rx) = mpsc::channel();
    tx.send((
        "config.save".into(),
        Ok(json!({"ok":false,"error":{"code":"SCHEMA.VALIDATION_FAILED"}})),
    ))
    .unwrap();
    p.workspace.rx = Some(rx);
    bind_reply(&mut p,"config.save");
    p.workspace_poll(&egui::Context::default());
    assert_eq!(p.workspace.current, "new_case");
    assert_eq!(p.workspace.drafts.len(), 1);
    deliver(
        &mut p,
        "config.save",
        json!({"valid":true,"config_id":"new_case","value":{"id":"new_case","name":"Saved item","enabled":true},"revision":1,"sha256":"savedhash"}),
    );
    assert!(p.workspace.drafts.is_empty());
    assert!(!p.workspace.dirty());
    assert_eq!(p.workspace.snapshot["items"].as_array().unwrap().len(), 6);
    deliver(
        &mut p,
        "config.create",
        json!({"config_id":"discard_me","value":{"id":"discard_me","name":"Discard"},"draft":true,"revision":0,"sha256":""}),
    );
    p.workspace_discard();
    assert!(p.workspace.drafts.is_empty());
    assert_eq!(p.workspace.snapshot["items"].as_array().unwrap().len(), 6);
}
#[test]
fn draft_capacity_duplicate_id_and_late_result_are_rejected() {
    let mut p = super::tests::fixture(500);
    p.workspace_send("config.create", json!({"config_id":"extra"}));
    assert_eq!(p.workspace.error, "CONFIG.REPOSITORY_LIMIT");
    assert!(p.workspace.rx.is_none());
    let mut p = super::tests::fixture(5);
    p.workspace_send("config.create", json!({"config_id":"CASE_000"}));
    assert_eq!(p.workspace.error, "CONFIG.ID_CONFLICT");
    assert!(p.workspace.rx.is_none());
    p.workspace.matrix_current = "case_001".into();
    p.workspace.result_key = "other-key".into();
    deliver(
        &mut p,
        "result.get",
        json!({"config_id":"case_000","run_id":"old","attempt":1,"available":true,"value":{"product_verdict":"PASS"}}),
    );
    assert!(p.workspace.result_details.is_null());
}

#[test]
fn matrix_row_pointer_selects_details_without_changing_execution_selection() {
    let mut p = super::tests::fixture(20);
    p.workspace.selected.insert("case_000".into());
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1200.0));
    let frame = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| p.paint_project_matrix(ui, Lang::En));
        },
    );
    let pos = frame
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "测试配置 / Test 010" => {
                Some(t.pos + egui::vec2(5.0, 5.0))
            }
            _ => None,
        })
        .expect("The tenth row is visible");
    for pressed in [true, false] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| p.paint_project_matrix(ui, Lang::En));
            },
        );
    }
    assert_eq!(p.workspace.matrix_current, "case_010");
    assert_eq!(p.workspace.selected.len(), 1);
    assert!(p.workspace.selected.contains("case_000"));
}
