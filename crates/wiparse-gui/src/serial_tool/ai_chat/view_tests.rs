use super::*;

fn panel() -> SerialToolPanel {
    let mut config = AppConfig::default();
    config.log_monitor.save_live_to_disk = false;
    config.log_monitor.open_log_files.clear();
    let mut panel = SerialToolPanel::new(&config, Lang::Zh);
    panel.ai_chat.set_context(
        "layout-project".into(),
        &AiConfig {
            enabled: true,
            model: "mock-model".into(),
            ..Default::default()
        },
    );
    panel.ai_chat.open = true;
    panel
}

fn text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                if t.galley.text() == text {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("Missing text: {text}"))
}

#[test]
fn short_and_narrow_panes_keep_composer_visible_and_outside_conversation() {
    for width in [260., 360., 520.] {
        for height in [320., 500., 800.] {
            for lang in [Lang::Zh, Lang::En] {
                for light in [false, true] {
                    let mut p = panel();
                    let state = p.ai_state("layout-project", false);
                    p.ai_chat.entry(
                        "assistant",
                        "## 分析结果\n\n长段落用于检查布局。\n".repeat(30),
                    );
                    p.ai_chat.prompt = "分析异常".into();
                    let ctx = egui::Context::default();
                    crate::fonts::install_cjk_fonts(&ctx);
                    let t = ui_theme::apply_theme(&ctx, light);
                    let out = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, height),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                p.ai_chat.ui(ui, lang, &t, &state);
                            });
                        },
                    );
                    let send = text_rect(&out, label(lang, "发送", "Send"));
                    assert!(
                        send.left() >= 0.0 && send.right() <= width && send.bottom() <= height,
                        "composer outside {width}x{height}: {send:?}"
                    );
                    let analysis = text_rect(&out, label(lang, "智能对话", "Assistant"));
                    assert!(analysis.bottom() < send.top());
                    let control = text_rect(&out, label(lang, "允许控制", "Allow control"));
                    assert!(control.right() <= width && control.bottom() <= height);
                }
            }
        }
    }
}

#[test]
fn chat_typography_and_activity_match_a_narrow_conversation_not_a_report() {
    for light in [false, true] {
        let mut p = panel();
        let state = p.ai_state("layout-project", false);
        p.ai_chat.note("AI配置已更新 / AI settings updated", false);
        p.ai_chat.note("AI配置已更新 / AI settings updated", false);
        assert_eq!(
            p.ai_chat
                .entries
                .iter()
                .filter(|e| e.role == "host")
                .count(),
            1
        );
        p.ai_chat.entry("user", "检查确认你的功能和能力");
        p.ai_chat.entry("assistant","# 能力与当前状态确认\n\n我可以分析串口数据，并按授权调用平台工具。\n\n- 解析 Qi ASK/FSK 行与 AA55 指标帧\n- 监控数据并提供证据\n\n| 项目 | 状态 |\n|---|---|\n| AI分析 | 已暂停（`ai_monitoring: false`） |\n| 串口源 | `tcp://127.0.0.1:52787` |\n\n```text\nVIN=4.20 V\n```\n\n未确认的物理状态不会作为测试结论。");
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let t = ui_theme::apply_theme(&ctx, light);
        let size = egui::vec2(452., 930.);
        let mut out = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    p.ai_chat.ui(ui, Lang::Zh, &t, &state);
                });
            },
        );
        let texts = out
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    Some(t)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(
            !texts
                .iter()
                .any(|s| s.galley.text().contains("AI配置已更新")),
            "info activity should be folded"
        );
        for text in &texts {
            for section in &text.galley.job.sections {
                assert!(
                    (12.0..=14.0).contains(&section.format.font_id.size),
                    "Unexpected chat font: {} ({})",
                    section.format.font_id.size,
                    text.galley.text()
                );
            }
        }
        let send = text_rect(&out, "发送");
        assert!(send.bottom() < 930.0);
        assert!(text_rect(&out, "已就绪").bottom() < 100.0); // model remains available as header/scope tooltip
        let question = text_rect(&out, "检查确认你的功能和能力");
        let answer = text_rect(&out, "我可以分析串口数据，并按授权调用平台工具。");
        assert!(
            (question.left() - answer.left()).abs() < 0.5,
            "Conversation content must share an inset"
        );
        out.textures_delta
            .set
            .retain(|(id, _)| *id != egui::TextureId::Managed(0));
        out.textures_delta.set.push((
            egui::TextureId::Managed(0),
            egui::epaint::ImageDelta::full(ctx.fonts(|f| f.image()), egui::TextureOptions::LINEAR),
        ));
        crate::ui_audit::export_frame(
            &ctx,
            out,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-chat-polish-ui"),
            &format!("chat-452-{}", if light { "light" } else { "dark" }),
            size,
        );
    }
}

#[test]
fn data_scope_never_falls_back_to_recent_logs_for_empty_selection() {
    let mut p = panel();
    p.tabs[0].append_lines(vec!["private serial data".into()], true);
    let none = p.ai_scoped_observation(DataScope::None).unwrap();
    assert!(none.get("recent_lines").is_none());
    assert_eq!(none["read_authorized_for_selected_scope"], false);
    assert_eq!(none["raw_logs_uploaded"], false);
    assert!(p.ai_scoped_observation(DataScope::Selection).is_err());
    let current = p.ai_scoped_observation(DataScope::Current).unwrap();
    assert!(current.get("recent_lines").is_none());
    assert_eq!(current["raw_logs_uploaded"], false);
    assert_eq!(current["read_authorized_for_selected_scope"], true);
    p.tabs[0].live = false;
    p.primary_device = Some("COM-test".into());
    let state = p.ai_state("layout-project", false);
    assert!(state.binding.device.is_none());
    assert!(!state.reception_requested);
    assert!(state.status.contains("Offline"));
}

#[test]
fn source_change_clears_old_context_and_control_authorization() {
    let mut p = panel();
    let cfg = p.ai_chat.config.clone().unwrap();
    let ctx = egui::Context::default();
    p.tick_ai(&ctx, Lang::Zh, "layout-project".into(), &cfg, false);
    p.ai_chat.allow_automation = true;
    p.ai_chat.entry("assistant", "old source analysis");
    p.tabs.push(LogTabPage::live_tab(Lang::Zh));
    p.active_tab = 1;
    p.tick_ai(&ctx, Lang::Zh, "layout-project".into(), &cfg, false);
    assert!(!p.ai_chat.allow_automation);
    assert!(p.ai_chat.watch.is_none());
    assert!(p.ai_chat.entries.iter().any(|e| e.text.contains("old source analysis")));
    assert!(!format!("{:?}",p.ai_chat.history).contains("old source analysis"));
}

#[test]
fn analyse_and_monitor_views_keep_their_own_results_and_suggestions_only_fill_draft() {
    let mut p = panel();
    let state = p.ai_state("layout-project", false);
    p.ai_chat.entry("assistant", "MANUAL_RESULT");
    p.ai_chat.entry("assistant", "WATCH_RESULT");
    p.ai_chat.entries.last_mut().unwrap().automatic = true;
    let ctx = egui::Context::default();
    let t = ui_theme::apply_theme(&ctx, false);
    for mode in [Mode::Analyse, Mode::Monitor] {
        p.ai_chat.mode = mode;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420., 800.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    p.ai_chat.ui(ui, Lang::En, &t, &state);
                });
            },
        );
        let all = out
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    Some(t.galley.text())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(all.contains("MANUAL_RESULT"), mode == Mode::Analyse);
        if mode == Mode::Monitor {
            assert!(all.contains("WATCH_RESULT"));
        } else {
            assert!(all.contains("Monitoring analysis"));
        }
    }
}

#[test]
fn assistant_and_monitor_dashboard_export_aligned_source_frames() {
    for width in [1100., 1440.] {
        for light in [false, true] {
            for mode in [Mode::Analyse, Mode::Monitor] {
                let mut p = panel();
                p.api_select_port(Lang::Zh, Some("COM7".into()), Some(115200))
                    .unwrap();
                p.primary_device = Some("COM7".into());
                p.monitoring = true;
                p.ai_confirmed.insert("COM7".into(), 115200);
                let mut chat = std::mem::take(&mut p.ai_chat);
                chat.allow_automation = true;
                chat.mode = mode;
                chat.watch_goal =
                    "持续检查 VIN 和通信超时；VIN 低于 4.5 V 时预警并采集示波器证据。".into();
                chat.rules.limits = vec![alerts::Limit {
                    key: "VIN".into(),
                    min: Some(4.5),
                    max: Some(5.5),
                }];
                p.begin_ai_watch(&mut chat, "监控 VIN 与通信异常".into(), false)
                    .unwrap();
                chat.entry("user", "监控 VIN，低于4.5 V时预警，并读取示波器实际频率。");
                chat.entry("tool","执行完成 · instrument.command\n\n```json\n{\"device_id\":1,\"ok\":true,\"frequency\":100000,\"unit\":\"Hz\"}\n```");
                let lines = (0..38)
                    .map(|n| {
                        format!(
                            "12:30:{n:02}.000 RX VIN={} V IIN=0.82 A state=CHARGING",
                            if n == 37 { "4.20" } else { "5.01" }
                        )
                    })
                    .collect::<Vec<_>>();
                p.tabs[0].append_lines(lines.clone(), true);
                chat.observe(p.tabs[0].ai_view_id(), &lines);
                chat.entry("assistant","## 已发现 VIN 越限\n\n最新日志为 **4.20 V**，低于你设置的 **4.5 V** 下限。原始行已保留在预警卡片中。\n\n- 示波器实际频率：**100 kHz**\n- 建议关联检查输入适配器、线缆压降与负载变化。\n\n这条预警是监控事件，尚未改写产品测试结论。");
                chat.reports = 1;
                p.ai_chat = chat;
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = ui_theme::apply_theme(&ctx, light);
                let size = egui::vec2(width, 850.);
                let mut out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| p.ui(ui, Lang::Zh, &t));
                    },
                );
                out.textures_delta
                    .set
                    .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                out.textures_delta.set.push((
                    egui::TextureId::Managed(0),
                    egui::epaint::ImageDelta::full(
                        ctx.fonts(|f| f.image()),
                        egui::TextureOptions::LINEAR,
                    ),
                ));
                crate::ui_audit::export_frame(
                    &ctx,
                    out,
                    &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-ai-r3-ui"),
                    &format!(
                        "assistant-{}-{}-{}",
                        if mode == Mode::Analyse {
                            "chat"
                        } else {
                            "monitor"
                        },
                        width as i32,
                        if light { "light" } else { "dark" }
                    ),
                    size,
                );
            }
        }
    }
}

#[test]
fn local_library_settings_entry_preserves_composer_on_narrow_and_short_panes(){
 for width in [260.,360.,520.]{for height in [320.,500.,800.]{for lang in [Lang::Zh,Lang::En]{for scope in [DataScope::Current,DataScope::Range,DataScope::Selection,DataScope::Library]{
  let mut p=panel();p.ai_chat.scope=scope;let state=p.ai_state("layout-project",false);let ctx=egui::Context::default();crate::fonts::install_cjk_fonts(&ctx);let theme=ui_theme::apply_theme(&ctx,false);
  let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,height))),..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{p.ai_chat.ui(ui,lang,&theme,&state);});});
  let send=text_rect(&output,label(lang,"发送","Send"));let local=text_rect(&output,"…");assert!(send.bottom()<=height&&send.right()<=width);assert!(local.bottom()<send.top()&&local.right()<=width,"library entry overflows {width}x{height}: {local:?}");
 }}}}
}

#[test]
fn local_library_popup_keeps_preview_accessible_inside_small_windows(){
 for width in [260.,420.]{
  let height=500.;let mut p=panel();if width==260.{p.ai_chat.config=None;}p.ai_chat.scope=DataScope::Library;let state=p.ai_state("layout-project",false);let ctx=egui::Context::default();crate::fonts::install_cjk_fonts(&ctx);let theme=ui_theme::apply_theme(&ctx,false);
  ctx.style_mut(|style|style.animation_time=0.);
  let mut draw=|events|ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,height))),events,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{p.ai_chat.ui(ui,Lang::Zh,&theme,&state);});});
  let out=draw(vec![]);let position=text_rect(&out,"…").center();
  let _=draw(vec![egui::Event::PointerMoved(position),egui::Event::PointerButton{pos:position,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}]);
  let _=draw(vec![egui::Event::PointerButton{pos:position,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}]);let menu=draw(vec![]);let sub=text_rect(&menu,"日志选项与预处理").center();
  let _=draw(vec![egui::Event::PointerMoved(sub)]);let mut output=draw(vec![]);
  if !output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="本地预分析")){
   let _=draw(vec![egui::Event::PointerButton{pos:sub,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}]);let _=draw(vec![egui::Event::PointerButton{pos:sub,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}]);output=draw(vec![]);
  }
  let preview=text_rect(&output,"本地预分析");assert!(preview.min.x>=0.&&preview.max.x<=width&&preview.min.y>=0.&&preview.max.y<=height,"preview clipped at {width}: {preview:?}");
  if width==420.{output.textures_delta.set.retain(|(id,_)|*id!=egui::TextureId::Managed(0));output.textures_delta.set.push((egui::TextureId::Managed(0),egui::epaint::ImageDelta::full(ctx.fonts(|fonts|fonts.image()),egui::TextureOptions::LINEAR)));crate::ui_audit::export_frame(&ctx,output,&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-local-log-ui"),"local-library-menu",egui::vec2(width,height));}
 }
}

#[test]
fn serial_skills_menu_keyboard_reaches_and_toggles_skill() {
    let mut p = panel();
    let state = p.ai_state("layout-project", false);
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    let theme = ui_theme::apply_theme(&ctx, false);
    ctx.style_mut(|style| style.animation_time = 0.);
    let mut draw = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 800.))),
        events, ..Default::default()
    }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { p.ai_chat.ui(ui, Lang::En, &theme, &state); }); });
    let out = draw(vec![]);
    let position = text_rect(&out, "…").center();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary,
        pressed, modifiers: egui::Modifiers::NONE };
    draw(vec![egui::Event::PointerMoved(position), pointer(position, true)]);
    draw(vec![pointer(position, false)]);
    let menu = draw(vec![]);
    let select = text_rect(&menu, "Select skills").center();
    draw(vec![egui::Event::PointerMoved(select), pointer(select, true)]);
    draw(vec![pointer(select, false)]);
    let menu = draw(vec![]);
    let skill = text_rect(&menu, "Serial log analysis").center();
    draw(vec![egui::Event::PointerMoved(skill), pointer(skill, true)]);
    draw(vec![pointer(skill, false)]);
    drop(draw);
    assert!(p.ai_chat.selected_skills.iter().any(|id| id == "serial_log"),
        "pointer action should first select the skill");
    let mut draw = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 800.))),
        events, ..Default::default()
    }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { p.ai_chat.ui(ui, Lang::En, &theme, &state); }); });
    let key = |key, pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false,
        modifiers: egui::Modifiers::NONE };
    draw(vec![key(egui::Key::Space, true)]);
    draw(vec![key(egui::Key::Space, false)]);
    drop(draw);
    assert!(!p.ai_chat.selected_skills.iter().any(|id| id == "serial_log"),
        "Space should toggle off the focused skill after pointer selection");
}

#[test]
fn serial_control_checkbox_is_keyboard_operable_after_pointer_selection() {
    let mut p = panel();
    let state = p.ai_state("layout-project", false);
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    let theme = ui_theme::apply_theme(&ctx, false);
    let mut draw = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 800.))),
        events, ..Default::default()
    }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { p.ai_chat.ui(ui, Lang::En, &theme, &state); }); });
    let control = text_rect(&draw(vec![]), "Allow control").center();
    let event = |pressed| egui::Event::PointerButton { pos: control, button: egui::PointerButton::Primary,
        pressed, modifiers: egui::Modifiers::NONE };
    draw(vec![egui::Event::PointerMoved(control), event(true)]);
    draw(vec![event(false)]);
    drop(draw);
    assert!(p.ai_chat.allow_automation, "pointer action should select control checkbox");
    let mut draw = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 800.))),
        events, ..Default::default()
    }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { p.ai_chat.ui(ui, Lang::En, &theme, &state); }); });
    let key = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed,
        repeat: false, modifiers: egui::Modifiers::NONE };
    draw(vec![key(true)]);
    draw(vec![key(false)]);
    drop(draw);
    assert!(!p.ai_chat.allow_automation, "Space should revoke the focused control grant");
}

#[test]
fn serial_skill_and_control_are_reachable_using_only_keyboard() {
    fn frame(ctx: &egui::Context, p: &mut SerialToolPanel, state: &Snapshot,
        theme: &ui_theme::Tokens, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO,
            egui::vec2(420., 800.))), events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { p.ai_chat.ui(ui, Lang::En, theme, state); });
        })
    }
    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
    }
    fn focused_label(out: &egui::FullOutput) -> Option<String> {
        let update = out.platform_output.accesskit_update.as_ref()?;
        update.nodes.iter().find(|(id, _)| *id == update.focus)
            .and_then(|(_, node)| node.label().map(str::to_owned))
    }
    let mut p = panel();
    let state = p.ai_state("layout-project", false);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::fonts::install_cjk_fonts(&ctx);
    let theme = ui_theme::apply_theme(&ctx, false);
    ctx.style_mut(|style| style.animation_time = 0.);
    frame(&ctx, &mut p, &state, &theme, vec![]);
    let mut goto = |p: &mut SerialToolPanel, target: &str| {
        for _ in 0..80 {
            let out = frame(&ctx, p, &state, &theme, vec![key(egui::Key::Tab, true)]);
            let release = frame(&ctx, p, &state, &theme, vec![key(egui::Key::Tab, false)]);
            if focused_label(&out).as_deref() == Some(target) || focused_label(&release).as_deref() == Some(target) { return; }
        }
        panic!("keyboard could not reach {target}");
    };
    goto(&mut p, "…");
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Enter, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Enter, false)]);
    goto(&mut p, "Select skills");
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Enter, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Enter, false)]);
    goto(&mut p, "Serial log analysis");
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, false)]);
    assert!(p.ai_chat.selected_skills.iter().any(|id| id == "serial_log"));
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Escape, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Escape, false)]);
    goto(&mut p, "Allow control");
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, false)]);
    assert!(p.ai_chat.allow_automation, "keyboard must enable the isolated control grant");
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, true)]);
    frame(&ctx, &mut p, &state, &theme, vec![key(egui::Key::Space, false)]);
    assert!(!p.ai_chat.allow_automation, "keyboard must also revoke the control grant");
}

#[test]
fn ai_divider_drag_changes_panel_width() {
    let ctx=egui::Context::default();
    let theme=ui_theme::apply_theme(&ctx,false);
    let mut chat=SerialAiChat::default();chat.open=true;
    let width=chat.panel_width;
    let full=egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1000.0,600.0));
    let start=egui::pos2(full.right()-width-2.0,300.0);
    let end=start+egui::vec2(80.0,0.0);
    let mut draw=|events|ctx.run(egui::RawInput{screen_rect:Some(full),events,..Default::default()},|ctx|{
        egui::CentralPanel::default().show(ctx,|ui|{
            let (main,right)=chat.split(ui,full,&theme);
            // Simulate the subsequently drawn content panels competing for
            // pointer input, as they do in the full serial tool layout.
            ui.interact(main,ui.id().with("main-content"),egui::Sense::click_and_drag());
            ui.interact(right.unwrap(),ui.id().with("ai-content"),egui::Sense::click_and_drag());
            chat.resize_split(ui,main,right.unwrap(),&theme);
        });
    });
    let _=draw(vec![]);
    let _=draw(vec![egui::Event::PointerMoved(start),egui::Event::PointerButton{pos:start,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}]);
    let _=draw(vec![egui::Event::PointerMoved(end)]);
    let _=draw(vec![egui::Event::PointerButton{pos:end,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}]);
    drop(draw);
    assert!(chat.panel_width < width-20.0,"divider drag did not resize the AI panel");
}
