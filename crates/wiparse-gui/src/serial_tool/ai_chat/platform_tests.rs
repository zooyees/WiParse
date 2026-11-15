use super::*;

#[test]
fn control_failure_text_distinguishes_declined_from_unknown_outcomes() {
    let declined = json!({"ok": false, "executed": false, "error": "PLATFORM.USER_DECLINED"});
    assert!(platform::control_failure_message(&declined, Lang::En).contains("cancelled before dispatch"));
    let blocked = json!({"ok": false, "executed": false, "error": "AI.APPROVAL_STALE"});
    assert!(platform::control_failure_message(&blocked, Lang::Zh).contains("未派发"));
    let unknown = json!({"ok": false, "error": "AI.TOOL_TIMEOUT"});
    assert!(platform::control_failure_message(&unknown, Lang::En).contains("state unknown"));
}

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
fn invalid_management_arguments_are_returned_as_repairable_local_result() {
    let ctx=egui::Context::default();
    let mut panel=fixture();
    let mut chat=std::mem::take(&mut panel.ai_chat);
    let mut run=run::Run::new();run.scope_key=panel.local_log_scope_key(&chat);chat.run=Some(run);
    assert!(panel.begin_platform_call(&ctx,&mut chat,"agent.plan".into(),json!({"steps":"invalid"}),"plan".into(),false).is_ok());
    assert!(chat.tool.is_some());
    panel.poll_platform(&ctx,&mut chat,Lang::Zh);
    assert!(chat.tool_results.iter().any(|row|row["method"]=="agent.plan" && row["result"]["error"]=="AGENT.INVALID_MANAGEMENT_ARGUMENTS"));
}

#[test]
fn invalid_log_rule_returns_nonexecution_and_keeps_task_available_for_repair() {
    let ctx=egui::Context::default();let mut panel=fixture();
    let mut chat=std::mem::take(&mut panel.ai_chat);
    let mut run=run::Run::new();run.scope_key=panel.local_log_scope_key(&chat);chat.run=Some(run);
    assert!(panel.begin_platform_call(&ctx,&mut chat,"log.analyze".into(),json!({"path":"C:/secret"}),"analyze".into(),false).is_ok());
    panel.poll_platform(&ctx,&mut chat,Lang::Zh);
    assert!(chat.tool_results.iter().any(|row|row["method"]=="log.analyze" && row["result"]["ok"]==false && row["result"]["executed"]==false));
    assert!(chat.run.as_ref().is_some_and(|run|run.phase!="cancelled"));
}

#[test]
fn reviewed_automatic_device_action_is_rejected_before_dispatch() {
    let ctx = egui::Context::default();
    let mut panel = fixture();
    let mut chat = std::mem::take(&mut panel.ai_chat);
    chat.review_outbound = true;
    let error = panel.begin_platform_call(&ctx, &mut chat, "serial.monitor.start".into(), json!({}), "start".into(), true).unwrap_err();
    assert!(error.contains("AI.MONITOR_REVIEW_CONTROL_UNAVAILABLE"));
    assert!(chat.tool.is_none());
}

#[test]
fn late_platform_reply_cannot_enter_replacement_task() {
    let ctx = egui::Context::default();
    let (broker, requests) = crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), broker));
    let mut panel = fixture();
    let mut chat = std::mem::take(&mut panel.ai_chat);
    chat.run = Some(run::Run::new());
    panel.begin_platform_call(&ctx, &mut chat, "instrument.list".into(), json!({}), "old task".into(), false).unwrap();
    let request = requests.try_recv().unwrap();
    let old_id = chat.run.as_ref().unwrap().id.clone();
    chat.run = Some(run::Run::new());
    assert_ne!(old_id, chat.run.as_ref().unwrap().id);
    request.reply.send(crate::backend::InvokeReply { ok:true, cmd:"instrument.list".into(), data:json!({"old_marker":"STALE"}), error:None }).unwrap();
    panel.poll_platform(&ctx, &mut chat, Lang::Zh);
    assert!(chat.tool_results.is_empty());
    assert!(chat.run.as_ref().unwrap().workflow.citation_method("S1").is_none());
    assert!(chat.pending.is_none());
    assert!(!chat.entries.iter().any(|entry|entry.text.contains("STALE")));
}

#[test]
fn broker_guard_rejects_project_switch_even_if_device_is_unchanged() {
    let mut panel = fixture();
    let guard = panel.test_broker_control_guard("ai-tools");
    assert!(panel.ai_platform_guard_valid(&guard));
    panel.ai_chat.project = "other-project".into();
    assert!(!panel.ai_platform_guard_valid(&guard));
}

#[test]
fn automatic_reply_does_not_enter_a_replacement_watch_on_same_port() {
    let ctx = egui::Context::default();
    let (broker, requests) = crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), broker));
    let mut panel = fixture();
    panel.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut panel.ai_chat);
    panel.begin_ai_watch(&mut chat, "first".into(), false).unwrap();
    panel.begin_platform_call(&ctx, &mut chat, "instrument.list".into(), json!({}), "first".into(), true).unwrap();
    let request = requests.try_recv().unwrap();
    chat.watch = None;
    panel.begin_ai_watch(&mut chat, "second".into(), false).unwrap();
    request.reply.send(crate::backend::InvokeReply{ok:true,cmd:"instrument.list".into(),data:json!({"old_marker":"STALE_WATCH"}),error:None}).unwrap();
    panel.poll_platform(&ctx, &mut chat, Lang::Zh);
    assert!(chat.automatic_tool_results.is_empty());
    assert!(!chat.watch.as_ref().unwrap().ai_paused);
    assert!(!chat.entries.iter().any(|entry|entry.text.contains("STALE_WATCH")));
}

#[test]
fn serial_charging_skill_uses_log_events_and_sourced_waveform() {
    use std::io::Read;
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let log=std::fs::read_to_string(root.join("tools/agent-eval/charging-fixture.txt")).unwrap();
    let mut wave=crate::waveform_analysis::WaveformAnalysisPanel::new(&AppConfig::default());
    assert!(wave.test_load_wave_without_persisting(&root.join("sample_waveforms/Tektronix_WFM/tek_4ch_sine_square_tri_saw.wfm")).unwrap()>0);
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||for round in 0..4 {
        let mut req=server.recv_timeout(Duration::from_secs(15)).unwrap().expect("model request");let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();let _:serde_json::Value=serde_json::from_str(&body).unwrap();
        let(name,args)=match round {
            0=>("wiparse_log_events",json!({"active_pattern":"power_state:1","inactive_pattern":"power_state:0","reason_pattern":"cause:([A-Z_]+)","context_lines":1})),
            1=>("wiparse_call",json!({"method":"engineering.wave.measure","params":{"channel":0}})),
            2=>("wiparse_plan",json!({"steps":[{"id":"skill_charging_protection","title":"充电保护与复充","state":"done","deliverable":"analysis","evidence_ids":["S2","S3"]}]})),
            _=>("wiparse_answer",json!({"reply":"独立样本时钟未同步，不能断言因果 [S2][S3]。","status":"completed","evidence_ids":["S2","S3"]})),
        };
        let output=json!({"model":"mock","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":format!("call-{round}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string();
        req.respond(tiny_http::Response::from_string(output).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);let api=crate::backend::ApiBridge::new();ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"),api.clone()));
    let mut panel=fixture();panel.ai_chat.set_context("ai-tools".into(),&config);panel.tabs[0].append_lines(log.lines().map(str::to_owned).collect::<Vec<_>>(),true);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.selected_skills=vec!["charging_protection".into()];let state=panel.ai_state("ai-tools",false);
    panel.begin_manual_agent(&ctx,&mut chat,&state,"分析充电保护".into()).unwrap();panel.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(20);let mut calls=0;
    while Instant::now()<deadline {panel.tick_ai(&ctx,Lang::Zh,"ai-tools".into(),&config,false);if let Some(req)=api.try_recv(){
        assert_eq!(req.method,"engineering.wave.measure");let guard=req.ai_guard.as_ref().unwrap();assert!(panel.ai_platform_guard_valid(guard));
        let value=wave.agent_measure(&req.params,&ctx,guard.cancelled.clone()).receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value["ok"],true);assert!(value["data"]["source_window"]["trace_sha256"].as_str().is_some_and(|s|s.len()==64));assert_eq!(value["data"]["external_alignment_verified"],false);
        req.reply.send(crate::backend::InvokeReply{ok:true,cmd:req.method,data:value["data"].clone(),error:None}).unwrap();calls+=1;
    }if !panel.ai_chat.working(){break;}thread::sleep(Duration::from_millis(5));}
    assert_eq!(calls,1);assert!(!panel.ai_chat.status_error,"{}",panel.ai_chat.status);assert_eq!(panel.ai_chat.run.as_ref().unwrap().phase,"completed");
    assert!(panel.ai_chat.tool_results.iter().any(|r|r["result"]["data"]["state_transition_analysis"]["confirmed_active_to_inactive_transitions"]==2));
    assert!(panel.ai_chat.entries.iter().any(|e|e.role=="assistant"&&e.text.contains("不能断言因果")));worker.join().unwrap();
}

#[test]
fn serial_qi_skill_reads_decoded_isf_through_broker_and_finishes() {
    use std::io::Read;
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/ddsss_vctx.isf");
    let mut wave=crate::waveform_analysis::WaveformAnalysisPanel::new(&AppConfig::default());
    assert!(wave.test_load_wave_without_persisting(&path).unwrap()>0);
    let started=wave.agent_decode(&json!({"kind":"ddsss","signal":0,"sequence":"seqa","extension":"off","fop":128000}),Lang::Zh);
    assert!(started["_agent_wait"].is_object(),"{started}");let until=Instant::now()+Duration::from_secs(20);
    loop {wave.agent_tick(Lang::Zh);let result=wave.agent_frames(&json!({"offset":0,"limit":20}));
        if result["ok"]==true {assert_eq!(result["data"]["total_matches"],5);break;}
        assert!(Instant::now()<until,"{result}");thread::sleep(Duration::from_millis(5));}
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||for round in 0..2 {
        let mut req=server.recv_timeout(Duration::from_secs(15)).unwrap().expect("model request");let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();
        let input:serde_json::Value=serde_json::from_str(&body).unwrap();assert!(input.to_string().contains("S1"));
        let (name,args)=if round==0 {("wiparse_plan",json!({"steps":[{"id":"skill_qi_protocol","title":"Qi 协议分析","state":"done","deliverable":"analysis","evidence_ids":["S1"]}]}))}
            else {("wiparse_answer",json!({"reply":"合成波形包含 5 帧 [S1]，不代表实测通信质量。","status":"completed","evidence_ids":["S1"]}))};
        let output=json!({"model":"mock","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":format!("call-{round}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string();
        req.respond(tiny_http::Response::from_string(output).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let (tx,rx)=crossbeam_channel::bounded::<crate::ai::broker::Request>(8);ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-canonical-agent"),tx));
    let mut panel=fixture();panel.ai_chat.set_context("ai-tools".into(),&config);let mut chat=std::mem::take(&mut panel.ai_chat);chat.selected_skills=vec!["qi_protocol".into()];
    let state=panel.ai_state("ai-tools",false);panel.begin_manual_agent(&ctx,&mut chat,&state,"分析 Qi 协议".into()).unwrap();panel.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(20);let mut calls=0;
    while Instant::now()<deadline {
        panel.tick_ai(&ctx,Lang::Zh,"ai-tools".into(),&config,false);
        if let Ok(req)=rx.try_recv() {
            assert_eq!(req.method,"wave.frames");assert!(panel.ai_platform_guard_valid(&req.guard));
            let mut call=wiparse_core::engineering_agent::Call{method:req.method.clone(),params:req.params.clone()};
            let mut item=crate::ai::execution::HostItem::default();item.collect_frames(&call);let mut internal=false;
            let value=loop {let page=wave.agent_frames(&call.params);calls+=1;if let Some(result)=item.advance(page,&call,internal){break result;}
                call=item.take_due(Instant::now()).expect("frame continuation");internal=true;};
            assert_eq!(value["ok"],true);assert_eq!(value["data"]["total_matches"],5);assert_eq!(value["data"]["frames"].as_array().unwrap().len(),5);assert_eq!(value["data"]["decoder_truncated"],false);
            req.reply.send(crate::backend::InvokeReply{ok:true,cmd:req.method,data:value["data"].clone(),error:None}).unwrap();
        }
        if !panel.ai_chat.working(){break;}thread::sleep(Duration::from_millis(5));
    }
    assert!(calls>=1);assert!(!panel.ai_chat.status_error,"{}",panel.ai_chat.status);assert_eq!(panel.ai_chat.run.as_ref().unwrap().phase,"completed");
    assert!(panel.ai_chat.entries.iter().any(|e|e.role=="assistant"&&e.text.contains("5 帧")));worker.join().unwrap();
}

#[test]
fn serial_wave_skill_measures_wfm_through_api_bridge_and_finishes() {
    use std::io::Read;
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sample_waveforms/Tektronix_WFM/tek_4ch_sine_square_tri_saw.wfm");
    let mut wave=crate::waveform_analysis::WaveformAnalysisPanel::new(&AppConfig::default());
    assert!(wave.test_load_wave_without_persisting(&path).unwrap()>0);
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||{
        for round in 0..3 {
            let mut req=server.recv_timeout(Duration::from_secs(15)).unwrap().expect("model request");let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();
            let input:serde_json::Value=serde_json::from_str(&body).unwrap();if round>0 {assert!(input.to_string().contains("S1"));}
            let (name,args)=match round {
                0=>("wiparse_call",json!({"method":"engineering.wave.measure","params":{"channel":0}})),
                1=>("wiparse_plan",json!({"steps":[{"id":"skill_waveform","title":"波形分析","state":"done","deliverable":"analysis","evidence_ids":["S1"]}]})),
                _=>("wiparse_answer",json!({"reply":"完成波形测量；未设置判定限值 [S1]。","status":"completed","evidence_ids":["S1"]})),
            };
            let output=json!({"model":"mock","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":format!("call-{round}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string();
            req.respond(tiny_http::Response::from_string(output).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
        }
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let api=crate::backend::ApiBridge::new();ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"),api.clone()));
    let mut panel=fixture();panel.ai_chat.set_context("ai-tools".into(),&config);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.selected_skills=vec!["waveform".into()];
    let state=panel.ai_state("ai-tools",false);panel.begin_manual_agent(&ctx,&mut chat,&state,"分析波形".into()).unwrap();panel.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(20);let mut calls=0;
    while Instant::now()<deadline {
        panel.tick_ai(&ctx,Lang::Zh,"ai-tools".into(),&config,false);
        if let Some(req)=api.try_recv() {
            assert_eq!(req.method,"engineering.wave.measure");let guard=req.ai_guard.as_ref().unwrap();assert!(panel.ai_platform_guard_valid(guard));
            let value=wave.agent_measure(&req.params,&ctx,guard.cancelled.clone()).receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(value["ok"],true);assert!(value["data"]["sample_count"].as_u64().is_some_and(|n|n>0));
            assert_eq!(value["data"]["quality_complete"],true);assert_eq!(value["data"]["external_alignment_verified"],false);
            assert!(value["data"]["source_window"]["trace_sha256"].as_str().is_some_and(|s|s.len()==64));
            req.reply.send(crate::backend::InvokeReply{ok:true,cmd:req.method,data:value["data"].clone(),error:None}).unwrap();calls+=1;
        }
        if !panel.ai_chat.working(){break;}thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(calls,1);assert!(!panel.ai_chat.status_error,"{}",panel.ai_chat.status);
    assert_eq!(panel.ai_chat.run.as_ref().unwrap().phase,"completed");
    assert!(panel.ai_chat.entries.iter().any(|e|e.role=="assistant"&&e.text.contains("未设置判定限值")));worker.join().unwrap();
}

#[test]
fn mock_serial_session_reads_adapter_matrix_through_real_broker_and_runs_recipe() {
    use std::io::Read;
    let root=tempfile::tempdir().unwrap();let ctx=egui::Context::default();
    let mut host=crate::app::agent_host::SerialBrokerTestHost::new(root.path(),&ctx);
    let project=host.project();let dir=wiparse_core::project::project_dir(root.path(),&project);std::fs::create_dir_all(&dir).unwrap();
    let path=dir.join("report.md");std::fs::write(&path,"# Results\n[Matrix](adapter-matrix.json)\n").unwrap();
    std::fs::write(dir.join("adapter-matrix.json"),json!({"run_id":"demo","mode":"replay","rows":[{"case_id":"A","check_id":"1"},{"case_id":"A","check_id":"2"}]}).to_string()).unwrap();host.select_report(&path);
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||{
        for round in 0..4 {
            let mut req=server.recv_timeout(Duration::from_secs(15)).unwrap().expect("model request");
            let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();
            let (name,args)=match round {
                0=>("wiparse_report_read",json!({"attachment":"adapter-matrix.json"})),
                1=>{assert!(body.contains("adapter_rows"));("wiparse_call",json!({"method":"agent.recipe","params":{"recipe":"recipe.adapter_compatibility","evidence_id":"S2"}}))},
                2=>{assert!(body.contains("check_rows"));("wiparse_plan",json!({"steps":[{"id":"skill_test_result","title":"测试结果分析","state":"done","deliverable":"analysis","evidence_ids":["S1"]}]}))},
                _=>("wiparse_answer",json!({"reply":"模拟矩阵共 1 个用例、2 个检查项 [S2]。","status":"completed","evidence_ids":["S2"]}))
            };
            let output=json!({"model":"mock","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":format!("call-{round}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string();
            req.respond(tiny_http::Response::from_string(output).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
        }
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut panel=fixture();panel.ai_chat.set_context(project.clone(),&config);let mut chat=std::mem::take(&mut panel.ai_chat);chat.selected_skills=vec!["test_result".into()];
    let state=panel.ai_state(&project,false);panel.begin_manual_agent(&ctx,&mut chat,&state,"分析测试报告和适配器矩阵".into()).unwrap();panel.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(30);
    while Instant::now()<deadline {
        panel.tick_ai(&ctx,Lang::Zh,project.clone(),&config,false);host.tick(&ctx,&mut panel);
        if !panel.ai_chat.working(){break;}thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.ai_chat.working(),"{}",panel.ai_chat.status);assert!(!panel.ai_chat.status_error,"{}",panel.ai_chat.status);
    assert_eq!(panel.ai_chat.run.as_ref().unwrap().phase,"completed");
    assert!(panel.ai_chat.tool_results.iter().any(|r|r["method"]=="agent.recipe" && r.to_string().contains("check_rows")));
    assert_eq!(host.approvals,0,"read-only analysis must not invoke approval");worker.join().unwrap();
}

#[test]
fn serial_report_skill_reads_demo_markdown_through_broker_and_finishes() {
    use std::io::Read;
    use serde_json::Value;
    let root=tempfile::tempdir().unwrap();
    let project=wiparse_core::project::project_dir(root.path(),"ai-tools");
    std::fs::create_dir_all(&project).unwrap();
    let source=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../projects/default/tests/ScopeSerialDemo/runs/20261016_120000/artifacts/reports/ScopeSerialDemo_summary_20261016_120000.md");
    let path=project.join("demo-report.md");std::fs::copy(source,&path).unwrap();
    let mut report_config=AppConfig::default();report_config.apps.project.active_id="ai-tools".into();
    let mut report=crate::test_report::TestReportPanel::new(&report_config);
    report.test_select_report_without_persisting(&path);
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
    let worker=thread::spawn(move||{
        for round in 0..2 {
            let mut req=server.recv_timeout(Duration::from_secs(15)).unwrap().expect("model request");
            let mut body=String::new();req.as_reader().read_to_string(&mut body).unwrap();
            let input:Value=serde_json::from_str(&body).unwrap();assert!(input.to_string().contains("S1"));
            let (name,args)=if round==0 {("wiparse_plan",json!({"steps":[{"id":"skill_test_result","title":"测试结果分析","state":"done","deliverable":"analysis","evidence_ids":["S1"]}]}))}
                else {("wiparse_answer",json!({"reply":"这是模拟报告，不能作为实测结论 [S1]。","status":"completed","evidence_ids":["S1"]}))};
            let output=json!({"model":"mock","choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":format!("call-{round}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string();
            req.respond(tiny_http::Response::from_string(output).with_header(tiny_http::Header::from_bytes("Content-Type","application/json").unwrap())).unwrap();
        }
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let (broker_tx,broker_rx)=crossbeam_channel::bounded::<crate::ai::broker::Request>(8);
    ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-canonical-agent"),broker_tx));
    let mut panel=fixture();panel.ai_chat.set_context("ai-tools".into(),&config);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.selected_skills=vec!["test_result".into()];
    let state=panel.ai_state("ai-tools",false);panel.begin_manual_agent(&ctx,&mut chat,&state,"分析测试报告".into()).unwrap();panel.ai_chat=chat;
    let deadline=Instant::now()+Duration::from_secs(20);let mut pages=0;
    while Instant::now()<deadline {
        panel.tick_ai(&ctx,Lang::Zh,"ai-tools".into(),&config,false);
        if let Ok(req)=broker_rx.try_recv() {
            assert_eq!(req.method,"report.read");assert!(panel.ai_platform_guard_valid(&req.guard));
            let mut call=wiparse_core::engineering_agent::Call{method:req.method.clone(),params:req.params.clone()};
            call.params["limit"]=json!(8);
            let mut item=crate::ai::execution::HostItem::default();item.collect_report(&call);
            let mut internal=false;
            let value=loop {
                let response=report.agent_read(&call.params,root.path(),&ctx,req.guard.cancelled.clone()).receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
                pages+=1;
                if let Some(result)=item.advance(response,&call,internal){break result;}
                call=item.take_due(Instant::now()).expect("continuation");internal=true;
            };
            assert_eq!(value["ok"],true);assert_eq!(value["data"]["reached_eof"],true);
            assert_eq!(value["data"]["lines"].as_array().unwrap().len(),125);
            req.reply.send(crate::backend::InvokeReply{ok:true,cmd:req.method,data:value["data"].clone(),error:None}).unwrap();
        }
        if !panel.ai_chat.working(){break;}thread::sleep(Duration::from_millis(5));
    }
    assert!(pages>=2);assert!(!panel.ai_chat.status_error,"{}",panel.ai_chat.status);
    assert_eq!(panel.ai_chat.run.as_ref().unwrap().phase,"completed");
    assert!(panel.ai_chat.entries.iter().any(|entry|entry.role=="assistant"&&entry.text.contains("模拟报告")));
    worker.join().unwrap();
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
    assert!(platform::validate("instrument.waveform_source", &json!({"device_id":1})).unwrap());
}

#[test]
fn queued_controls_are_fenced_and_cancelled_before_dispatch() {
    let ctx = egui::Context::default();
    let (broker, requests) = crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("wiparse-canonical-agent"), broker));
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
    let root = tempfile::tempdir().unwrap();
    let mut task = run::Run::new();
    task.journal = Some(crate::ai::journal::Journal::at(root.path(), "ai-tools"));
    task.scope_key=p.local_log_scope_key(&chat);
    chat.run = Some(task);
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
    let req = requests.try_recv().unwrap();
    let guard = req.guard;
    assert!(p.ai_platform_guard_valid(&guard));
    p.baud = "9600".into();
    assert!(!p.ai_platform_guard_valid(&guard));
    p.ai_chat.cancel();
    assert!(guard.cancelled.load(Ordering::Acquire));
    assert!(!p.ai_chat.allow_automation);
}

#[test]
fn late_automatic_broker_reply_cannot_cross_source_switch_or_consume_queue() {
    let ctx = egui::Context::default();
    let (broker, requests) = crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), broker));
    let mut p = fixture();
    let config = AiConfig { enabled: true, model: "test-model".into(), ..Default::default() };
    p.primary_device = Some("COM-test".into());
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.conversation_source = Some((state.binding.source_id, state.binding.device.clone()));
    p.begin_ai_watch(&mut chat, "old watch".into(), false).unwrap();
    let scope_key = p.local_log_scope_key(&chat);
    chat.enqueue_message("queued manual question".into(), &state, scope_key).unwrap();
    p.begin_platform_call(&ctx, &mut chat, "instrument.list".into(), json!({}), "old watch".into(), true).unwrap();
    let request = requests.try_recv().unwrap();
    p.ai_chat = chat;

    p.tabs[0].clear_display();
    p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
    assert!(request.guard.cancelled.load(Ordering::Acquire));
    assert!(p.ai_chat.watch.is_none());
    assert!(p.ai_chat.queue_paused);
    assert_eq!(p.ai_chat.messages.len(), 1);
    let late = crate::backend::InvokeReply { ok: true, cmd: "instrument.list".into(), error: None,
        data: json!({"late_source_marker":"OLD_SOURCE"}) };
    assert!(request.reply.try_send(late).is_err(), "old result receiver must have been dropped");
    assert!(!p.ai_chat.entries.iter().any(|entry| entry.text.contains("OLD_SOURCE")));
    assert!(p.ai_chat.automatic_tool_results.is_empty());

    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "new watch".into(), false).unwrap();
    let before = chat.alert_count;
    chat.observe(p.tabs[0].ai_view_id(), &["ERROR from new source".into()]);
    assert_eq!(chat.alert_count, before + 1);
    assert_eq!(chat.messages.len(), 1);
    assert!(chat.queue_paused);
}

#[test]
fn delayed_automatic_model_reply_is_discarded_after_source_switch() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let config = AiConfig { enabled: true, base_url: format!("http://{}", server.server_addr()), model: "mock".into(), ..Default::default() };
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let request = server.recv_timeout(Duration::from_secs(8)).unwrap().expect("mock request");
        arrived_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(8)).unwrap();
        let content = json!({"reply":"OLD_SOURCE_MARKER", "actions":[], "status":"done"});
        let body = json!({"model":"mock","choices":[{"message":{"content":content.to_string()},"finish_reason":"stop"}]}).to_string();
        let _ = request.respond(tiny_http::Response::from_string(body));
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture();
    p.ai_chat.set_context("serial-ai-test".into(), &config);
    p.primary_device = Some("COM-test".into());
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.conversation_source = Some((state.binding.source_id, state.binding.device.clone()));
    p.begin_ai_watch(&mut chat, "old watch".into(), false).unwrap();
    let key = p.local_log_scope_key(&chat);
    chat.enqueue_message("manual queued question".into(), &state, key).unwrap();
    chat.submit(&ctx, &state, json!({}), "old watch".into(), true).unwrap();
    p.ai_chat = chat;
    arrived_rx.recv_timeout(Duration::from_secs(8)).unwrap();
    p.tabs[0].clear_display();
    p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
    assert!(p.ai_chat.pending.is_none());
    assert!(p.ai_chat.watch.is_none());
    assert!(p.ai_chat.queue_paused);
    assert_eq!(p.ai_chat.messages.len(), 1);
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
    assert!(!p.ai_chat.entries.iter().any(|entry| entry.text.contains("OLD_SOURCE_MARKER")));
    assert_eq!(p.ai_chat.messages.len(), 1);
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "new watch".into(), false).unwrap();
    let before = chat.alert_count;
    chat.observe(p.tabs[0].ai_view_id(), &["ERROR from new source".into()]);
    assert_eq!(chat.alert_count, before + 1);
}

#[test]
fn revoked_automatic_model_request_never_regains_control_after_reenable() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let config = AiConfig { enabled: true, base_url: format!("http://{}", server.server_addr()), model: "mock".into(), ..Default::default() };
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let request = server.recv_timeout(Duration::from_secs(8)).unwrap().expect("mock request");
        arrived_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(8)).unwrap();
        let content = json!({"reply":"mock stop", "actions":[{"type":"serial.stop"}], "status":"working"});
        let body = json!({"model":"mock","choices":[{"message":{"content":content.to_string()},"finish_reason":"stop"}]}).to_string();
        let _ = request.respond(tiny_http::Response::from_string(body));
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut p = fixture();
    p.ai_chat.set_context("serial-ai-test".into(), &config);
    p.primary_device = Some("COM-test".into());
    p.ai_confirmed.insert("COM-test".into(), 115200);
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.conversation_source = Some((state.binding.source_id, state.binding.device.clone()));
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    chat.allow_automation = true;
    chat.submit(&ctx, &state, json!({}), "watch".into(), true).unwrap();
    assert!(chat.pending.as_ref().unwrap().automation_granted);
    p.ai_chat = chat;
    arrived_rx.recv_timeout(Duration::from_secs(8)).unwrap();
    p.ai_chat.allow_automation = false;
    p.ai_chat.revoke_controls();
    p.ai_chat.allow_automation = true;
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while p.ai_chat.pending.is_some() && Instant::now() < deadline {
        p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(p.ai_chat.pending.is_none());
    assert!(p.ai_chat.control_approval.is_none(), "old request regained control authority");
    assert!(p.ai_chat.tool.is_none());
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn revoked_dispatched_control_ignores_late_success_and_requires_readback() {
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let (broker, requests) = crossbeam_channel::bounded::<crate::ai::broker::Request>(2);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), broker));
    let mut p = fixture();
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let mut task = run::Run::new();
    let journal = crate::ai::journal::Journal::at(root.path(), "serial-ai-test");
    task.journal = Some(journal);
    task.scope_key=p.local_log_scope_key(&chat);
    chat.run = Some(task);
    let params = json!({"device_id":1,"command":{"SourceVoltage":{"channel":1,"value":5.0}}});
    p.begin_platform_call(&ctx, &mut chat, "instrument.command".into(), params.clone(), "demo only".into(), false).unwrap();
    let request = requests.try_recv().unwrap();
    // The fake Broker has received the request; treat its physical outcome as unknown.
    chat.allow_automation = false;
    chat.revoke_controls();
    assert!(request.guard.cancelled.load(Ordering::Acquire));
    request.reply.send(crate::backend::InvokeReply { ok: true, cmd: "instrument.command".into(), error: None,
        data: json!({"state":"completed","late_marker":"LATE_SUCCESS"}) }).unwrap();
    p.poll_platform(&ctx, &mut chat, Lang::Zh);
    assert!(!chat.allow_automation);
    assert!(chat.run.as_ref().unwrap().uncertain.len() == 1);
    assert!(!chat.tool_results.iter().any(|result| result.to_string().contains("LATE_SUCCESS")));
    assert!(chat.tool_results.iter().any(|result| result["result"]["unknown_if_already_dispatched"] == true));
    let db_path = wiparse_core::project::project_dir(root.path(), "serial-ai-test").join("agent-tasks.sqlite3");
    let db = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let dispatched: i64 = db.query_row("SELECT count(*) FROM observations WHERE json_extract(reference,'$.state')='dispatching' AND json_extract(reference,'$.command_fingerprint') IS NOT NULL", [], |row| row.get(0)).unwrap();
    assert_eq!(dispatched, 1, "missing persistent dispatch audit");
    let unresolved: i64 = db.query_row("SELECT count(*) FROM observations d WHERE json_extract(d.reference,'$.state')='dispatching' AND NOT EXISTS (SELECT 1 FROM observations c WHERE c.task=d.task AND c.seq>d.seq AND json_extract(c.reference,'$.state') IN ('accepted','completed') AND json_extract(c.reference,'$.command_id')=json_extract(d.reference,'$.command_id'))", [], |row| row.get(0)).unwrap();
    assert_eq!(unresolved, 1, "late success must not resolve the dispatch audit");
    chat.allow_automation = true;
    let error = p.begin_platform_call(&ctx, &mut chat, "instrument.command".into(), params, "retry".into(), false).unwrap_err();
    assert!(error.contains("AI.OUTCOME_UNKNOWN"), "{error}");
    assert!(requests.try_recv().is_err(), "uncertain command was retried");
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
    chat.observe(p.tabs[0].ai_view_id(),&["VIN stable".into()]);
    p.ai_chat = chat;
    p.tabs[0].append_lines(vec!["ERROR timeout".into()], true);
    p.ai_chat
        .observe(p.tabs[0].ai_view_id(), &["ERROR timeout".into()]);
    p.tick_ai(&ctx, Lang::Zh, "ai-tools".into(), &config, false);
    let frozen=p.ai_chat.watch.as_ref().unwrap().first_trigger.as_ref().unwrap();
    assert!(frozen.contains("VIN stable") && frozen.contains("ERROR timeout"));
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
                json!({"reply":"先读取实际频率。","actions":[{"type":"platform.call","method":"instrument.command","params":{"device_id":1,"command":{"ScopeMeasure":{"channel":1,"meas_type":"Frequency"}}}}]})
            } else {
                json!({"reply":"实际测量频率为1 kHz。","actions":[]})
            };
            let response=json!({"model":"mock","choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]}).to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
    });
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let root = tempfile::tempdir().unwrap();
    let mut host = crate::app::agent_host::SerialBrokerTestHost::new(root.path(), &ctx);
    let project = host.project();
    let mut p = fixture();
    p.ai_chat.set_context(project.clone(), &config);
    p.ai_chat.allow_automation = true;
    let mut task = run::Run::new();
    task.journal = Some(crate::ai::journal::Journal::at(root.path(), &project));
    task.scope_key = p.local_log_scope_key(&p.ai_chat);
    p.ai_chat.run = Some(task);
    let state = p.ai_state(&project, false);
    p.ai_chat
        .submit(&ctx, &state, json!({}), "读取示波器实际频率".into(), false)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        p.tick_ai(&ctx, Lang::Zh, project.clone(), &config, false);
        host.tick(&ctx, &mut p);
        if p.ai_chat
            .entries
            .iter()
            .any(|e| e.text == "实际测量频率为1 kHz。")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(host.approvals, 1);
    assert!(host.observed_waits >= 1);
    assert!(p.ai_chat.tool.is_none());
    assert!(p.ai_chat.pending.is_none());
    assert!(p
        .ai_chat
        .entries
        .iter()
        .any(|e| e.text == "实际测量频率为1 kHz。"));
    let _ = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(second.contains("1 kHz"));
    assert!(second.contains("completed"));
    assert!(!p.ai_chat.history.iter().any(|message|matches!(message,AiMessage::User(text) if text.contains("Verified platform result"))));
    assert!(p.ai_chat.tool_results.iter().any(|value|value.to_string().contains("1 kHz")), "actual tool receipts: {:?}", p.ai_chat.tool_results);
    server.join().unwrap();
}

#[test]
fn broker_control_guard_rejects_scope_change_without_device_change() {
    let mut panel=fixture();
    let guard=panel.test_broker_control_guard("ai-tools");
    assert!(panel.ai_platform_guard_valid(&guard));
    panel.ai_chat.scope=DataScope::Range;panel.ai_chat.range_first=1;panel.ai_chat.range_last=5;
    assert!(!panel.ai_platform_guard_valid(&guard));
}

#[test]
fn failed_read_receipt_does_not_release_later_queued_device_action() {
    let ctx=egui::Context::default();
    let (broker,requests)=crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|data|data.insert_temp(egui::Id::new("wiparse-canonical-agent"),broker));
    let mut panel=fixture();let mut chat=panel.take_ai_chat();
    let mut run=run::Run::new();run.scope_key=panel.local_log_scope_key(&chat);
    run.queued.push_back(Action::SerialStop);chat.run=Some(run);
    panel.begin_platform_call(&ctx,&mut chat,"instrument.list".into(),json!({}),"fixture".into(),false).unwrap();
    let request=requests.try_recv().unwrap();
    assert!(request.reply.send(crate::backend::InvokeReply{ok:false,cmd:"instrument.list".into(),data:json!({}),error:Some("read failure".into())}).is_ok());
    panel.poll_platform(&ctx,&mut chat,Lang::Zh);
    assert!(chat.queue_paused);
    assert!(chat.run.as_ref().unwrap().queued.is_empty());
    assert!(chat.control_approval.is_none());
    assert!(chat.tool.is_none());
}

#[test]
fn test_control_permission_does_not_unlock_instrument_control() {
    let mut panel=fixture();
    let guard=panel.test_broker_control_guard("ai-tools");
    panel.ai_test_locked=true;
    let run=panel.ai_chat.run.as_mut().unwrap();run.control_cap=false;run.test_control_cap=true;
    assert!(panel.ai_platform_guard_for(&guard,"testing.stop",&json!({})));
    assert!(panel.ai_platform_guard_for(&guard,"testing.pause",&json!({})));
    assert!(panel.ai_platform_guard_for(&guard,"testing.resume",&json!({})));
    assert!(!panel.ai_platform_guard_for(&guard,"instrument.command",&json!({})));
    panel.ai_chat.revoke_controls();
    assert!(!panel.ai_platform_guard_for(&guard,"testing.stop",&json!({})));
}
