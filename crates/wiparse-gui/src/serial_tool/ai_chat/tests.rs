use super::*;

#[test]
fn host_source_transition_retires_old_evidence_and_native_context() {
    let panel = fixture();
    let binding = panel.ai_state("serial-ai-test", false).binding;
    let mut chat = SerialAiChat::default();
    let mut run = run::Run::new();
    run.source_binding = Some(binding.clone());
    run.scope_key = "old".into();
    run.workflow.record_call("S1", "log.analyze", &json!({}), &json!({"ok":true,"data":{"value":1}}));
    run.previous_evidence.push(json!({"old":true}));
    run.uncertain.push("unknown-device-action".into());
    chat.run = Some(run);
    chat.history.push(AiMessage::User("old context".into()));
    chat.tool_results.push(json!({"old":true}));
    let mut new_binding = binding; new_binding.capture_epoch += 1;
    chat.rebind_host_source(&new_binding,"new");
    let run = chat.run.as_ref().unwrap();
    assert!(run.workflow.citation_method("S1").is_none());
    assert_eq!(run.workflow.source_epoch(),1);
    assert_eq!(run.source_binding.as_ref(),Some(&new_binding));
    assert!(run.previous_evidence.is_empty());
    assert_eq!(run.uncertain,vec!["unknown-device-action"]);
    assert!(chat.history.is_empty() && chat.tool_results.is_empty());
}

#[test]
fn control_approval_is_bound_to_source_skill_task_permission_and_deadline() {
    let p = fixture();
    let state = p.ai_state("serial-ai-test", false);
    let now = Instant::now();
    let approval = ControlApproval { action: Action::SerialStop, binding: state.binding.clone(),
        skills: vec!["serial_log".into()], question: "stop".into(), automatic: false,
        created: now, run_id: Some("task-1".into()), monitor_revision: None };
    let skills = vec!["serial_log".into()];
    assert!(approval.valid(&state, &skills, Some("task-1"), true, "", now));
    assert!(!approval.valid(&state, &skills, Some("task-1"), false, "", now));
    assert!(!approval.valid(&state, &[], Some("task-1"), true, "", now));
    assert!(!approval.valid(&state, &skills, Some("task-2"), true, "", now));
    assert!(!approval.valid(&state, &skills, Some("task-1"), true, "", now + Duration::from_secs(600)));
    let mut changed = state.clone(); changed.binding.capture_epoch += 1;
    assert!(!approval.valid(&changed, &skills, Some("task-1"), true, "", now));
    let mut changed = state.clone(); changed.binding.baud += 1;
    assert!(!approval.valid(&changed, &skills, Some("task-1"), true, "", now));
    let mut changed = state.clone(); changed.binding.selected_device = Some("another device".into());
    assert!(!approval.valid(&changed, &skills, Some("task-1"), true, "", now));
}

#[test]
fn monitoring_changes_require_confirmation_but_not_a_device_grant() {
    let p = fixture();
    let state = p.ai_state("serial-ai-test", false);
    let now = Instant::now();
    let approval = ControlApproval { action: Action::MonitorStop, binding: state.binding.clone(),
        skills: vec![], question: "stop alerts".into(), automatic: false, created: now, run_id: None,
        monitor_revision: Some("rules-v1".into()) };
    assert!(approval.valid(&state, &[], None, false, "rules-v1", now));
    assert!(!approval.valid(&state, &[], None, false, "rules-v2", now));
    let mut changed = state.clone(); changed.binding.capture_epoch += 1;
    assert!(!approval.valid(&changed, &[], None, false, "rules-v1", now));
    let mut chat = SerialAiChat::default();
    let initial = chat.monitor_revision();
    chat.rules.contains.push("OVP".into());
    assert_ne!(initial, chat.monitor_revision());
}

#[test]
fn confirmed_platform_permit_is_exact_and_one_use() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let ctx = egui::Context::default();
    chat.confirmed_platform_action = Some(("serial.monitor.start".into(), json!({})));
    p.test_bind_serial_permit(&mut chat);
    let error = p.begin_platform_call(&ctx, &mut chat, "serial.monitor.stop".into(), json!({}), "stop".into(), false).unwrap_err();
    assert!(error.contains("AI.CONCRETE_CONFIRMATION_REQUIRED"));
    assert!(chat.confirmed_platform_action.is_none());
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
    assert!(p.begin_platform_call(&ctx, &mut chat, "serial.monitor.stop".into(), json!({}), "stop".into(), false).is_err());
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn serial_confirmation_cannot_follow_a_session_or_authorization_change() {
    for change in ["session", "authorization", "skills", "task"] {
        let mut panel=fixture();
        panel.primary_device=Some("COM-test".into());
        let mut chat=std::mem::take(&mut panel.ai_chat);
        chat.allow_automation=true;
        chat.run=Some(run::Run::new());
        chat.confirmed_platform_action=Some(("serial.monitor.stop".into(),json!({})));
        panel.test_bind_serial_permit(&mut chat);
        match change {
            "session" => panel.ai_epoch=panel.ai_epoch.wrapping_add(1),
            "authorization" => chat.allow_automation=false,
            "skills" => chat.selected_skills.push("serial_log".into()),
            _ => chat.run=Some(run::Run::new()),
        }
        assert!(panel.begin_platform_call(&egui::Context::default(),&mut chat,
            "serial.monitor.stop".into(),json!({}),"stop".into(),false).is_err());
        assert!(chat.confirmed_platform_action.is_none());
        assert!(chat.confirmed_serial_permit.is_none());
        assert_eq!(panel.primary_device.as_deref(),Some("COM-test"));
    }
}

#[test]
fn stale_dispatched_control_stops_remaining_actions_and_pauses_queue() {
    let root=tempfile::tempdir().unwrap();
    let ctx=egui::Context::default();
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.allow_automation=true;
    let mut run=run::Run::new();run.journal=Some(crate::ai::journal::Journal::at(root.path(),"serial-ai-test"));
    run.queued.push_back(Action::SerialStart);chat.run=Some(run);
    chat.confirmed_platform_action=Some(("serial.monitor.stop".into(),json!({})));
    panel.test_bind_serial_permit(&mut chat);
    panel.begin_platform_call(&ctx,&mut chat,"serial.monitor.stop".into(),json!({}),"stop".into(),false).unwrap();
    panel.ai_epoch=panel.ai_epoch.wrapping_add(1);
    panel.poll_platform(&ctx,&mut chat,Lang::Zh);
    assert!(chat.queue_paused);
    assert!(chat.run.as_ref().unwrap().queued.is_empty());
    assert!(chat.control_approval.is_none());
    assert!(chat.tool.is_none());
}

#[test]
fn source_switch_keeps_unknown_device_receipt_visible_but_out_of_new_context() {
    let root=tempfile::tempdir().unwrap();let ctx=egui::Context::default();
    let config=AiConfig{enabled:true,model:"test-model".into(),..Default::default()};
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    let state=panel.ai_state("serial-ai-test",false);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.allow_automation=true;
    chat.conversation_source=Some((state.binding.source_id,state.binding.device.clone()));
    let mut run=run::Run::new();run.journal=Some(crate::ai::journal::Journal::at(root.path(),"serial-ai-test"));chat.run=Some(run);
    chat.confirmed_platform_action=Some(("serial.monitor.stop".into(),json!({})));panel.test_bind_serial_permit(&mut chat);
    panel.begin_platform_call(&ctx,&mut chat,"serial.monitor.stop".into(),json!({}),"stop".into(),false).unwrap();
    panel.ai_chat=chat;panel.tabs[0].clear_display();panel.tick_ai(&ctx,Lang::Zh,"serial-ai-test".into(),&config,false);
    assert!(panel.ai_chat.entries.iter().any(|entry|entry.text.contains("AI.CANCELLED_OUTCOME_UNKNOWN")));
    assert!(panel.ai_chat.tool_results.is_empty());
    assert!(panel.ai_chat.queue_paused);
}

#[test]
fn observed_serial_error_invalidates_prior_session_approval() {
    let mut panel=fixture();
    panel.primary_device=Some("COM-test".into());
    panel.ai_confirmed.insert("COM-test".into(),115200);
    let before=panel.ai_state("serial-ai-test",false);
    let (tx,rx)=crossbeam_channel::bounded(2);
    panel.rx=Some(rx);
    tx.send(SerialEvent::Error("transport error".into())).unwrap();
    panel.drain_events(false);
    let after=panel.ai_state("serial-ai-test",false);
    assert_ne!(before.binding.capture_epoch,after.binding.capture_epoch);
    let approval=ControlApproval { action:Action::SerialStop,binding:before.binding,skills:vec![],question:"stop".into(),automatic:false,created:Instant::now(),run_id:None,monitor_revision:None };
    assert!(!approval.valid(&after,&[],None,true,"",Instant::now()));
}

#[test]
fn failed_intent_journal_blocks_serial_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let invalid_root = root.path().join("not-a-directory");
    std::fs::write(&invalid_root, b"file").unwrap();
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let mut run = run::Run::new();
    run.journal = Some(crate::ai::journal::Journal::at(&invalid_root, "serial-ai-test"));
    chat.run = Some(run);
    chat.confirmed_platform_action = Some(("serial.monitor.stop".into(), json!({})));
    p.test_bind_serial_permit(&mut chat);
    let error = p.begin_platform_call(&egui::Context::default(), &mut chat,
        "serial.monitor.stop".into(), json!({}), "stop".into(), false).unwrap_err();
    assert!(error.contains("AGENT.JOURNAL_UNAVAILABLE"));
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn failed_intent_journal_prevents_broker_enqueue() {
    let root = tempfile::tempdir().unwrap();
    let invalid_root = root.path().join("not-a-directory");
    std::fs::write(&invalid_root, b"file").unwrap();
    let ctx = egui::Context::default();
    let (tx, rx) = crossbeam_channel::bounded::<crate::ai::broker::Request>(1);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), tx));
    let mut p = fixture();
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let mut run = run::Run::new();
    run.journal = Some(crate::ai::journal::Journal::at(&invalid_root, "serial-ai-test"));
    run.scope_key=p.local_log_scope_key(&chat);
    chat.run = Some(run);
    let error = p.begin_platform_call(&ctx, &mut chat, "instrument.command".into(),
        json!({"device_id":7,"command":"ScopeStop"}), "stop scope".into(), false).unwrap_err();
    assert!(error.contains("AGENT.JOURNAL_UNAVAILABLE"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn automatic_confirmed_control_also_requires_a_durable_intent() {
    let root = tempfile::tempdir().unwrap();
    let invalid_root = root.path().join("not-a-directory");
    std::fs::write(&invalid_root, b"file").unwrap();
    let ctx = egui::Context::default();
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-ai-project-root"), invalid_root));
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    chat.confirmed_platform_action = Some(("serial.monitor.stop".into(), json!({})));
    p.test_bind_serial_permit(&mut chat);
    let error = p.begin_platform_call(&ctx, &mut chat, "serial.monitor.stop".into(), json!({}),
        "watch".into(), true).unwrap_err();
    assert!(error.contains("AGENT.JOURNAL_UNAVAILABLE"));
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}


#[test]
fn revocation_records_unexecuted_approval_and_pauses_queue() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.run = Some(run::Run::new());
    chat.control_approval = Some(ControlApproval { action: Action::SerialStop, binding: state.binding,
        skills: vec![], question: "stop".into(), automatic: false, created: Instant::now(),
        run_id: chat.run.as_ref().map(|r| r.id.clone()), monitor_revision: None });
    chat.revoke_controls();
    assert!(chat.control_approval.is_none());
    assert!(chat.queue_paused);
    assert_eq!(chat.tool_results.last().unwrap()["error"], "AI.AUTHORIZATION_REVOKED");
    assert_eq!(chat.tool_results.last().unwrap()["executed"], false);
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn automatic_rejection_does_not_consume_manual_evidence_id() {
    let p = fixture();
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = SerialAiChat::default();
    chat.evidence_counter = 7;
    chat.control_approval = Some(ControlApproval { action: Action::SerialStop, binding: state.binding,
        skills: vec![], question: "watch".into(), automatic: true, created: Instant::now(),
        run_id: None, monitor_revision: None });
    assert!(chat.reject_control_approval("AI.ACTION_REJECTED"));
    assert_eq!(chat.evidence_counter, 7);
    assert_eq!(chat.automatic_tool_results.last().unwrap()["evidence_id"], "W1");
    assert!(chat.entries.last().is_some_and(|entry| entry.automatic && entry.text.contains("仅保留在当前会话")));
}

#[test]
fn background_failure_preserves_manual_status_and_marks_background_entry() {
    let mut chat=SerialAiChat::default();
    chat.status="manual task running".into();
    chat.status_error=false;
    chat.note_background("background failure",true);
    assert_eq!(chat.status,"manual task running");
    assert!(!chat.status_error);
    assert!(chat.entries.last().is_some_and(|entry|entry.automatic && entry.error));
}

#[test]
fn cancelling_automatic_control_preserves_manual_task_and_local_alerts() {
    let mut panel = fixture();
    panel.primary_device = Some("COM-test".into());
    let state = panel.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut panel.ai_chat);
    panel.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    chat.run = Some(run::Run::new());
    let task_id = chat.run.as_ref().unwrap().id.clone();
    let revision=chat.monitor_revision();
    chat.control_approval = Some(ControlApproval { action: Action::SerialStop, binding: state.binding,
        skills: vec![], question: "watch".into(), automatic: true, created: Instant::now(),
        run_id: None, monitor_revision: Some(revision) });
    chat.cancel_current_from_ui();
    assert!(chat.watch.as_ref().is_some_and(|watch| watch.ai_paused));
    assert_eq!(chat.run.as_ref().unwrap().id, task_id);
    assert!(chat.control_approval.is_none());
    let alerts = chat.alert_count;
    chat.observe(panel.tabs[0].ai_view_id(), &["ERROR after cancel".into()]);
    assert_eq!(chat.alert_count, alerts + 1);
}

#[test]
fn revoking_automatic_pending_control_restores_watch_and_keeps_local_alerts() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let state = p.ai_state("serial-ai-test", false);
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let binding = chat.watch.as_ref().unwrap().binding.clone();
    chat.monitor_inflight = Some((chat.watch.as_ref().unwrap().id.clone(),binding, 20, WindowStats { received_lines: 2, ..Default::default() }));
    chat.watch.as_mut().unwrap().cursor = 30;
    chat.watch.as_mut().unwrap().stats.received_lines = 3;
    chat.evidence_counter = 7;
    chat.run = Some(run::Run::new());
    let manual_run = chat.run.as_ref().unwrap().id.clone();
    chat.allow_automation = true;
    let revision=chat.monitor_revision();
    chat.control_approval = Some(ControlApproval { action: Action::SerialStop, binding: state.binding,
        skills: vec![], question: "watch".into(), automatic: true, created: Instant::now(),
        run_id: None, monitor_revision: Some(revision) });

    chat.allow_automation = false;
    chat.revoke_controls();
    assert!(chat.control_approval.is_none());
    assert!(chat.monitor_inflight.is_none());
    let watch = chat.watch.as_ref().unwrap();
    assert!(watch.ai_paused);
    assert_eq!(watch.cursor, 20);
    assert_eq!(watch.stats.received_lines, 5);
    assert_eq!(chat.run.as_ref().unwrap().id, manual_run);
    assert!(!chat.queue_paused, "automatic rejection must not pause independent manual messages");
    assert_eq!(chat.evidence_counter, 7);
    assert_eq!(chat.automatic_tool_results.last().unwrap()["executed"], false);
    assert!(chat.entries.iter().any(|entry| entry.automatic && entry.text.contains("AI.AUTHORIZATION_REVOKED")));
    let previous_alerts = chat.alert_count;
    chat.observe(p.tabs[0].ai_view_id(), &["ERROR after control revocation".into()]);
    assert_eq!(chat.alert_count, previous_alerts + 1);
}

#[test]
fn background_error_preserves_manual_run_and_history() {
    let mut chat = SerialAiChat::default();
    chat.run = Some(run::Run::new());
    chat.history.push(AiMessage::User("manual task".into()));
    let phase = chat.run.as_ref().unwrap().phase;
    chat.note_background("monitor error", true);
    assert_eq!(chat.run.as_ref().unwrap().phase, phase);
    assert_eq!(chat.history.len(), 1);
    assert!(!chat.status_error);
    assert!(chat.entries.last().unwrap().error);
}

#[test]
fn failed_monitor_request_restores_checkpoint_without_losing_new_samples() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let binding = chat.watch.as_ref().unwrap().binding.clone();
    let previous = WindowStats { received_lines: 2, error_keyword_lines: 1, ..Default::default() };
    let watch = chat.watch.as_mut().unwrap();
    watch.cursor = 100;
    watch.stats.received_lines = 3;
    watch.stats.warning_keyword_lines = 1;
    chat.restore_watch_checkpoint(&binding, Some((50, previous)));
    let watch = chat.watch.as_ref().unwrap();
    assert_eq!(watch.cursor, 50);
    assert_eq!(watch.stats.received_lines, 5);
    assert_eq!(watch.stats.error_keyword_lines, 1);
    assert_eq!(watch.stats.warning_keyword_lines, 1);
}

#[test]
fn monitor_transaction_survives_tool_round_and_restores_at_most_once() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let binding = chat.watch.as_ref().unwrap().binding.clone();
    chat.monitor_inflight = Some((chat.watch.as_ref().unwrap().id.clone(),binding, 20, WindowStats { received_lines: 2, ..Default::default() }));
    let watch = chat.watch.as_mut().unwrap();
    watch.cursor = 30;
    watch.stats.received_lines = 3;
    // A tool round leaves the transaction active; only final success or failure closes it.
    assert!(chat.monitor_inflight.is_some());
    chat.finish_watch_transaction(false);
    chat.finish_watch_transaction(false);
    assert_eq!(chat.watch.as_ref().unwrap().cursor, 20);
    assert_eq!(chat.watch.as_ref().unwrap().stats.received_lines, 5);
}

#[test]
fn stale_monitor_action_drops_old_checkpoint_without_replaying_it() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    p.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let old_binding = chat.watch.as_ref().unwrap().binding.clone();
    chat.monitor_inflight = Some((chat.watch.as_ref().unwrap().id.clone(),old_binding, 20, WindowStats { received_lines: 2, ..Default::default() }));
    chat.watch.as_mut().unwrap().cursor = 30;
    // The host invokes this on stale responses, including responses with actions.
    chat.discard_stale_watch_transaction();
    chat.finish_watch_transaction(false);
    assert_eq!(chat.watch.as_ref().unwrap().cursor, 30);
    assert!(chat.monitor_inflight.is_none());
}

#[test]
fn monitor_reply_requires_a_real_final_result_before_commit() {
    let mut response = protocol::Response { reply: String::new(), actions: vec![], error: None,
        finalized: true, needs_input: false };
    assert_eq!(host::watch_reply(&response), host::WatchReply::Invalid);
    response.reply = "done".into();
    assert_eq!(host::watch_reply(&response), host::WatchReply::Final);
    response.error = Some("truncated".into());
    assert_eq!(host::watch_reply(&response), host::WatchReply::Invalid);
    response.error = None;
    response.actions.push(Action::SerialStop);
    assert_eq!(host::watch_reply(&response), host::WatchReply::Continue);
    response.needs_input = true;
    assert_eq!(host::watch_reply(&response), host::WatchReply::Invalid);
}

#[test]
fn queued_model_control_waits_for_its_own_confirmation() {
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let mut run = run::Run::new();
    run.scope_key=p.local_log_scope_key(&chat);
    run.queued.push_back(Action::Platform { method: "serial.monitor.stop".into(), params: json!({}) });
    chat.run = Some(run);
    assert!(p.dispatch_queued_reads(&egui::Context::default(), &mut chat, "summarize"));
    assert!(chat.control_approval.is_some());
    assert!(chat.tool.is_none());
    assert_eq!(p.primary_device.as_deref(), Some("COM-test"));
}

#[test]
fn cancelling_a_dispatched_control_preserves_an_unknown_outcome() {
    let root = tempfile::tempdir().unwrap();
    let mut p = fixture();
    p.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut p.ai_chat);
    chat.allow_automation = true;
    let mut run = run::Run::new();
    run.journal = Some(crate::ai::journal::Journal::at(root.path(), "serial-ai-test"));
    chat.run = Some(run);
    chat.confirmed_platform_action = Some(("serial.monitor.stop".into(), json!({})));
    p.test_bind_serial_permit(&mut chat);
    p.begin_platform_call(&egui::Context::default(), &mut chat, "serial.monitor.stop".into(),
        json!({}), "stop".into(), false).unwrap();
    chat.cancel();
    assert_eq!(chat.tool_results.last().unwrap()["error"], "AI.CANCELLED_OUTCOME_UNKNOWN");
    assert_eq!(chat.run.as_ref().unwrap().uncertain.len(), 1);
    assert!(!chat.allow_automation);
}

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
fn reviewed_standalone_question_contains_no_log_and_has_source_permit() {
    let config = AiConfig { enabled:true, base_url:"http://127.0.0.1:9".into(), model:"mock".into(), ..Default::default() };
    let ctx = egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut panel = fixture();
    panel.ai_chat.set_context("serial-ai-test".into(), &config);
    panel.ai_chat.review_outbound = true;
    let state = panel.ai_state("serial-ai-test", false);
    panel.ai_chat.submit(&ctx, &state, json!({"secret_log_fixture":"do-not-send"}),
        "仅解释电压单位，一句话回答，不要读日志、调用工具或画图。".into(), false).unwrap();
    let pending = panel.ai_chat.pending.as_ref().unwrap();
    assert!(pending.reviewed_source_guarded);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let preview = loop {
        if let Some(preview) = pending.task.outbound_preview() { break preview; }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(!preview.body.to_string().contains("do-not-send"));
    pending.task.cancel();
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
    let root = tempfile::tempdir().unwrap();
    let mut run = run::Run::new();
    run.journal = Some(crate::ai::journal::Journal::at(root.path(), "serial-ai-test"));
    run.scope_key = p.local_log_scope_key(&p.ai_chat);
    p.ai_chat.run = Some(run);
    p.ai_chat
        .submit(&ctx, &state, data, "分析并停止接收".into(), false)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(8);
    while p.ai_chat.pending.is_some() && Instant::now() < until {
        p.tick_ai(&ctx, Lang::Zh, "serial-ai-test".into(), &config, false);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(p.ai_chat.pending.is_none());
    assert!(p.ai_chat.control_approval.is_some());
    assert!(p.primary_device.is_some(), "confirmation must precede serial stop");
    let mut chat = std::mem::take(&mut p.ai_chat);
    let approval = chat.control_approval.take().unwrap();
    let current = p.ai_state("serial-ai-test", false);
    assert!(approval.valid(&current, &chat.selected_skills, chat.run.as_ref().map(|r|r.id.as_str()),
        chat.control_allowed(&current), &chat.monitor_revision(), Instant::now()));
    chat.confirmed_platform_action = Some(("serial.monitor.stop".into(), json!({})));
    p.test_bind_serial_permit(&mut chat);
    p.begin_platform_call(&ctx, &mut chat, "serial.monitor.stop".into(), json!({}),
        "分析并停止接收".into(), false).unwrap();
    p.poll_platform(&ctx, &mut chat, Lang::Zh);
    p.ai_chat = chat;
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
    assert!(serialized.contains("Serial device control allowed: true"));
    assert!(!serialized.contains("JSON compatibility envelope"));
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

#[test]
fn serial_control_permit_rejects_changed_scope_even_with_same_device() {
    let mut p=fixture();let mut chat=std::mem::take(&mut p.ai_chat);
    chat.allow_automation=true;chat.project="serial-ai-test".into();
    let state=p.ai_state("serial-ai-test",false);let scope=p.local_log_scope_key(&chat);
    let permit=SerialControlPermit{binding:state.binding.clone(),scope_key:scope.clone(),skills:chat.selected_skills.clone(),run_id:None,created:Instant::now()};
    assert!(permit.valid(&state,&chat,&scope,Instant::now()));
    chat.scope=DataScope::Range;chat.range_first=1;chat.range_last=5;
    let changed=p.local_log_scope_key(&chat);assert_ne!(scope,changed);
    assert!(!permit.valid(&state,&chat,&changed,Instant::now()));
}

#[test]
fn confirmed_stop_of_inactive_serial_port_is_not_reported_as_dispatched() {
    let mut panel=fixture();let mut chat=std::mem::take(&mut panel.ai_chat);chat.allow_automation=true;
    chat.run=Some(run::Run::new());chat.confirmed_platform_action=Some(("serial.monitor.stop".into(),json!({})));
    panel.test_bind_serial_permit(&mut chat);
    let error=panel.begin_platform_call(&egui::Context::default(),&mut chat,"serial.monitor.stop".into(),json!({}),"stop".into(),false).unwrap_err();
    assert!(error.contains("SERIAL.NOT_RECEIVING"));assert!(chat.tool.is_none());
    assert!(chat.confirmed_platform_action.is_none());assert!(chat.confirmed_serial_permit.is_none());
}

#[test]
fn automatic_control_approval_expires_when_monitor_instance_is_replaced() {
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.allow_automation=true;
    panel.begin_ai_watch(&mut chat,"watch".into(),false).unwrap();
    chat.watch.as_mut().unwrap().ai_paused=true;
    let state=panel.ai_state("serial-ai-test",false);let revision=chat.monitor_revision();
    let approval=ControlApproval{action:Action::SerialStop,binding:state.binding.clone(),skills:vec![],question:"watch".into(),automatic:true,created:Instant::now(),run_id:None,monitor_revision:Some(revision.clone())};
    assert!(approval.valid(&state,&[],None,true,&revision,Instant::now()));
    chat.watch.as_mut().unwrap().id=uuid::Uuid::new_v4().to_string();
    assert!(!approval.valid(&state,&[],None,true,&chat.monitor_revision(),Instant::now()));
}

#[test]
fn replaced_watch_never_inherits_old_inflight_statistics_on_rejection() {
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    let mut chat=std::mem::take(&mut panel.ai_chat);
    panel.begin_ai_watch(&mut chat,"first".into(),false).unwrap();
    let watch=chat.watch.as_ref().unwrap();
    chat.monitor_inflight=Some((watch.id.clone(),watch.binding.clone(),20,WindowStats{received_lines:12,..Default::default()}));
    chat.watch=None;panel.begin_ai_watch(&mut chat,"replacement".into(),false).unwrap();
    chat.watch.as_mut().unwrap().cursor=30;chat.watch.as_mut().unwrap().stats.received_lines=3;
    chat.finish_watch_transaction(false);
    assert_eq!(chat.watch.as_ref().unwrap().cursor,30);
    assert_eq!(chat.watch.as_ref().unwrap().stats.received_lines,3);
    assert!(chat.monitor_inflight.is_none());
}

#[test]
fn cancelling_replaced_watch_approval_preserves_new_watch() {
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    let mut chat=std::mem::take(&mut panel.ai_chat);panel.begin_ai_watch(&mut chat,"first".into(),false).unwrap();
    let old=chat.watch.as_ref().unwrap();let binding=old.binding.clone();let id=old.id.clone();
    chat.monitor_inflight=Some((id,binding.clone(),20,WindowStats{received_lines:12,..Default::default()}));
    chat.watch.as_mut().unwrap().ai_paused=true;let revision=chat.monitor_revision();
    chat.control_approval=Some(ControlApproval{action:Action::SerialStop,binding,skills:vec![],question:"old".into(),automatic:true,created:Instant::now(),run_id:None,monitor_revision:Some(revision)});
    chat.watch=None;panel.begin_ai_watch(&mut chat,"second".into(),false).unwrap();
    chat.watch.as_mut().unwrap().stats.received_lines=3;
    chat.cancel_current_from_ui();
    let current=chat.watch.as_ref().unwrap();assert!(!current.ai_paused);assert_eq!(current.stats.received_lines,3);
}

#[test]
fn stale_manual_review_does_not_block_independent_local_preview() {
    let ctx=egui::Context::default();let mut panel=fixture();
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.review_outbound=true;chat.scope=DataScope::Current;
    let lease=wiparse_core::ai::source_lease::SourceLease::default();
    let source=wiparse_core::ai::ReviewedSource::capture(&lease,||run::ReviewedLog {
        source:wiparse_core::log_analysis::Source::memory("fixture",vec![Arc::<str>::from("old")],1,0),
        range:None,snapshot_id:uuid::Uuid::new_v4().to_string(),retained_rows:1,
    }).unwrap();
    lease.mutate_always(||());assert!(!source.is_current());
    let mut run=run::Run::new();run.reviewed_log=Some(source);run.scope_key=panel.local_log_scope_key(&chat);chat.run=Some(run);
    chat.local_preview_request=true;
    assert!(panel.begin_platform_call(&ctx,&mut chat,"log.files".into(),json!({}),"preview".into(),false).is_ok());
    chat.cancel();
}

#[test]
fn stale_watch_cleanup_never_discards_another_watch_transaction() {
    let mut chat = SerialAiChat::default();
    let binding = fixture().ai_state("serial-ai-test", false).binding;
    chat.monitor_inflight = Some(("new-watch".into(), binding, 10, WindowStats::default()));
    chat.discard_watch_transaction_for(Some("old-watch"));
    assert_eq!(chat.monitor_inflight.as_ref().map(|entry| entry.0.as_str()), Some("new-watch"));
    chat.discard_watch_transaction_for(None);
    assert!(chat.monitor_inflight.is_some());
    chat.discard_watch_transaction_for(Some("new-watch"));
    assert!(chat.monitor_inflight.is_none());
}


#[test]
fn monitor_lease_retires_on_watch_creation_and_control_revocation() {
    let mut panel = fixture();
    panel.primary_device = Some("COM-test".into());
    let mut chat = std::mem::take(&mut panel.ai_chat);
    let before = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || ()).unwrap();
    panel.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    assert!(!before.is_current());
    let before_revocation = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || ()).unwrap();
    chat.revoke_controls();
    assert!(!before_revocation.is_current());
    let before_stop = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || ()).unwrap();
    panel.apply_ai_action(&mut chat, Action::MonitorStop, "", Lang::Zh, false).unwrap();
    assert!(!before_stop.is_current());
}

#[test]
fn reviewed_monitor_window_is_frozen_and_exposes_coverage() {
    let mut panel = fixture();
    panel.tabs[0].append_lines(vec!["first".into(), "second".into()], true);
    let snapshot = wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,
        || panel.ai_observation(true, Some(0), None)).unwrap();
    panel.serial_source_lease.clone().mutate_always(|| panel.tabs[0].append_lines(vec!["later".into()], true));
    assert!(snapshot.is_current());
    assert_eq!(snapshot.value()["window"]["end_exclusive_zero_based"], 2);
    assert_eq!(snapshot.value()["recent_lines"].as_array().unwrap().len(), 2);
    panel.serial_review_lease.clone().mutate_always(|| panel.tabs[0].clear_display());
    assert!(!snapshot.is_current());
}

#[test]
fn reviewed_monitor_cannot_prepare_request_after_authorization_revoked() {
    let mut chat = SerialAiChat::default();
    let source_lease = wiparse_core::ai::source_lease::SourceLease::default();
    let sample = wiparse_core::ai::ReviewedSource::capture(&source_lease, || json!({"recent_lines":["sample"]})).unwrap();
    let identity = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || json!({"watch_id":"watch-1"})).unwrap();
    chat.revoke_controls();
    let request = identity.join(sample).into_request(|_,_| (AiRequest {purpose:"monitor".into(),text:"sample".into()}, Vec::new(), None));
    assert!(request.is_none());
}

#[test]
fn monitor_review_permit_cannot_survive_overlapping_update() {
    let chat = SerialAiChat::default();
    let source = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || "old-watch").unwrap();
    let update = chat.monitor_lease.begin_update();
    assert!(!source.is_current());
    assert!(wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || "new-watch").is_none());
    drop(update);
    assert!(!source.is_current());
    assert!(wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || "new-watch").is_some());
}

#[test]
fn reviewed_monitor_preparation_rejects_stale_authority_and_unleased_tool_results() {
    let mut panel = fixture();
    panel.primary_device = Some("COM-test".into());
    panel.tabs[0].append_lines(vec!["VIN=5".into()], true);
    let mut chat = std::mem::take(&mut panel.ai_chat);
    chat.review_outbound = true;
    panel.begin_ai_watch(&mut chat, "watch".into(), false).unwrap();
    let state = panel.ai_state("serial-ai-test", false);
    let sample = wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,
        || panel.ai_observation(true, Some(0), None)).unwrap();
    let identity = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || json!({
        "watch_id":chat.watch.as_ref().unwrap().id,"revision":chat.monitor_revision(),
        "binding":state.binding,"skills":chat.selected_skills,"control_allowed":chat.control_allowed(&state)
    })).unwrap();
    let source = json!({"binding":state.binding,"reception_requested":state.reception_requested,
        "connection_confirmed":state.connection_confirmed,"automation_locked":state.automation_locked});
    let request = AiRequest {purpose:"monitor".into(), text:json!({"user_request":"watch","source":source,"ai_monitoring":true,"observation":sample.value()}).to_string()};
    chat.monitor_review = Some((identity, sample));
    assert!(chat.prepare_reviewed_monitor_request(&state, request.clone(), vec![]).is_ok());
    let mut wrong_body: serde_json::Value = serde_json::from_str(&request.text).unwrap();
    wrong_body["source"]["automation_locked"] = json!(!state.automation_locked);
    let altered = AiRequest {purpose:request.purpose.clone(),text:wrong_body.to_string()};
    assert!(chat.prepare_reviewed_monitor_request(&state, altered, vec![]).is_err());
    let params = json!({"query":"VIN"});
    let sample = chat.monitor_review.as_ref().unwrap().1.value().clone();
    let result = monitor_review::execute("monitor.sample.search", &params, &sample).unwrap();
    chat.automatic_tool_results.push(json!({"method":"monitor.sample.search","params":params,"result":result}));
    chat.automatic_tool_round = 1;
    let continuation = AiRequest {purpose:"monitor".into(),text:json!({"user_request":"watch","source":source,"ai_monitoring":true,"observation":{"frozen_sample":sample,"platform_results":chat.automatic_tool_results}}).to_string()};
    assert!(chat.prepare_reviewed_monitor_request(&state, continuation.clone(), vec![]).is_ok());
    for _ in 1..9 { chat.automatic_tool_results.push(chat.automatic_tool_results[0].clone()); }
    chat.automatic_tool_round = 9;
    let ninth = AiRequest {purpose:"monitor".into(),text:json!({"user_request":"watch","source":source,"ai_monitoring":true,"observation":{"frozen_sample":sample,"platform_results":chat.automatic_tool_results}}).to_string()};
    assert!(chat.prepare_reviewed_monitor_request(&state, ninth, vec![]).is_ok());
    chat.automatic_tool_results[0]["result"]["data"]["count"] = json!(999);
    assert!(chat.prepare_reviewed_monitor_request(&state, continuation, vec![]).is_err());
    chat.automatic_tool_results.clear(); chat.automatic_tool_round = 0;
    chat.revoke_controls();
    assert!(chat.prepare_reviewed_monitor_request(&state, request, vec![]).is_err());
    assert!(!chat.reviewed_monitor_dispatch_ready());
}

#[test]
fn detached_chat_and_host_placeholder_share_monitor_invalidation() {
    let mut panel = fixture();
    let chat = panel.take_ai_chat();
    let original = wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease, || ()).unwrap();
    panel.ai_chat.monitor_lease.mutate_always(|| ());
    assert!(!original.is_current());
    let replacement = wiparse_core::ai::ReviewedSource::capture(&panel.ai_chat.monitor_lease, || ()).unwrap();
    chat.monitor_lease.mutate_always(|| ());
    assert!(!replacement.is_current());
    panel.ai_chat = chat;
}

#[test]
fn serial_error_event_blocks_review_until_host_publishes_error_state() {
    let mut panel=fixture();
    panel.primary_device=Some("COM-test".into());
    panel.ai_confirmed.insert("COM-test".into(),115200);
    let lease=panel.serial_review_lease.clone();
    let before=wiparse_core::ai::ReviewedSource::capture(&lease,||()).unwrap();
    let (tx,rx)=crossbeam_channel::bounded(2);panel.rx=Some(rx);
    let update=lease.begin_update();
    assert!(tx.send(SerialEvent::StateChange(Box::new(SerialEvent::Error("transport error".into())),update)).is_ok());
    assert!(!before.is_current());
    assert!(wiparse_core::ai::ReviewedSource::capture(&lease,||()).is_none());
    panel.drain_events(false);
    assert!(wiparse_core::ai::ReviewedSource::capture(&lease,||()).is_some());
    let current=panel.ai_state("serial-ai-test",false);
    assert!(!current.connection_confirmed);
    assert!(current.status.contains("transport error"));
}

#[test]
fn serial_worker_unwind_blocks_review_until_stopped_is_published() {
    let mut panel=fixture();
    panel.primary_device=Some("COM-test".into());
    panel.ai_confirmed.insert("COM-test".into(),115200);
    let lease=panel.serial_review_lease.clone();
    let old=wiparse_core::ai::ReviewedSource::capture(&lease,||()).unwrap();
    let (tx,rx)=crossbeam_channel::bounded(2);panel.rx=Some(rx);
    let worker_lease=lease.clone();
    let handle=std::thread::spawn(move || {
        let _exit=SerialWorkerExit{tx,source_lease:worker_lease,stop:Arc::new(AtomicBool::new(false)),repaint:None,pending_update:None,armed:true,session:None,opened:None,completion:None,claimed_stop:None};
        panic!("synthetic worker failure");
    });
    assert!(handle.join().is_err());
    assert!(!old.is_current());
    assert!(wiparse_core::ai::ReviewedSource::capture(&lease,||()).is_none());
    panel.drain_events(false);
    assert!(panel.primary_device.is_none());
    assert!(!panel.ai_state("serial-ai-test",false).connection_confirmed);
}

#[test]
fn unexpected_serial_event_channel_loss_fails_closed() {
    let lease=wiparse_core::ai::source_lease::SourceLease::default();
    let old=wiparse_core::ai::ReviewedSource::capture(&lease,||()).unwrap();
    let (tx,rx)=crossbeam_channel::bounded(1);drop(rx);
    let stop=AtomicBool::new(false);
    assert!(!publish_serial_state(&tx,SerialEvent::Stopped,lease.begin_update(),&stop,&lease));
    assert!(!old.is_current());
    assert!(wiparse_core::ai::ReviewedSource::capture(&lease,||()).is_none());
}

#[test]
fn reviewed_automatic_call_rejects_unprotected_tool_before_dispatch() {
    let mut panel=fixture();
    let mut chat=std::mem::take(&mut panel.ai_chat);
    chat.review_outbound=true;
    let result=panel.begin_platform_call(&egui::Context::default(),&mut chat,
        "serial.monitor.status".into(),json!({}),"watch".into(),true);
    assert!(result.is_err());
    assert!(chat.tool.is_none());
}

#[test]
fn monitor_context_changes_retire_lease_without_invalidating_unchanged_context() {
    let mut chat=SerialAiChat::default();
    let config=AiConfig::default();
    chat.set_context("one".into(), &config);
    let unchanged=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||()).unwrap();
    chat.set_context("one".into(), &config);
    assert!(unchanged.is_current());
    chat.set_context("two".into(), &config);
    assert!(!unchanged.is_current());
}

#[test]
fn pending_connection_does_not_report_confirmed_after_transport_error() {
    let mut panel=fixture();
    panel.primary_device=Some("COM-test".into());
    panel.ai_confirmed.insert("COM-test".into(),115200);
    panel.ai_serial_error.insert("COM-test".into(),"transport error".into());
    panel.ai_chat.awaiting_connection=Some(("COM-test".into(), Instant::now()+Duration::from_secs(5)));
    let config=panel.ai_chat.config.clone().unwrap();
    panel.tick_ai(&egui::Context::default(),Lang::Zh,"serial-ai-test".into(),&config,false);
    assert!(panel.ai_chat.awaiting_connection.is_some());
    assert!(!panel.ai_chat.entries.iter().any(|entry|entry.text.contains("Connection confirmed")));
}

#[test]
fn queued_serial_state_change_invalidates_monitor_reply_and_tool_sources_before_drain() {
    let mut panel=fixture();
    let mut chat=panel.take_ai_chat();
    let identity=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||json!({"watch_id":"one"})).unwrap();
    let sample=wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,||json!({"recent_lines":[]})).unwrap();
    chat.monitor_review=Some((identity,sample));
    assert!(chat.reviewed_monitor_sources_current());
    let (tx,rx)=crossbeam_channel::bounded(2);panel.rx=Some(rx);
    let update=panel.serial_review_lease.begin_update();
    assert!(tx.send(SerialEvent::StateChange(Box::new(SerialEvent::Error("lost".into())),update)).is_ok());
    assert!(!chat.reviewed_monitor_sources_current());
    panel.ai_chat=chat;
    panel.drain_events(false);
    assert!(!panel.ai_chat.reviewed_monitor_sources_current());
}

#[test]
fn invalid_queued_tool_stops_batch_before_later_device_action() {
    let mut panel=fixture();
    let mut chat=panel.take_ai_chat();
    let mut run=run::Run::new();
    run.queued.push_back(Action::Platform{method:"unknown.invalid.tool".into(),params:json!({})});
    run.queued.push_back(Action::SerialStop);
    chat.run=Some(run);
    assert!(panel.dispatch_queued_reads(&egui::Context::default(),&mut chat,"fixture"));
    assert!(chat.queue_paused);
    assert!(chat.control_approval.is_none());
    assert!(chat.tool.is_none());
    assert!(chat.run.as_ref().unwrap().queued.is_empty());
}

#[test]
fn primary_stop_signals_worker_and_retires_reviewed_session() {
    let mut panel=fixture();
    let flag=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    panel.primary_device=Some("COM-test".into());
    panel.stop_flag=Some(flag.clone());
    panel.monitoring=true;
    let source=wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,||panel.primary_device.clone()).unwrap();
    panel.stop_monitor(Lang::Zh);
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    assert!(panel.primary_device.is_none());
    assert!(!source.is_current());
}

#[test]
fn serial_completion_survives_closed_log_receiver_and_carries_session_command() {
    let lease=wiparse_core::ai::source_lease::SourceLease::default();
    let (events, event_rx)=crossbeam_channel::bounded(1);drop(event_rx);
    let (finished, finished_rx)=crossbeam_channel::bounded(1);
    let session=uuid::Uuid::new_v4();
    let stop=Arc::new(AtomicBool::new(true));
    let command=Arc::new(std::sync::Mutex::new(Some(SerialStopIntent{command_id:"command-1".into(),project:"p".into(),fingerprint:"fingerprint".into(),audit:None})));
    drop(SerialWorkerExit{tx:events,source_lease:lease,stop,repaint:None,pending_update:None,armed:true,
        session:None,opened:Some(false),completion:Some((session,"COM-test".into(),finished,command)),claimed_stop:None});
    let receipt=finished_rx.try_recv().unwrap();
    assert_eq!(receipt.session_id,session);
    assert_eq!(receipt.stop_intent.as_ref().map(|intent|intent.command_id.as_str()),Some("command-1"));
    assert_eq!(receipt.host_session_dropped,None);
    assert_eq!(receipt.opened,Some(false));
}

#[test]
fn disconnected_completion_is_unknown_and_cannot_impersonate_another_session() {
    let mut panel=fixture();
    let (tx,rx)=crossbeam_channel::bounded(1);drop(tx);
    let session_id=uuid::Uuid::new_v4();
    panel.serial_workers.push(SerialWorkerCompletion{session_id,port:"COM-test".into(),rx});
    panel.poll_serial_worker_completions();
    let receipt=panel.serial_release_receipts.back().unwrap();
    assert_eq!(receipt.session_id,session_id);
    assert_eq!(receipt.host_session_dropped,None);
    assert!(receipt.stop_intent.is_none());
}

#[test]
fn shared_test_ownership_blocks_controls_before_next_ui_sync() {
    let panel=fixture();
    assert!(!panel.ai_controls_locked(false));
    panel.integrated_test_state().store(true, std::sync::atomic::Ordering::Release);
    assert!(panel.ai_controls_locked(false));
    assert!(panel.ai_state("serial-ai-test",false).automation_locked);
    panel.integrated_test_state().store(false, std::sync::atomic::Ordering::Release);
    assert!(!panel.ai_controls_locked(false));
}

#[test]
fn reviewed_automatic_monitor_stops_before_creating_unprotected_request() {
    let config=AiConfig{enabled:true,model:"fixture".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());
    panel.ai_confirmed.insert("COM-test".into(),115200);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.set_context("serial-ai-test".into(),&config);chat.review_outbound=true;
    panel.begin_ai_watch(&mut chat,"watch".into(),false).unwrap();
    let state=panel.ai_state("serial-ai-test",false);
    let error=chat.submit(&ctx,&state,json!({}),"watch".into(),true).unwrap_err();
    assert!(error.contains("AI.MONITOR_REVIEW_UNAVAILABLE"));
    assert!(chat.pending.is_none());
}

#[test]
fn reviewed_monitor_build_gate_matches_compiled_configuration() {
    let chat=SerialAiChat::default();
    assert_eq!(std::env::var("WIPARSE_REVIEWED_MONITOR_VERIFIED").ok().as_deref(),option_env!("WIPARSE_REVIEWED_MONITOR_VERIFIED"));
    assert_eq!(chat.reviewed_monitor_dispatch_ready(),option_env!("WIPARSE_REVIEWED_MONITOR_VERIFIED")==Some("1"));
}

#[test]
fn reviewed_monitor_read_only_request_and_continuation_use_frozen_sample() {
    let config=AiConfig{enabled:true,base_url:"http://127.0.0.1:9".into(),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let service=crate::ai::service(&ctx).unwrap();
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());panel.ai_confirmed.insert("COM-test".into(),115200);
    panel.tabs[0].append_lines(vec!["VIN=5".into(),"ERROR OVP".into()],true);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.set_context("serial-ai-test".into(),&config);
    chat.review_outbound=true;chat.reviewed_monitor_test_enabled=true;
    panel.begin_ai_watch(&mut chat,"inspect".into(),false).unwrap();
    let state=panel.ai_state("serial-ai-test",false);
    let sample=wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,||panel.ai_observation(true,Some(0),None)).unwrap();
    let identity=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||json!({"watch_id":chat.watch.as_ref().unwrap().id,
        "revision":chat.monitor_revision(),"binding":state.binding,"skills":chat.selected_skills,"control_allowed":chat.control_allowed(&state),
        "service_id":service.instance_id().to_string(),"service_revision":service.revision_for_config(&config)})).unwrap();
    let frozen=sample.value().clone();chat.monitor_review=Some((identity,sample));
    chat.submit(&ctx,&state,frozen.clone(),"inspect".into(),true).unwrap();
    let first=chat.pending.take().unwrap();assert!(first.reviewed_source_guarded);
    let deadline=Instant::now()+Duration::from_secs(5);
    while first.task.outbound_preview().is_none(){assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
    assert!(first.task.outbound_preview().unwrap().body.to_string().contains("ERROR OVP"));
    first.task.cancel();
    let params=json!({"query":"ERROR"});let result=monitor_review::execute("monitor.sample.search",&params,&frozen).unwrap();
    chat.automatic_tool_round=1;chat.automatic_tool_results.push(json!({"method":"monitor.sample.search","params":params,"result":result}));
    chat.submit(&ctx,&state,json!({}),"inspect".into(),true).unwrap();
    let second=chat.pending.take().unwrap();assert!(second.reviewed_source_guarded);
    let deadline=Instant::now()+Duration::from_secs(5);
    while second.task.outbound_preview().is_none(){assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
    let body=second.task.outbound_preview().unwrap().body.to_string();
    assert!(body.contains("monitor.sample.search") && body.contains("ERROR OVP"));
    second.task.cancel();
}

#[test]
fn reviewed_monitor_sends_frozen_sample_only_after_explicit_outbound_approval() {
    use std::io::Read;
    let server=tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address=server.server_addr().to_ip().unwrap();
    let (seen_tx,seen_rx)=std::sync::mpsc::channel();
    let worker=thread::spawn(move||{
        let mut request=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        let mut body=String::new();request.as_reader().read_to_string(&mut body).unwrap();
        seen_tx.send(body).unwrap();
        request.respond(tiny_http::Response::from_string(json!({"model":"mock","choices":[{"message":{"content":"监控样本正常"},"finish_reason":"stop"}]}).to_string())).unwrap();
    });
    let config=AiConfig{enabled:true,base_url:format!("http://{address}"),model:"mock".into(),..Default::default()};
    let ctx=egui::Context::default();crate::ai::AiPanel::new(&config).install(&ctx);
    let service=crate::ai::service(&ctx).unwrap();
    let mut panel=fixture();panel.primary_device=Some("COM-test".into());panel.ai_confirmed.insert("COM-test".into(),115200);
    panel.tabs[0].append_lines(vec!["VIN=5".into(),"ERROR OVP".into()],true);
    let mut chat=std::mem::take(&mut panel.ai_chat);chat.set_context("serial-ai-test".into(),&config);
    chat.review_outbound=true;chat.reviewed_monitor_test_enabled=true;
    panel.begin_ai_watch(&mut chat,"inspect".into(),false).unwrap();
    let state=panel.ai_state("serial-ai-test",false);
    let sample=wiparse_core::ai::ReviewedSource::capture(&panel.serial_review_lease,||panel.ai_observation(true,Some(0),None)).unwrap();
    let identity=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||json!({"watch_id":chat.watch.as_ref().unwrap().id,
        "revision":chat.monitor_revision(),"binding":state.binding,"skills":chat.selected_skills,"control_allowed":chat.control_allowed(&state),
        "service_id":service.instance_id().to_string(),"service_revision":service.revision_for_config(&config)})).unwrap();
    let frozen=sample.value().clone();chat.monitor_review=Some((identity,sample));
    chat.submit(&ctx,&state,frozen,"inspect".into(),true).unwrap();
    let pending=chat.pending.take().unwrap();
    let deadline=Instant::now()+Duration::from_secs(5);
    while pending.task.outbound_preview().is_none(){assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(5));}
    assert!(seen_rx.try_recv().is_err());
    assert!(pending.task.approve_outbound());
    let body=seen_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(body.contains("ERROR OVP"));
    assert!(pending.task.result.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
    worker.join().unwrap();
}

#[test]
fn changing_serial_scope_retires_monitor_authority() {
    let mut chat=SerialAiChat::default();
    let source=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||()).unwrap();
    chat.set_data_scope(DataScope::None);
    assert!(!source.is_current());
    let unchanged=wiparse_core::ai::ReviewedSource::capture(&chat.monitor_lease,||()).unwrap();
    chat.set_data_scope(DataScope::None);
    assert!(unchanged.is_current());
}
