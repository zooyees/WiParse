use super::*;

fn fixture() -> SerialToolPanel {
    let mut config = AppConfig::default();
    config.log_monitor.save_live_to_disk = false;
    config.log_monitor.open_log_files.clear();
    let mut panel = SerialToolPanel::new(&config, Lang::Zh);
    panel.ai_chat.open = true;
    panel.ai_chat.set_context(
        "serial-ai-test".into(),
        &AiConfig {
            enabled: true,
            model: "test-model".into(),
            ..Default::default()
        },
    );
    panel
}

#[test]
fn protocol_rejects_unknown_arguments_and_conflicting_actions() {
    for actions in [
        r#"[{"type":"serial.write","data":"reboot"}]"#,
        r#"[{"type":"serial.start","port":"COM9"}]"#,
        r#"[{"type":"serial.stop","command":"erase"}]"#,
        r#"[{"type":"serial.start"},{"type":"serial.stop"}]"#,
        r#"[{"type":"monitor.start","interval_seconds":0}]"#,
        r#"[{"type":"monitor.start","interval_seconds":301}]"#,
        r#"[{"type":"monitor.start"},{"type":"monitor.stop"}]"#,
    ] {
        let response = protocol::parse(&format!(r#"{{"reply":"说明","actions":{actions}}}"#));
        assert!(response.actions.is_empty(), "{actions}");
        assert!(response.error.is_some(), "{actions}");
        assert_eq!(response.reply, "说明");
    }
    let response = protocol::parse("```json\n{\"reply\":\"## 结论\\n正常\",\"actions\":[{\"type\":\"serial.start\"},{\"type\":\"monitor.start\",\"interval_seconds\":30}]}\n```");
    assert_eq!(response.actions.len(), 2);
    assert_eq!(response.reply, "## 结论\n正常");
    assert!(response.error.is_none());
    assert_eq!(protocol::parse("**正常回复**").reply, "**正常回复**");
}

#[test]
fn snapshots_are_bounded_and_clear_changes_source_identity() {
    let mut p = fixture();
    p.tabs[0].append_lines(
        (0..350)
            .map(|n| format!("{n}: {}", "电压".repeat(300)))
            .collect::<Vec<_>>(),
        true,
    );
    let before = p.ai_state("serial-ai-test", false);
    let sample = p.ai_observation(true, Some(0), None);
    let lines = sample["recent_lines"].as_array().unwrap();
    assert_eq!(lines.len(), 100);
    assert!(lines.iter().all(|line| line.as_str().unwrap().len() <= 400));
    assert_eq!(sample["omitted_lines"], 250);
    assert!(p.ai_observation(false, None, None)["recent_lines"]
        .as_array()
        .unwrap()
        .is_empty());
    p.tabs[0].clear_display();
    assert_ne!(
        before.binding.source_id,
        p.ai_state("serial-ai-test", false).binding.source_id
    );
    assert!(bounded("波形", 4).is_char_boundary(bounded("波形", 4).len()));
}

#[test]
fn automation_permissions_test_lock_and_offline_sources_are_enforced() {
    let mut p = fixture();
    let mut chat = std::mem::take(&mut p.ai_chat);
    assert!(p
        .apply_ai_action(&mut chat, Action::SerialStop, "stop", Lang::Zh, false)
        .is_err());
    chat.allow_automation = true;
    assert!(p
        .apply_ai_action(&mut chat, Action::SerialStop, "stop", Lang::Zh, true)
        .is_err());
    p.tabs[0].live = false;
    assert!(p
        .apply_ai_action(&mut chat, Action::SerialStop, "stop", Lang::Zh, false)
        .is_err());
    p.tabs[0].live = true;
    assert!(p.begin_ai_watch(&mut chat, "watch".into(), false).is_err());
    p.primary_device = Some("COM-test".into());
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    chat.observe(
        p.tabs[0].ai_view_id(),
        &[
            "WARN temperature".into(),
            "ERROR timeout".into(),
            "normal".into(),
        ],
    );
    let stats = &chat.watch.as_ref().unwrap().stats;
    assert_eq!(stats.received_lines, 3);
    assert_eq!(stats.error_keyword_lines, 1);
    assert_eq!(stats.warning_keyword_lines, 1);
    chat.close();
    assert!(chat.watch.is_none());
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn project_switch_stops_monitoring_and_resets_permission() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    chat.entry("assistant", "old project response");
    chat.set_context("different-project".into(), &AiConfig::default());
    assert!(!chat.allow_automation);
    assert!(chat.watch.is_none());
    assert!(!chat
        .entries
        .iter()
        .any(|entry| entry.text.contains("old project response")));
}

#[test]
fn stale_requests_cancel_before_result_and_late_permission_cannot_upgrade_request() {
    // A closed loopback endpoint is enough: no result may be applied after the
    // identity changes, irrespective of whether the worker has already failed.
    for change in ["source", "baud", "session", "project"] {
        let config = AiConfig {
            enabled: true,
            base_url: "http://127.0.0.1:9".into(),
            model: "mock".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        crate::ai::AiPanel::new(&config).install(&ctx);
        let mut p = fixture();
        p.ai_chat.set_context("serial-ai-test".into(), &config);
        let state = p.ai_state("serial-ai-test", false);
        p.ai_chat
            .submit(&ctx, &state, json!({}), "start".into(), false)
            .unwrap();
        assert!(!p.ai_chat.pending.as_ref().unwrap().automation_granted);
        p.ai_chat.allow_automation = true;
        assert!(!p.ai_chat.pending.as_ref().unwrap().automation_granted);
        let project = if change == "project" {
            "new-project"
        } else {
            "serial-ai-test"
        };
        match change {
            "source" => p.tabs[0].clear_display(),
            "baud" => p.baud = "9600".into(),
            "session" => p.ai_epoch += 1,
            _ => {}
        }
        p.tick_ai(&ctx, Lang::Zh, project.into(), &config, false);
        assert!(p.ai_chat.pending.is_none());
        assert!(!p.ai_chat.entries.iter().any(|e| e.role == "assistant"));
        assert!(!p.monitoring);
    }
}

#[test]
fn shared_ai_transport_decodes_reply_and_records_actual_stop_result() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{}", listener.local_addr().unwrap()),
        model: "mock-serial".into(),
        ..Default::default()
    };
    let (sent, received) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(8);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(_) if Instant::now() < until => thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("{error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
            assert!(header.len() < 32768);
        }
        let length = String::from_utf8(header)
            .unwrap()
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse::<usize>().ok())
            })
            .unwrap();
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        sent.send(serde_json::from_slice::<serde_json::Value>(&body).unwrap())
            .unwrap();
        let content =
            json!({"reply":"## 分析\n\n**已发现超时**。","actions":[{"type":"serial.stop"}]})
                .to_string();
        let body=json!({"model":"mock-serial","choices":[{"message":{"content":content},"finish_reason":"stop"}]}).to_string();
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    p.tabs[0].append_lines(vec!["ERROR timeout".into()], true);
    let state = p.ai_state("serial-ai-test", false);
    let data = p.ai_observation(true, None, None);
    p.ai_chat.set_context("serial-ai-test".into(), &config);
    p.ai_chat.allow_automation = true;
    p.ai_chat
        .submit(&ctx, &state, data, "分析并停止接收".into(), false)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(8);
    while p.ai_chat.pending.is_some() && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(p.ai_chat.pending.is_none());
    assert!(
        p.primary_device.is_none(),
        "{}",
        p.ai_chat
            .entries
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.role == "assistant" && e.text.starts_with("## 分析")));
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.role == "host" && e.text.contains("Reception stopped")));
    let request = received.recv_timeout(Duration::from_secs(2)).unwrap();
    let serialized = request.to_string();
    assert!(serialized.contains("ERROR timeout"));
    assert!(serialized.contains("automation_enabled=true"));
    server.join().unwrap();
}

#[test]
fn ai_start_waits_for_actual_tcp_open_and_watch_captures_new_data() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("tcp://{}", listener.local_addr().unwrap());
    let (sent, release) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("{error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket.write_all(b"VIN=5.0\nERROR timeout\n").unwrap();
        let _ = release.recv_timeout(Duration::from_secs(5));
    });
    let ctx = egui::Context::default();
    let mut p = fixture();
    let config = p.ai_chat.config.clone().unwrap();
    p.api_select_port(Lang::Zh, Some(address.clone()), Some(115200))
        .unwrap();
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    p.apply_ai_action(&mut chat, Action::SerialStart, "start", Lang::Zh, false)
        .unwrap();
    assert!(chat.awaiting_connection.is_some());
    assert!(!p.ai_state("serial-ai-test", false).connection_confirmed);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    p.ai_chat = chat;
    let deadline = Instant::now() + Duration::from_secs(5);
    while (p.tabs[0].line_count() < 2 || p.ai_chat.awaiting_connection.is_some())
        && Instant::now() < deadline
    {
        p.drain_events(false);
        p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(p.ai_state("serial-ai-test", false).connection_confirmed);
    assert!(p.ai_chat.awaiting_connection.is_none());
    assert_eq!(p.ai_chat.watch.as_ref().unwrap().stats.received_lines, 2);
    assert_eq!(
        p.ai_chat.watch.as_ref().unwrap().stats.error_keyword_lines,
        1
    );
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.apply_ai_action(&mut chat, Action::SerialStop, "stop", Lang::Zh, false)
        .unwrap();
    assert!(!p.device_is_open(&address));
    assert!(chat.watch.is_none());
    let _ = sent.send(());
    server.join().unwrap();
}

#[test]
fn serial_ai_pane_renders_responsively_with_markdown_and_no_split_controls() {
    for lang in [Lang::Zh, Lang::En] {
        for light in [false, true] {
            for width in [640., 1100., 1440.] {
                let mut p = fixture();
                p.ai_chat.entry("user", "分析当前数据");
                p.ai_chat.entry("assistant","## 结论\n\n- **异常**：超时\n- 建议检查连接\n\n| 项目 | 值 |\n|---|---|\n| 电压 | 5 V |");
                p.tabs[0].append_lines(vec!["RX: VIN=5.00 V".into(), "WARN timeout".into()], true);
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let t = ui_theme::apply_theme(&ctx, light);
                let size = egui::vec2(width, 850.);
                let mut output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| p.ui(ui, lang, &t));
                    },
                );
                output
                    .textures_delta
                    .set
                    .retain(|(id, _)| *id != egui::TextureId::Managed(0));
                output.textures_delta.set.push((
                    egui::TextureId::Managed(0),
                    egui::epaint::ImageDelta::full(
                        ctx.fonts(|f| f.image()),
                        egui::TextureOptions::LINEAR,
                    ),
                ));
                let labels = output
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
                assert!(labels.contains(if lang == Lang::Zh {
                    "串口 AI"
                } else {
                    "Serial AI"
                }));
                assert!(!labels.contains("数据分窗"));
                if width == 1440. {
                    crate::ui_audit::export_frame(
                        &ctx,
                        output,
                        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../tmp/serial-ai-ui-audit"),
                        &format!(
                            "serial-ai-{}-{}",
                            if lang == Lang::Zh { "zh" } else { "en" },
                            if light { "light" } else { "dark" }
                        ),
                        size,
                    );
                }
            }
        }
    }
}

#[test]
fn ai_local_log_tool_scans_full_files_and_sends_only_one_bounded_summary(){
    let _serial=library::LOCAL_TEST_LOCK.lock().unwrap();
    use std::io::{Read,Write};
    let root=std::env::temp_dir().join(format!("wiparse-ai-full-log-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
    let raw=format!("ERROR first\n{}ERROR last\n",(0..10000).map(|i|format!("UNSENT_BULK_{i} normal value=5.0\n")).collect::<String>());std::fs::write(root.join("complete.log"),&raw).unwrap();
    let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    let config=AiConfig{enabled:true,base_url:format!("http://{}",listener.local_addr().unwrap()),model:"local-log-fixture".into(),..Default::default()};
    let (sent,received)=std::sync::mpsc::channel();
    let server=thread::spawn(move||{
        for round in 0..2{
            let until=Instant::now()+Duration::from_secs(15);let mut socket=loop{match listener.accept(){Ok((socket,_))=>break socket,Err(_) if Instant::now()<until=>thread::sleep(Duration::from_millis(2)),Err(error)=>panic!("{error}")}};
            socket.set_nonblocking(false).unwrap();socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let mut header=vec![];let mut byte=[0];
            while !header.ends_with(b"\r\n\r\n"){socket.read_exact(&mut byte).unwrap();header.push(byte[0]);assert!(header.len()<32768);}
            let length=String::from_utf8(header).unwrap().lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|s|s.trim().parse::<usize>().ok())).unwrap();let mut body=vec![0;length];socket.read_exact(&mut body).unwrap();sent.send(serde_json::from_slice::<serde_json::Value>(&body).unwrap()).unwrap();
            let answer=if round==0{json!({"reply":"先在本地扫描全部日志。","actions":[{"type":"platform.call","method":"log.analyze","params":{"contains":["ERROR"],"samples":2,"context":0}}]})}else{json!({"reply":"已读取10002行，找到首尾两处错误线索。","actions":[]})};
            let body=json!({"model":"local-log-fixture","choices":[{"message":{"content":answer.to_string()},"finish_reason":"stop"}]}).to_string();write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);let mut p=fixture();p.ai_chat.set_context("serial-ai-test".into(),&config);p.ai_chat.scope=DataScope::Library;p.ai_chat.library.directory=Some(root.clone());
    let state=p.ai_state("serial-ai-test",false);let data=p.ai_scoped_observation(DataScope::Library).unwrap();p.ai_chat.submit(&ctx,&state,data,"分析全部日志中的错误".into(),false).unwrap();
    let until=Instant::now()+Duration::from_secs(15);while (p.ai_chat.pending.is_some()||p.ai_chat.tool.is_some())&&Instant::now()<until{p.tick_ai(&ctx,Lang::Zh,"serial-ai-test".into(),&config,false);thread::sleep(Duration::from_millis(2));}
    assert!(p.ai_chat.pending.is_none()&&p.ai_chat.tool.is_none());assert_eq!(p.ai_chat.entries.iter().filter(|entry|entry.role=="user").count(),1);
    let first=received.recv_timeout(Duration::from_secs(2)).unwrap();let second=received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!first.to_string().contains("UNSENT_BULK"));assert!(!second.to_string().contains("UNSENT_BULK"));
    let message=second["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();let envelope:serde_json::Value=serde_json::from_str(message).unwrap();let report=&envelope["observation"]["platform_results"][0]["result"]["data"];
    assert_eq!(report["coverage"]["complete"],true);assert_eq!(report["coverage"]["lines_read"],10002);assert_eq!(report["coverage"]["matched_lines"],2);assert!(envelope["observation"]["local_log_library"]["last_analysis"].is_null());assert!(message.len()<16000);assert!(raw.len()>message.len()*10);
    server.join().unwrap();std::fs::remove_dir_all(root).unwrap();
}
