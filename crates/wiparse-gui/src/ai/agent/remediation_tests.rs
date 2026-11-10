use super::*;
fn state() -> ContextState {
    ContextState {
        page: "serial".into(),
        project: "p1".into(),
        binding: json!({"serial":"s1","waveform":"w1","data_analysis":"d1"}).to_string(),
        testing: false,
    }
}
fn session() -> Session {
    let mut s = Session::default();
    s.set_context(state());
    s
}
fn call(method: &str) -> Call {
    Call {
        method: method.into(),
        params: json!({}),
    }
}
#[test]
fn evidence_survives_followups_and_cancelled_actions_are_audited() {
    let mut s = session();
    s.task = "first".into();
    s.enqueue(call("serial.inspect"), false);
    let r = s.take_request().unwrap();
    s.attach(
        &r.id,
        HostReply::value(json!({"ok":true,"data":{"rows":5}})),
    );
    s.cancel_task("done");
    s.prompt = "compare previous evidence".into();
    s.start(&AiService::new(AiConfig::default()));
    assert_eq!(s.evidence("E1")["data"]["result"]["data"]["rows"], 5);
    s.enqueue(call("serial.start"), false);
    s.take_request();
    s.cancel_task("operator cancelled while connecting");
    assert_eq!(s.steps.len(), 2);
    assert_eq!(s.steps[1].id, "E2");
    assert_eq!(s.steps[1].data["state"], "outcome_unknown");
    assert_eq!(s.steps[1].call.method, "serial.start");
    assert_eq!(s.steps[0].task, "first");
    assert_ne!(s.steps[0].task, s.steps[1].task);
}
#[test]
fn unrelated_tool_changes_do_not_cancel_source_bound_work() {
    let mut s = session();
    s.cross_tools = true;
    s.enqueue(call("serial.inspect"), false);
    let r = s.take_request().unwrap();
    let mut changed = state();
    changed.page = "waveform".into();
    changed.binding = json!({"serial":"s1","waveform":"w2","data_analysis":"d1"}).to_string();
    s.set_context(changed.clone());
    assert!(s.busy());
    assert!(s.authorize(&r, &changed));
    changed.binding = json!({"serial":"s2","waveform":"w2","data_analysis":"d1"}).to_string();
    s.set_context(changed);
    assert!(!s.busy());
    assert_eq!(s.steps[0].data["state"], "cancelled");
}
#[test]
fn background_run_completion_survives_ai_timeout_and_project_change_without_leaking_evidence() {
    let mut s = session();
    s.task = "test-origin".into();
    s.mark_test(Some("run-123".into()));
    s.cancel_task("AI deadline");
    assert!(s.tracking_test());
    let mut other = state();
    other.project = "p2".into();
    s.set_context(other);
    assert!(!s.owns_test(Some("run-123")));
    s.update_test(
        json!({"running":false,"completed_run_identity":"run-123","process_exit_code":0}),
    );
    assert!(!s.tracking_test());
    assert_eq!(s.steps[0].scope.project, "p1");
    assert_eq!(s.steps[0].task, "test-origin");
    assert!(s.observations.is_empty());
    assert_eq!(s.evidence("E1")["ok"], false);
}
#[test]
fn bounded_results_keep_quality_units_and_continuation() {
    let result = bounded_evidence(
        json!({"ok":true,"data":{"unit":"mV","coverage":{"complete":false,"invalid":12},"continuation":"opaque","text":"x".repeat(50000)}}),
        0,
    );
    assert_eq!(result["data"]["unit"], "mV");
    assert_eq!(result["data"]["coverage"]["complete"], false);
    assert_eq!(result["data"]["continuation"], "opaque");
    assert_eq!(result["data"]["text"]["truncated"], true);
}
#[test]
fn truncated_but_valid_action_is_never_executed() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut block = [0; 4096];
        loop {
            let n = socket.read(&mut block).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&block[..n]);
            if let Some(at) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..at]);
                let size = head
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if bytes.len() >= at + 4 + size {
                    break;
                }
            }
        }
        let content =
            json!({"reply":"execute","call":{"method":"serial.start","params":{}}}).to_string();
        let body = json!({"choices":[{"message":{"content":content},"finish_reason":"length"}]})
            .to_string();
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let service = AiService::new(AiConfig {
        enabled: true,
        base_url: url,
        model: "mock".into(),
        ..Default::default()
    });
    let mut s = session();
    s.prompt = "start".into();
    s.start(&service);
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.busy() && Instant::now() < deadline {
        s.poll(&Context::default(), &service);
        std::thread::sleep(Duration::from_millis(5));
    }
    server.join().unwrap();
    assert!(s.error);
    assert!(s.approval.is_none());
    assert!(s.outgoing.is_empty());
    assert!(s.status.contains("截断"));
}

#[test]
fn cancelling_during_an_internal_poll_retains_original_dispatch_and_provider_change_clears_prompt_history(
) {
    let mut s = session();
    s.enqueue(call("serial.start"), false);
    let issued = s.take_request().unwrap();
    s.attach(
        &issued.id,
        HostReply::value(json!({"_agent_wait":{"method":"serial.ready","params":{}}})),
    );
    let mut poll = issued.clone();
    poll.internal = true;
    poll.call = Call {
        method: "serial.ready".into(),
        params: json!({}),
    };
    s.outgoing.push_back(poll);
    s.cancel_task("cancel during internal poll");
    assert_eq!(s.steps[0].data["dispatched"], true);
    assert_eq!(s.steps[0].data["state"], "outcome_unknown");
    s.message("user", "sensitive source from provider A".into());
    let a = AiConfig::default();
    s.configure(&a);
    let mut b = a;
    b.model = "different-provider-model".into();
    s.configure(&b);
    assert!(s.messages.is_empty());
    assert!(s.observations.is_empty());
    assert!(!s.steps.is_empty());
}
