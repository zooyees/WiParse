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
pub use events::EventBus;
pub use stateful::drain_api_requests;
pub use ui::UiHost;

use chrono::Local;
use crossbeam_channel::{bounded, Receiver, Sender};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
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
    pub deadline: Instant,
    pub ai_guard: Option<AiGuard>,
}

/// Internal AI calls are fenced again when the GUI consumes them.
#[derive(Debug, Clone)]
pub struct AiGuard {
    pub binding: String,
    pub project: String,
    pub controls: bool,
    pub cancelled: Arc<AtomicBool>,
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
    active_invokes: Arc<AtomicUsize>,
    ai_token: Arc<String>,
    ai_token_ref: Arc<String>,
    ai_auth_ready: Arc<AtomicBool>,
    control_auth_required: bool,
}

impl ApiBridge {
    pub fn new() -> Self {
        // Keep overload visible to callers instead of allowing an unbounded
        // queue to consume memory while the GUI is busy.
        let (request_tx, requests) = bounded(256);
        Self {
            status: Arc::new(Mutex::new(BackendStatus::default())),
            events: EventBus::new(),
            requests,
            request_tx,
            serial_write: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(true)),
            egui_ctx: Arc::new(Mutex::new(None)),
            active_invokes: Arc::new(AtomicUsize::new(0)),
            ai_token: Arc::new(std::env::var("WIPARSE_LOCAL_API_TOKEN").ok().filter(|value|uuid::Uuid::parse_str(value).is_ok()).unwrap_or_else(||uuid::Uuid::new_v4().to_string())),
            ai_token_ref: Arc::new(uuid::Uuid::new_v4().to_string()),
            ai_auth_ready: Arc::new(AtomicBool::new(false)),
            control_auth_required: crate::companion_window(),
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

struct AiTokenGuard {path:std::path::PathBuf,encrypted:String}
impl Drop for AiTokenGuard {fn drop(&mut self){
    if std::fs::symlink_metadata(&self.path).is_ok_and(|meta|meta.is_file()&&!meta.file_type().is_symlink())
        &&std::fs::read_to_string(&self.path).is_ok_and(|value|value==self.encrypted){let _=std::fs::remove_file(&self.path);}
}}
fn publish_ai_token(bridge:&ApiBridge)->Result<AiTokenGuard,String>{
    let path=wiparse_core::ai::ai_bridge_token_path(&bridge.ai_token_ref).ok_or("AI bridge token directory unavailable")?;
    publish_ai_token_at(bridge,path)
}
fn publish_ai_token_at(bridge:&ApiBridge,path:std::path::PathBuf)->Result<AiTokenGuard,String>{
    let directory=path.parent().ok_or("AI bridge token directory unavailable")?;
    std::fs::create_dir_all(directory).map_err(|_|"AI bridge token directory unavailable")?;
    let meta=std::fs::symlink_metadata(directory).map_err(|_|"AI bridge token directory unavailable")?;
    if !meta.is_dir()||meta.file_type().is_symlink(){return Err("AI bridge token directory invalid".into());}
    let encrypted=wiparse_core::ai::protect_key(&bridge.ai_token).map_err(|_|"AI bridge token protection unavailable")?;
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_|"AI bridge token creation failed")?;
    if file.write_all(encrypted.as_bytes()).and_then(|_|file.sync_all()).is_err(){drop(file);let _=std::fs::remove_file(&path);return Err("AI bridge token write failed".into());}
    Ok(AiTokenGuard{path,encrypted})
}

pub fn start(bridge: ApiBridge) -> Result<(), String> {
    let bind = std::env::var("WIPARSE_API_BIND").unwrap_or_else(|_| DEFAULT_BIND.into());
    if let Ok(mut s) = bridge.status.lock() {
        s.listening = bind.clone();
    }
    let server = Server::http(&bind).map_err(|e| format!("API bind {bind}: {e}"))?;
    let ai_token_guard=if bind==DEFAULT_BIND {match publish_ai_token(&bridge){Ok(guard)=>{bridge.ai_auth_ready.store(true,Ordering::Release);Some(guard)},Err(error)=>{tracing::warn!("{error}; protected AI API unavailable");None}}}else{None};
    tracing::info!("WiParse API listening on http://{bind}");
    let bridge_c = bridge.clone();
    thread::Builder::new()
        .name("wiparse-api".into())
        .spawn(move || {let _guard=ai_token_guard;serve_loop(server, bridge_c)})
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
                        "product_id": "com.wiparse.host",
                        "bridge_protocol": 2,
                        "control_auth_required": bridge.control_auth_required,
                        "recovery_mode":crate::companion_detached(),
                        "instance_id": bridge.ai_token_ref.as_str(),
                        "process_id": std::process::id(),
                        "features": ["companion-graceful-exit-v1","typed-control-v1","serial-ai-v1","config-document-edit-v1","project-transfer-v1","live-source-identity-v1","scope-rules-document-v1","ui-navigation-v1","serial-log-editor-v1","serial-recording-settings-v1","serial-log-lifecycle-v1","instrument-acquisition-v1","manual-connect-v1","document-origin-bindings-v1","preferences-v1","migration-complete-api-v1"],
                        "listening": status.listening,
                        "monitoring": status.monitoring,
                        "port": status.port,
                        "baud": status.baud,
                        "instrument_devices": status.instrument_devices,
                        "ai_auth_ref": if bridge.ai_auth_ready.load(Ordering::Acquire){Some(bridge.ai_token_ref.as_str())}else{None},
                    }),
                ),
            )
        }
        (Method::Get, "/v1/capabilities") => {
            respond_json(request, 200, &envelope_ok("capabilities", capabilities_json()))
        }
        (Method::Post, "/v1/invoke") => {
            if bridge.active_invokes.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < 32).then_some(n + 1)).is_err() {
                return respond_json(request, 503, &envelope_err("invoke", "too many concurrent requests"));
            }
            let bridge = bridge.clone();
            let active = bridge.active_invokes.clone();
            let active_thread = active.clone();
            let spawn = thread::Builder::new()
                .name("wiparse-api-invoke".into())
                .spawn(move || {
                    if let Err(e) = handle_invoke(request, &bridge) {
                        tracing::warn!("API invoke error: {e}");
                    }
                    active_thread.fetch_sub(1, Ordering::SeqCst);
                });
            if let Err(e) = spawn {
                active.fetch_sub(1, Ordering::SeqCst);
                return Err(e.to_string());
            }
            Ok(())
        }
        (Method::Get, "/v1/events") => {
            let authorization=request.headers().iter().find(|header|header.field.equiv("Authorization")).map(|header|header.value.as_str());
            if bridge.control_auth_required&&!ai_request_authorized(true,authorization,&bridge.ai_token){return respond_json(request,401,&envelope_err("events","API.AUTH_REQUIRED"));}
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

fn ai_request_authorized(ready:bool,authorization:Option<&str>,expected:&str)->bool{
    ready&&authorization.and_then(|value|value.strip_prefix("Bearer ")).is_some_and(|token|token==expected)
}

#[cfg(test)]
mod ai_auth_tests {
    #[test]
    fn companion_rejects_unauthorized_requests_before_dispatch(){
        let mut bridge=super::ApiBridge::new();bridge.control_auth_required=true;
        let token=(*bridge.ai_token).clone();
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_ip().unwrap();
        let worker=std::thread::spawn(move||{for _ in 0..3{let request=server.recv().unwrap();super::handle_request(request,&bridge).unwrap();}});
        let url=format!("http://{address}/v1/invoke");
        for supplied in [None,Some("wrong")] {
            let mut request=ureq::post(&url).timeout(std::time::Duration::from_secs(3));if let Some(value)=supplied{request=request.set("Authorization",&format!("Bearer {value}"));}
            let response=request.send_json(serde_json::json!({"method":"system.version","params":{}}));
            assert!(matches!(response,Err(ureq::Error::Status(401,_))));
        }
        let reply:serde_json::Value=ureq::post(&url).set("Authorization",&format!("Bearer {token}")).send_json(serde_json::json!({"method":"system.version","params":{}})).unwrap().into_json().unwrap();
        assert_eq!(reply["ok"],true);worker.join().unwrap();
    }
    #[test]fn migrated_endpoints_are_dispatched_to_the_gui_thread(){
        for method in ["ui.ai.settings.get_safe","ui.ai.settings.set_safe","ui.ai.test_safe","ui.ai.chat.submit_safe","ui.ai.chat.state_safe","ui.report.tree_safe","ui.test_tool.state_safe","ui.test_tool.generic_run_safe","ui.test_tool.generic_stop_safe","ui.test_tool.scope_rule_enabled_safe","ui.test_tool.scope_rule_add_safe","ui.test_tool.scope_rule_text_safe","ui.test_tool.scope_rule_exclusion_safe","ui.test_tool.scope_rule_order_safe","ui.test_tool.scope_settings_import_safe","ui.test_tool.scope_settings_export_safe","ui.test_tool.scope_edge_mode_safe","ui.test_tool.scope_serial_settings_safe","ui.test_tool.scope_instrument_safe","ui.test_tool.scope_preflight_safe","ui.test_tool.scope_run_safe","ui.test_tool.scope_stop_safe","ui.instrument.source_action_safe","ui.instrument.load_action_safe","ui.instrument.waveform_source_safe","ui.serial.open_safe","ui.serial.close_safe","ui.serial.clear_live_safe"] {assert!(super::capabilities::is_stateful(method),"{method}");}
    }
    #[test]fn protected_ai_methods_reject_absent_wrong_and_unpublished_tokens(){
        use super::ai_request_authorized;
        assert!(!ai_request_authorized(false,Some("Bearer right"),"right"));
        assert!(!ai_request_authorized(true,None,"right"));
        assert!(!ai_request_authorized(true,Some("Bearer wrong"),"right"));
        assert!(!ai_request_authorized(true,Some("right"),"right"));
        assert!(ai_request_authorized(true,Some("Bearer right"),"right"));
    }
    #[test]fn http_ai_settings_rejects_a_missing_token_before_dispatch(){
        use std::io::{Read,Write};
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_string();
        let bridge=super::ApiBridge::new();bridge.ai_auth_ready.store(true,std::sync::atomic::Ordering::Release);
        let task=std::thread::spawn(move||{let request=server.recv().unwrap();super::handle_request(request,&bridge).unwrap();assert!(bridge.try_recv().is_none());});
        let mut stream=std::net::TcpStream::connect(address).unwrap();stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let body=r#"{"method":"ui.ai.settings.get_safe","params":{}}"#;
        write!(stream,"POST /v1/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        let mut response=String::new();stream.read_to_string(&mut response).unwrap();assert!(response.starts_with("HTTP/1.1 401"));assert!(response.contains("AI.AUTH_REQUIRED"));task.join().unwrap();
    }
    #[test]fn http_ai_settings_with_token_reaches_the_stateful_dispatch(){
        use std::io::{Read,Write};use std::time::{Duration,Instant};
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr().to_string();
        let bridge=super::ApiBridge::new();bridge.ai_auth_ready.store(true,std::sync::atomic::Ordering::Release);let token=bridge.ai_token.to_string();let consumer=bridge.clone();
        let worker=std::thread::spawn(move||{let deadline=Instant::now()+Duration::from_secs(3);loop{if let Some(req)=consumer.try_recv(){assert_eq!(req.method,"ui.ai.settings.get_safe");req.reply.send(super::invoke_ok(&req.method,serde_json::json!({"fixture":true}))).unwrap();break;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(5));}});
        let task=std::thread::spawn(move||{let request=server.recv().unwrap();super::handle_request(request,&bridge).unwrap();});
        let mut stream=std::net::TcpStream::connect(address).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let body=r#"{"method":"ui.ai.settings.get_safe","params":{}}"#;
        write!(stream,"POST /v1/invoke HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",token,body.len(),body).unwrap();
        let mut response=String::new();stream.read_to_string(&mut response).unwrap();assert!(response.starts_with("HTTP/1.1 200"));assert!(response.contains("fixture"));task.join().unwrap();worker.join().unwrap();
    }
    #[test]
    #[cfg(windows)]
    #[ignore = "requires the interactive Windows user profile for DPAPI"]
    fn protected_ai_token_file_is_user_bound_and_removed_by_its_owner(){
        let root=std::env::temp_dir().join(format!("wiparse-ai-token-test-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let path=root.join("token");let bridge=super::ApiBridge::new();let guard=super::publish_ai_token_at(&bridge,path.clone()).unwrap();
        let encrypted=std::fs::read_to_string(&path).unwrap();assert!(!encrypted.contains(bridge.ai_token.as_str()));
        assert_eq!(wiparse_core::ai::unprotect_bridge_token(&encrypted).unwrap(),*bridge.ai_token);
        assert!(super::publish_ai_token_at(&bridge,path.clone()).is_err());assert_eq!(std::fs::read_to_string(&path).unwrap(),encrypted);
        drop(guard);assert!(!path.exists());std::fs::remove_dir(root).unwrap();
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
    if bridge.control_auth_required || method_name.starts_with("ui.ai.") {
        let authorization=request.headers().iter().find(|header|header.field.equiv("Authorization")).map(|header|header.value.as_str());
        if !ai_request_authorized(bridge.control_auth_required||bridge.ai_auth_ready.load(Ordering::Acquire),authorization,&bridge.ai_token){
            return respond_json(request,401,&envelope_err("invoke",if bridge.control_auth_required{"API.AUTH_REQUIRED"}else{"AI.AUTH_REQUIRED"}));
        }
    }
    let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));
    let reply = dispatch::invoke(bridge, &method_name, params);
    let code = if reply.ok { 200 } else { 400 };
    let payload = if reply.ok {
        envelope_ok(&reply.cmd, reply.data)
    } else {
        let mut error=envelope_err(&reply.cmd, reply.error.as_deref().unwrap_or("error"));
        if reply.cmd.starts_with("ui.test_tool.config.")||reply.cmd.starts_with("ui.test_tool.project.")||matches!(reply.cmd.as_str(),"ui.test_tool.matrix.get"|"ui.test_tool.preflight.state"){
            error["error"]["code"]=json!(reply.error.as_deref().unwrap_or("CONFIG.OPERATION_FAILED"));
            if reply.data.is_object(){error["error"]["details"]=reply.data;}
        }
        error
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

    // The Next serial pane needs invalidations, not duplicated raw-line payloads.
    // Existing unfiltered CLI/MCP clients retain their original stream.
    let serial_only=request.url().split('?').nth(1).unwrap_or("").split('&').any(|pair|pair=="types=serial.log.changed");
    let rx = if serial_only{bridge.events.subscribe_serial_logs(since)}else{bridge.events.subscribe(since)}.map_err(str::to_string)?;
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
