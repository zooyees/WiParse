//! Handle stateful API invokes on the GUI thread.

use super::dispatch::{err, ok};
use super::{ApiBridge, InvokeReply, PendingRequest};
use crate::instrument_control::InstrumentControlPanel;
use crate::serial_tool::SerialToolPanel;
use serde_json::{json, Value};
use wiparse_core::i18n::Lang;
use wiparse_core::instrument::ControlCommand;

pub fn drain_api_requests(
    bridge: &ApiBridge,
    serial: &mut SerialToolPanel,
    instruments: &mut InstrumentControlPanel,
    lang: Lang,
    active_tab: &str,
) {
    while let Some(req) = bridge.try_recv() {
        let reply = handle(bridge, serial, instruments, lang, active_tab, req);
        let _ = reply; // reply already sent inside handle via req.reply
    }
    let (port, baud) = serial.current_port_baud();
    bridge.set_monitoring(serial.is_monitoring(), port, Some(baud));
}

fn handle(
    bridge: &ApiBridge,
    serial: &mut SerialToolPanel,
    instruments: &mut InstrumentControlPanel,
    lang: Lang,
    active_tab: &str,
    req: PendingRequest,
) {
    let PendingRequest {
        method,
        params,
        reply,
    } = req;
    let result = match method.as_str() {
        "system.ui.state" => ok(
            &method,
            json!({
                "active_tab": active_tab,
                "monitoring": serial.is_monitoring(),
                "status": serial.status_text(),
                "instrument_devices": instruments.device_count(),
            }),
        ),
        "serial.monitor.start" => serial_start(bridge, serial, lang, &params),
        "serial.monitor.stop" => {
            serial.api_stop_monitor(lang);
            bridge.set_monitoring(false, None, None);
            *bridge.serial_write.lock().unwrap_or_else(|e| e.into_inner()) = None;
            bridge.publish_serial_status("stopped");
            ok(&method, json!({ "stopped": true }))
        }
        "serial.monitor.status" => {
            let (port, baud) = serial.current_port_baud();
            ok(
                &method,
                json!({
                    "monitoring": serial.is_monitoring(),
                    "port": port,
                    "baud": baud,
                    "status": serial.monitor_status_text(),
                }),
            )
        }
        "serial.select" => serial_select(serial, lang, &params),
        "serial.send" => serial_send(bridge, serial, &params),
        "serial.read" => serial_read(serial, &params),
        "log.tabs.list" => ok(&method, serial.api_tabs_list()),
        "log.lines.get" => serial.api_lines_get(&params),
        "log.brief" => serial.api_brief(&params),
        "test.start" => match serial.api_test_start(lang, &params) {
            Ok(data) => {
                if let Some(tx) = serial.take_write_sender() {
                    *bridge.serial_write.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
                }
                let (port, baud) = serial.current_port_baud();
                if serial.is_monitoring() {
                    bridge.set_monitoring(true, port, Some(baud));
                }
                ok(&method, data)
            }
            Err(e) => err(&method, &e),
        },
        "test.status" => ok(&method, serial.api_test_status()),
        "test.abort" => {
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("aborted");
            match serial.api_test_abort(reason) {
                Ok(data) => ok(&method, data),
                Err(e) => err(&method, &e),
            }
        }
        "test.pack" => match serial.api_test_pack() {
            Ok(data) => ok(&method, data),
            Err(e) => err(&method, &e),
        },
        "instrument.scan" => instruments.api_scan(&params),
        "instrument.list" => ok(&method, instruments.api_list()),
        "instrument.connect" => instruments.api_connect(&params, lang),
        "instrument.disconnect" => instruments.api_disconnect(&params),
        "instrument.command" => instruments.api_command(&params),
        "instrument.measure" => instruments.api_measure(&params),
        "instrument.capture" => instruments.api_capture(&params),
        "instrument.waveform" => instruments.api_waveform(&params),
        _ => err(&method, &format!("stateful method not implemented: {method}")),
    };
    bridge.set_instrument_count(instruments.device_count());
    let _ = reply.send(result);
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
    match serial.api_start_monitor(lang, port, baud) {
        Ok((port, baud)) => {
            if let Some(tx) = serial.take_write_sender() {
                *bridge.serial_write.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
            }
            bridge.set_monitoring(true, Some(port.clone()), Some(baud));
            bridge.publish_serial_status(&format!("open {port} @ {baud}"));
            ok(
                "serial.monitor.start",
                json!({ "port": port, "baud": baud, "monitoring": true }),
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
    let guard = bridge.serial_write.lock().unwrap_or_else(|e| e.into_inner());
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

pub fn parse_control_command(value: &Value) -> Result<ControlCommand, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("invalid ControlCommand: {e}"))
}
