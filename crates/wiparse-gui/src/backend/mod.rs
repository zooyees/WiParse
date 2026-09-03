//! Embedded localhost API for Agent / CLI attach (C+E architecture).
//!
//! Endpoints:
//! - `GET  /v1/health`
//! - `GET  /v1/capabilities`
//! - `POST /v1/invoke`   `{ "method": "...", "params": { ... } }`
//! - `GET  /v1/events`   NDJSON event stream

mod capabilities;
mod dispatch;
mod events;
mod stateful;
mod ui;

pub use capabilities::capabilities_json;
pub use dispatch::{err as invoke_err, ok as invoke_ok};
pub use events::{EventBus, EventEnvelope};
pub use stateful::drain_api_requests;
pub use ui::UiHost;

use chrono::Local;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub const DEFAULT_BIND: &str = "127.0.0.1:7878";

#[derive(Debug, Clone)]
pub struct BackendStatus {
    pub listening: String,
    pub monitoring: bool,
    pub port: Option<String>,
    pub baud: Option<u32>,
    pub instrument_devices: usize,
}

impl Default for BackendStatus {
    fn default() -> Self {
        Self {
            listening: DEFAULT_BIND.into(),
            monitoring: false,
            port: None,
            baud: None,
            instrument_devices: 0,
        }
    }
}

#[derive(Debug)]
pub struct PendingRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<InvokeReply>,
}

#[derive(Debug, Clone)]
pub struct InvokeReply {
    pub ok: bool,
    pub cmd: String,
    pub data: Value,
    pub error: Option<String>,
}

/// Shared handle between HTTP thread and GUI app.
#[derive(Clone)]
pub struct ApiBridge {
    pub status: Arc<Mutex<BackendStatus>>,
    pub events: EventBus,
    pub requests: Receiver<PendingRequest>,
    pub(crate) request_tx: Sender<PendingRequest>,
    pub serial_write: Arc<Mutex<Option<Sender<Vec<u8>>>>>,
    pub running: Arc<AtomicBool>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl ApiBridge {
    pub fn new() -> Self {
        let (request_tx, requests) = unbounded();
        Self {
            status: Arc::new(Mutex::new(BackendStatus::default())),
            events: EventBus::new(),
            requests,
            request_tx,
            serial_write: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(true)),
            egui_ctx: Arc::new(Mutex::new(None)),
        }
    }

    pub fn try_recv(&self) -> Option<PendingRequest> {
        self.requests.try_recv().ok()
    }

    /// Called from the UI thread so Agent/CLI can wake a sleeping event loop.
    pub fn attach_egui_ctx(&self, ctx: &egui::Context) {
        let mut slot = self.egui_ctx.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(ctx.clone());
        }
    }

    pub fn wake_ui(&self) {
        if let Ok(slot) = self.egui_ctx.lock() {
            if let Some(ctx) = slot.as_ref() {
                ctx.request_repaint();
            }
        }
    }

    pub fn set_monitoring(&self, monitoring: bool, port: Option<String>, baud: Option<u32>) {
        if let Ok(mut s) = self.status.lock() {
            s.monitoring = monitoring;
            // Keep last selected port/baud when stopping so health matches brief.
            if port.is_some() {
                s.port = port;
            }
            if baud.is_some() {
                s.baud = baud;
            }
        }
    }

    pub fn set_instrument_count(&self, n: usize) {
        if let Ok(mut s) = self.status.lock() {
            s.instrument_devices = n;
        }
    }

    pub fn publish_serial_line(&self, line: &str) {
        self.events.publish(
            "serial.line",
            json!({ "line": line }),
            Some("live".into()),
        );
    }

    pub fn publish_serial_status(&self, status: &str) {
        self.events
            .publish("serial.status", json!({ "status": status }), None);
    }
}

pub fn start(bridge: ApiBridge) -> Result<(), String> {
    let bind = std::env::var("WIPARSE_API_BIND").unwrap_or_else(|_| DEFAULT_BIND.into());
    if let Ok(mut s) = bridge.status.lock() {
        s.listening = bind.clone();
    }
    let server = Server::http(&bind).map_err(|e| format!("API bind {bind}: {e}"))?;
    tracing::info!("WiParse API listening on http://{bind}");
    let bridge_c = bridge.clone();
    thread::Builder::new()
        .name("wiparse-api".into())
        .spawn(move || serve_loop(server, bridge_c))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn serve_loop(server: Server, bridge: ApiBridge) {
    while bridge.running.load(Ordering::SeqCst) {
        let request = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(_) => break,
        };
        if let Err(e) = handle_request(request, &bridge) {
            tracing::warn!("API request error: {e}");
        }
    }
}

fn handle_request(mut request: Request, bridge: &ApiBridge) -> Result<(), String> {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url);
    let method = request.method().clone();

    match (method, path) {
        (Method::Get, "/v1/health") => {
            let status = bridge.status.lock().map_err(|e| e.to_string())?.clone();
            respond_json(
                request,
                200,
                &envelope_ok(
                    "health",
                    json!({
                        "version": env!("CARGO_PKG_VERSION"),
                        "listening": status.listening,
                        "monitoring": status.monitoring,
                        "port": status.port,
                        "baud": status.baud,
                        "instrument_devices": status.instrument_devices,
                    }),
                ),
            )
        }
        (Method::Get, "/v1/capabilities") => {
            respond_json(request, 200, &envelope_ok("capabilities", capabilities_json()))
        }
        (Method::Post, "/v1/invoke") => {
            let bridge = bridge.clone();
            thread::Builder::new()
                .name("wiparse-api-invoke".into())
                .spawn(move || {
                    if let Err(e) = handle_invoke(request, &bridge) {
                        tracing::warn!("API invoke error: {e}");
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        (Method::Get, "/v1/events") => {
            let bridge = bridge.clone();
            thread::Builder::new()
                .name("wiparse-api-events".into())
                .spawn(move || {
                    if let Err(e) = serve_events(request, &bridge) {
                        tracing::warn!("API events error: {e}");
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        (Method::Options, _) => {
            let mut response = Response::empty(204);
            if let Ok(h) = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]) {
                response = response.with_header(h);
            }
            if let Ok(h) = Header::from_bytes(
                &b"Access-Control-Allow-Methods"[..],
                &b"GET, POST, OPTIONS"[..],
            ) {
                response = response.with_header(h);
            }
            if let Ok(h) =
                Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..])
            {
                response = response.with_header(h);
            }
            request.respond(response).map_err(|e| e.to_string())
        }
        _ => {
            respond_json(
                request,
                404,
                &envelope_err("http", &format!("not found: {path}")),
            )
        }
    }
}

fn handle_invoke(mut request: Request, bridge: &ApiBridge) -> Result<(), String> {
    let body = match read_body(&mut request) {
        Ok(b) => b,
        Err(e) => {
            return respond_json(request, 400, &envelope_err("invoke", &e));
        }
    };
    let parsed: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return respond_json(
                request,
                400,
                &envelope_err("invoke", &format!("invalid JSON: {e}")),
            );
        }
    };
    let method_name = match parsed.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => {
            return respond_json(request, 400, &envelope_err("invoke", "missing method"));
        }
    };
    let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));
    let reply = dispatch::invoke(bridge, &method_name, params);
    let code = if reply.ok { 200 } else { 400 };
    let payload = if reply.ok {
        envelope_ok(&reply.cmd, reply.data)
    } else {
        envelope_err(&reply.cmd, reply.error.as_deref().unwrap_or("error"))
    };
    respond_json(request, code, &payload)
}

fn serve_events(request: Request, bridge: &ApiBridge) -> Result<(), String> {
    let since = request
        .url()
        .split('?')
        .nth(1)
        .unwrap_or("")
        .split('&')
        .find_map(|pair| {
            let mut it = pair.splitn(2, '=');
            match (it.next(), it.next()) {
                (Some("since_seq"), Some(v)) => v.parse::<u64>().ok(),
                _ => None,
            }
        })
        .unwrap_or(0);

    let rx = bridge.events.subscribe(since);
    let mut writer = request.into_writer();
    write!(
        writer,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nCache-Control: no-cache\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;

    while bridge.running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(line) => {
                if writeln!(writer, "{line}").is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // keepalive comment line (invalid JSON ignored by clients that skip non-objects)
                if writeln!(writer, "{}", json!({"type":"ping","ts": now_iso()})).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
            Err(_) => break,
        }
    }
    Ok(())
}

fn read_body(request: &mut Request) -> Result<String, String> {
    let len = request.body_length().unwrap_or(0);
    let mut buf = vec![0u8; len.min(4 * 1024 * 1024)];
    let mut read = 0;
    while read < buf.len() {
        match request.as_reader().read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(e.to_string()),
        }
    }
    buf.truncate(read);
    String::from_utf8(buf).map_err(|e| e.to_string())
}

fn respond_json(request: Request, code: u16, value: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut response = Response::from_data(body).with_status_code(StatusCode(code));
    if let Ok(h) =
        Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..])
    {
        response = response.with_header(h);
    }
    if let Ok(h) = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]) {
        response = response.with_header(h);
    }
    if let Ok(h) =
        Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, OPTIONS"[..])
    {
        response = response.with_header(h);
    }
    if let Ok(h) = Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..])
    {
        response = response.with_header(h);
    }
    request.respond(response).map_err(|e| e.to_string())
}

pub fn envelope_ok(cmd: &str, data: Value) -> Value {
    json!({
        "ok": true,
        "cmd": cmd,
        "ts": now_iso(),
        "data": data,
    })
}

pub fn envelope_err(cmd: &str, message: &str) -> Value {
    json!({
        "ok": false,
        "cmd": cmd,
        "ts": now_iso(),
        "error": { "code": "ERROR", "message": message },
    })
}

fn now_iso() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
