use super::*;

#[test]
fn revocation_invalidates_deferred_control_even_after_reenable() {
    let mut chat = SerialAiChat::default();
    chat.allow_automation = true;
    chat.deferred_guard = Some(DeferredGuard { task_id:"old".into(), scope_key:"scope".into(), skills:vec![], controls:true, test_controls:false, revoked:false });
    chat.revoke_controls();
    chat.allow_automation = true;
    assert!(chat.deferred_guard.as_ref().unwrap().revoked);
}
fn fixture(config: &AiConfig) -> SerialToolPanel {
    let mut cfg = AppConfig::default();
    cfg.log_monitor.open_log_files.clear();
    cfg.log_monitor.save_live_to_disk = false;
    let mut p = SerialToolPanel::new(&cfg, Lang::Zh);
    p.ai_chat.set_context("queue-test".into(), config);
    p.ai_chat.open = true;
    p
}
#[test]
fn queued_library_request_is_paused_when_directory_changes() {
    let config = AiConfig::default();
    let mut panel = fixture(&config);
    panel.ai_chat.scope = DataScope::Library;
    panel.ai_chat.library.directory = Some(std::path::PathBuf::from("first-library"));
    let state = panel.ai_state("queue-test", false);
    let key = panel.local_log_scope_key(&panel.ai_chat);
    panel.ai_chat.enqueue_message("分析日志".into(), &state, key.clone()).unwrap();
    panel.ai_chat.library.directory = Some(std::path::PathBuf::from("second-library"));
    assert_ne!(key, panel.local_log_scope_key(&panel.ai_chat));
    let mut chat = std::mem::take(&mut panel.ai_chat);
    panel.pump_message_queue(&egui::Context::default(), &mut chat, &state);
    assert!(chat.queue_paused);
    assert_eq!(chat.messages.len(), 1);
}
#[test]
fn closing_chat_permanently_revokes_queued_control_grants() {
    let config=AiConfig::default();let mut panel=fixture(&config);
    panel.ai_chat.allow_automation=true;
    let state=panel.ai_state("queue-test",false);
    let key=panel.local_log_scope_key(&panel.ai_chat);
    panel.ai_chat.enqueue_message("停止设备".into(),&state,key).unwrap();
    assert!(panel.ai_chat.messages.front().unwrap().controls);
    panel.ai_chat.close();
    assert!(!panel.ai_chat.messages.front().unwrap().controls);
    panel.ai_chat.open=true;panel.ai_chat.allow_automation=true;
    assert!(!panel.ai_chat.messages.front().unwrap().controls);
    assert!(panel.ai_chat.queue_paused);
}
#[test]
fn queued_test_control_does_not_grant_device_control_and_revocation_is_permanent() {
    let config=AiConfig::default();let mut panel=fixture(&config);
    panel.ai_chat.allow_automation=true;
    let state=panel.ai_state("queue-test",true);
    let key=panel.local_log_scope_key(&panel.ai_chat);
    panel.ai_chat.enqueue_message("停止测试".into(),&state,key).unwrap();
    let message=panel.ai_chat.messages.front().unwrap();
    assert!(!message.controls);
    assert!(message.test_controls);
    panel.ai_chat.revoke_controls();
    panel.ai_chat.allow_automation=true;
    let message=panel.ai_chat.messages.front().unwrap();
    assert!(!message.controls && !message.test_controls);
}
fn reply(text: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":text}}],"usage":{"prompt_tokens":100,"completion_tokens":20}}).to_string()).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())
}
#[test]
fn messages_submitted_during_analysis_run_fifo_and_accumulate_reported_usage() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (seen_tx, seen_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        for i in 0..2 {
            let mut request = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .unwrap();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let input: serde_json::Value = serde_json::from_str(&body).unwrap();
            let question = input["messages"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .filter_map(|m| {
                    m["content"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                })
                .find_map(|v| v["user_request"].as_str().map(str::to_owned))
                .unwrap();
            seen_tx.send(question).unwrap();
            if i == 0 {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            request
                .respond(reply(&format!("完成问题{}", i + 1)))
                .unwrap();
        }
    });
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{addr}"),
        model: "fixture".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture(&config);
    let mut chat = std::mem::take(&mut p.ai_chat);
    let state = p.ai_state("queue-test", false);
    p.begin_manual_agent(&ctx, &mut chat, &state, "先说明分析能力".into())
        .unwrap();
    p.ai_chat = chat;
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(p.ai_chat.working());
    let key = p.local_log_scope_key(&p.ai_chat);
    p.ai_chat
        .enqueue_message("再说明证据规则".into(), &state, key)
        .unwrap();
    release_tx.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while (p.ai_chat.working() || !p.ai_chat.messages.is_empty()) && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "queue-test".into(), &config, false);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(!p.ai_chat.working());
    assert!(p.ai_chat.messages.is_empty());
    assert_eq!(
        seen_rx.try_iter().collect::<Vec<_>>(),
        ["先说明分析能力", "再说明证据规则"]
    );
    assert_eq!(
        p.ai_chat
            .entries
            .iter()
            .filter(|e| e.role == "user")
            .count(),
        2
    );
    assert_eq!(p.ai_chat.usage_total.input_tokens, Some(200));
    assert_eq!(
        p.ai_chat.run.as_ref().unwrap().usage.input_tokens,
        Some(100)
    );
    worker.join().unwrap();
}
#[test]
fn changed_source_pauses_queue_without_sending_or_losing_draft() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{addr}"),
        model: "fixture".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture(&config);
    let state = p.ai_state("queue-test", false);
    let key = p.local_log_scope_key(&p.ai_chat);
    p.ai_chat
        .enqueue_message("分析原来源".into(), &state, key)
        .unwrap();
    p.tabs.push(LogTabPage::live_tab(Lang::Zh));
    p.active_tab = 1;
    p.tick_ai(&ctx, Lang::Zh, "queue-test".into(), &config, false);
    assert!(p.ai_chat.queue_paused);
    assert_eq!(p.ai_chat.messages.front().unwrap().text, "分析原来源");
    assert!(server
        .recv_timeout(Duration::from_millis(120))
        .unwrap()
        .is_none());
}
#[test]
fn failed_queued_start_preserves_both_message_and_current_draft() {
    let config = AiConfig { enabled: false, ..Default::default() };
    let ctx = egui::Context::default();
    let mut panel = fixture(&config);
    let state = panel.ai_state("queue-test", false);
    let key = panel.local_log_scope_key(&panel.ai_chat);
    panel.ai_chat.enqueue_message("排队任务".into(), &state, key).unwrap();
    panel.ai_chat.prompt = "正在编辑的另一条消息".into();
    let mut chat = std::mem::take(&mut panel.ai_chat);
    panel.pump_message_queue(&ctx, &mut chat, &state);
    assert!(chat.queue_paused);
    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages.front().unwrap().text, "排队任务");
    assert_eq!(chat.prompt, "正在编辑的另一条消息");
}
#[test]
fn incompatible_recovery_keeps_serial_draft_and_recovery() {
    let config = AiConfig { enabled: true, model: "fixture".into(), ..Default::default() };
    let ctx = egui::Context::default();
    let mut panel = fixture(&config);
    let state = panel.ai_state("queue-test", false);
    let mut chat = std::mem::take(&mut panel.ai_chat);
    chat.prompt = "恢复任务".into();
    chat.recovery = Some(crate::ai::journal::Recovery::incompatible_test_fixture(
        &state.binding.project, "恢复任务"));
    let result = panel.begin_manual_agent_capped(&ctx, &mut chat, &state, "恢复任务".into(), false);
    assert!(result.unwrap_err().contains("SKILL.VERSION_CHANGED"));
    assert_eq!(chat.prompt, "恢复任务");
    assert!(chat.recovery.is_some());
    assert!(chat.run.is_none());
}
#[test]
fn edited_serial_recovery_starts_new_task_and_releases_pending_selection() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        if let Some(request) = server.recv_timeout(Duration::from_secs(15)).unwrap() {
            request.respond(reply("已说明")).unwrap();
        }
    });
    let config = AiConfig { enabled: true, model: "fixture".into(), base_url: format!("http://{addr}"), ..Default::default() };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut panel = fixture(&config);
    let state = panel.ai_state("queue-test", false);
    let mut chat = std::mem::take(&mut panel.ai_chat);
    chat.recovery = Some(crate::ai::journal::Recovery::incompatible_test_fixture(&state.binding.project, "恢复任务"));
    panel.begin_manual_agent_capped(&ctx, &mut chat, &state, "仅解释充电保护".into(), false).unwrap();
    assert!(chat.recovery.is_none());
    assert_eq!(chat.run.as_ref().unwrap().question, "仅解释充电保护");
    worker.join().unwrap();
}
#[test]
fn older_queued_messages_never_gain_new_device_permission_and_queue_is_bounded() {
    let config = AiConfig {
        enabled: true,
        model: "fixture".into(),
        ..Default::default()
    };
    let mut p = fixture(&config);
    let state = p.ai_state("queue-test", false);
    let key = p.local_log_scope_key(&p.ai_chat);
    for i in 0..16 {
        p.ai_chat
            .enqueue_message(format!("问题{i}"), &state, key.clone())
            .unwrap();
    }
    p.ai_chat.prompt = "尚未发送的草稿".into();
    assert!(p
        .ai_chat
        .enqueue_message("超额".into(), &state, key)
        .is_err());
    assert_eq!(p.ai_chat.prompt, "尚未发送的草稿");
    assert_eq!(p.ai_chat.messages.len(), 16);
    p.ai_chat.allow_automation = true;
    assert!(p.ai_chat.messages.iter().all(|m| !m.controls));
    p.ai_chat.revoke_controls();
    assert!(p.ai_chat.messages.iter().all(|m| !m.controls));
}
#[test]
fn queued_composer_keeps_tabs_buttons_and_input_visible_in_both_themes() {
    for width in [260.0, 420.0, 600.0] {
        for light in [false, true] {
            let config = AiConfig {
                enabled: true,
                model: "fixture-model".into(),
                ..Default::default()
            };
            let mut p = fixture(&config);
            let mut state = p.ai_state("queue-test", false);
            state.retained_lines = 1_413_682;
            let key = p.local_log_scope_key(&p.ai_chat);
            p.ai_chat
                .enqueue_message("完成当前分析后，检查协议错误并形成报告".into(), &state, key)
                .unwrap();
            p.ai_chat.deferred = Some((json!({}), "active".into(), state.clone()));
            p.ai_chat.prompt = "再计算这一轮功率".into();
            p.ai_chat.entry("assistant", "正在核对源数据和任务目标。");
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let t = ui_theme::apply_theme(&ctx, light);
            let size = egui::vec2(width, if width < 300.0 { 500.0 } else { 800.0 });
            let mut frame = None;
            for _ in 0..2 {
                frame = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            p.ai_chat.ui(ui, Lang::Zh, &t, &state);
                        });
                    },
                ));
            }
            let mut out = frame.unwrap();
            let text_rect = |text: &str| {
                out.shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape {
                            (t.galley.text() == text)
                                .then(|| egui::Rect::from_min_size(t.pos, t.galley.size()))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| panic!("missing {text}"))
            };
            let send = text_rect("排队发送");
            assert!(send.right() <= width && send.bottom() <= size.y);
            let tabs = text_rect("智能对话");
            assert!(tabs.bottom() < 110.0);
            assert!(text_rect("待发送 · 1/16").bottom() < send.top());
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
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tmp/ai-intelligence-20261110/ui"),
                &format!("queue-{}-{}", width, if light { "light" } else { "dark" }),
                size,
            );
        }
    }
}

#[test]
fn busy_send_button_really_enqueues_through_the_host_ui() {
    let config = AiConfig {
        enabled: true,
        model: "fixture".into(),
        ..Default::default()
    };
    let mut p = fixture(&config);
    let state = p.ai_state("queue-test", false);
    p.ai_chat.deferred = Some((json!({}), "active".into(), state));
    p.ai_chat.prompt = "界面点击提交的第二个问题".into();
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    let t = ui_theme::apply_theme(&ctx, false);
    let mut draw = |events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420.0, 620.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    p.paint_ai_sidebar(ui, rect, Lang::Zh, &t);
                });
            },
        )
    };
    let out = draw(vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                (t.galley.text() == "排队发送")
                    .then(|| egui::Rect::from_min_size(t.pos, t.galley.size()).center())
            } else {
                None
            }
        })
        .unwrap();
    let _ = draw(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let _ = draw(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    drop(draw);
    assert_eq!(p.ai_chat.messages.len(), 1);
    assert_eq!(p.ai_chat.messages[0].text, "界面点击提交的第二个问题");
    assert!(p.ai_chat.prompt.is_empty());
}

#[test]
fn full_queue_feedback_is_visible_and_keeps_draft_without_failing_active_task() {
    let config = AiConfig {
        enabled: true,
        model: "fixture".into(),
        ..Default::default()
    };
    let mut p = fixture(&config);
    let state = p.ai_state("queue-test", false);
    let key = p.local_log_scope_key(&p.ai_chat);
    for _ in 0..4 {
        p.ai_chat
            .enqueue_message("x".repeat(16 * 1024), &state, key.clone())
            .unwrap();
    }
    p.ai_chat.deferred = Some((json!({}), "active".into(), state));
    p.ai_chat.prompt = "必须保留的新草稿".into();
    let ctx = egui::Context::default();
    crate::fonts::install_cjk_fonts(&ctx);
    let t = ui_theme::apply_theme(&ctx, false);
    let mut draw = |events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420.0, 620.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    p.paint_ai_sidebar(ui, rect, Lang::Zh, &t);
                });
            },
        )
    };
    let out = draw(vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|s| {
            if let egui::Shape::Text(t) = &s.shape {
                (t.galley.text() == "排队发送")
                    .then(|| egui::Rect::from_min_size(t.pos, t.galley.size()).center())
            } else {
                None
            }
        })
        .unwrap();
    let _ = draw(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let _ = draw(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let output = draw(vec![]);
    drop(draw);
    assert_eq!(p.ai_chat.messages.len(), 4);
    assert_eq!(p.ai_chat.prompt, "必须保留的新草稿");
    assert!(!p.ai_chat.status_error);
    assert!(p.ai_chat.working());
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("队列已满"))));
}

#[test]
fn queued_question_waits_for_operator_clarification() {
    let ctx=egui::Context::default();
    let mut panel=fixture(&AiConfig::default());
    let state=panel.ai_state("queue-test",false);
    let mut chat=std::mem::take(&mut panel.ai_chat);
    chat.run=Some(run::Run::new());
    chat.run.as_mut().unwrap().phase="needs_input";
    let key=panel.local_log_scope_key(&chat);
    chat.enqueue_message("另行分析温升".into(),&state,key).unwrap();
    assert!(!chat.should_queue_input());
    panel.pump_message_queue(&ctx,&mut chat,&state);
    assert_eq!(chat.messages.len(),1);
    assert!(!chat.queue_paused);
}
