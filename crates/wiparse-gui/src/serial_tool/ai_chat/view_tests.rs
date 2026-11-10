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
        assert!(text_rect(&out, "mock-model").bottom() < 930.0);
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
    assert!(none["recent_lines"].as_array().unwrap().is_empty());
    assert_eq!(none["selected_text"], "");
    assert!(p.ai_scoped_observation(DataScope::Selection).is_err());
    assert_eq!(
        p.ai_scoped_observation(DataScope::Current).unwrap()["recent_lines"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
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
    assert!(!p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.text.contains("old source analysis")));
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
  let send=text_rect(&output,label(lang,"发送","Send"));let local=text_rect(&output,label(lang,"本地日志：设置与预分析…","Local logs: settings and preview…"));assert!(send.bottom()<=height&&send.right()<=width);assert!(local.bottom()<send.top()&&local.right()<=width,"library entry overflows {width}x{height}: {local:?}");
 }}}}
}

#[test]
fn local_library_popup_keeps_preview_accessible_inside_small_windows(){
 for width in [260.,420.]{
  let height=500.;let mut p=panel();if width==260.{p.ai_chat.config=None;}p.ai_chat.scope=DataScope::Library;let state=p.ai_state("layout-project",false);let ctx=egui::Context::default();crate::fonts::install_cjk_fonts(&ctx);let theme=ui_theme::apply_theme(&ctx,false);
  ctx.style_mut(|style|style.animation_time=0.);
  let mut draw=|events|ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,height))),events,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{p.ai_chat.ui(ui,Lang::Zh,&theme,&state);});});
  let out=draw(vec![]);let position=text_rect(&out,"本地日志：设置与预分析…").center();
  let _=draw(vec![egui::Event::PointerMoved(position),egui::Event::PointerButton{pos:position,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}]);
  let _=draw(vec![egui::Event::PointerButton{pos:position,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}]);let mut output=draw(vec![]);
  let preview=text_rect(&output,"本地预分析");assert!(preview.min.x>=0.&&preview.max.x<=width&&preview.min.y>=0.&&preview.max.y<=height,"preview clipped at {width}: {preview:?}");
  if width==420.{output.textures_delta.set.retain(|(id,_)|*id!=egui::TextureId::Managed(0));output.textures_delta.set.push((egui::TextureId::Managed(0),egui::epaint::ImageDelta::full(ctx.fonts(|fonts|fonts.image()),egui::TextureOptions::LINEAR)));crate::ui_audit::export_frame(&ctx,output,&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/serial-local-log-ui"),"local-library-menu",egui::vec2(width,height));}
 }
}
