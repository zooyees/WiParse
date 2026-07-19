//! Persistent diagnostics for release GUI (no console window).
//!
//! - `logs/wiparse.log` — tracing (info+)
//! - `logs/crash.log` — panics and fatal eframe errors

use chrono::Local;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;
use wiparse_core::paths::project_path;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn log_dir() -> PathBuf {
    project_path("logs")
}

pub fn app_log_path() -> PathBuf {
    log_dir().join("wiparse.log")
}

pub fn crash_log_path() -> PathBuf {
    log_dir().join("crash.log")
}

/// Create `logs/`, install panic hook, and route tracing to file (+ stderr when present).
pub fn init() {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);

    let app_log = app_log_path();
    rotate_if_huge(&app_log);
    let crash_log = crash_log_path();
    rotate_if_huge(&crash_log);

    install_panic_hook(crash_log.clone());

    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&app_log)
    {
        Ok(f) => f,
        Err(err) => {
            // Fall back to stderr-only if the log file cannot be opened.
            let _ = writeln!(
                io::stderr(),
                "WiParse: failed to open {}: {err}",
                app_log.display()
            );
            tracing_subscriber::fmt()
                .with_env_filter(env_filter())
                .init();
            return;
        }
    };

    let file_writer = SharedWriter(Arc::new(Mutex::new(file)));
    let writer = std::io::stderr.and(file_writer);
    tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_writer(writer)
        .with_ansi(false)
        .init();

    tracing::info!(
        "logging initialized; app_log={} crash_log={}",
        app_log.display(),
        crash_log.display()
    );
}

pub fn append_crash(message: &str) {
    let path = crash_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f %z");
    let _ = writeln!(file, "----- {ts} -----");
    let _ = writeln!(file, "{message}");
    let _ = writeln!(file);
    let _ = file.flush();
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

fn install_panic_hook(crash_log: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload = panic_payload(info);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let body = format!(
            "PANIC at {location}\n{payload}\n\nBacktrace:\n{backtrace}"
        );
        // Write before chaining so we keep a record even if the previous hook aborts.
        append_crash_to(&crash_log, &body);
        let _ = writeln!(io::stderr(), "WiParse panic written to {}", crash_log.display());
        let _ = writeln!(io::stderr(), "{body}");
        previous(info);
    }));
}

fn append_crash_to(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f %z");
    let _ = writeln!(file, "----- {ts} -----");
    let _ = writeln!(file, "{message}");
    let _ = writeln!(file);
    let _ = file.flush();
}

fn panic_payload(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".into()
    }
}

fn rotate_if_huge(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    let bak = path.with_extension("log.1");
    let _ = fs::remove_file(&bak);
    let _ = fs::rename(path, bak);
}

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<File>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?
            .flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
