//! Forward commands to WiParse GUI embedded API (C+E attach mode).

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::Duration;

pub fn default_url() -> String {
    std::env::var("WIPARSE_URL").unwrap_or_else(|_| "http://127.0.0.1:7878".into())
}

pub fn health(url: &str) -> Result<Value, String> {
    get_json(&format!("{}/v1/health", url.trim_end_matches('/')))
}

pub fn capabilities(url: &str) -> Result<Value, String> {
    get_json(&format!("{}/v1/capabilities", url.trim_end_matches('/')))
}

pub fn invoke(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "method": method, "params": params });
    post_json(&format!("{}/v1/invoke", url.trim_end_matches('/')), &body)
}

fn get_json(url: &str) -> Result<Value, String> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("API connect failed ({url}): {e}. Is WiParse.exe running?"))?;
    read_json(resp)
}

fn post_json(url: &str, body: &Value) -> Result<Value, String> {
    let resp = ureq::post(url)
        .timeout(Duration::from_secs(90))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| format!("API invoke failed ({url}): {e}. Is WiParse.exe running?"))?;
    read_json(resp)
}

fn read_json(resp: ureq::Response) -> Result<Value, String> {
    let mut text = String::new();
    resp.into_reader()
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("invalid API JSON: {e}; body={text}"))
}

pub fn stream_events(url: &str, since_seq: u64) -> Result<(), String> {
    let url = format!(
        "{}/v1/events?since_seq={since_seq}",
        url.trim_end_matches('/')
    );
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(24 * 3600))
        .call()
        .map_err(|e| format!("events connect failed: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 4096];
    let mut pending = String::new();
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(pos) = pending.find('\n') {
            let line = pending[..pos].trim_end_matches('\r').to_string();
            pending = pending[pos + 1..].to_string();
            if !line.is_empty() {
                println!("{line}");
                let _ = io_flush();
            }
        }
    }
    Ok(())
}

fn io_flush() -> std::io::Result<()> {
    std::io::stdout().flush()
}
