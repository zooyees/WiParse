//! Stateless invoke handlers (run on API thread) + helper to queue stateful ones.

use super::capabilities::is_stateful;
use super::{ApiBridge, InvokeReply, PendingRequest};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use wiparse_core::config::load_config;
use wiparse_core::db::{get_session, list_sessions, open_db, session_log_count, session_metric_count};
use wiparse_core::metrics::parse_metric_frame;
use wiparse_core::protocol::parse_qi_line;
use wiparse_core::scope::{self, scope_capabilities};
use wiparse_core::serial::list_ports;
use wiparse_core::wave::{
    export_metrics_csv, export_metrics_json, fetch_session_metrics, metrics_to_wave, DEFAULT_CHANNELS,
};
use wiparse_core::VERSION;

pub fn invoke(bridge: &ApiBridge, method: &str, params: Value) -> InvokeReply {
    if is_stateful(method) {
        return invoke_stateful(bridge, method, params);
    }
    match method {
        "system.version" => ok(
            method,
            json!({
                "version": VERSION,
                "name": "wiparse",
                "edition": "rust",
                "api": env!("CARGO_PKG_VERSION"),
            }),
        ),
        "serial.ports" => match list_ports() {
            Ok(ports) => ok(method, serde_json::to_value(ports).unwrap_or(json!([]))),
            Err(e) => err(method, &e.to_string()),
        },
        "parse.line" => {
            let text = params_str(&params, "text").unwrap_or("");
            ok(
                method,
                serde_json::to_value(parse_qi_line(text)).unwrap_or(json!(null)),
            )
        }
        "parse.metrics" => {
            let text = params_str(&params, "text").unwrap_or("");
            match parse_metric_frame(text) {
                Some(m) => ok(
                    method,
                    serde_json::to_value(m).unwrap_or(json!(null)),
                ),
                None => err(method, "invalid AA55 frame"),
            }
        }
        "parse.file" => parse_file(method, &params),
        "session.list" => session_list(method, &params),
        "session.show" => session_show(method, &params),
        "wave.session" => wave_session(method, &params),
        "wave.export" => wave_export(method, &params),
        "scope.list" => scope_list(method),
        "scope.shot" => scope_shot(method, &params),
        "scope.wave" => scope_wave(method, &params),
        "convert.radix" => convert_radix(method, &params),
        "convert.expr" => err(
            method,
            "use GUI converter or convert.radix; expr evaluation is GUI-side",
        ),
        _ => err(method, &format!("unknown method: {method}")),
    }
}

fn invoke_stateful(bridge: &ApiBridge, method: &str, params: Value) -> InvokeReply {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let pending = PendingRequest {
        method: method.to_string(),
        params,
        reply: tx,
    };
    if bridge.request_tx.send(pending).is_err() {
        return err(method, "GUI API bridge closed");
    }
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(reply) => reply,
        Err(_) => err(method, "timeout waiting for GUI (is WiParse running?)"),
    }
}

fn parse_file(method: &str, params: &Value) -> InvokeReply {
    let path = match params_str(params, "path") {
        Some(p) if !p.is_empty() => p,
        _ => return err(method, "missing path"),
    };
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    match fs::read_to_string(path) {
        Ok(text) => ok(method, parse_many(text.lines(), limit)),
        Err(e) => err(method, &e.to_string()),
    }
}

fn parse_many<'a>(lines: impl Iterator<Item = &'a str>, limit: Option<usize>) -> Value {
    let mut qi = Vec::new();
    let mut metrics = Vec::new();
    for (i, line) in lines.enumerate() {
        if limit.is_some_and(|n| i >= n) {
            break;
        }
        if let Some(m) = parse_metric_frame(line) {
            metrics.push(m);
        } else if line.contains("ASK ") || line.contains("FSK ") {
            qi.push(parse_qi_line(line));
        }
    }
    json!({
        "qi": qi,
        "metrics": metrics,
        "qi_count": qi.len(),
        "metrics_count": metrics.len()
    })
}

fn db_conn() -> Result<rusqlite::Connection, String> {
    let cfg = load_config().map_err(|e| e.to_string())?;
    open_db(wiparse_core::db::default_db_path(&cfg.system.db_name)).map_err(|e| e.to_string())
}

fn session_list(method: &str, params: &Value) -> InvokeReply {
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    match db_conn().and_then(|c| list_sessions(&c, limit).map_err(|e| e.to_string())) {
        Ok(rows) => ok(method, serde_json::to_value(rows).unwrap_or(json!([]))),
        Err(e) => err(method, &e),
    }
}

fn session_show(method: &str, params: &Value) -> InvokeReply {
    let id = match params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return err(method, "missing id"),
    };
    match db_conn() {
        Ok(conn) => match get_session(&conn, id) {
            Ok(Some(s)) => {
                let logs = session_log_count(&conn, id).unwrap_or(0);
                let metrics = session_metric_count(&conn, id).unwrap_or(0);
                ok(
                    method,
                    json!({
                        "session": s,
                        "log_count": logs,
                        "metric_count": metrics,
                    }),
                )
            }
            Ok(None) => err(method, "session not found"),
            Err(e) => err(method, &e.to_string()),
        },
        Err(e) => err(method, &e),
    }
}

fn parse_channels(spec: &str) -> Vec<&str> {
    let parts: Vec<&str> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        DEFAULT_CHANNELS.to_vec()
    } else {
        parts
    }
}

fn wave_session(method: &str, params: &Value) -> InvokeReply {
    let session_id = match params.get("session_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return err(method, "missing session_id"),
    };
    let channels_owned = params
        .get("channels")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let channels = parse_channels(&channels_owned);
    let from = params.get("from").and_then(|v| v.as_f64());
    let to = params.get("to").and_then(|v| v.as_f64());
    match db_conn().and_then(|c| {
        fetch_session_metrics(&c, session_id, from, to).map_err(|e| e.to_string())
    }) {
        Ok(rows) => {
            let wave = metrics_to_wave(&rows, &channels);
            ok(method, wave)
        }
        Err(e) => err(method, &e),
    }
}

fn wave_export(method: &str, params: &Value) -> InvokeReply {
    let session_id = match params.get("session_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return err(method, "missing session_id"),
    };
    let out = match params_str(params, "out") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return err(method, "missing out"),
    };
    let format = params
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("csv");
    let from = params.get("from").and_then(|v| v.as_f64());
    let to = params.get("to").and_then(|v| v.as_f64());
    match db_conn().and_then(|c| {
        fetch_session_metrics(&c, session_id, from, to).map_err(|e| e.to_string())
    }) {
        Ok(rows) => {
            let result = if format.eq_ignore_ascii_case("json") {
                export_metrics_json(&out, &rows)
            } else {
                export_metrics_csv(&out, &rows)
            };
            match result {
                Ok(n) => ok(
                    method,
                    json!({ "path": out.display().to_string(), "count": n, "format": format }),
                ),
                Err(e) => err(method, &e.to_string()),
            }
        }
        Err(e) => err(method, &e),
    }
}

fn scope_list(method: &str) -> InvokeReply {
    match scope::list_scopes() {
        Ok(scopes) => ok(
            method,
            json!({
                "scopes": scopes,
                "capabilities": scope_capabilities(),
            }),
        ),
        Err(e) => err(method, &e.to_string()),
    }
}

fn scope_shot(method: &str, params: &Value) -> InvokeReply {
    let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let out = params_str(params, "out").map(PathBuf::from);
    match scope::capture_shot(index, out.as_deref()) {
        Ok(data) => ok(method, data),
        Err(e) => err(method, &e.to_string()),
    }
}

fn scope_wave(method: &str, params: &Value) -> InvokeReply {
    let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let channel = params
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("CH1");
    let points = params
        .get("points")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    match scope::read_waveform_json(index, channel, points) {
        Ok(data) => ok(method, data),
        Err(e) => err(method, &e.to_string()),
    }
}

fn convert_radix(method: &str, params: &Value) -> InvokeReply {
    let input = params_str(params, "input").unwrap_or("");
    let source = params
        .get("source_base")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as u32;
    let target = params
        .get("target_base")
        .and_then(|v| v.as_u64())
        .unwrap_or(16) as u32;
    match parse_i128_radix(input, source).and_then(|v| format_i128_radix(v, target)) {
        Ok(s) => ok(
            method,
            json!({ "value": s, "source_base": source, "target_base": target }),
        ),
        Err(e) => err(method, &e),
    }
}

fn parse_i128_radix(input: &str, base: u32) -> Result<i128, String> {
    if !(2..=36).contains(&base) {
        return Err("base must be 2..36".into());
    }
    let compact: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    let (neg, rest) = if let Some(r) = compact.strip_prefix('-') {
        (true, r)
    } else {
        (false, compact.as_str())
    };
    let digits = match base {
        16 => rest
            .strip_prefix("0x")
            .or_else(|| rest.strip_prefix("0X"))
            .unwrap_or(rest),
        2 => rest
            .strip_prefix("0b")
            .or_else(|| rest.strip_prefix("0B"))
            .unwrap_or(rest),
        8 => rest
            .strip_prefix("0o")
            .or_else(|| rest.strip_prefix("0O"))
            .unwrap_or(rest),
        _ => rest,
    };
    let mag = u128::from_str_radix(digits, base).map_err(|e| e.to_string())?;
    if neg {
        if mag == 1_u128 << 127 {
            Ok(i128::MIN)
        } else if mag <= i128::MAX as u128 {
            Ok(-(mag as i128))
        } else {
            Err("i128 overflow".into())
        }
    } else if mag <= i128::MAX as u128 {
        Ok(mag as i128)
    } else {
        Err("i128 overflow".into())
    }
}

fn format_i128_radix(value: i128, base: u32) -> Result<String, String> {
    if !(2..=36).contains(&base) {
        return Err("base must be 2..36".into());
    }
    if value == 0 {
        return Ok("0".into());
    }
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let neg = value < 0;
    let mut mag = value.unsigned_abs();
    let mut out = Vec::new();
    while mag > 0 {
        out.push(DIGITS[(mag % base as u128) as usize] as char);
        mag /= base as u128;
    }
    if neg {
        out.push('-');
    }
    out.reverse();
    Ok(out.into_iter().collect())
}

fn params_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|v| v.as_str())
}

pub fn ok(cmd: &str, data: Value) -> InvokeReply {
    InvokeReply {
        ok: true,
        cmd: cmd.into(),
        data,
        error: None,
    }
}

pub fn err(cmd: &str, message: &str) -> InvokeReply {
    InvokeReply {
        ok: false,
        cmd: cmd.into(),
        data: json!({}),
        error: Some(message.into()),
    }
}
