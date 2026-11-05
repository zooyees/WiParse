use super::*;

fn panel() -> SerialToolPanel {
    let mut cfg = AppConfig::default();
    cfg.log_monitor.save_live_to_disk = false;
    cfg.log_monitor.open_log_files.clear();
    let mut p = SerialToolPanel::new(&cfg, Lang::Zh);
    p.ai_chat.open = true;
    p.ai_chat.set_context(
        "fast-ai".into(),
        &AiConfig {
            enabled: true,
            model: "mock".into(),
            ..Default::default()
        },
    );
    p
}

#[test]
fn one_second_interval_is_valid_and_an_active_watch_can_be_reconfigured() {
    for kind in ["monitor.start", "monitor.configure"] {
        for (seconds, valid) in [
            (0, false),
            (1, true),
            (5, true),
            (15, true),
            (300, true),
            (301, false),
        ] {
            let r = protocol::parse(
                &json!({"reply":"ok","actions":[{"type":kind,"interval_seconds":seconds}]})
                    .to_string(),
            );
            assert_eq!(r.error.is_none(), valid, "{kind} {seconds}");
        }
    }
    let mut p = panel();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    assert_eq!(
        chat.watch.as_ref().unwrap().interval,
        Duration::from_secs(5)
    );
    p.apply_ai_action(
        &mut chat,
        Action::MonitorStart {
            interval_seconds: Some(1),
        },
        "watch",
        Lang::Zh,
        false,
    )
    .unwrap();
    assert_eq!(
        chat.watch.as_ref().unwrap().interval,
        Duration::from_secs(1)
    );
    assert!(p
        .apply_ai_action(
            &mut chat,
            Action::MonitorStart {
                interval_seconds: Some(0)
            },
            "watch",
            Lang::Zh,
            false
        )
        .is_err());
    chat.rules.contains = vec!["timeout".into()];
    chat.observe(p.tabs[0].ai_view_id(), &["timeout".into()]);
    assert_eq!(chat.alert_count, 1); // local alert does not wait for the interval or HTTP.
    assert!(!chat.allow_automation);
    assert!(protocol::parse(
        r#"{"reply":"ok","actions":[{"type":"monitor.configure","automation_enabled":true}]}"#
    )
    .error
    .is_some());
}

#[test]
fn serial_tools_require_permission_and_reject_other_ports_and_payloads() {
    for method in [
        "serial.monitor.start",
        "serial.monitor.stop",
        "serial.monitor.status",
    ] {
        assert!(platform::validate(method, &json!({})).is_ok());
        assert!(platform::validate(method, &json!({"port":"COM-other"})).is_err());
        assert!(platform::validate(method, &json!({"payload":"reboot"})).is_err());
    }
    let ctx = egui::Context::default();
    let mut p = panel();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    assert!(p
        .begin_platform_call(
            &ctx,
            &mut chat,
            "serial.monitor.stop".into(),
            json!({}),
            "stop".into(),
            false
        )
        .is_err());
    assert!(p.primary_device.is_some());
    chat.allow_automation = true;
    p.ai_test_locked = true;
    assert!(p
        .begin_platform_call(
            &ctx,
            &mut chat,
            "serial.monitor.stop".into(),
            json!({}),
            "stop".into(),
            false
        )
        .is_err());
    p.ai_test_locked = false;
    p.begin_platform_call(
        &ctx,
        &mut chat,
        "serial.monitor.stop".into(),
        json!({}),
        "stop".into(),
        false,
    )
    .unwrap();
    p.poll_platform(&ctx, &mut chat, Lang::Zh);
    assert!(p.primary_device.is_none());
    assert_eq!(chat.tool_results[0]["result"]["ok"], true);
    assert_eq!(
        chat.tool_results[0]["result"]["data"]["reception_requested"],
        false
    );
    assert!(chat.tool_results[0]["result"]["port_release"]
        .as_str()
        .unwrap()
        .contains("asynchronous"));
}

#[test]
fn serial_start_tool_waits_for_actual_connection_before_returning_success() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("tcp://{}", listener.local_addr().unwrap());
    let ctx = egui::Context::default();
    let mut p = panel();
    p.api_select_port(Lang::Zh, Some(address.clone()), Some(115200))
        .unwrap();
    p.primary_device = Some("COM-other".into());
    p.monitoring = true;
    p.tabs.push(LogTabPage::live_tab(Lang::Zh));
    p.active_tab = 1;
    let mut chat = std::mem::take(&mut p.ai_chat);
    let before = p.ai_state("fast-ai", false);
    chat.conversation_source = Some((before.binding.source_id, before.binding.device));
    chat.allow_automation = true;
    p.begin_platform_call(
        &ctx,
        &mut chat,
        "serial.monitor.start".into(),
        json!({}),
        "start".into(),
        false,
    )
    .unwrap();
    let after = p.ai_state("fast-ai", false);
    assert_eq!(
        chat.conversation_source,
        Some((after.binding.source_id, after.binding.device))
    );
    p.poll_platform(&ctx, &mut chat, Lang::Zh);
    assert!(chat.tool.is_some());
    assert!(chat.tool_results.is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    let socket = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("{e}"),
        }
    };
    while chat.tool.is_some() && Instant::now() < deadline {
        p.drain_events(false);
        p.poll_platform(&ctx, &mut chat, Lang::Zh);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        chat.tool_results[0]["result"]["data"]["connection_confirmed"],
        true
    );
    p.api_stop_device(Lang::Zh, &address);
    drop(socket);
}

#[test]
fn serial_stop_result_drives_the_next_instrument_tool_without_a_stale_binding() {
    use std::io::Read;
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{}", server.server_addr()),
        model: "mock".into(),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let mut req = server
            .recv_timeout(Duration::from_secs(8))
            .unwrap()
            .unwrap();
        let mut body = String::new();
        req.as_reader().read_to_string(&mut body).unwrap();
        tx.send(body).unwrap();
        let content = json!({"reply":"串口已停止，下一步停止指定示波器。","actions":[{"type":"platform.call","method":"instrument.command","params":{"device_id":7,"command":"ScopeStop"}}]});
        req.respond(tiny_http::Response::from_string(json!({"model":"mock","choices":[{"message":{"content":content.to_string()},"finish_reason":"stop"}]}).to_string())).unwrap();
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let api = crate::backend::ApiBridge::new();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"), api.clone()));
    let mut p = panel();
    p.ai_chat.set_context("fast-ai".into(), &config);
    p.api_select_port(Lang::Zh, Some("COM-test".into()), Some(115200))
        .unwrap();
    p.primary_device = Some("COM-test".into());
    p.ai_confirmed.insert("COM-test".into(), 115200);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    p.begin_platform_call(
        &ctx,
        &mut chat,
        "serial.monitor.stop".into(),
        json!({}),
        "停止串口，再停止已指定的7号示波器".into(),
        false,
    )
    .unwrap();
    p.ai_chat = chat;
    let deadline = Instant::now() + Duration::from_secs(8);
    let req = loop {
        p.tick_ai(&ctx, Lang::Zh, "fast-ai".into(), &config, false);
        if let Some(req) = api.try_recv() {
            break req;
        }
        assert!(Instant::now() < deadline, "{}", p.ai_chat.status);
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(req.method, "instrument.command");
    assert_eq!(req.params["command"], "ScopeStop");
    assert!(p.ai_platform_guard_valid(req.ai_guard.as_ref().unwrap()));
    let body = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(body.contains("serial.monitor.stop"));
    assert!(body.contains("asynchronous"));
    p.ai_chat.cancel();
    worker.join().unwrap();
}

#[test]
fn slow_ai_requests_do_not_overlap_or_queue_missed_ticks() {
    use std::io::Read;
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{}", server.server_addr()),
        model: "mock".into(),
        ..Default::default()
    };
    let (seen_tx, seen_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        for round in 0..2 {
            let mut req = server
                .recv_timeout(Duration::from_secs(8))
                .unwrap()
                .unwrap();
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            seen_tx.send((round, body)).unwrap();
            if round == 0 {
                release_rx.recv_timeout(Duration::from_secs(8)).unwrap();
            }
            req.respond(tiny_http::Response::from_string(json!({"model":"mock","choices":[{"message":{"content":"{\"reply\":\"已分析\",\"actions\":[]}"},"finish_reason":"stop"}]}).to_string())).unwrap();
        }
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = panel();
    p.ai_chat.set_context("fast-ai".into(), &config);
    p.primary_device = Some("COM-test".into());
    p.ai_confirmed.insert("COM-test".into(), 115200);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.interval_seconds = 1;
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    p.ai_chat = chat;
    p.tabs[0].append_lines(vec!["first".into()], true);
    p.ai_chat.watch.as_mut().unwrap().next = Instant::now();
    p.tick_ai(&ctx, Lang::Zh, "fast-ai".into(), &config, false);
    assert_eq!(seen_rx.recv_timeout(Duration::from_secs(3)).unwrap().0, 0);
    p.tabs[0].append_lines(vec!["second".into()], true);
    for _ in 0..20 {
        p.ai_chat.watch.as_mut().unwrap().next = Instant::now();
        p.tick_ai(&ctx, Lang::Zh, "fast-ai".into(), &config, false);
    }
    assert!(seen_rx.try_recv().is_err());
    p.ai_chat.watch.as_mut().unwrap().last_request = Instant::now() - Duration::from_secs(2);
    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while p.ai_chat.reports < 2 && Instant::now() < deadline {
        p.tick_ai(&ctx, Lang::Zh, "fast-ai".into(), &config, false);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(p.ai_chat.reports, 2);
    assert_eq!(seen_rx.recv_timeout(Duration::from_secs(1)).unwrap().0, 1);
    p.ai_chat.watch.as_mut().unwrap().next = Instant::now();
    p.tick_ai(&ctx, Lang::Zh, "fast-ai".into(), &config, false);
    assert!(
        p.ai_chat.pending.is_none(),
        "No new data means no third request"
    );
    worker.join().unwrap();
}
