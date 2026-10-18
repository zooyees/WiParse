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
    pub(super) fn paint_project_matrix(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let zh = matches!(lang, Lang::Zh);
        ui.heading(if zh { "测试矩阵" } else { "Test matrix" });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.workspace.query)
                    .desired_width(220.0)
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
            ui.colored_label(egui::Color32::LIGHT_RED, error);
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
        ui.collapsing(if zh { "运行来源" } else { "Run provenance" }, |ui| {
            for key in [
                "run_id",
                "updated_at",
                "preflight_revision",
                "project_config",
            ] {
                ui.add(egui::Label::new(format!("{key}: {}", m[key])).wrap());
            }
        });
        ui.horizontal_wrapped(|ui| {
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
        let wide = ui.available_width() >= 980.0;
        if wide {
            let h = ui.available_height().max(180.0);
            let w = (ui.available_width() * 0.58).clamp(560.0, 780.0);
            let left = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(w - 10.0, h));
            let right = egui::Rect::from_min_max(
                egui::pos2(left.right() + 14.0, left.top()),
                ui.max_rect().max,
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
        let name_width = (ui.available_width() - 340.0).clamp(200.0, 380.0);
        ui.horizontal(|ui| {
            for (label, width) in [
                (if zh { "选" } else { "✓" }, 18.0),
                ("#", 25.0),
                (if zh { "测试名称" } else { "Test name" }, name_width),
                (if zh { "状态" } else { "Status" }, 85.0),
                (if zh { "阶段" } else { "Phase" }, 85.0),
                (if zh { "耗时" } else { "Elapsed" }, 60.0),
            ] {
                ui.add_sized([width, 24.0], egui::Label::new(label));
            }
        });
        let active = self.workspace.snapshot["matrix"]["active_item_id"]
            .as_str()
            .and_then(|id| rows.iter().position(|r| r["id"] == id));
        let mut scroll = egui::ScrollArea::both().id_salt("test_matrix");
        if ui.input(|i| i.raw_scroll_delta.y != 0.0) && ui.rect_contains_pointer(ui.max_rect()) {
            self.workspace.follow = false;
        }
        if self.workspace.follow {
            if let Some(i) = active {
                scroll = scroll.vertical_scroll_offset(i as f32 * 36.0);
            }
        }
        scroll.show_rows(ui, 36.0, rows.len(), |ui, range| {
            for i in range {
                let r = &rows[i];
                let id = r["id"].as_str().unwrap_or("");
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
                        let mut checked = self.workspace.selected.contains(id);
                        if ui
                            .add_enabled(
                                self.job.is_none() && r["draft"] != true && r["enabled"] != false,
                                egui::Checkbox::without_text(&mut checked),
                            )
                            .changed()
                        {
                            if checked {
                                self.workspace.selected.insert(id.into());
                            } else {
                                self.workspace.selected.remove(id);
                            }
                        }
                        ui.add_sized([25.0, 28.0], egui::Label::new(format!("{}", i + 1)));
                        if ui
                            .add_sized(
                                [name_width, 28.0],
                                egui::Button::new(r["name"].as_str().unwrap_or(id))
                                    .selected(self.workspace.matrix_current == id)
                                    .truncate(),
                            )
                            .clicked()
                        {
                            self.workspace.matrix_current = id.into();
                            self.workspace.result_details = Value::Null;
                            self.workspace.result_key.clear();
                        }
                        let state = if r["draft"] == true {
                            if zh {
                                "未保存"
                            } else {
                                "Draft"
                            }
                        } else if r["enabled"] == false {
                            if zh {
                                "已禁用"
                            } else {
                                "Disabled"
                            }
                        } else {
                            matrix_status_label(r["status"].as_str().unwrap_or("pending"), zh)
                        };
                        ui.add_sized([85.0, 28.0], egui::Label::new(state));
                        ui.add_sized(
                            [85.0, 28.0],
                            egui::Label::new(r["phase"].as_str().unwrap_or("—")),
                        );
                        ui.label(
                            r["elapsed_s"]
                                .as_f64()
                                .map(|n| format!("{n:.1}s"))
                                .unwrap_or_else(|| "—".into()),
                        );
                    });
                });
            }
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
            if let Some(error)=details["error"].as_str(){ui.colored_label(egui::Color32::LIGHT_RED,error);}
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
        });
    }
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
fn deliver(panel: &mut TestToolPanel, method: &str, data: Value) {
    let (tx, rx) = mpsc::channel();
    tx.send((method.into(), Ok(json!({"ok":true,"data":data}))))
        .unwrap();
    panel.workspace.rx = Some(rx);
    panel.workspace_poll(&egui::Context::default());
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
