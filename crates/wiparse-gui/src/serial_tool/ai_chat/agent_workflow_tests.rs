use super::*;
use std::io::Read;
fn panel(config: &AiConfig) -> SerialToolPanel {
    let mut c = AppConfig::default();
    c.log_monitor.open_log_files.clear();
    c.log_monitor.save_live_to_disk = false;
    let mut p = SerialToolPanel::new(&c, Lang::Zh);
    p.ai_chat.open = true;
    p.ai_chat.set_context("agent-test".into(), config);
    p
}
fn response(name: &str, args: serde_json::Value) -> serde_json::Value {
    json!({"model":"fixture-model","choices":[{"finish_reason":"tool_calls","message":{"content":null,"reasoning_content":"Use actual log fields and verify state changes.","tool_calls":[{"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]})
}
#[test]
fn one_user_request_acquires_evidence_repairs_stalling_and_finishes_without_confirmation() {
    let _lock = library::LOCAL_TEST_LOCK.lock().unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        for round in 0..3 {
            let mut req = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .expect("model request");
            let mut text = String::new();
            req.as_reader().read_to_string(&mut text).unwrap();
            let input: serde_json::Value = serde_json::from_str(&text).unwrap();
            tx.send(input.clone()).unwrap();
            let reply = match round {
                0 => {
                    json!({"model":"fixture-model","choices":[{"finish_reason":"stop","message":{"content":"请先确认开始扫描，我还没有数据。"}}]})
                }
                1 => response(
                    "wiparse_log_events",
                    json!({"active_pattern":"power_state:1","inactive_pattern":"power_state:0","reason_pattern":"cause:([A-Z_]+)","context_lines":1}),
                ),
                _ => {
                    let messages = input["messages"].as_array().unwrap();
                    let tool = messages
                        .iter()
                        .rev()
                        .find(|v| v["role"] == "tool")
                        .expect("native tool result");
                    let result: serde_json::Value =
                        serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
                    assert_eq!(
                        result["data"]["state_transition_analysis"]
                            ["confirmed_active_to_inactive_transitions"],
                        2
                    );
                    assert!(messages
                        .iter()
                        .any(|v| v["role"] == "assistant"
                            && v["reasoning_content"].as_str().is_some()));
                    response(
                        "wiparse_answer",
                        json!({"reply":"检测到2次确认的充电中断。\n\n|次数|关联观测|\n|---|---|\n|1|UV，需结合电压波形确认|\n|2|THERMAL，需结合温度数据确认|\n\n依据 S1；重复的断开状态不重复计数。关联观测不等于已证明的物理根因。","status":"completed","evidence_ids":["S1"]}),
                    )
                }
            };
            req.respond(
                tiny_http::Response::from_string(reply.to_string()).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                ),
            )
            .unwrap();
        }
    });
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{address}"),
        model: "fixture-model".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = panel(&config);
    let log = [
        "boot version=X",
        "power_state:1",
        "cause:UV",
        "power_state:0",
        "power_state:0",
        "status:idle",
        "power_state:1",
        "cause:THERMAL",
        "power_state:0",
        "idle",
    ];
    p.tabs[0].append_lines(log.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true);
    let mut chat = std::mem::take(&mut p.ai_chat);
    let state = p.ai_state("agent-test", false);
    p.begin_manual_agent(
        &ctx,
        &mut chat,
        &state,
        "分析日志的断充次数和原因，简单表列".into(),
    )
    .unwrap();
    p.ai_chat = chat;
    let until = Instant::now() + Duration::from_secs(30);
    while p.ai_chat.working() && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "agent-test".into(), &config, false);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(!p.ai_chat.working(), "{}", p.ai_chat.status);
    assert!(!p.ai_chat.status_error, "{}", p.ai_chat.status);
    assert_eq!(p.ai_chat.run.as_ref().unwrap().phase, "completed");
    assert_eq!(
        p.ai_chat
            .entries
            .iter()
            .filter(|e| e.role == "user")
            .count(),
        1
    );
    assert!(!p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.role == "assistant" && e.text.contains("请先确认")));
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.role == "assistant" && e.text.contains("2次")));
    assert!(!p.ai_chat.allow_automation);
    assert!(!p.monitoring);
    let requests = rx.try_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 3);
    let first = &requests[0];
    assert!(first["tools"].is_array());
    let user = first["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    let evidence: serde_json::Value = serde_json::from_str(user).unwrap();
    assert!(evidence["observation"]["initial_evidence"].is_null());
    assert!(requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "tool"));
    assert!(evidence["observation"].get("recent_lines").is_none());
    assert!(evidence["observation"].get("data_included").is_none());
    assert!(evidence["observation"].get("omitted_lines").is_none());
    worker.join().unwrap();
}
#[test]
fn native_adapter_rejects_unknown_actions_and_requires_explicit_final_references() {
    use wiparse_core::ai::{tools, AiReply};
    let r = AiReply {
        text: String::new(),
        model: "mock".into(),
        usage: json!({}),
        finish_reason: Some("tool_calls".into()),
        tool_message: Some(tools::Assistant {
            content: None,
            reasoning_content: None,
            calls: vec![tools::ToolCall {
                id: "1".into(),
                name: "wiparse_answer".into(),
                arguments: json!({"reply":"done","status":"completed"}).to_string(),
            }],
        }),
    };
    assert!(run::decode(&r).error.is_some());
    assert!(platform::validate("engineering.inspect", &json!({"tool":"waveform"})).is_ok());
    assert!(platform::validate(
        "engineering.inspect",
        &json!({"tool":"all","path":"../secret"})
    )
    .is_err());
    assert!(platform::validate("engineering.wave.measure", &json!({"channel":0})).is_ok());
    assert!(platform::validate(
        "engineering.wave.measure",
        &json!({"channel":0,"path":"arbitrary"})
    )
    .is_err());
}
#[test]
fn selected_scope_authorizes_reads_but_not_device_actions() {
    let config = AiConfig {
        enabled: true,
        model: "fixture".into(),
        ..Default::default()
    };
    let mut p = panel(&config);
    let ctx = egui::Context::default();
    let mut chat = std::mem::take(&mut p.ai_chat);
    assert!(p
        .begin_platform_call(
            &ctx,
            &mut chat,
            "instrument.command".into(),
            json!({"device_id":1,"command":"ScopeStop"}),
            "analyse logs".into(),
            false
        )
        .is_err());
    let functions = run::functions(DataScope::Current, false);
    let methods = functions
        .iter()
        .find(|f| f.name == "wiparse_call")
        .unwrap()
        .parameters["properties"]["method"]["enum"]
        .as_array()
        .unwrap();
    assert!(methods.contains(&json!("log.analyze")));
    assert!(!methods.contains(&json!("instrument.command")));
    assert!(run::functions(DataScope::None, false)
        .iter()
        .find(|f| f.name == "wiparse_call")
        .unwrap()
        .parameters["properties"]["method"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v != "log.analyze"));
}

#[test]
fn native_timeline_then_standard_markdown_finishes_one_task_and_exposes_real_chart() {
    let _lock = library::LOCAL_TEST_LOCK.lock().unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        for round in 0..3 {
            let mut req = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .expect("request");
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let input: serde_json::Value = serde_json::from_str(&body).unwrap();
            let output = if round == 0 {
                response(
                    "wiparse_log_timeline",
                    json!({"timestamp_pattern":"(\\d{2}:\\d{2}:\\d{2}\\.\\d+)","start_time":"20:05:09.865","start_pattern":"START","end_pattern":"DONE","fields":[{"name":"v","pattern":"V=(\\d+)","unit":"V"},{"name":"i","pattern":"I=(\\d+)","unit":"A"}],"power_pair":["v","i"]}),
                )
            } else if round==1 {
                let tool = input["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|m| m["role"] == "tool")
                    .unwrap();
                let evidence: serde_json::Value =
                    serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
                let series = &evidence["data"]["timeline_analysis"]["sources"][0];
                assert_eq!(series["observed_duration_s"], 2.0);
                assert_eq!(series["statistics"][2]["max"], 10.0);
                response("wiparse_call",json!({"method":"report.create","params":{"title":"充电观测","markdown":"功率峰值 10 W。","evidence_ids":["S1"]}}))
            }else{
                let rows=input["messages"].as_array().unwrap();
                let body=rows.iter().rev().find(|v|v["role"]=="tool").unwrap()["content"].as_str().unwrap();
                let report:serde_json::Value=serde_json::from_str(body).unwrap();
                assert!(report["data"]["report_id"].is_string());
                json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":"## 充电观测报告\n\n依据 S1，指定起点至 DONE 的观测时长 2 秒，功率峰值 10 W。实际曲线已由本地引擎提取。日志未给出协议帧，无法证明协议合规。"}}]})
            };
            req.respond(
                tiny_http::Response::from_string(output.to_string()).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                ),
            )
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
    let root=std::env::temp_dir().join(format!("wiparse-fit-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-ai-project-root"),root.clone()));
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = panel(&config);
    p.tabs[0].append_lines(
        [
            "20:05:09.865 START V=5 I=2",
            "20:05:10.865 V=5 I=1",
            "20:05:11.865 V=5 I=0 DONE",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
        true,
    );
    let mut chat = std::mem::take(&mut p.ai_chat);
    let state = p.ai_state("agent-test", false);
    p.begin_manual_agent(
        &ctx,
        &mut chat,
        &state,
        "从20:05:09.865开始计算充电时长、画功率曲线并形成报告".into(),
    )
    .unwrap();
    p.ai_chat = chat;
    let deadline = Instant::now() + Duration::from_secs(30);
    while p.ai_chat.working() && Instant::now() < deadline {
        p.tick_ai(&ctx, Lang::Zh, "agent-test".into(), &config, false);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(!p.ai_chat.status_error, "{}", p.ai_chat.status);
    assert!(!p.ai_chat.working());
    assert_eq!(p.ai_chat.run.as_ref().unwrap().phase, "completed");
    assert_eq!(
        p.ai_chat
            .entries
            .iter()
            .filter(|e| e.role == "user")
            .count(),
        1
    );
    assert!(p.ai_chat.entries.iter().any(|e| e.timeline.is_some()));
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.role == "assistant" && e.text.contains("2 秒")));
    worker.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn plan_read_batch_recall_and_completion_check_finish_without_user_orchestration() {
    let _lock = library::LOCAL_TEST_LOCK.lock().unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        let mut stable_prefix = None;
        for round in 0..6 {
            let mut req = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .expect("model request");
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let input: serde_json::Value = serde_json::from_str(&body).unwrap();
            let messages = input["messages"].as_array().unwrap();
            let prefix = messages
                .iter()
                .take_while(|m| m["role"] != "assistant")
                .cloned()
                .collect::<Vec<_>>();
            if let Some(saved) = &stable_prefix {
                assert_eq!(&prefix, saved, "base request/history must stay stable");
            } else {
                stable_prefix = Some(prefix);
            }
            let pending = json!({"steps":[{"id":"voltage","title":"检查过压","state":"pending"},{"id":"temperature","title":"检查过温","state":"pending"},{"id":"report","title":"输出汇总","state":"pending","deliverable":"report"}]});
            let output = match round {
                0 => response("wiparse_plan", pending),
                1 => {
                    json!({"model":"fixture","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"voltage-read","type":"function","function":{"name":"wiparse_log_analyze","arguments":json!({"contains":["OVP"],"samples":1,"group":false}).to_string()}},{"id":"temperature-read","type":"function","function":{"name":"wiparse_log_analyze","arguments":json!({"contains":["OTP"],"samples":1,"group":false}).to_string()}}]}}]})
                }
                2 => {
                    let messages = input["messages"].as_array().unwrap();
                    for id in ["voltage-read", "temperature-read"] {
                        let t = messages
                            .iter()
                            .find(|m| m["tool_call_id"] == id)
                            .expect("every batch call paired");
                        let v: serde_json::Value =
                            serde_json::from_str(t["content"].as_str().unwrap()).unwrap();
                        assert_eq!(v["data"]["coverage"]["matched_lines"], 1);
                    }
                    json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":"已发现1次过压。"}}]})
                    // misses a declared goal: host must continue
                }
                3 => {
                    assert!(body.contains("AGENT.UNFINISHED_OUTCOMES"));
                    response("wiparse_evidence", json!({"evidence_id":"S2"}))
                }
                4 => {
                    let last = input["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .rev()
                        .find(|m| m["role"] == "tool")
                        .unwrap();
                    let recalled: serde_json::Value =
                        serde_json::from_str(last["content"].as_str().unwrap()).unwrap();
                    assert_eq!(
                        recalled["data"]["result"]["data"]["coverage"]["matched_lines"],
                        1
                    );
                    response(
                        "wiparse_plan",
                        json!({"steps":[{"id":"voltage","title":"检查过压","state":"done","evidence_ids":["S2"]},{"id":"temperature","title":"检查过温","state":"done","evidence_ids":["S3"]},{"id":"report","title":"输出汇总","state":"done","deliverable":"report","evidence_ids":["S2","S3"]}]}),
                    )
                }
                _ => {
                    json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":"## 异常汇总\n\nOVP、OTP各命中1行，依据S2/S3；这是日志事件，不代表已经证明物理根因。"}}]})
                }
            };
            req.respond(
                tiny_http::Response::from_string(output.to_string()).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                ),
            )
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
    let mut p = panel(&config);
    p.tabs[0].append_lines(
        ["normal", "OVP VIN=19.8", "OTP TEMP=85", "normal"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        true,
    );
    let mut chat = std::mem::take(&mut p.ai_chat);
    let state = p.ai_state("agent-test", false);
    p.begin_manual_agent(&ctx, &mut chat, &state, "检查过压和过温，汇总证据".into())
        .unwrap();
    p.ai_chat = chat;
    let until = Instant::now() + Duration::from_secs(40);
    while p.ai_chat.working() && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "agent-test".into(), &config, false);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(!p.ai_chat.working());
    assert!(!p.ai_chat.status_error, "{}", p.ai_chat.status);
    let run = p.ai_chat.run.as_ref().unwrap();
    assert_eq!(run.phase, "completed");
    assert!(run.workflow.open_goals().is_empty());
    assert_eq!(run.workflow.index()["retained_count"], 2);
    assert_eq!(
        p.ai_chat
            .entries
            .iter()
            .filter(|e| e.role == "user")
            .count(),
        1
    );
    assert!(!p.ai_chat.allow_automation);
    worker.join().unwrap();
}

#[test]
fn capability_question_needs_no_log_scan_or_artificial_tool_call() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        let req = server
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        req.respond(tiny_http::Response::from_string(json!({"choices":[{"finish_reason":"stop","message":{"content":"我可以检索日志、提取事件和功率曲线，操作设备需要相应授权。"}}]}).to_string()).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
    });
    let config = AiConfig {
        enabled: true,
        base_url: format!("http://{addr}"),
        model: "fixture".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = panel(&config);
    let mut chat = std::mem::take(&mut p.ai_chat);
    let state = p.ai_state("agent-test", false);
    p.begin_manual_agent(&ctx, &mut chat, &state, "你能做什么？".into())
        .unwrap();
    p.ai_chat = chat;
    let until = Instant::now() + Duration::from_secs(15);
    while p.ai_chat.working() && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "agent-test".into(), &config, false);
        thread::sleep(Duration::from_millis(3));
    }
    assert!(!p.ai_chat.status_error);
    assert_eq!(p.ai_chat.tool_round, 0);
    assert!(!p.ai_chat.entries.iter().any(|e| e.role == "tool"));
    worker.join().unwrap();
}

#[test]
fn open_serial_goals_wait_for_real_input_without_scanning_or_repair(){
    for native in [true,false]{
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr=server.server_addr().to_ip().unwrap();
        let thread=thread::spawn(move||{
            let req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
            let msg=if native{response("wiparse_answer",json!({"status":"needs_input","reply":"请提供目标电压","evidence_ids":[]}))}
                else{json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":json!({"reply":"请提供目标电压","actions":[],"status":"needs_input"}).to_string()}}]})};
            req.respond(tiny_http::Response::from_string(msg.to_string())).unwrap();
        });
        let config=AiConfig{enabled:true,base_url:format!("http://{addr}"),model:"fixture".into(),..Default::default()};
        let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
        let mut p=panel(&config);
        let mut chat=std::mem::take(&mut p.ai_chat);
        let state=p.ai_state("agent-test",false);
        p.begin_manual_agent(&ctx,&mut chat,&state,"计算充电时长、绘制曲线、形成报告".into()).unwrap();
        p.ai_chat=chat;
        let deadline=Instant::now()+Duration::from_secs(10);
        while p.ai_chat.working()&&Instant::now()<deadline{p.tick_ai(&ctx,Lang::Zh,"agent-test".into(),&config,false);thread::sleep(Duration::from_millis(3));}
        thread.join().unwrap();
        let run=p.ai_chat.run.as_ref().unwrap();assert_eq!(run.phase,"needs_input","{}",p.ai_chat.status);
        assert_eq!(run.repairs,0);assert!(!run.workflow.open_goals().is_empty());assert!(p.ai_chat.status.contains("等待补充"));
    }
}

#[test]
fn repeated_native_stalls_switch_this_task_to_json_and_keep_scan_result() {
    let _lock=library::LOCAL_TEST_LOCK.lock().unwrap();
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||{
        for round in 0..4 {
            let mut req=server.recv_timeout(Duration::from_secs(8)).unwrap().unwrap();
            let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();
            let input:serde_json::Value=serde_json::from_str(&body).unwrap();
            assert_eq!(input["tools"].is_array(),round<3);
            let reply=match round {
                0=>response("wiparse_log_analyze",json!({"contains":["overheat"],"samples":0})),
                1|2=>response("wiparse_call",json!({"params":{}})),
                _=>{
                    let payload=input["messages"].as_array().unwrap().iter().rev().filter_map(|m|serde_json::from_str::<serde_json::Value>(m["content"].as_str()?).ok()).find(|v|v["observation"].is_object()).unwrap();
                    let result=&payload["observation"]["platform_results"][0];
                    assert_eq!(result["evidence_id"],"S1");assert_eq!(result["method"],"log.analyze");assert_eq!(result["result"]["ok"],true);
                    json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"status":"completed","reply":"S1：发现过温记录。","actions":[]}).to_string()}}]})
                }
            };
            req.respond(tiny_http::Response::from_string(reply.to_string())).unwrap();
        }
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{addr}"),model:"fixture".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p=panel(&config);
    p.tabs[0].append_lines(vec!["overheat".into()],true);
    let state=p.ai_state("agent-test",false);
    let mut chat=std::mem::take(&mut p.ai_chat);
    p.begin_manual_agent(&ctx,&mut chat,&state,"查找过温".into()).unwrap();p.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(25);
    while p.ai_chat.working()&&Instant::now()<deadline{p.tick_ai(&ctx,Lang::Zh,"agent-test".into(),&config,false);thread::sleep(Duration::from_millis(3));}
    worker.join().unwrap();assert!(!p.ai_chat.status_error,"{}",p.ai_chat.status);
    assert_eq!(p.ai_chat.run.as_ref().unwrap().phase,"completed");assert_eq!(p.ai_chat.tool_round,1);
    assert!(!run::Run::new().json_fallback);
}
