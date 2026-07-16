use chrono::Local;
use serde::Serialize;
use std::io::{self, Write};

pub struct OutputOptions {
    pub pretty: bool,
    pub quiet: bool,
}

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;

fn now_iso() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn emit_ok(cmd: &str, data: serde_json::Value, opts: &OutputOptions) -> i32 {
    if opts.quiet {
        write_json(&data, opts.pretty, &mut io::stdout());
    } else {
        let payload = serde_json::json!({
            "ok": true,
            "cmd": cmd,
            "ts": now_iso(),
            "data": data,
        });
        write_json(&payload, opts.pretty, &mut io::stdout());
    }
    EXIT_OK
}

pub fn emit_error(cmd: &str, message: &str, opts: &OutputOptions) -> i32 {
    let error_body = serde_json::json!({
        "code": "ERROR",
        "message": message,
    });
    if opts.quiet {
        write_json(&error_body, opts.pretty, &mut io::stderr());
    } else {
        let payload = serde_json::json!({
            "ok": false,
            "cmd": cmd,
            "ts": now_iso(),
            "error": error_body,
        });
        write_json(&payload, opts.pretty, &mut io::stderr());
    }
    EXIT_ERROR
}

pub fn emit_ndjson(value: &impl Serialize, pretty: bool) {
    write_json(value, pretty, &mut io::stdout());
}

fn write_json(value: &impl Serialize, pretty: bool, stream: &mut dyn Write) {
    let text = if pretty {
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
    };
    let _ = writeln!(stream, "{text}");
    let _ = stream.flush();
}
