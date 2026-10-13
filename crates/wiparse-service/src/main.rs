//! Independent local automation service. No product rules or guessed IO firmware.
mod jobs;
mod storage;
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tiny_http::{Header, Response, Server, StatusCode};

fn main() {
    if let Err(e) = run() {
        eprintln!("WiParse service: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("automation-data");
    let mut port = 7879u16;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-root" => root = PathBuf::from(args.next().ok_or("missing data root")?),
            "--port" => {
                port = args
                    .next()
                    .ok_or("missing port")?
                    .parse()
                    .map_err(|_| "invalid port")?
            }
            "--help" => {
                println!("WiParse-Service --data-root <directory> --port 7879\nLocal authenticated automation API; hardware IO requires approved ICD.");
                return Ok(());
            }
            _ => return Err(format!("unknown option {a}")),
        }
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    // Bind before writing token or recovering ledger so a second instance cannot
    // mutate the active instance's root. A root lock also prevents different ports.
    let server = Server::http(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let _guard = storage::RootLock::acquire(&root)?;
    let token = uuid::Uuid::new_v4().to_string();
    storage::atomic_json(
        &root.join("service-token.json"),
        &json!({"token":token,"port":port}),
    )?;
    let state = Arc::new(Mutex::new(jobs::State::open(root.clone())?));
    println!(
        "WiParse-Service {} listening on 127.0.0.1:{port}; data={}",
        wiparse_core::VERSION,
        root.display()
    );
    for mut request in server.incoming_requests() {
        let auth = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .map(|h| h.value.as_str());
        if auth != Some(format!("Bearer {token}").as_str()) {
            let _ = request
                .respond(Response::from_string("unauthorized").with_status_code(StatusCode(401)));
            continue;
        }
        let mut stop = false;
        let reply = if request.method() == &tiny_http::Method::Post && request.url() == "/v1/stop" {
            if state.lock().map_err(|_| "SERVICE.LOCK")?.idle() {
                stop = true;
                Ok(json!({"stopped":true}))
            } else {
                Err("SERVICE.ACTIVE_JOBS_CANCEL_FIRST".into())
            }
        } else if request.method() == &tiny_http::Method::Get && request.url() == "/v1/health" {
            Ok(
                json!({"version":wiparse_core::VERSION,"schema_version":"wiparse.automation/1","industrial_approved":false,"gui_independent":true,"service":state.lock().map_err(|_|"SERVICE.LOCK")?.health()}),
            )
        } else if request.method() == &tiny_http::Method::Post && request.url() == "/v1/invoke" {
            let mut body = String::new();
            match request
                .as_reader()
                .take(1_048_577)
                .read_to_string(&mut body)
            {
                Ok(_) if body.len() <= 1_048_576 => serde_json::from_str::<Value>(&body)
                    .map_err(|e| e.to_string())
                    .and_then(|v| jobs::invoke(&state, &v)),
                _ => Err("API.INVALID_OR_OVERSIZE_BODY".into()),
            }
        } else {
            Err("API.UNKNOWN_ROUTE".into())
        };
        let value = match reply {
            Ok(data) => json!({"ok":true,"data":data}),
            Err(error) => {
                json!({"ok":false,"error":error,"error_record":{"schema_version":"wiparse.automation/1","error_id":uuid::Uuid::new_v4().to_string(),"code":error.split([' ','(']).next().unwrap_or("SERVICE.ERROR"),"source":"wiparse-service","severity":"error","utc":chrono::Utc::now().to_rfc3339(),"recovery_id":"inspect_job_and_evidence_before_retry","evidence_refs":[]}})
            }
        };
        let response = Response::from_string(value.to_string())
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
        let _ = request.respond(response);
        if stop {
            let _ = fs::remove_file(root.join("service-token.json"));
            break;
        }
    }
    Ok(())
}
