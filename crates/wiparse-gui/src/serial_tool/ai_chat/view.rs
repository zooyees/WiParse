use super::*;

/// Fixed header and composer; only the conversation/dashboard scrolls.
impl SerialAiChat {
    pub(super) fn ready(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.enabled && !config.model.trim().is_empty())
    }

    fn readable(text: &str, lang: Lang) -> String {
        if let Some((zh, en)) = text.split_once("/ ") {
            let zh = zh.trim_end();
            if lang == Lang::Zh {
                let suffix = en.split_once(": ").map(|(_, suffix)| suffix);
                if !zh.contains(": ") {
                    return suffix.map_or_else(|| zh.into(), |suffix| format!("{zh}：{suffix}"));
                }
                return zh.into();
            }
            return en.into();
        }
        text.into()
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        state: &Snapshot,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();
        {
            let style = ui.style_mut();
            style.text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::proportional(ui_theme::FONT_BODY),
            );
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
        }
        let full = ui.available_rect_before_wrap();
        let _ = ui.allocate_exact_size(full.size(), egui::Sense::hover());
        let short = full.height() < 520.0;
        let header_h = 100.0_f32.min(full.height() * 0.38);
        let bottom_h = (if self.mode == Mode::Analyse {
            if short {132.0_f32} else {148.0_f32}
        } else {
            32.0_f32
        })
        .min((full.height() - header_h).max(0.0));
        let header =
            egui::Rect::from_min_max(full.min, egui::pos2(full.right(), full.top() + header_h));
        let bottom =
            egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - bottom_h), full.max);
        let body = egui::Rect::from_min_max(
            egui::pos2(full.left(), header.bottom() + 6.0),
            egui::pos2(
                full.right(),
                (bottom.top() - 6.0).max(header.bottom() + 6.0),
            ),
        );
        for (id, rect) in [("header", header), ("body", body), ("composer", bottom)] {
            ui_theme::scope_in_allocated_rect(
                ui,
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                    ui.set_min_size(rect.size());
                    ui.set_max_size(rect.size());
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                    match id {
                        "header" => self.header(ui, lang, t, state, &mut actions),
                        "body" => self.body(ui, lang, t, state, &mut actions),
                        _ => self.composer(ui, lang, t, state, short, &mut actions),
                    }
                },
            );
        }
        actions
    }

    fn header(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        state: &Snapshot,
        actions: &mut Vec<UiAction>,
    ) {
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        if let Some(run)=self.run.as_mut(){
            if let Some((key,description))=run.pending_readback.clone(){
                ui.group(|ui|{
                    ui.label(format!("设备动作结果待核对 · {description}"));
                    if ui.button(label(lang,"已核对状态，允许再次提交此操作","State checked; allow this operation again")).clicked(){
                        if let Some(j)=&run.journal {j.record(&run.id,"serial",&run.question,"readback",&run.workflow,Some(("device.readback",&serde_json::json!({"state":"readback_acknowledged","command_fingerprint":key}))));}
                        run.uncertain.retain(|k|k!=&key);
                        run.pending_readback=None;
                    }
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(label(lang, "串口 AI", "Serial AI"))
                    .strong()
                    .size(ui_theme::FONT_TITLE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized([28.0, 28.0], egui::Button::new("×"))
                    .on_hover_text(label(
                        lang,
                        "关闭窗口；串口继续接收",
                        "Close window; reception continues",
                    ))
                    .clicked()
                {
                    actions.push(UiAction::Close);
                }
                ui.menu_button("…", |ui| {
                    if self.working() && self.run.is_some() && ui.add_enabled(!self.prompt.trim().is_empty(),egui::Button::new(label(lang,"用输入内容纠正当前任务","Steer current task with draft"))).clicked(){
                        actions.push(UiAction::Correct(self.prompt.clone()));ui.close_menu();
                    }
                    if !self.working(){if let Some(j)=crate::ai::journal::Journal::for_project(ui.ctx(),&state.binding.project){if let Some(recovery)=j.menu(ui){self.prompt=recovery.draft.clone();self.recovery=Some(recovery);}}}
                    ui.menu_button(label(lang,"日志选项与预处理","Log options and processing"),|ui|{ui.set_width((ui.ctx().screen_rect().width()-24.0).clamp(180.0,260.0));self.library_controls(ui,lang,actions);});
                    if ui
                        .button(label(lang, "AI 服务设置…", "AI service settings…"))
                        .clicked()
                    {
                        actions.push(UiAction::Settings);
                        ui.close_menu();
                    }
                    if ui
                        .button(label(
                            lang,
                            "新对话（停止AI监控）",
                            "New chat (stop AI watch)",
                        ))
                        .clicked()
                    {
                        actions.push(UiAction::Clear);
                        ui.close_menu();
                    }
                });
                let busy=self.working();
                let color = if busy {t.accent}else if self.ready() { t.success } else { t.warning };
                ui.colored_label(
                    color,
                    if busy {label(lang,"处理中","Working")} else if self.ready() {
                        label(lang, "已就绪", "Ready")
                    } else {
                        label(lang, "未配置", "Setup needed")
                    },
                )
                .on_hover_text(self.config.as_ref().map(|c| c.model.as_str()).unwrap_or(""));
            });
        });
        ui.horizontal(|ui| {
            let color = if state.connection_confirmed {
                t.success
            } else {
                t.text_muted
            };
            ui.colored_label(
                color,
                if !state.binding.live {
                    label(lang, "文件", "File")
                } else if state.connection_confirmed {
                    label(lang, "接收中", "Receiving")
                } else if state.reception_requested {
                    label(lang, "连接中", "Connecting")
                } else {
                    label(lang, "已停止", "Stopped")
                },
            );
            let source = if state.binding.live {
                state
                    .binding
                    .device
                    .as_deref()
                    .unwrap_or(&state.source_title)
            } else {
                &state.source_title
            };
            ui.add(egui::Label::new(source).truncate())
                .on_hover_text(format!(
                    "{}\n{}\n{}",
                    state.binding.project,
                    state.source_title,
                    Self::readable(&state.status, lang)
                ));
        });
        ui.horizontal(|ui| {
            let width = ((ui.available_width() - 8.0) / 2.0).max(1.0);
            for (mode, name) in [
                (Mode::Analyse, label(lang, "智能对话", "Assistant")),
                (Mode::Monitor, label(lang, "监控任务", "Monitor task")),
            ] {
                if ui
                    .add_sized(
                        [width, 28.0],
                        egui::Button::new(egui::RichText::new(name).size(13.0).color(
                            if self.mode == mode {
                                t.accent
                            } else {
                                t.text_muted
                            },
                        ))
                        .fill(if self.mode==mode{t.accent_soft}else{t.header_bg}).stroke(Stroke::new(1.0_f32,t.border)),
                    )
                    .clicked()
                {
                    self.mode = mode;
                    ui.ctx().request_repaint();
                }
            }
        });
        ui.separator();
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        state: &Snapshot,
        actions: &mut Vec<UiAction>,
    ) {
        let mode = self.mode;
        egui::ScrollArea::vertical().id_salt(("serial-ai-conversation",mode==Mode::Monitor)).auto_shrink([false,false]).max_height(ui.available_height()).stick_to_bottom(mode==Mode::Analyse).show(ui,|ui|{
            ui.set_width(ui.available_width());
            if !self.ready(){
                ui.label(egui::RichText::new(label(lang,"连接 AI 服务后开始对话","Connect an AI service to start")).size(ui_theme::FONT_BODY));
                if ui.button(label(lang,"配置 AI 服务","Configure AI service")).clicked(){actions.push(UiAction::Settings);}
                ui.add_space(12.0);
            }
            if mode==Mode::Monitor {self.monitor_controls(ui,lang,t,state,actions);ui.add_space(12.0);}
            if let Some(run)=&self.run{crate::ai::journal::show_progress(ui,&run.workflow);}
            if self.usage_total.responses>0||self.usage_total.failed_attempts>0{egui::CollapsingHeader::new(label(lang,"AI用量（服务报告）","AI usage (provider reported)")).id_salt("ai-usage").show(ui,|ui|{if let Some(run)=&self.run{ui.add(egui::Label::new(format!("{} {}",label(lang,"本任务","Task"),run.usage.summary())).wrap());}ui.add(egui::Label::new(egui::RichText::new(format!("{} {}",label(lang,"本会话","Session"),self.usage_total.summary())).size(12.0).color(t.text_muted)).wrap());});}
            let activities:Vec<_>=self.entries.iter().filter(|e|e.role=="host"&&!e.error).collect();
            if !activities.is_empty(){
                egui::CollapsingHeader::new(egui::RichText::new(format!("{} · {}",label(lang,"活动记录","Activity"),activities.len())).size(12.0).color(t.text_muted)).id_salt("chat-activities").show(ui,|ui|{
                    for entry in &activities {ui.add(egui::Label::new(egui::RichText::new(format!("{}  {}",entry.time,Self::readable(&entry.text,lang))).size(12.0).color(t.text_muted)).wrap());}
                });ui.add_space(10.0);
            }
            let visible=self.entries.iter().any(|e|e.role!="host"&&(mode==Mode::Analyse||e.automatic));
            if !visible && mode==Mode::Analyse && self.ready(){
                ui.add_space(6.0);ui.label(egui::RichText::new(label(lang,"今天要检查什么？","What would you like to check?")).size(ui_theme::FONT_TITLE).strong());
                ui.add(egui::Label::new(egui::RichText::new(label(lang,"分析日志、设定监控目标，或让 AI 调用平台工具。","Analyse logs, set a monitoring goal, or use platform tools.")).size(ui_theme::FONT_BODY).color(t.text_muted)).wrap());
                ui.add_space(12.0);
                for (title,question) in [(label(lang,"分析日志异常","Analyse log anomalies"),label(lang,"分析当前日志中的异常，列出证据和下一步检查。","Analyse current log anomalies with evidence and next checks.")),(label(lang,"设置监控与预警","Set up monitoring"),label(lang,"帮我设置当前串口的监控与预警，先读取并识别数据字段，明确保护性阈值缺失时再询问。","Help configure monitoring and alerts; first inspect the fields; ask only for missing protection thresholds."))]{if ui.add_sized([ui.available_width(),30.0],egui::Button::new(title).fill(t.header_bg).stroke(Stroke::new(1.0_f32,t.border))).clicked(){self.prompt=question.into();}}
            }
            for(index,entry)in self.entries.iter().enumerate().filter(|(_,e)|(e.role!="host"||e.error)&&(mode==Mode::Analyse||e.role=="host"||e.role=="alert"||e.automatic)){
                ui.push_id(index,|ui|{
                    let width=ui.available_width();
                    match entry.role {
                        "host"=>{ui.add(egui::Label::new(egui::RichText::new(Self::readable(&entry.text,lang)).size(12.0).color(t.warning)).wrap());}
                        "tool"=>{egui::CollapsingHeader::new(egui::RichText::new(Self::readable(entry.text.lines().next().unwrap_or("Platform call").trim_start_matches('#').trim(),lang)).size(12.0).color(if entry.error {t.warning}else{t.text_muted})).id_salt("tool").show(ui,|ui|entry.markdown.ui_chat(ui,t,lang)); if let Some(data)=&entry.timeline{timeline_view::show(ui,data,lang,t);}}
                        "alert"=>{Frame::NONE.fill(t.warning.gamma_multiply(0.08)).corner_radius(8).inner_margin(12).show(ui,|ui|{ui.set_width((width-24.0).max(1.0));ui.label(egui::RichText::new(label(lang,"监控预警","Monitoring alert")).size(13.0).strong().color(t.warning));ui.add(egui::Label::new(egui::RichText::new(&entry.text).size(ui_theme::FONT_BODY)).wrap());});}
                        "user" => {
                            Frame::NONE.fill(t.header_bg).corner_radius(8).inner_margin(10).show(ui, |ui| {
                                ui.set_width((width - 20.0).max(1.0));
                                message_header(ui, lang, t, &entry.time, true, None);
                                ui.add_space(4.0);
                                ui.add(egui::Label::new(egui::RichText::new(&entry.text)
                                    .size(ui_theme::FONT_BODY)).wrap().selectable(true));
                            });
                        }
                        _ => {
                            if mode == Mode::Analyse && entry.automatic {
                                egui::CollapsingHeader::new(egui::RichText::new(label(lang,"监控分析","Monitoring analysis"))
                                    .size(ui_theme::FONT_BODY).color(t.text_muted)).id_salt("watch")
                                    .show(ui, |ui| entry.markdown.ui_chat(ui,t,lang));
                            } else {
                                Frame::NONE.inner_margin(10).show(ui, |ui| {
                                    ui.set_width((width - 20.0).max(1.0));
                                    message_header(ui, lang, t, &entry.time, false, Some(&entry.text));
                                    ui.add_space(4.0);
                                    entry.markdown.ui_chat(ui,t,lang);
                                });
                            }
                        }
                    }
                    ui.add_space(10.0);
                });
            }
            if self.status_error&&!self.entries.last().is_some_and(|e|e.role=="host"&&e.error&&e.text==self.status){ui.add(egui::Label::new(egui::RichText::new(Self::readable(&self.status,lang)).size(13.0).color(t.warning)).wrap());}
            if let Some(notice)=&self.queue_notice{ui.add(egui::Label::new(egui::RichText::new(Self::readable(notice,lang)).size(13.0).color(t.warning)).wrap());}
            if let Some(pending) = &self.pending { if let Some(preview) = &pending.preview {
                ui.push_id("streaming-answer", |ui| {
                    ui.label(egui::RichText::new(label(lang,"AI · 正在生成（尚未完成）","AI · Generating (not final)")).size(12.0).color(t.text_muted));
                    preview.ui_chat(ui,t,lang);
                });
            }}
            self.queue_ui(ui,lang,t,actions);
            if self.pending.as_ref().is_some_and(|p|mode==Mode::Analyse||p.automatic){ui.horizontal(|ui|{ui.spinner();ui.label(egui::RichText::new(label(lang,"正在分析…","Analysing…")).size(13.0).color(t.text_muted));});}
        });
    }

    fn monitor_controls(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        state: &Snapshot,
        actions: &mut Vec<UiAction>,
    ) {
        let running = self.watch.is_some();
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            ui.label(
                egui::RichText::new(if running {
                    label(lang, "监控运行中", "Monitoring active")
                } else {
                    label(lang, "监控未开启", "Monitoring off")
                })
                .strong()
                .color(if running { t.success } else { t.text_primary }),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        running
                            || (self.ready() && state.binding.live && state.connection_confirmed),
                        egui::Button::new(if running {
                            label(lang, "停止监控", "Stop watch")
                        } else {
                            label(lang, "开始监控", "Start watch")
                        })
                        .min_size(egui::vec2(88.0, 28.0)),
                    )
                    .clicked()
                {
                    actions.push(UiAction::ToggleWatch);
                }
            });
        });
        if !state.binding.live || !state.connection_confirmed {
            ui.add(egui::Label::new(egui::RichText::new(label(lang,"实时监控需要先连接串口；文件日志可直接在对话中分析。","Connect serial reception for live monitoring. Log files can be analysed in the conversation.")).color(t.text_muted)).wrap());
        }
            if let Some(snapshot)=self.watch.as_ref().and_then(|watch|watch.first_trigger.as_ref()) {
                ui.collapsing(label(lang,"首个告警与触发前日志","First alert and preceding log"),|ui|{
                    ui.add(egui::Label::new(snapshot.as_str()).wrap().selectable(true));
                });
            }
        ui.label(egui::RichText::new(label(lang, "任务目标", "Task goal")).strong());
        if running {
            let width = ui.available_width();
            Frame::NONE
                .fill(t.header_bg)
                .corner_radius(6)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width((width - 20.0).max(1.0));
                    let goal = self
                        .watch
                        .as_ref()
                        .map(|w| w.goal.as_str())
                        .unwrap_or(&self.watch_goal);
                    ui.add(egui::Label::new(goal).wrap());
                });
        } else {
            ui.add(egui::TextEdit::multiline(&mut self.watch_goal).desired_width(f32::INFINITY).desired_rows(3).char_limit(2048).hint_text(label(lang,"可在对话中描述监控目标与阈值，例如 VIN 低于4.5 V时预警。","Describe the goal and limits in chat, e.g. alert when VIN falls below 4.5 V.")));
        }
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(80.0, 28.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(label(lang, "AI 分析间隔", "AI interval"));
                },
            );
            let width = ui.available_width().max(40.0);
            egui::ComboBox::from_id_salt("serial-ai-interval")
                .width(width)
                .selected_text(format!("{} s", self.interval_seconds))
                .show_ui(ui, |ui| {
                    for n in [1, 2, 3, 5, 10, 15, 30, 60, 120, 300] {
                        ui.selectable_value(&mut self.interval_seconds, n, format!("{n} s"));
                    }
                });
        });
        let received = self.watch.as_ref().map_or(0, |w| w.stats.received_lines);
        let width = ((ui.available_width() - 16.0) / 3.0).max(1.0);
        ui.horizontal(|ui| {
            for (name, value) in [
                (label(lang, "新增日志", "New lines"), received),
                (label(lang, "分析轮次", "Reports"), self.reports),
                (label(lang, "本地预警", "Local alerts"), self.alert_count),
            ] {
                Frame::NONE
                    .fill(t.header_bg)
                    .corner_radius(6)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width((width - 16.0).max(1.0));
                        ui.set_min_height(42.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name).small().color(t.text_muted),
                                )
                                .truncate(),
                            );
                            ui.label(
                                egui::RichText::new(value.to_string())
                                    .size(ui_theme::FONT_TITLE)
                                    .strong(),
                            );
                        });
                    });
            }
        });
        if self.watch.as_ref().is_some_and(|w| w.ai_paused) {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(label(
                        lang,
                        "AI分析已暂停，本地规则仍在预警",
                        "AI analysis paused; local alerts remain active",
                    ))
                    .color(t.warning),
                )
                .wrap(),
            );
            if ui
                .add_enabled(
                    self.ready(),
                    egui::Button::new(label(lang, "恢复 AI 分析", "Resume AI analysis")),
                )
                .clicked()
            {
                actions.push(UiAction::ResumeAi);
            }
        }
        if let Some(watch) = &mut self.watch {
            let interval = Duration::from_secs(self.interval_seconds);
            if watch.interval != interval {
                watch.interval = interval;
                watch.next = (watch.last_request + interval).max(Instant::now());
            }
            let status = if watch.ai_paused {
                label(lang, "本地规则监控运行中", "Local rule monitoring active").into()
            } else if self.deferred.is_some(){
            label(lang,"等待AI资源，自动继续…","Waiting for AI capacity…").into()
        } else if self.tool.is_some() {
                label(
                    lang,
                    "正在等待平台任务完成",
                    "Waiting for platform completion",
                )
                .into()
            } else if self.pending.is_some() {
                label(lang, "AI 正在分析", "AI is analysing").into()
            } else if watch.stats.received_lines == 0 {
                label(lang, "等待新增日志", "Waiting for new log data").into()
            } else if watch.next <= Instant::now() {
                label(lang, "准备提交新增数据", "Preparing the next sample").into()
            } else {
                format!(
                    "{} {} s",
                    label(lang, "预计下次分析", "Next analysis in"),
                    watch
                        .next
                        .saturating_duration_since(Instant::now())
                        .as_secs()
                )
            };
            ui.add(egui::Label::new(egui::RichText::new(status).color(t.text_muted)).wrap());
        }
        egui::CollapsingHeader::new(label(
            lang,
            "预警规则与设备控制",
            "Alert rules and device control",
        ))
        .id_salt("monitor-rules")
        .default_open(true)
        .show(ui, |ui| {
            for limit in &self.rules.limits {
                ui.label(format!(
                    "{}: {} ~ {}",
                    limit.key,
                    limit.min.map_or("−∞".into(), |v| v.to_string()),
                    limit.max.map_or("+∞".into(), |v| v.to_string())
                ));
            }
            ui.add(egui::Label::new(self.rules.contains.join(" · ")).wrap());
            self.control_permission(ui, lang, t, state);
        });
        ui.add(egui::Label::new(egui::RichText::new(label(lang,"本地规则及时预警 · AI 异步分析","Local alerts · Asynchronous AI analysis")).small().color(t.text_muted)).wrap()).on_hover_text(label(lang,"每次最多100行与有限异常样例；预警优先提交，最短1秒一次；服务器慢时顺延且不排队。采样不等于完整实时检验，停止AI监控不会停止串口。","At most 100 lines plus bounded anomaly examples. Alerts prioritise analysis, at most once per second, delayed by slow responses without a backlog. Not exhaustive real-time verification. Stopping watch keeps serial reception active."));
    }

    fn control_permission(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens, state: &Snapshot) {
        if state.automation_locked {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(label(
                        lang,
                        "测试运行中，串口控制已锁定",
                        "Test running; serial controls locked",
                    ))
                    .small()
                    .color(t.warning),
                )
                .wrap(),
            );
        } else {
            ui.checkbox(&mut self.allow_automation,label(lang,"允许平台设备控制","Allow platform device control")).on_hover_text(label(lang,"允许串口接收启停和已连接仪器的类型化控制。切换数据源后需重新开启。","Allow current reception and typed control of connected instruments. Resets when the source changes."));
        }
    }

    fn library_controls(&mut self, ui:&mut egui::Ui, lang:Lang, actions:&mut Vec<UiAction>) {
        let busy=self.working();
        ui.spacing_mut().item_spacing=egui::vec2(6.,6.);
        let width=ui.available_width();
        ui.add_enabled_ui(!busy,|ui|{
            if self.scope==DataScope::Range {
                let old=(self.range_first,self.range_last);
                ui.label(label(lang,"行范围（含首尾）","Rows (inclusive)"));
                ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.range_first).range(1..=u64::MAX));ui.label("–");ui.add(egui::DragValue::new(&mut self.range_last).range(self.range_first..=u64::MAX));});
                if old!=(self.range_first,self.range_last){self.cancel();self.queue_paused=!self.messages.is_empty();self.run=None;self.library.reset();self.tool_results.clear();self.history.clear();}
            }
            let root=if self.scope==DataScope::Library{self.library.directory.as_ref().map(|path|path.display().to_string()).unwrap_or_else(||label(lang,"已打开文件与已落盘记录","Opened files and saved capture").into())}else{label(lang,"当前日志或选区，在本地处理","Current log or selection, processed locally").into()};
            ui.add_sized([width,20.],egui::Label::new(&root).truncate()).on_hover_text(&root);
            if ui.add_sized([width,26.],egui::Button::new(label(lang,"选择日志目录…","Choose log directory…"))).clicked(){actions.push(UiAction::ChooseLogDirectory);ui.close_menu();}
            if self.library.directory.is_some()&&self.scope==DataScope::Library&&ui.add_sized([width,24.],egui::Button::new(label(lang,"清除所选目录","Clear selected directory"))).clicked(){actions.push(UiAction::ClearLogDirectory);ui.close_menu();}
            ui.separator();
            ui.horizontal(|ui|{
                ui.label(label(lang,"每批扫描","Scan budget"));
                egui::ComboBox::from_id_salt("serial-ai-local-budget").selected_text(format!("{} MiB",self.library.budget_mib)).width(90.).show_ui(ui,|ui|{for mib in [64,256,1024,4096]{ui.selectable_value(&mut self.library.budget_mib,mib,format!("{mib} MiB"));}});
            });
            let continuing=self.library.preview_params().get("cursor").is_some();
            if ui.add_sized([width,28.],egui::Button::new(if continuing{label(lang,"继续本地扫描","Continue local scan")}else{label(lang,"本地预分析","Local preview")})).clicked(){actions.push(UiAction::PreviewLogs);ui.close_menu();}
        });
        ui.add(egui::Label::new(egui::RichText::new(label(lang,"本地处理全范围，仅发送统计与证据。预分析不消耗token。实时历史全量请选落盘文件；不含未保存编辑。","Full scope stays local; AI receives statistics and evidence. Preview uses no tokens. For full live history select the recorded file. Unsaved edits excluded.")).size(ui_theme::FONT_CAPTION)).wrap());
    }

    fn composer(&mut self,ui:&mut egui::Ui,lang:Lang,t:&Tokens,state:&Snapshot,short:bool,actions:&mut Vec<UiAction>){
        ui.separator();
        if self.mode==Mode::Monitor {
            if ui.add_sized([ui.available_width(),28.0],egui::Button::new(label(lang,"返回对话，调整任务或继续分析","Back to conversation"))).clicked(){self.mode=Mode::Analyse;ui.ctx().request_repaint();}return;
        }
        let width=ui.available_width();
        Frame::NONE.fill(t.input_bg).stroke(Stroke::new(1.0_f32,t.border)).corner_radius(10).inner_margin(8).show(ui,|ui|{
            ui.set_width((width-16.0).max(1.0));
            ui.spacing_mut().item_spacing=egui::vec2(6.0,6.0);
            ui.horizontal(|ui|{
                let old=self.scope;
                egui::ComboBox::from_id_salt("serial-ai-data-scope").width((ui.available_width()*0.62).clamp(70.0,164.0)).truncate().selected_text(match self.scope{
                    DataScope::Current=>label(lang,"当前日志 · 全范围","Current log · all rows"),DataScope::Range=>label(lang,"指定行范围","Row range"),DataScope::Selection=>label(lang,"选中内容","Selected text"),DataScope::None=>label(lang,"仅提问","Question only"),DataScope::Library=>label(lang,"本地日志库","Local logs")
                }).show_ui(ui,|ui|{for (scope,title) in [(DataScope::Current,label(lang,"当前日志 · 全范围","Current log · all rows")),(DataScope::Selection,label(lang,"选中内容","Selected text")),(DataScope::Range,label(lang,"指定行范围（本地处理）","Row range (local)")),(DataScope::Library,label(lang,"本地日志库（本地扫描）","Local logs (local scan)")),(DataScope::None,label(lang,"仅提问，不附日志","Question only, no log"))]{ui.selectable_value(&mut self.scope,scope,title);}});
                if old!=self.scope{self.cancel();self.queue_paused=!self.messages.is_empty();self.run=None;self.library.reset();self.tool_results.clear();self.history.clear();}
                let count=match self.scope{DataScope::Current=>format!("{} {}",state.retained_lines,label(lang,"行","lines")),DataScope::Range=>format!("{}–{}",self.range_first,self.range_last),DataScope::Selection=>format!("{} B",state.selected_bytes),DataScope::Library=>label(lang,"本地处理","Local processing").into(),_=>String::new()};
                ui.add(egui::Label::new(egui::RichText::new(count).size(12.0).color(t.text_muted)).truncate()).on_hover_text(self.config.as_ref().map(|c|c.model.as_str()).unwrap_or(""));
            });
            let id=ui.make_persistent_id("serial-ai-question");
            let shortcut=ui.memory(|m|m.has_focus(id))&&ui.input_mut(|i|i.consume_key(egui::Modifiers::CTRL,egui::Key::Enter));
            let working=self.working();
            ui.add_sized([ui.available_width(),if short{32.0}else{48.0}],egui::TextEdit::multiline(&mut self.prompt).id(id).font(egui::FontId::proportional(ui_theme::FONT_BODY)).frame(false).desired_rows(2).desired_width(f32::INFINITY).margin(egui::vec2(0.0,4.0)).char_limit(4096).hint_text(if working{label(lang,"继续输入，发送后排队…","Continue typing; send to queue…")}else{label(lang,"输入问题或测试目标…","Ask a question or describe a test…")}));
            ui.horizontal(|ui|{
                let old=self.allow_automation;
                let narrow=ui.available_width()<210.0;
                ui.add_enabled(!state.automation_locked,egui::Checkbox::new(&mut self.allow_automation,if narrow{label(lang,"控制","Control")}else{label(lang,"允许控制","Allow control")})).on_hover_text(label(lang,"仅授权当前串口和已连接设备；撤销会影响当前任务及排队消息，重新勾选不会给旧消息补授权。","Current serial and connected devices only. Revocation affects active and queued work; re-enabling does not grant older messages new permissions."));
                if old&&!self.allow_automation{self.revoke_controls();}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    let busy=self.working();let queued=busy||(!self.messages.is_empty()&&!self.queue_paused);
                    let can_send=self.ready()&&!self.prompt.trim().is_empty()&&(self.scope!=DataScope::Selection||state.selected_bytes>0)&&(!queued||self.messages.len()<16);
                    let title=if queued{label(lang,"排队发送","Queue")}else{label(lang,"发送","Send")};
                    if ui.add_enabled(can_send,egui::Button::new(egui::RichText::new(title).color(t.accent_text)).min_size(egui::vec2(76.0,28.0)).fill(t.accent_fill)).on_hover_text(label(lang,"Ctrl+Enter；分析中可继续提交，消息依次执行。","Ctrl+Enter; messages submitted during analysis run in order.")).clicked()||(shortcut&&can_send){actions.push(UiAction::Send(self.prompt.clone()));}
                    if busy&&ui.add_sized([28.0,28.0],egui::Button::new("■")).on_hover_text(label(lang,"取消当前任务并暂停队列","Cancel current task and pause queue")).clicked(){actions.push(UiAction::Cancel);}
                    if !busy&&ui.available_width()>80.0 {if let Some(retry)=&self.retry{if ui.small_button(label(lang,"重试","Retry")).clicked(){actions.push(UiAction::Send(retry.clone()));}}}
                });
            });
        });
    }

}

/// Every message uses the same content inset and metadata baseline.
fn message_header(
    ui: &mut egui::Ui,
    lang: Lang,
    t: &Tokens,
    time: &str,
    user: bool,
    text: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new(if user {
                label(lang, "你", "You")
            } else {
                "AI"
            })
            .size(ui_theme::FONT_BODY)
            .strong()
            .color(t.text_muted),
        );
        ui.label(
            egui::RichText::new(time)
                .size(ui_theme::FONT_CAPTION)
                .color(t.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(text) = text {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(label(lang, "复制", "Copy"))
                                .size(ui_theme::FONT_CAPTION),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    ui.ctx().copy_text(text.to_owned());
                }
            }
        });
    });
}
