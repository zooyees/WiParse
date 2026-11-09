//! Handle stateful API invokes on the GUI thread.

use super::dispatch::{err, ok};
use super::ui::{self, UiHost};
use super::{ApiBridge, InvokeReply, PendingRequest};
use crate::serial_tool::SerialToolPanel;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wiparse_core::i18n::Lang;

const MAX_API_REQUESTS_PER_FRAME: usize = 32;
const API_FRAME_BUDGET: Duration = Duration::from_millis(4);
static REPORT_TREE_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn drain_api_requests(bridge: &ApiBridge, mut host: UiHost<'_>) -> bool {
    let mut persist = false;
    let started = Instant::now();
    for _ in 0..MAX_API_REQUESTS_PER_FRAME {
        if started.elapsed() >= API_FRAME_BUDGET {
            break;
        }
        let Some(req) = bridge.try_recv() else {
            break;
        };
        if let Some(guard)=&req.ai_guard {
            if guard.cancelled.load(std::sync::atomic::Ordering::Acquire)
                || guard.project!=host.test_tool.current_project_id()
                || !host.serial.ai_platform_guard_valid(guard)
                || (guard.controls && host.test_tool.running_plugin_id().is_some()) {
                let _=req.reply.try_send(err(&req.method,"AI.STALE_OR_LOCKED: target changed, cancelled or test running"));
                continue;
            }
        }
        if Instant::now() > req.deadline {
            let _ = req
                .reply
                .try_send(err(&req.method, "request expired before execution"));
            continue;
        }
        persist |= handle(bridge, &mut host, req);
    }
    let (port, baud) = host.serial.current_port_baud();
    bridge.set_monitoring(host.serial.is_monitoring(), port, Some(baud));
    bridge.set_instrument_count(host.instruments.device_count());
    persist
}

fn handle(bridge: &ApiBridge, host: &mut UiHost<'_>, req: PendingRequest) -> bool {
    if req.method.starts_with("ui.ai.serial.") {
        let ctx=bridge.egui_ctx.lock().ok().and_then(|slot|slot.clone());
        let result=if let Some(ctx)=ctx {
            host.serial.api_serial_ai(&ctx,&host.cfg.ai,&host.test_tool.current_project_id(),host.test_tool.running_plugin_id().is_some(),&req.method,&req.params)
        }else{err(&req.method,"AI.UI_NOT_READY")};
        let _=req.reply.send(result);return false;
    }

    if req.method=="ui.report.tree_safe" {
        use std::sync::atomic::Ordering;
        if REPORT_TREE_BUSY.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){let _=req.reply.send(err("ui.report.tree_safe","REPORT.INDEX_BUSY"));return false;}
        let root=host.test_tool.data_root_path();let project=host.test_tool.current_project_id();let plugins=host.test_tool.plugin_metas();
        let _=std::thread::spawn(move||{
            struct Reset;impl Drop for Reset{fn drop(&mut self){REPORT_TREE_BUSY.store(false,Ordering::Release);}}let _reset=Reset;
            let result=match wiparse_core::project::load_project(&root,&project){
                None=>err("ui.report.tree_safe","PROJECT.NOT_FOUND"),
                Some(doc)=>{
                    let index=wiparse_core::project::index_project(&root,&doc,&plugins);
                    let tree=wiparse_core::project::tree_snapshot(&index);
                    let present:Vec<String>=index.tests.iter().map(|test| test.id.clone()).collect();
                    let addable=wiparse_core::project::addable_report_tests(&plugins,&present);
                    let value=json!({"project_id":project,"data_root":root.display().to_string(),"tree":tree,"addable":addable});
                    if serde_json::to_vec(&value).map_or(true,|data|data.len()>4*1024*1024){err("ui.report.tree_safe","REPORT.INDEX_TOO_LARGE")}
                    else{ok("ui.report.tree_safe",value)}
                }
            };let _=req.reply.send(result);
        });
        return false;
    }
    if (req.method.starts_with("ui.test_tool.config.") || req.method.starts_with("ui.test_tool.project.") || req.method == "ui.test_tool.result.get") || matches!(req.method.as_str(),"ui.test_tool.matrix.get"|"ui.test_tool.preflight.state"|"ui.test_tool.run.control") { host.test_tool.api_workspace(req);return false; }
    let PendingRequest {
        method,
        params,
        reply,
        deadline: _,
        ai_guard: _,
    } = req;
    let lang = *host.lang;
    let (result, persist) = if let Some(pair) = ui::handle(host, &method, &params) {
        pair
    } else {
        let result = match method.as_str() {
            "ui.serial.monitor.start_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() {
                    err(&method,"SERIAL.TEST_IN_PROGRESS")
                } else if params.get("port").and_then(|v|v.as_str()).is_none_or(|p|p.trim().is_empty()) || params.get("baud").and_then(|v|v.as_u64()).is_none_or(|b|b==0||b>12_000_000) {
                    err(&method,"SERIAL.INVALID_PARAMETERS")
                } else {
                    let port=params["port"].as_str().unwrap();
                    if let Some(existing)=host.serial.monitor_sessions().into_iter().find(|session|session.get("port").and_then(|v|v.as_str()).is_some_and(|p|p.eq_ignore_ascii_case(port))) {
                        ok(&method,json!({"accepted":true,"already_open":true,"session":existing}))
                    }else{serial_start(bridge,host.serial,lang,&params)}
                }
            }
            "ui.serial.monitor.stop_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() {
                    err(&method,"SERIAL.TEST_IN_PROGRESS")
                } else if let Some(port)=params.get("port").and_then(|v|v.as_str()).filter(|p|!p.trim().is_empty()) {
                    let sessions=host.serial.monitor_sessions();
                    let actual=sessions.iter().filter_map(|session|session.get("port").and_then(|v|v.as_str())).find(|p|p.eq_ignore_ascii_case(port));
                    if actual.is_none() {
                        err(&method,"SERIAL.NOT_MONITORING")
                    } else {
                        host.serial.api_stop_device(lang,actual.unwrap());
                        let still=host.serial.is_monitoring();bridge.set_monitoring(still,None,None);
                        *bridge.serial_write.lock().unwrap_or_else(|e|e.into_inner())=None;
                        bridge.publish_serial_status("stop requested");
                        ok(&method,json!({"accepted":true,"port":port}))
                    }
                } else { err(&method,"SERIAL.INVALID_PARAMETERS") }
            }
            "ui.serial.monitor.send_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() {
                    err(&method,"SERIAL.TEST_IN_PROGRESS")
                } else {
                    let port=params.get("port").and_then(|v|v.as_str()).filter(|p|!p.trim().is_empty());
                    let bytes=params.get("bytes").and_then(|v|v.as_array()).filter(|a|!a.is_empty()&&a.len()<=4096)
                        .and_then(|a|a.iter().map(|v|v.as_u64().and_then(|n|u8::try_from(n).ok())).collect::<Option<Vec<_>>>());
                    match (port,bytes) {
                        (Some(port),Some(bytes))=>{let len=bytes.len();match host.serial.api_queue_device_bytes(port,bytes){Ok(())=>ok(&method,json!({"queued":true,"bytes":len,"port":port})),Err(e)=>err(&method,&e)}},
                        _=>err(&method,"SERIAL.INVALID_PARAMETERS"),
                    }
                }
            }
            "serial.monitor.start" => serial_start(bridge, host.serial, lang, &params),
            "serial.monitor.stop" => {
                let port = params
                    .get("port")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let Some(port) = port {
                    host.serial.api_stop_device(lang, port);
                } else if let Some(selected) = host.serial.selected_device() {
                    host.serial.api_stop_device(lang, &selected);
                } else {
                    host.serial.api_stop_monitor(lang);
                }
                let still = host.serial.is_monitoring();
                bridge.set_monitoring(still, None, None);
                *bridge
                    .serial_write
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
                bridge.publish_serial_status("stopped");
                ok(&method, json!({ "stopped": true }))
            }
            "serial.monitor.status" => {
                let (port, baud) = host.serial.current_port_baud();
                let sessions = host.serial.monitor_sessions();
                let tab_id = port.as_deref().and_then(|p| host.serial.tab_for_port(p));
                ok(
                    &method,
                    json!({
                        "monitoring": host.serial.is_monitoring(),
                        "port": port,
                        "baud": baud,
                        "tab_id": tab_id,
                        "sessions": sessions,
                        "errors": host.serial.monitor_errors(),
                        "recording": host.serial.api_recording_status(),
                        "baud_options": host.serial.baud_presets(),
                        "status": host.serial.monitor_status_text(),
                    }),
                )
            }
            "ui.serial.browser_root_safe" => host.serial.api_browser_root_safe(host.cfg,&params),
            "ui.serial.recording_settings_safe" => {
                if host.test_tool.running_plugin_id().is_some()||host.serial.is_test_running(){err(&method,"SERIAL.TEST_IN_PROGRESS")}
                else {host.serial.api_recording_settings_safe(host.cfg,&params)}
            }
            "ui.serial.recording_safe" => {
                if host.test_tool.running_plugin_id().is_some()||host.serial.is_test_running(){err(&method,"SERIAL.TEST_IN_PROGRESS")}
                else {host.serial.api_set_recording_safe(&params)}
            }
            "serial.select" => serial_select(host.serial, lang, &params),
            "serial.send" => serial_send(bridge, host.serial, &params),
            "serial.read" => serial_read(host.serial, &params),
            "ui.serial.select_log_safe" => host.serial.api_select_log_safe(&params),
            "log.tabs.list" => ok(&method, host.serial.api_tabs_list()),
            "log.lines.get" => host.serial.api_lines_get(&params),
            "log.brief" => host.serial.api_brief(&params),
            "test.start" => {
                if host.test_tool.running_plugin_id()==Some("scope-serial-monitor") {
                    err(&method,"RUN.STATION_BUSY")
                } else {
                let mut params = params.clone();
                if let Some(plan) = params.get_mut("plan") {
                    wiparse_core::testrun::resolve_plan_placeholders(
                        plan,
                        &host.cfg.apps.instruments.waveform_source_dir,
                    );
                }
                match host.serial.api_test_start(lang, &params) {
                    Ok(data) => {
                        if let Some(tx) = host.serial.take_write_sender() {
                            *bridge
                                .serial_write
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = Some(tx);
                        }
                        let (port, baud) = host.serial.current_port_baud();
                        if host.serial.is_monitoring() {
                            bridge.set_monitoring(true, port, Some(baud));
                        }
                        ok(&method, data)
                    }
                    Err(e) => err(&method, &e),
                }
                }
            }
            "test.status" => ok(&method, host.serial.api_test_status()),
            "test.abort" => {
                let reason = params
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("aborted");
                match host.serial.api_test_abort(reason) {
                    Ok(data) => ok(&method, data),
                    Err(e) => err(&method, &e),
                }
            }
            "test.pack" => match host.serial.api_test_pack() {
                Ok(data) => ok(&method, data),
                Err(e) => err(&method, &e),
            },
            "ui.instrument.scan_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() {
                    err(&method,"INSTRUMENT.TEST_IN_PROGRESS")
                } else if host.instruments.is_scanning() {
                    err(&method,"INSTRUMENT.SCAN_IN_PROGRESS")
                } else if host.instruments.api_operations_pending() {
                    err(&method,"INSTRUMENT.OPERATION_IN_PROGRESS")
                } else { host.instruments.api_scan(&params) }
            },
            "instrument.scan" => host.instruments.api_scan(&params),
            "instrument.list" => ok(&method, host.instruments.api_list(lang, &params)),
            "instrument.overview" => ok(&method, host.instruments.api_overview(lang)),
            "instrument.connect" => host.instruments.api_connect(&params, lang),
            "ui.instrument.connect_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_connect_discovered_safe(&params, lang) }
            },
            "instrument.connect_all" => host.instruments.api_connect_all(lang),
            "ui.instrument.connect_manual_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_connect_manual_safe(&params, lang) }
            },
            "instrument.disconnect" => host.instruments.api_disconnect(&params),
            "ui.instrument.disconnect_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_disconnect_safe(&params) }
            },
            "parse.line"|"parse.metrics"|"convert.radix" => super::dispatch::invoke(bridge,&method,params.clone()),
            "instrument.job.get" => host.instruments.api_job_get(&params),
            "instrument.command" => host.instruments.api_command(&params),
            "ui.instrument.acquisition_safe" => {
                if (host.test_tool.running_plugin_id().is_some()||host.serial.is_test_running()) && !matches!(params["action"].as_str(),Some("state"|"stop"|"pause")){err(&method,"INSTRUMENT.TEST_IN_PROGRESS")}
                else{host.instruments.api_acquisition_safe(&params)}
            },
            "ui.instrument.export_samples_safe"=>host.instruments.api_export_samples_safe(&params),
            "ui.instrument.probe_operation_safe" => {
                if host.test_tool.running_plugin_id().is_some()||host.serial.is_test_running(){err(&method,"INSTRUMENT.TEST_IN_PROGRESS")}else{host.instruments.api_probe_operation(&params)}
            },
            "ui.instrument.control_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_control_safe(&params) }
            },
            "ui.instrument.scope_action_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_scope_action_safe(&params) }
            },
            "ui.instrument.source_action_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else {host.instruments.api_source_action_safe(&params)}
            },
            "ui.instrument.load_action_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else {host.instruments.api_load_action_safe(&params)}
            },
            "instrument.measure" => host.instruments.api_measure(&params),
            "ui.instrument.measure_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_measure_safe(&params) }
            },
            "instrument.capture" => host.instruments.api_capture(&params),
            "ui.instrument.capture_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else { host.instruments.api_capture_safe(&params) }
            },
            "ui.instrument.screenshot_safe" => host.instruments.api_screenshot_safe(&params),
            "instrument.waveform" => host.instruments.api_waveform(&params),
            "instrument.waveform_source" => host.instruments.api_waveform_source(&params),
            "ui.instrument.waveform_source_safe" => {
                if host.test_tool.running_plugin_id().is_some() || host.serial.is_test_running() { err(&method,"INSTRUMENT.TEST_IN_PROGRESS") }
                else {host.instruments.api_waveform_source_safe(&params)}
            },
            _ => err(
                &method,
                &format!("stateful method not implemented: {method}"),
            ),
        };
        (result, false)
    };
    let _ = reply.send(result);
    persist
}

fn serial_start(
    bridge: &ApiBridge,
    serial: &mut SerialToolPanel,
    lang: Lang,
    params: &Value,
) -> InvokeReply {
    let port = params
        .get("port")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let baud = params
        .get("baud")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let already_open = port
        .as_deref()
        .and_then(|p| serial.tab_for_port(p))
        .is_some();
    match serial.api_start_monitor(lang, port, baud) {
        Ok((port, baud)) => {
            if let Some(tx) = serial.take_write_sender() {
                *bridge
                    .serial_write
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(tx);
            }
            let tab_id = serial.tab_for_port(&port).unwrap_or(0);
            bridge.set_monitoring(true, Some(port.clone()), Some(baud));
            bridge.publish_serial_status(&format!("open {port} @ {baud}"));
            ok(
                "serial.monitor.start",
                json!({
                    "port": port,
                    "baud": baud,
                    "monitoring": true,
                    "tab_id": tab_id,
                    "already_open": already_open,
                }),
            )
        }
        Err(e) => err("serial.monitor.start", &e),
    }
}

fn serial_select(serial: &mut SerialToolPanel, lang: Lang, params: &Value) -> InvokeReply {
    let port = params
        .get("port")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let baud = params
        .get("baud")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    if port.is_none() && baud.is_none() {
        return err("serial.select", "missing port or baud");
    }
    match serial.api_select_port(lang, port, baud) {
        Ok((p, b)) => ok(
            "serial.select",
            json!({ "port": p, "baud": b, "monitoring": false }),
        ),
        Err(e) => err("serial.select", &e),
    }
}

fn serial_send(bridge: &ApiBridge, serial: &SerialToolPanel, params: &Value) -> InvokeReply {
    let hex = params
        .get("hex")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if hex.is_empty() {
        return err("serial.send", "missing hex");
    }
    if !serial.is_monitoring() {
        return err(
            "serial.send",
            "serial monitor is not running; call serial.monitor.start first",
        );
    }
    let cleaned: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.len() % 2 != 0 {
        return err("serial.send", "hex length must be even");
    }
    let mut bytes = Vec::with_capacity(cleaned.len() / 2);
    let mut i = 0;
    while i < cleaned.len() {
        match u8::from_str_radix(&cleaned[i..i + 2], 16) {
            Ok(b) => bytes.push(b),
            Err(e) => return err("serial.send", &e.to_string()),
        }
        i += 2;
    }
    let guard = bridge
        .serial_write
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(tx) => match tx.send(bytes) {
            Ok(()) => ok(
                "serial.send",
                json!({ "written": cleaned.len() / 2, "queued": true }),
            ),
            Err(_) => err("serial.send", "serial write channel closed"),
        },
        None => err("serial.send", "no active serial write channel"),
    }
}

fn serial_read(serial: &SerialToolPanel, params: &Value) -> InvokeReply {
    if !serial.is_monitoring() {
        return err(
            "serial.read",
            "serial monitor is not running; call serial.monitor.start first",
        );
    }
    let limit = params
        .get("max_logs")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;
    let lines = serial.api_recent_live_lines(limit);
    ok(
        "serial.read",
        json!({
            "monitoring": true,
            "logs": lines.iter().map(|line| json!({ "line": line })).collect::<Vec<_>>(),
            "count": lines.len(),
        }),
    )
}
