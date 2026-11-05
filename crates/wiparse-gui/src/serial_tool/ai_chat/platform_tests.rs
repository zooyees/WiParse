use super::*;

fn fixture() -> SerialToolPanel {
    let mut cfg = AppConfig::default();
    cfg.log_monitor.open_log_files.clear();
    cfg.log_monitor.save_live_to_disk = false;
    let mut p = SerialToolPanel::new(&cfg, Lang::Zh);
    p.ai_chat.open = true;
    p.ai_chat.set_context(
        "ai-tools".into(),
        &AiConfig {
            enabled: true,
            model: "mock".into(),
            ..Default::default()
        },
    );
    p
}

#[test]
fn platform_registry_rejects_raw_commands_arbitrary_files_and_extra_fields() {
    for (method, params) in [
        ("instrument.connect", json!({"resource":"unknown"})),
        (
            "instrument.command",
            json!({"device_id":1,"command":{"RawWrite":"*RST"}}),
        ),
        (
            "instrument.command",
            json!({"device_id":1,"command":"ProbeErase"}),
        ),
        (
            "instrument.command",
            json!({"device_id":1,"command":{"SourceVoltage":{"channel":1,"value":-5}}}),
        ),
        (
            "instrument.capture",
            json!({"device_id":1,"path":"C:/arbitrary.png"}),
        ),
        (
            "instrument.waveform_source",
            json!({"device_id":1,"dir":"../escape"}),
        ),
        ("instrument.list", json!({"command":"erase"})),
    ] {
        assert!(
            platform::validate(method, &params).is_err(),
            "{method} {params}"
        );
    }
    assert!(!platform::validate("instrument.list", &json!({})).unwrap());
    assert!(platform::validate(
        "instrument.command",
        &json!({"device_id":1,"command":{"SourceOutput":{"channel":1,"enabled":false}}})
    )
    .unwrap());
    assert!(platform::validate("parse.line", &json!({"text":"ASK"})).is_ok());
}

#[test]
fn queued_controls_are_fenced_and_cancelled_before_dispatch() {
    let ctx = egui::Context::default();
    let api = crate::backend::ApiBridge::new();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"), api.clone()));
    let mut p = fixture();
    let mut chat = std::mem::take(&mut p.ai_chat);
    assert!(p
        .begin_platform_call(
            &ctx,
            &mut chat,
            "instrument.command".into(),
            json!({"device_id":1,"command":"ScopeStop"}),
            "stop".into(),
            false
        )
        .is_err());
    chat.allow_automation = true;
    p.begin_platform_call(
        &ctx,
        &mut chat,
        "instrument.command".into(),
        json!({"device_id":1,"command":"ScopeStop"}),
        "stop".into(),
        false,
    )
    .unwrap();
    p.ai_chat = chat;
    let req = api.try_recv().unwrap();
    let guard = req.ai_guard.unwrap();
    assert!(p.ai_platform_guard_valid(&guard));
    p.baud = "9600".into();
    assert!(!p.ai_platform_guard_valid(&guard));
    p.ai_chat.cancel();
    assert!(guard.cancelled.load(Ordering::Acquire));
    assert!(!p.ai_chat.allow_automation);
}

#[test]
fn local_numeric_alerts_are_immediate_bounded_and_never_product_verdicts() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.apply_ai_action(
        &mut chat,
        Action::MonitorConfigure {
            goal: Some("VIN下限4.5".into()),
            interval_seconds: Some(30),
            contains: Some(vec!["ERROR".into()]),
            limits: Some(vec![alerts::Limit {
                key: "VIN".into(),
                min: Some(4.5),
                max: Some(5.5),
            }]),
        },
        "watch",
        Lang::Zh,
        false,
    )
    .unwrap();
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let source = p.tabs[0].ai_view_id();
    let lines = vec!["VIN=4.1 V".to_string(); 500];
    let start = Instant::now();
    chat.observe(source, &lines);
    assert_eq!(chat.alert_count, 500);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(chat.entries.iter().filter(|e| e.role == "alert").count(), 1);
    assert!(chat.notice.is_some());
    assert!(chat.watch.as_ref().unwrap().stats.alert_examples.len() <= 12);
    assert!(!chat.watch.as_ref().unwrap().stats.alert_examples.is_empty());
    assert!(chat.rules.check("AVIN=4.1").is_none());
    assert!(chat.rules.check("VIN=NaN").is_none());
    assert!(chat.rules.check("VIN=5.0").is_none());
    assert!(chat.rules.check(r#"{"VIN":4.1}"#).is_some());
    assert!(chat.rules.check("VIN_ref=1 VIN=5.0").is_none());
    assert!(chat.rules.check("VIN: 4.0e0").is_some());
    assert!(alerts::validate(
        &None,
        &Some(vec![alerts::Limit {
            key: "VIN".into(),
            min: Some(6.),
            max: Some(5.)
        }])
    )
    .is_err());
}

#[test]
fn local_alerts_keep_running_when_ai_is_paused_or_service_changes() {
    let mut p = fixture();
    let config = AiConfig {
        enabled: true,
        base_url: "http://127.0.0.1:9".into(),
        model: "mock".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    p.ai_chat.set_context("ai-tools".into(), &config);
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    chat.watch.as_mut().unwrap().ai_paused = true;
    chat.watch.as_mut().unwrap().next = Instant::now() - Duration::from_secs(1);
    p.ai_chat = chat;
    p.tabs[0].append_lines(vec!["ERROR timeout".into()], true);
    p.ai_chat
        .observe(p.tabs[0].ai_view_id(), &["ERROR timeout".into()]);
    p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
    assert!(p.ai_chat.pending.is_none());
    assert!(p.ai_chat.watch.is_some());
    assert_eq!(p.ai_chat.alert_count, 1);
    p.set_live_view_visible(false);
    assert!(p.repaint_visible.load(Ordering::Relaxed));
    let mut changed = config.clone();
    changed.enabled = false;
    p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &changed, false);
    assert!(p.ai_chat.watch.as_ref().unwrap().ai_paused);
    p.ai_chat
        .observe(p.tabs[0].ai_view_id(), &["ERROR another fault".into()]);
    assert_eq!(p.ai_chat.alert_count, 2);
    p.ai_chat.close();
    p.set_live_view_visible(false);
    assert!(!p.repaint_visible.load(Ordering::Relaxed));
}

#[test]
fn three_cloud_failures_pause_only_ai_and_keep_local_alerts() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{}", server.server_addr()),
        model: "mock".into(),
        ..Default::default()
    };
    let worker = thread::spawn(move || {
        for _ in 0..3 {
            let request = server
                .recv_timeout(Duration::from_secs(8))
                .unwrap()
                .unwrap();
            request
                .respond(
                    tiny_http::Response::from_string("simulated service failure")
                        .with_status_code(tiny_http::StatusCode(503)),
                )
                .unwrap();
        }
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture();
    p.ai_chat.set_context("ai-tools".into(), &config);
    p.primary_device = Some("COM-test".into());
    p.ai_confirmed.insert("COM-test".into(), 115200);
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    p.ai_chat = chat;
    for n in 1..=3 {
        let line = format!("ERROR sample {n}");
        p.tabs[0].append_lines(vec![line.clone()], true);
        p.ai_chat.observe(p.tabs[0].ai_view_id(), &[line]);
        p.ai_chat.watch.as_mut().unwrap().next = Instant::now();
        p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
        let deadline = Instant::now() + Duration::from_secs(8);
        while p.ai_chat.pending.is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
        }
        assert!(p.ai_chat.pending.is_none());
        assert_eq!(p.ai_chat.watch.as_ref().unwrap().failures, n);
    }
    assert!(p.ai_chat.watch.as_ref().unwrap().ai_paused);
    p.ai_chat
        .observe(p.tabs[0].ai_view_id(), &["ERROR still monitored".into()]);
    assert_eq!(p.ai_chat.alert_count, 4);
    p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
    assert!(p.ai_chat.pending.is_none());
    worker.join().unwrap();
}

#[test]
fn ai_tool_loop_waits_for_completed_job_then_uses_actual_result() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{}", listener.local_addr().unwrap()),
        model: "mock".into(),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        for round in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                    Err(e) => panic!("{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let len = String::from_utf8(header)
                .unwrap()
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap();
            let mut body = vec![0; len];
            socket.read_exact(&mut body).unwrap();
            tx.send(String::from_utf8(body).unwrap()).unwrap();
            let content = if round == 0 {
                json!({"reply":"先读取实际频率。","actions":[{"type":"platform.call","method":"instrument.command","params":{"device_id":7,"command":{"ScopeMeasure":{"channel":1,"meas_type":"Frequency"}}}}]})
            } else {
                json!({"reply":"实际测量频率为100 kHz。","actions":[]})
            };
            let response=json!({"model":"mock","choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]}).to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let api = crate::backend::ApiBridge::new();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"), api.clone()));
    let mut p = fixture();
    p.ai_chat.set_context("ai-tools".into(), &config);
    p.ai_chat.allow_automation = true;
    let state = p.ai_state("ai-tools", false);
    p.ai_chat
        .submit(&ctx, &state, json!({}), "读取示波器实际频率".into(), false)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut dispatched = 0;
    let mut job_polls = 0;
    while Instant::now() < deadline {
        p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
        if let Some(req) = api.try_recv() {
            assert!(p.ai_platform_guard_valid(req.ai_guard.as_ref().unwrap()));
            let data = if req.method == "instrument.command" {
                dispatched += 1;
                json!({"accepted":true,"job_id":77})
            } else {
                assert_eq!(req.method, "instrument.job.get");
                job_polls += 1;
                if job_polls == 1 {
                    json!({"state":"pending_or_unknown","job_id":77})
                } else {
                    json!({"state":"completed","ok":true,"job_id":77,"detail":{"value":100000,"unit":"Hz"}})
                }
            };
            req.reply
                .send(crate::backend::invoke_ok(&req.method, data))
                .unwrap();
        }
        if p.ai_chat
            .entries
            .iter()
            .any(|e| e.text == "实际测量频率为100 kHz。")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(dispatched, 1);
    assert!(job_polls >= 2);
    assert!(p.ai_chat.tool.is_none());
    assert!(p.ai_chat.pending.is_none());
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.text == "实际测量频率为100 kHz。"));
    let _ = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(second.contains("100000"));
    assert!(second.contains("completed"));
    assert!(p.ai_chat.history.iter().any(|message|matches!(message,AiMessage::User(text) if text.contains("Verified platform result") && text.contains("100000"))));
    server.join().unwrap();
}
