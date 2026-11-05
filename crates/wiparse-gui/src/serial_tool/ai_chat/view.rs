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
        let header_h = 100.0_f32.min(full.height() * 0.30);
        let bottom_h = (if self.mode == Mode::Analyse {
            if short {
                132.0
            } else {
                148.0
            }
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
                let color = if self.ready() { t.success } else { t.warning };
                ui.colored_label(
                    color,
                    if self.ready() {
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
                        .frame(false),
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
            let activities:Vec<_>=self.entries.iter().filter(|e|e.role=="host"&&!e.error).collect();
            if !activities.is_empty(){
                egui::CollapsingHeader::new(egui::RichText::new(format!("{} · {}",label(lang,"活动记录","Activity"),activities.len())).size(12.0).color(t.text_muted)).id_salt("chat-activities").show(ui,|ui|{
                    for entry in &activities {ui.add(egui::Label::new(egui::RichText::new(format!("{}  {}",entry.time,Self::readable(&entry.text,lang))).size(12.0).color(t.text_muted)).wrap());}
                });ui.add_space(10.0);
            }
            let visible=self.entries.iter().any(|e|e.role!="host"&&(mode==Mode::Analyse||e.automatic));
            if !visible && mode==Mode::Analyse && self.ready(){
                ui.add_space(16.0);ui.label(egui::RichText::new(label(lang,"今天要检查什么？","What would you like to check?")).size(ui_theme::FONT_TITLE).strong());
                ui.add(egui::Label::new(egui::RichText::new(label(lang,"分析日志、设定监控目标，或让 AI 调用平台工具。","Analyse logs, set a monitoring goal, or use platform tools.")).size(ui_theme::FONT_BODY).color(t.text_muted)).wrap());
                ui.add_space(12.0);
                for (title,question) in [(label(lang,"分析日志异常","Analyse log anomalies"),label(lang,"分析当前日志中的异常，列出证据和下一步检查。","Analyse current log anomalies with evidence and next checks.")),(label(lang,"设置监控与预警","Set up monitoring"),label(lang,"帮我设置当前串口的监控与预警，先确认需要关注的数据字段和阈值。","Help configure monitoring and alerts; first confirm the fields and thresholds."))]{if ui.add_sized([ui.available_width(),30.0],egui::Button::new(title).frame(false)).clicked(){self.prompt=question.into();}}
            }
            for(index,entry)in self.entries.iter().enumerate().filter(|(_,e)|(e.role!="host"||e.error)&&(mode==Mode::Analyse||e.role=="host"||e.role=="alert"||e.automatic)){
                ui.push_id(index,|ui|{
                    let width=ui.available_width();
                    match entry.role {
                        "host"=>{ui.add(egui::Label::new(egui::RichText::new(Self::readable(&entry.text,lang)).size(12.0).color(t.warning)).wrap());}
                        "tool"=>{egui::CollapsingHeader::new(egui::RichText::new(Self::readable(entry.text.lines().next().unwrap_or("Platform call"),lang)).size(12.0).color(if entry.error {t.warning}else{t.text_muted})).id_salt("tool").show(ui,|ui|entry.markdown.ui_chat(ui,t,lang));}
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

    fn composer(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        t: &Tokens,
        state: &Snapshot,
        short: bool,
        actions: &mut Vec<UiAction>,
    ) {
        ui.separator();
        if self.mode == Mode::Monitor {
            if ui
                .add_sized(
                    [ui.available_width(), 28.0],
                    egui::Button::new(label(
                        lang,
                        "返回对话，调整任务或继续分析",
                        "Back to conversation",
                    )),
                )
                .clicked()
            {
                self.mode = Mode::Analyse;
                ui.ctx().request_repaint();
            }
            return;
        }
        let status = if self.status_error {
            Self::readable(&self.status, lang)
        } else if self.tool.is_some() {
            label(lang, "平台任务执行中…", "Platform task running…").into()
        } else if self.pending.is_some() {
            label(lang, "AI 正在分析…", "AI is analysing…").into()
        } else {
            self.config
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default()
        };
        let frame_width = ui.available_width();
        Frame::NONE
            .fill(t.input_bg)
            .stroke(Stroke::new(1.0_f32, t.border))
            .corner_radius(10)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width((frame_width - 20.0).max(1.0));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("serial-ai-data-scope")
                        .width((ui.available_width() - 62.0).clamp(80.0, 150.0))
                        .truncate()
                        .selected_text(match self.scope {
                            DataScope::Recent => label(lang, "最近100行", "Recent 100 lines"),
                            DataScope::Selection => label(lang, "选中内容", "Selected text"),
                            DataScope::None => label(lang, "仅提问", "Question only"),
                        })
                        .show_ui(ui, |ui| {
                            for (scope, name) in [
                                (
                                    DataScope::Recent,
                                    label(lang, "最近100行", "Recent 100 lines"),
                                ),
                                (
                                    DataScope::Selection,
                                    label(lang, "选中内容", "Selected text"),
                                ),
                                (
                                    DataScope::None,
                                    label(lang, "仅提问，不附日志", "Question only, no log"),
                                ),
                            ] {
                                ui.selectable_value(&mut self.scope, scope, name);
                            }
                        });
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(match self.scope {
                                DataScope::Recent => format!(
                                    "{} {}",
                                    state.absolute_lines.min(100),
                                    label(lang, "行", "lines")
                                ),
                                DataScope::Selection => format!("{} B", state.selected_bytes),
                                DataScope::None => String::new(),
                            })
                            .size(12.0)
                            .color(t.text_muted),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&status)
                                    .size(ui_theme::FONT_CAPTION)
                                    .color(if self.status_error {
                                        t.warning
                                    } else {
                                        t.text_muted
                                    }),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&status);
                    });
                });
                let id = ui.make_persistent_id("serial-ai-question");
                let shortcut = ui.memory(|m| m.has_focus(id))
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
                ui.add_sized(
                    [ui.available_width(), if short { 32.0 } else { 48.0 }],
                    egui::TextEdit::multiline(&mut self.prompt)
                        .id(id)
                        .font(egui::FontId::proportional(ui_theme::FONT_BODY))
                        .frame(false)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .margin(egui::vec2(0.0, 4.0))
                        .char_limit(4096)
                        .hint_text(label(
                            lang,
                            "输入问题或测试目标…",
                            "Ask a question or describe a test…",
                        )),
                );
                ui.horizontal(|ui| {
                    ui.add_enabled(!state.automation_locked,
                        egui::Checkbox::new(&mut self.allow_automation, label(lang,"允许控制","Allow control")))
                        .on_hover_text(label(lang,
                            "由你授权AI启停当前串口、控制已连接仪器；勾选后请重新发送。测试运行时锁定，AI不能自行授权。",
                            "Authorize AI to start/stop this serial source and control connected devices. Resend after enabling. Locked during tests; AI cannot grant itself permission."));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let busy = self.pending.is_some() || self.tool.is_some();
                        if busy {
                            if ui
                                .add_sized(
                                    [60.0, 28.0],
                                    egui::Button::new(label(lang, "停止", "Stop")),
                                )
                                .clicked()
                            {
                                actions.push(UiAction::Cancel);
                            }
                        } else {
                            let send = self.ready()
                                && !self.prompt.trim().is_empty()
                                && (self.scope != DataScope::Selection || state.selected_bytes > 0);
                            if ui
                                .add_enabled(
                                    send,
                                    egui::Button::new(label(lang, "发送", "Send"))
                                        .min_size(egui::vec2(60.0, 28.0))
                                        .fill(t.accent),
                                )
                                .clicked()
                                || (shortcut && send)
                            {
                                actions.push(UiAction::Send(self.prompt.clone()));
                            }
                            if let Some(retry) = &self.retry {
                                if ui
                                    .add(
                                        egui::Button::new(label(lang, "重试", "Retry"))
                                            .frame(false),
                                    )
                                    .clicked()
                                {
                                    actions.push(UiAction::Send(retry.clone()));
                                }
                            }
                        }
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
