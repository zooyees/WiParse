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
        if req.ai_guard.is_some() && matches!(req.method.as_str(),"instrument.command"|"instrument.capture") {
            if let Err(error)=host.instruments.agent_validate_device(&req.params,req.method=="instrument.capture") {let _=req.reply.try_send(err(&req.method,&error));continue;}
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

pub(crate) fn handle(bridge: &ApiBridge, host: &mut UiHost<'_>, req: PendingRequest) -> bool {
    if req.method.starts_with("engineering.") {
        let method=req.method.clone();
        if req.ai_guard.is_none(){let _=req.reply.try_send(err(&method,"AI.GUARD_REQUIRED"));return false;}
        if let Err(e)=SerialToolPanel::validate_agent_instrument(&method,&req.params){let _=req.reply.try_send(err(&method,&e));return false;}
        let value=match method.as_str(){
            "engineering.inspect"=>{
                let tool=req.params["tool"].as_str().unwrap_or("");
                let data=match tool{"waveform"=>host.waveform.ai_summary(),"data_analysis"=>host.data_analysis.ai_summary(),"test_report"=>host.test_report.ai_summary(),"test_tool"=>host.test_tool.agent_run_status(),"calculator"=>host.calculator.ai_summary(),_=>host.instruments.ai_summary()};
                json!({"ok":true,"data":{"tool":tool,"project":host.test_tool.current_project_id(),"captured_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),"context":data}})
            },
            "engineering.wave.frames"=>host.waveform.agent_frames(&req.params),
            "engineering.calc.evaluate"=>crate::calculator::CalculatorPanel::new().agent_calculate(&req.params),
            "engineering.wave.measure"=>{
                let reply=host.waveform.agent_measure(&req.params,&egui::Context::default(),req.ai_guard.as_ref().unwrap().cancelled.clone());
                if let Some(rx)=reply.receiver{
                    std::thread::spawn(move||{let value=rx.recv().unwrap_or_else(|_|json!({"ok":false,"error":"AI.WORKER_DISCONNECTED"}));let _=req.reply.try_send(InvokeReply{ok:value["ok"]==true,cmd:method,data:value["data"].clone(),error:value["error"].as_str().map(str::to_owned)});});return false;
                }
                reply.value
            },_=>json!({"ok":false,"error":"AI.UNKNOWN_ENGINEERING_METHOD"})
        };
        let _=req.reply.try_send(InvokeReply{ok:value["ok"]==true,cmd:method,data:value["data"].clone(),error:value["error"].as_str().map(str::to_owned)});return false;
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
                        "status": host.serial.monitor_status_text(),
                    }),
                )
            }
            "serial.select" => serial_select(host.serial, lang, &params),
            "serial.send" => serial_send(bridge, host.serial, &params),
            "serial.read" => serial_read(host.serial, &params),
            "log.tabs.list" => ok(&method, host.serial.api_tabs_list()),
            "log.lines.get" => host.serial.api_lines_get(&params),
            "log.brief" => host.serial.api_brief(&params),
            "test.start" => {
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
            "instrument.scan" => host.instruments.api_scan(&params),
            "instrument.list" => ok(&method, host.instruments.api_list(lang, &params)),
            "instrument.overview" => ok(&method, host.instruments.api_overview(lang)),
            "instrument.connect" => host.instruments.api_connect(&params, lang),
            "instrument.connect_all" => host.instruments.api_connect_all(lang),
            "instrument.disconnect" => host.instruments.api_disconnect(&params),
            "parse.line"|"parse.metrics"|"convert.radix" => super::dispatch::invoke(bridge,&method,params.clone()),
            "instrument.job.get" => host.instruments.api_job_get(&params),
            "instrument.command" => host.instruments.api_command(&params),
            "instrument.measure" => host.instruments.api_measure(&params),
            "instrument.capture" => host.instruments.api_capture(&params),
            "instrument.waveform" => host.instruments.api_waveform(&params),
            "instrument.waveform_source" => host.instruments.api_waveform_source(&params),
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
