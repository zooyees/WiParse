//! Serial tool panel — left connect sidebar + `log_file_tabs` (Python `log_panel` parity).

use crate::log_tab::LogTabPage;
use crate::theme::{self as ui_theme, Tokens};
use chrono::{Local, NaiveDateTime, Timelike};
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, tr_fmt, tr_monitoring, Lang};
use wiparse_core::log::{build_file_store_worker, FileBuildEvent, LogStore};
use wiparse_core::paths::project_path;
use wiparse_core::serial::{list_ports, CapturedEvent, SerialSession};

/// One subfolder under the log browser root that contains `.txt` files.
struct LogBrowserFolder {
    name: String,
    files: Vec<(String, PathBuf)>,
}

/// Cap RX UI apply per frame so a post-dialog backlog cannot freeze the UI.
const DRAIN_LINES_PER_FRAME: usize = 400;
/// Backlog cap — drop oldest when exceeded (~200 KB at typical line size).
const MAX_PENDING_LINES: usize = 1_000;

enum SerialEvent {
    Line(String),
    Status(String),
    Error(String),
    Stopped,
}

struct PendingFileLoad {
    tab_idx: usize,
    rx: Receiver<FileBuildEvent>,
}

pub struct SerialToolPanel {
    ports: Vec<String>,
    selected_port: usize,
    baud: String,
    baud_options: Vec<String>,
    baud_custom: bool,
    monitoring: bool,
    live_name: String,
    committed_live_name: String,
    live_dir: String,
    open_dir: String,
    status: String,
    /// Index 0 is always the live tab.
    tabs: Vec<LogTabPage>,
    active_tab: usize,
    stop_flag: Option<Arc<AtomicBool>>,
    rx: Option<Receiver<SerialEvent>>,
    /// Hex/bytes to write while monitoring (API / Agent).
    write_tx: Option<Sender<Vec<u8>>>,
    live_file: Option<PathBuf>,
    live_writer: Option<BufWriter<File>>,
    last_lang: Lang,
    tab_scroll_x: f32,
    tab_wheel_last_switch_at: Option<f64>,
    pending_loads: Vec<PendingFileLoad>,
    /// Leftover RX lines when drain hits per-frame cap.
    pending_lines: VecDeque<String>,
    /// Cached elided tab titles: (source title, width_px) -> elided text.
    tab_elide_cache: HashMap<(String, i32), String>,
    /// Configurable root for the sidebar folder/txt browser.
    browser_dir: String,
    /// Last path that was scanned into `browser_folders`.
    browser_scanned_dir: String,
    browser_folders: Vec<LogBrowserFolder>,
}

impl SerialToolPanel {
    pub fn new(cfg: &AppConfig, lang: Lang) -> Self {
        let ports = list_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.device)
            .collect();
        let baud_options = if cfg.serial.default_baudrates.is_empty() {
            vec!["115200".into(), "1000000".into(), "2000000".into()]
        } else {
            cfg.serial.default_baudrates.clone()
        };
        let baud = baud_options
            .first()
            .cloned()
            .unwrap_or_else(|| "115200".into());
        let baud_custom = !baud_options.iter().any(|b| b == &baud);
        let live_dir = cfg.log_monitor.save_dir.clone();
        let live_name = cfg.log_monitor.default_filename.clone();
        let browser_dir = if cfg.log_monitor.log_browser_dir.trim().is_empty() {
            String::new()
        } else {
            cfg.log_monitor.log_browser_dir.clone()
        };
        let mut tabs = vec![LogTabPage::live_tab(lang)];
        tabs[0].title = live_name.clone();
        let mut panel = Self {
            ports,
            selected_port: 0,
            baud,
            baud_options,
            baud_custom,
            monitoring: false,
            committed_live_name: live_name.clone(),
            live_name,
            live_dir,
            open_dir: cfg.log_monitor.last_open_dir.clone(),
            status: tr(lang, "status.ready"),
            tabs,
            active_tab: 0,
            stop_flag: None,
            rx: None,
            write_tx: None,
            live_file: None,
            live_writer: None,
            last_lang: lang,
            tab_scroll_x: 0.0,
            tab_wheel_last_switch_at: None,
            pending_loads: Vec::new(),
            pending_lines: VecDeque::new(),
            tab_elide_cache: HashMap::new(),
            browser_dir,
            browser_scanned_dir: String::new(),
            browser_folders: Vec::new(),
        };
        panel.restore_open_log_files(&cfg.log_monitor.open_log_files);
        panel.refresh_log_browser();
        panel
    }

    fn collect_open_log_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            if tab.live {
                continue;
            }
            if let Some(p) = &tab.filepath {
                let norm = normalize_path(p);
                if !out.iter().any(|x| x == &norm) {
                    out.push(norm);
                }
            }
        }
        out
    }

    fn persist_open_log_files(&self) {
        let paths = self.collect_open_log_paths();
        match load_config() {
            Ok(mut cfg) => {
                cfg.log_monitor.default_filename = self.committed_live_name.clone();
                cfg.log_monitor.save_dir = self.live_dir.clone();
                cfg.log_monitor.open_log_files = paths;
                cfg.log_monitor.last_open_dir = self.open_dir.clone();
                cfg.log_monitor.log_browser_dir = self.browser_dir.clone();
                let _ = save_config(&cfg);
            }
            Err(_) => {
                let mut cfg = AppConfig::default();
                cfg.log_monitor.default_filename = self.committed_live_name.clone();
                cfg.log_monitor.save_dir = self.live_dir.clone();
                cfg.log_monitor.open_log_files = paths;
                cfg.log_monitor.last_open_dir = self.open_dir.clone();
                cfg.log_monitor.log_browser_dir = self.browser_dir.clone();
                let _ = save_config(&cfg);
            }
        }
    }

    fn restore_open_log_files(&mut self, paths: &[String]) {
        for raw in paths {
            let path = PathBuf::from(raw);
            if path.is_file() {
                let _ = self.open_path(path);
            }
        }
    }

    /// Returns the tab index for `path`, reusing an existing tab when the same
    /// file is already open.
    fn open_path(&mut self, path: PathBuf) -> Option<usize> {
        if !path.is_file() {
            return None;
        }
        let path_s = normalize_path(&path.to_string_lossy());
        if let Some(tab_idx) = self.tabs.iter().position(|tab| {
            tab.filepath
                .as_deref()
                .is_some_and(|open_path| same_log_path(open_path, &path_s))
        }) {
            return Some(tab_idx);
        }
        let tab = LogTabPage::file_tab_empty(path_s.clone());
        self.tabs.push(tab);
        let tab_idx = self.tabs.len() - 1;
        let (tx, rx) = unbounded();
        self.pending_loads.push(PendingFileLoad { tab_idx, rx });
        self.status = format!("loading {path_s}…");
        thread::spawn(move || build_file_store_worker(path, tx));
        Some(tab_idx)
    }

    fn refresh_ports(&mut self, lang: Lang) {
        self.ports = list_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.device)
            .collect();
        if self.selected_port >= self.ports.len() {
            self.selected_port = 0;
        }
        self.status = if self.monitoring {
            let port = self
                .ports
                .get(self.selected_port)
                .map(|s| s.as_str())
                .unwrap_or("?");
            let baud: u32 = self.baud.parse().unwrap_or(0);
            tr_monitoring(lang, port, baud)
        } else if self.ports.is_empty() {
            tr(lang, "status.port_none")
        } else {
            tr_fmt(lang, "status.ports_count", self.ports.len())
        };
    }

    fn live_path(&self) -> PathBuf {
        let dir = if self.live_dir.is_empty() {
            PathBuf::from("log")
        } else {
            PathBuf::from(&self.live_dir)
        };
        let name = if self.live_name.is_empty() {
            "Live Packet Log".into()
        } else {
            self.live_name.clone()
        };
        dir.join(format!("{name}.txt"))
    }

    fn commit_live_name(&mut self, lang: Lang) -> bool {
        let name = match normalized_log_name(&self.live_name) {
            Ok(name) => name,
            Err(()) => {
                self.live_name = self.committed_live_name.clone();
                self.status = tr(lang, "status.rename_invalid");
                return false;
            }
        };
        if name == self.committed_live_name {
            self.live_name = name;
            return true;
        }

        let old_name = self.committed_live_name.clone();
        let old_path = self.live_file.clone().or_else(|| {
            // Prefer the path that matches the committed name (before UI edit).
            let dir = if self.live_dir.is_empty() {
                PathBuf::from("log")
            } else {
                PathBuf::from(&self.live_dir)
            };
            let p = dir.join(format!("{old_name}.txt"));
            p.exists().then_some(p)
        });
        self.live_name = name;
        let new_path = self.live_path();

        if new_path.exists() {
            // Don't overwrite an existing log; revert name.
            self.live_name = old_name;
            self.status = tr(lang, "status.rename_exists");
            return false;
        }

        let empty = live_capture_is_empty(
            self.tabs.first().map(|t| t.line_count()).unwrap_or(0),
            old_path.as_deref(),
        );

        if empty {
            // No realtime content: rename the current (empty/BOM) file in place.
            if let Some(old_path) = old_path {
                if old_path != new_path && old_path.exists() {
                    self.close_live_writer();
                    if let Err(err) = rename_log_path(&old_path, &new_path) {
                        self.live_name = old_name;
                        self.status = if err.kind() == std::io::ErrorKind::AlreadyExists {
                            tr(lang, "status.rename_exists")
                        } else {
                            format!("{}: {err}", tr(lang, "status.rename_failed"))
                        };
                        return false;
                    }
                }
            }
            self.live_file = Some(new_path);
            self.live_writer = None;
            // Reopen writer if monitoring so subsequent lines keep flowing.
            if self.monitoring {
                if let Err(e) = self.ensure_live_file() {
                    self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
                    return false;
                }
            }
            self.committed_live_name = self.live_name.clone();
            if let Some(live) = self.tabs.get_mut(0) {
                live.title = self.committed_live_name.clone();
            }
            self.persist_live_log_name();
            self.status = format!(
                "{}: {}",
                tr(lang, "status.rename_ok"),
                self.committed_live_name
            );
            true
        } else {
            // Has data: keep the old file under the old name, start a fresh live file.
            self.close_live_writer();
            // old_path file is left as-is (already flushed).
            self.live_file = None;
            self.live_writer = None;
            self.pending_lines.clear();
            self.clear_live_display();
            if let Err(e) = self.ensure_live_file() {
                // Roll back name so UI matches the file that still holds data.
                self.live_name = old_name.clone();
                self.live_file = old_path;
                self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
                return false;
            }
            self.committed_live_name = self.live_name.clone();
            if let Some(live) = self.tabs.get_mut(0) {
                live.title = self.committed_live_name.clone();
            }
            self.persist_live_log_name();
            self.status = format!(
                "{}: {} → {}",
                tr(lang, "status.rename_split_ok"),
                old_name,
                self.committed_live_name
            );
            true
        }
    }

    fn persist_live_log_name(&self) {
        self.persist_open_log_files();
    }

    fn flush_live_writer(&mut self) {
        if let Some(w) = &mut self.live_writer {
            let _ = w.flush();
        }
    }

    fn close_live_writer(&mut self) {
        if let Some(mut w) = self.live_writer.take() {
            let _ = w.flush();
        }
    }

    fn ensure_live_file(&mut self) -> std::io::Result<()> {
        let path = self.live_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            // UTF-8 BOM for Notepad / Chinese-friendly parity with Python utf-8-sig
            fs::write(&path, b"\xEF\xBB\xBF")?;
        }
        let need_reopen = match (&self.live_writer, &self.live_file) {
            (None, _) => true,
            (Some(_), Some(old)) if old != &path => true,
            (Some(_), None) => true,
            (Some(_), Some(_)) => false,
        };
        if need_reopen {
            self.close_live_writer();
            self.live_writer = Some(open_live_writer(&path)?);
        }
        self.live_file = Some(path);
        Ok(())
    }

    fn append_to_live_file(&mut self, line: &str) {
        if self.live_writer.is_none() {
            if self.ensure_live_file().is_err() {
                return;
            }
        }
        if let Some(w) = &mut self.live_writer {
            let _ = writeln!(w, "{line}");
        }
    }

    fn start_monitor(&mut self, lang: Lang) {
        let _ = self.api_start_monitor(lang, None, None);
    }

    /// Start monitor; optional port/baud override for API.
    pub fn api_start_monitor(
        &mut self,
        lang: Lang,
        port_override: Option<String>,
        baud_override: Option<u32>,
    ) -> Result<(String, u32), String> {
        if self.monitoring {
            let port = self
                .ports
                .get(self.selected_port)
                .cloned()
                .unwrap_or_default();
            let baud: u32 = self.baud.parse().unwrap_or(0);
            return Ok((port, baud));
        }
        if self.live_name.trim() != self.committed_live_name && !self.commit_live_name(lang) {
            return Err(tr(lang, "status.create_failed").into());
        }
        if let Some(ref p) = port_override {
            if let Some(idx) = self.ports.iter().position(|x| x == p) {
                self.selected_port = idx;
            } else {
                self.ports.insert(0, p.clone());
                self.selected_port = 0;
            }
        }
        if let Some(b) = baud_override {
            self.baud = b.to_string();
            self.baud_custom = !self.baud_options.iter().any(|x| x == &self.baud);
        }
        let Some(port) = self.ports.get(self.selected_port).cloned() else {
            self.status = tr(lang, "status.port_none");
            return Err(self.status.clone());
        };
        let baud: u32 = match self.baud.parse() {
            Ok(v) if v > 0 => v,
            _ => {
                self.status = tr(lang, "status.baud_invalid");
                return Err(self.status.clone());
            }
        };
        if let Err(e) = self.ensure_live_file() {
            self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
            return Err(self.status.clone());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded();
        let (write_tx, write_rx) = unbounded();
        let stop_c = Arc::clone(&stop);
        let port_for_thread = port.clone();
        thread::spawn(move || serial_worker(port_for_thread, baud, stop_c, tx, write_rx));
        self.stop_flag = Some(stop);
        self.rx = Some(rx);
        self.write_tx = Some(write_tx);
        self.monitoring = true;
        self.status = tr_monitoring(lang, &port, baud);
        if let Some(live) = self.tabs.get_mut(0) {
            live.title = self.live_name.clone();
        }
        self.active_tab = 0;
        Ok((port, baud))
    }

    pub fn api_stop_monitor(&mut self, lang: Lang) {
        self.stop_monitor(lang);
    }

    pub fn take_write_sender(&mut self) -> Option<Sender<Vec<u8>>> {
        self.write_tx.clone()
    }

    pub fn api_tabs_list(&self) -> serde_json::Value {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(id, tab)| {
                serde_json::json!({
                    "tab_id": id,
                    "title": tab.title,
                    "live": tab.live,
                    "path": tab.filepath.clone(),
                    "lines": tab.line_count(),
                })
            })
            .collect();
        serde_json::json!({ "tabs": tabs, "active_tab": self.active_tab })
    }

    pub fn api_lines_get(&self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let tab_id = params.get("tab_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let from = params.get("from_row").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;
        let Some(tab) = self.tabs.get(tab_id) else {
            return err("log.lines.get", "tab not found");
        };
        let lines = tab.lines_slice(from, limit);
        ok(
            "log.lines.get",
            serde_json::json!({
                "tab_id": tab_id,
                "from_row": from,
                "lines": lines,
                "count": lines.len(),
            }),
        )
    }

    pub fn api_recent_live_lines(&self, limit: usize) -> Vec<String> {
        self.tabs
            .first()
            .map(|t| t.recent_lines(limit))
            .unwrap_or_default()
    }

    fn stop_monitor(&mut self, lang: Lang) {
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
        self.rx = None;
        self.write_tx = None;
        self.monitoring = false;
        self.close_live_writer();
        self.status = tr(lang, "status.stopped");
    }

    fn new_live_log(&mut self, lang: Lang) {
        // Flush current live to disk, then push it right as a file tab and
        // insert a fresh live tab at index 0 (newest live stays leftmost).
        if let Err(e) = self.ensure_live_file() {
            self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
            return;
        }
        self.close_live_writer();
        let Some(archived_path) = self.live_file.take() else {
            self.status = tr(lang, "status.create_failed").into();
            return;
        };
        self.live_writer = None;

        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        self.live_name = format!("{} {}", tr(lang, "log.default_filename"), stamp);
        self.committed_live_name = self.live_name.clone();

        let mut new_live = LogTabPage::live_tab(lang);
        new_live.title = self.live_name.clone();
        self.tabs.insert(0, new_live);
        for load in &mut self.pending_loads {
            load.tab_idx += 1;
        }

        // Previous live is now at index 1 — reopen it as a closable file tab.
        let path_s = normalize_path(&archived_path.to_string_lossy());
        let archived_tab = LogTabPage::file_tab_empty(path_s);
        if self.tabs.len() > 1 {
            self.tabs[1] = archived_tab;
        } else {
            self.tabs.push(archived_tab);
        }
        let (tx, rx) = unbounded();
        self.pending_loads.push(PendingFileLoad { tab_idx: 1, rx });
        let path_for_worker = archived_path;
        thread::spawn(move || build_file_store_worker(path_for_worker, tx));

        self.live_file = None;
        if let Err(e) = self.ensure_live_file() {
            self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
            return;
        }
        self.active_tab = 0;
        self.persist_open_log_files();
        self.status = format!("new live log: {}", self.live_name);
    }

    fn clear_live_display(&mut self) {
        if let Some(live) = self.tabs.get_mut(0) {
            live.clear_display();
        }
    }

    fn open_files(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("Log", &["txt", "log"]);
        if !self.open_dir.is_empty() {
            let remembered = PathBuf::from(&self.open_dir);
            if remembered.is_dir() {
                dialog = dialog.set_directory(remembered);
            }
        }
        let files = dialog.pick_files();
        let Some(files) = files else {
            return;
        };
        if let Some(parent) = files.first().and_then(|path| path.parent()) {
            self.open_dir = parent.to_string_lossy().into_owned();
        }
        for path in files {
            if let Some(tab_idx) = self.open_path(path) {
                self.activate_tab(tab_idx);
            }
        }
        self.persist_open_log_files();
    }

    /// Open dropped `.txt` / `.log` files (same filters as the Open Log dialog).
    pub fn handle_dropped_files(&mut self, files: &[egui::DroppedFile]) {
        let mut opened_any = false;
        for file in files {
            let Some(path) = file.path.as_ref() else {
                continue;
            };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "txt" && ext != "log" {
                continue;
            }
            if let Some(parent) = path.parent() {
                self.open_dir = parent.to_string_lossy().into_owned();
            }
            if let Some(tab_idx) = self.open_path(path.clone()) {
                self.activate_tab(tab_idx);
                opened_any = true;
            }
        }
        if opened_any {
            self.persist_open_log_files();
        }
    }

    fn browse_dir(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        if !self.live_dir.is_empty() {
            let remembered = PathBuf::from(&self.live_dir);
            if remembered.is_dir() {
                dialog = dialog.set_directory(remembered);
            }
        }
        if let Some(dir) = dialog.pick_folder() {
            self.live_dir = dir.to_string_lossy().into_owned();
            self.persist_open_log_files();
        }
    }

    fn browse_log_browser_dir(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        let start = if !self.browser_dir.trim().is_empty() {
            project_path(&self.browser_dir)
        } else if !self.live_dir.trim().is_empty() {
            project_path(&self.live_dir)
        } else {
            PathBuf::new()
        };
        if start.is_dir() {
            dialog = dialog.set_directory(start);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.browser_dir = dir.to_string_lossy().into_owned();
            self.refresh_log_browser();
            self.persist_open_log_files();
        }
    }

    fn resolve_browser_root(&self) -> Option<PathBuf> {
        let raw = self.browser_dir.trim();
        if raw.is_empty() {
            return None;
        }
        let root = project_path(raw);
        root.is_dir().then_some(root)
    }

    /// Scan `browser_dir` for *immediate* subfolders that contain `.txt` files.
    /// Nested subfolders are ignored (first level only).
    fn refresh_log_browser(&mut self) {
        let scanned_key = self.browser_dir.trim().to_owned();
        self.browser_scanned_dir = scanned_key;
        self.browser_folders.clear();
        let Some(root) = self.resolve_browser_root() else {
            return;
        };
        let Ok(entries) = fs::read_dir(&root) else {
            return;
        };
        let mut folders = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(&path) else {
                continue;
            };
            let mut txts = Vec::new();
            for file in files.flatten() {
                let file_path = file.path();
                // First level only: skip nested directories entirely.
                if !file_path.is_file() {
                    continue;
                }
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext != "txt" {
                    continue;
                }
                let name = file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file_path.display().to_string());
                txts.push((name, file_path));
            }
            if txts.is_empty() {
                continue;
            }
            txts.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            folders.push(LogBrowserFolder { name, files: txts });
        }
        folders.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        self.browser_folders = folders;
    }

    fn ensure_log_browser_fresh(&mut self) {
        if self.browser_dir.trim() != self.browser_scanned_dir.trim() {
            self.refresh_log_browser();
        }
    }

    fn open_browser_file(&mut self, path: PathBuf) {
        if let Some(parent) = path.parent() {
            self.open_dir = parent.to_string_lossy().into_owned();
        }
        if let Some(tab_idx) = self.open_path(path) {
            self.activate_tab(tab_idx);
            self.persist_open_log_files();
        }
    }

    fn close_tab(&mut self, idx: usize) {
        if idx == 0 {
            return; // never close live
        }
        if idx < self.tabs.len() {
            self.tabs.remove(idx);
            self.pending_loads.retain_mut(|load| {
                if load.tab_idx == idx {
                    return false;
                }
                if load.tab_idx > idx {
                    load.tab_idx -= 1;
                }
                true
            });
            if self.active_tab >= self.tabs.len() {
                self.active_tab = self.tabs.len().saturating_sub(1);
            } else if self.active_tab > idx {
                self.active_tab -= 1;
            }
            self.persist_open_log_files();
        }
    }

    fn poll_file_loads(&mut self) {
        let mut done = Vec::new();
        for (i, load) in self.pending_loads.iter().enumerate() {
            let mut finished = false;
            loop {
                match load.rx.try_recv() {
                    Ok(FileBuildEvent::Progress { lines }) => {
                        if let Some(tab) = self.tabs.get_mut(load.tab_idx) {
                            tab.set_index_progress(lines);
                            self.status = format!("indexing {}… {lines} lines", tab.title);
                        }
                    }
                    Ok(FileBuildEvent::Done(store)) => {
                        if let Some(tab) = self.tabs.get_mut(load.tab_idx) {
                            let total = store.line_count();
                            tab.set_file_store(store);
                            self.status = format!("opened {} ({total} lines)", tab.title);
                        }
                        finished = true;
                        break;
                    }
                    Ok(FileBuildEvent::Err(e)) => {
                        self.status = format!("open failed: {e}");
                        if let Some(tab) = self.tabs.get_mut(load.tab_idx) {
                            tab.finish_loading();
                        }
                        finished = true;
                        break;
                    }
                    Err(_) => break,
                }
            }
            if finished {
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            self.pending_loads.remove(i);
        }
    }

    pub fn drain_events(&mut self, update_live_filters: bool) {
        self.drain_events_with_bus(update_live_filters, None);
    }

    pub fn drain_events_with_bus(
        &mut self,
        update_live_filters: bool,
        api: Option<&crate::backend::ApiBridge>,
    ) {
        for tab in &mut self.tabs {
            tab.poll_background_tasks();
        }
        self.poll_file_loads();

        let mut budget = DRAIN_LINES_PER_FRAME;
        let mut batch = Vec::new();

        while budget > 0 {
            let Some(line) = self.pending_lines.pop_front() else {
                break;
            };
            batch.push(line);
            budget -= 1;
        }

        let mut extras = Vec::new();
        let mut stop_seen = false;
        if let Some(rx) = &self.rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    SerialEvent::Line(line) => {
                        if budget > 0 {
                            batch.push(line);
                            budget -= 1;
                        } else {
                            extras.push(line);
                        }
                    }
                    SerialEvent::Status(s) => {
                        if let Some(api) = api {
                            api.publish_serial_status(&s);
                        }
                        self.status = s;
                    }
                    SerialEvent::Error(e) => {
                        if let Some(api) = api {
                            api.publish_serial_status(&format!("error: {e}"));
                        }
                        self.status = e.clone();
                        if budget > 0 {
                            batch.push(format!("[ERR] {e}"));
                            budget -= 1;
                        } else {
                            extras.push(format!("[ERR] {e}"));
                        }
                    }
                    SerialEvent::Stopped => {
                        stop_seen = true;
                        break;
                    }
                }
            }
        }
        self.pending_lines.extend(extras);
        while self.pending_lines.len() > MAX_PENDING_LINES {
            self.pending_lines.pop_front();
        }

        if stop_seen {
            self.monitoring = false;
            self.stop_flag = None;
            self.rx = None;
            self.write_tx = None;
            self.close_live_writer();
            self.status = tr(self.last_lang, "status.stopped");
            if let Some(api) = api {
                api.set_monitoring(false, None, None);
                *api.serial_write.lock().unwrap_or_else(|e| e.into_inner()) = None;
                api.publish_serial_status("stopped");
            }
        }

        if !batch.is_empty() {
            for line in &batch {
                self.append_to_live_file(line);
                if let Some(api) = api {
                    api.publish_serial_line(line);
                }
            }
            self.flush_live_writer();
            if let Some(live) = self.tabs.get_mut(0) {
                live.append_lines(batch, update_live_filters);
            }
        }
    }

    pub fn needs_repaint(&self) -> bool {
        self.monitoring
            || !self.pending_loads.is_empty()
            || !self.pending_lines.is_empty()
            || self.tabs.iter().any(LogTabPage::has_background_filter)
    }

    /// Background file load / RX backlog — repaint even when another main tab is visible.
    pub fn has_background_io(&self) -> bool {
        !self.pending_loads.is_empty()
            || !self.pending_lines.is_empty()
            || self.tabs.iter().any(LogTabPage::has_background_filter)
    }

    pub fn is_monitoring(&self) -> bool {
        self.monitoring
    }

    pub fn active_live_visible(&self) -> bool {
        self.active_tab == 0
    }

    /// Call when the serial tool (and live tab) become visible again so deferred
    /// filter updates from background RX are applied once.
    pub fn sync_visible_live_filters(&mut self) {
        if self.active_tab == 0 {
            if let Some(live) = self.tabs.get_mut(0) {
                live.on_activated();
            }
        }
    }

    fn activate_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
            if let Some(tab) = self.tabs.get_mut(idx) {
                tab.on_activated();
            }
        }
    }

    fn close_tabs_left_of(&mut self, idx: usize) {
        if idx <= 1 {
            return;
        }
        self.tabs.drain(1..idx);
        self.pending_loads.retain_mut(|load| {
            if load.tab_idx > 0 && load.tab_idx < idx {
                return false;
            }
            if load.tab_idx >= idx {
                load.tab_idx -= idx - 1;
            }
            true
        });
        if self.active_tab >= idx {
            self.active_tab = self.active_tab.saturating_sub(idx - 1);
        }
        self.persist_open_log_files();
    }

    fn close_tabs_right_of(&mut self, idx: usize) {
        if idx + 1 >= self.tabs.len() {
            return;
        }
        self.tabs.truncate(idx + 1);
        self.pending_loads.retain_mut(|load| load.tab_idx <= idx);
        if self.active_tab > idx {
            self.active_tab = idx;
        }
        self.persist_open_log_files();
    }

    fn close_all_file_tabs(&mut self) {
        if self.tabs.len() > 1 {
            self.tabs.truncate(1);
        }
        self.pending_loads.clear();
        self.active_tab = 0;
        self.persist_open_log_files();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        self.last_lang = lang;
        // Events are drained from app::update so backlog clears even when this tab is hidden.
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.handle_dropped_files(&dropped);
        }

        // Python log_panel: fixed left sidebar + content (must force top-down
        // layouts — parent horizontal_top would otherwise stack every control in a row).
        // Fixed sidebar (+25% vs original 172). Cards leave 1px inset so Inside strokes show.
        let sidebar_w = 172.0 * 1.25; // 215
        let avail_h = ui.available_height();
        let avail_w = ui.available_width();
        const SIDE_MAIN_GAP: f32 = 4.0;

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = SIDE_MAIN_GAP;
            // ── Left sidebar: serial controls + log browser (two groups) ──
            // Exact allocation so expanding the tree cannot push the main pane right.
            let (side_rect, _) =
                ui.allocate_exact_size(egui::vec2(sidebar_w, avail_h), egui::Sense::hover());
            // 1px left/right padding so card borders are never clipped by the allocate edge.
            const SIDE_PAD: f32 = 1.0;
            let side_inner = egui::Rect::from_min_max(
                egui::pos2(side_rect.min.x + SIDE_PAD, side_rect.min.y),
                egui::pos2(side_rect.max.x - SIDE_PAD, side_rect.max.y),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(side_inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    let card_w = side_inner.width();
                    ui.set_min_size(side_inner.size());
                    ui.set_max_size(side_inner.size());
                    ui.set_clip_rect(side_rect.intersect(ui.clip_rect()));
                    const CARD_MARGIN_X: i8 = 8;
                    let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(1.0);

                    // Group 1: serial port controls — fill + Inside stroke (no Frame stroke clip).
                    ui.set_width(card_w);
                    let g1 = Frame::NONE
                        .fill(t.surface_bg)
                        .inner_margin(Margin::symmetric(CARD_MARGIN_X, 6))
                        .corner_radius(CornerRadius::same(6))
                        .show(ui, |ui| {
                            ui.set_width(inner_w);
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                            let ctrl_w = inner_w;

                            let port_text = self
                                .ports
                                .get(self.selected_port)
                                .cloned()
                                .unwrap_or_else(|| tr(lang, "status.port_none"));
                            egui::ComboBox::from_id_salt("serial_port")
                                .width(ctrl_w)
                                .selected_text(port_text)
                                .show_ui(ui, |ui| {
                                    if self.ports.is_empty() {
                                        ui.label(tr(lang, "status.port_none"));
                                    }
                                    for (i, p) in self.ports.iter().enumerate() {
                                        ui.selectable_value(&mut self.selected_port, i, p);
                                    }
                                    ui.separator();
                                    if ui.button(tr(lang, "btn.refresh_ports")).clicked() {
                                        self.refresh_ports(lang);
                                    }
                                });

                            let baud_display = if self.baud_custom {
                                format!(
                                    "{} ({})",
                                    tr(lang, "serial.baud_custom"),
                                    if self.baud.is_empty() {
                                        "-"
                                    } else {
                                        &self.baud
                                    }
                                )
                            } else {
                                self.baud.clone()
                            };
                            egui::ComboBox::from_id_salt("serial_baud")
                                .width(ctrl_w)
                                .selected_text(baud_display)
                                .show_ui(ui, |ui| {
                                    for b in self.baud_options.clone() {
                                        if ui
                                            .selectable_label(
                                                !self.baud_custom && self.baud == b,
                                                &b,
                                            )
                                            .clicked()
                                        {
                                            self.baud = b;
                                            self.baud_custom = false;
                                        }
                                    }
                                    ui.separator();
                                    if ui
                                        .selectable_label(
                                            self.baud_custom,
                                            tr(lang, "serial.baud_custom"),
                                        )
                                        .clicked()
                                    {
                                        self.baud_custom = true;
                                        if self.baud_options.iter().any(|b| b == &self.baud) {
                                            self.baud.clear();
                                        }
                                    }
                                });
                            if self.baud_custom {
                                let baud_edit = ui.add(
                                    egui::TextEdit::singleline(&mut self.baud)
                                        .desired_width(ctrl_w)
                                        .hint_text(tr(lang, "serial.baud_custom_hint"))
                                        .margin(egui::vec2(6.0, 4.0)),
                                );
                                if baud_edit.changed() {
                                    self.baud = self
                                        .baud
                                        .chars()
                                        .filter(|c| c.is_ascii_digit())
                                        .collect();
                                }
                            }

                            if self.monitoring {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(tr(lang, "status.live"))
                                            .size(11.0)
                                            .strong()
                                            .color(t.accent_text),
                                    );
                                });
                                ui.add_space(4.0);
                            }

                            let start_resp = if self.monitoring {
                                ui_theme::stop_button(ui, t, tr(lang, "btn.stop"))
                            } else {
                                ui_theme::accent_button(ui, t, tr(lang, "btn.start"))
                            };
                            if start_resp.clicked() {
                                if self.monitoring {
                                    self.stop_monitor(lang);
                                } else {
                                    self.start_monitor(lang);
                                }
                            }

                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.new")).clicked() {
                                self.new_live_log(lang);
                            }
                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.clear")).clicked() {
                                self.clear_live_display();
                            }

                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(tr(lang, "log.filename"))
                                    .size(12.0)
                                    .color(t.text_muted),
                            );
                            let name_edit = ui.add(
                                egui::TextEdit::singleline(&mut self.live_name)
                                    .desired_width(ctrl_w)
                                    .margin(egui::vec2(6.0, 4.0)),
                            );
                            let enter_pressed = ui.input(|input| {
                                input.events.iter().any(|event| {
                                    matches!(
                                        event,
                                        egui::Event::Key {
                                            key: egui::Key::Enter,
                                            pressed: true,
                                            ..
                                        }
                                    )
                                })
                            });
                            if (name_edit.has_focus() || name_edit.lost_focus()) && enter_pressed {
                                self.commit_live_name(lang);
                            }

                            ui.label(
                                egui::RichText::new(tr(lang, "log.save_dir"))
                                    .size(12.0)
                                    .color(t.text_muted),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.live_dir)
                                    .desired_width(ctrl_w)
                                    .margin(egui::vec2(6.0, 4.0)),
                            );

                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.browse_dir"))
                                .clicked()
                            {
                                self.browse_dir();
                            }
                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.open_log"))
                                .clicked()
                            {
                                self.open_files();
                            }
                        });
                    paint_sidebar_card_border(ui, g1.response.rect, card_w, t.border);

                    ui.add_space(8.0);

                    // Group 2: log directory browser — same fixed width as Group 1.
                    let browser_h = ui.available_height().max(120.0);
                    let browser_rect = egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(card_w, browser_h),
                    );
                    ui.advance_cursor_after_rect(browser_rect);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(browser_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                        |ui| {
                            ui.set_min_size(browser_rect.size());
                            ui.set_max_size(browser_rect.size());
                            ui.set_clip_rect(browser_rect.intersect(ui.clip_rect()));
                            ui.set_width(card_w);
                            // Fill first; border painted after so it sits above fill and isn't clipped.
                            ui.painter().rect_filled(
                                browser_rect,
                                CornerRadius::same(6),
                                t.surface_bg,
                            );
                            ui.painter().rect_stroke(
                                browser_rect,
                                CornerRadius::same(6),
                                Stroke::new(1.0_f32, t.border),
                                egui::StrokeKind::Inside,
                            );
                            let content = browser_rect.shrink2(egui::vec2(
                                1.0 + f32::from(CARD_MARGIN_X),
                                1.0 + 6.0,
                            ));
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(content)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                                |ui| {
                                    ui.set_width(content.width());
                                    ui.set_max_width(content.width());
                                    ui.set_min_height(content.height());
                                    ui.set_clip_rect(content.intersect(ui.clip_rect()));
                                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                                    let ctrl_w = content.width().max(1.0);
                                    let inner_w = ctrl_w;

                                    ui.label(
                                        egui::RichText::new(tr(lang, "log.browser_dir"))
                                            .size(12.0)
                                            .strong()
                                            .color(t.text_primary),
                                    );
                                    let browser_edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.browser_dir)
                                            .desired_width(ctrl_w)
                                            .hint_text(tr(lang, "log.browser_hint"))
                                            .margin(egui::vec2(6.0, 4.0)),
                                    );
                                    if browser_edit.lost_focus() {
                                        self.refresh_log_browser();
                                        self.persist_open_log_files();
                                    }

                                    if ui_theme::secondary_button(
                                        ui,
                                        t,
                                        tr(lang, "btn.browse_dir"),
                                    )
                                    .clicked()
                                    {
                                        self.browse_log_browser_dir();
                                    }
                                    if ui_theme::secondary_button(
                                        ui,
                                        t,
                                        tr(lang, "btn.refresh_browser"),
                                    )
                                    .clicked()
                                    {
                                        self.refresh_log_browser();
                                    }

                                    self.ensure_log_browser_fresh();

                                    if self.browser_dir.trim().is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(tr(lang, "log.browser_hint"))
                                                    .size(11.0)
                                                    .color(t.text_muted),
                                            )
                                            .wrap(),
                                        );
                                    } else if self.browser_folders.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(tr(lang, "log.browser_empty"))
                                                    .size(11.0)
                                                    .color(t.text_muted),
                                            )
                                            .wrap(),
                                        );
                                    } else {
                                        let list_h = ui.available_height().max(72.0);
                                        let list_w = ctrl_w;
                                        let mut open_path: Option<PathBuf> = None;
                                        egui::ScrollArea::new([false, true]) // vertical only
                                            .id_salt("log_browser_tree")
                                            .max_height(list_h)
                                            .max_width(list_w)
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                ui.set_width(list_w);
                                                ui.set_max_width(list_w);
                                                ui.spacing_mut().item_spacing.y = 2.0;
                                                for folder in &self.browser_folders {
                                                    let header = format!(
                                                        "{}  ({})",
                                                        folder.name,
                                                        folder.files.len()
                                                    );
                                                    egui::CollapsingHeader::new(
                                                        egui::RichText::new(header)
                                                            .size(12.0)
                                                            .strong(),
                                                    )
                                                    .id_salt((
                                                        "log-browser-folder",
                                                        folder.name.as_str(),
                                                    ))
                                                    .default_open(false)
                                                    .show(ui, |ui| {
                                                        // Indent leaves less width — use available.
                                                        let row_w =
                                                            ui.available_width().min(list_w).max(1.0);
                                                        ui.set_width(row_w);
                                                        ui.set_max_width(row_w);
                                                        ui.set_clip_rect(
                                                            ui.max_rect().intersect(ui.clip_rect()),
                                                        );
                                                        ui.spacing_mut().item_spacing.y = 1.0;
                                                        for (name, path) in &folder.files {
                                                            let resp = log_browser_file_row(
                                                                ui,
                                                                name,
                                                                row_w,
                                                                t.text_primary,
                                                            );
                                                            if resp.clicked() {
                                                                open_path = Some(path.clone());
                                                            }
                                                        }
                                                    });
                                                }
                                            });
                                        if let Some(path) = open_path {
                                            self.open_browser_file(path);
                                        }
                                    }
                                },
                            ); // end browser content
                        },
                    ); // end browser card scope
                },
            ); // end sidebar scope_builder

            // ── Right: file tabs + LogTabPage ─────────────────────────
            // Gap already applied via item_spacing (SIDE_MAIN_GAP).
            let content_w = (avail_w - sidebar_w - SIDE_MAIN_GAP).max(200.0);
            ui.allocate_ui_with_layout(
                egui::vec2(content_w, avail_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    Frame::NONE
                        .fill(t.header_bg)
                        .inner_margin(Margin::symmetric(10, 6))
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, t.border))
                        .show(ui, |ui| {
                            ui.set_min_height(avail_h - 4.0);
                            ui.set_width(ui.available_width());

                            // Tab strip: titles scroll in a fixed-width lane; ◀▶ pinned
                            // to the panel's far right (not after the last tab).
                            #[derive(Clone, Copy)]
                            enum TabMenuAction {
                                CloseCurrent(usize),
                                CloseLeft(usize),
                                CloseRight(usize),
                                CloseAll,
                            }
                            let mut close_idx = None;
                            let mut activate_idx = None;
                            let mut wheel_tab_direction = 0i8;
                            let mut wheel_event_time = 0.0;
                            let mut menu_action: Option<TabMenuAction> = None;
                            // Match LogTabPage toolbar row (checkbox / filter) height & type size.
                            const TAB_ROW_H: f32 = 28.0;
                            const TAB_CTRL_H: f32 = 24.0;
                            const TAB_FONT: f32 = 12.5;
                            const NAV_W: f32 = 22.0;
                            const TAB_TITLE_W: f32 = 132.0;
                            const TAB_CLOSE_W: f32 = 22.0;
                            let row_w = ui.available_width();
                            let tab_stride = TAB_TITLE_W + TAB_CLOSE_W + 4.0;
                            let tab_lane_w = (row_w - NAV_W * 2.0 - 8.0).max(80.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(row_w, TAB_ROW_H),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                ui.set_min_height(TAB_ROW_H);
                                ui.set_max_height(TAB_ROW_H);
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let scroll_step = 180.0_f32;

                                // Draw ▶ then ◀ (right-to-left → appear as ◀▶ on the right).
                                let right_clicked = ui
                                    .add_sized(
                                        [NAV_W, TAB_CTRL_H],
                                        egui::Button::new(
                                            egui::RichText::new("▶")
                                                .size(11.0)
                                                .color(t.text_primary),
                                        )
                                        .fill(t.button_bg)
                                        .stroke(Stroke::new(1.0_f32, t.border))
                                        .corner_radius(CornerRadius::same(3)),
                                    )
                                    .on_hover_text(tr(lang, "log.tab.scroll_right"))
                                    .clicked();
                                let left_clicked = ui
                                    .add_sized(
                                        [NAV_W, TAB_CTRL_H],
                                        egui::Button::new(
                                            egui::RichText::new("◀")
                                                .size(11.0)
                                                .color(t.text_primary),
                                        )
                                        .fill(t.button_bg)
                                        .stroke(Stroke::new(1.0_f32, t.border))
                                        .corner_radius(CornerRadius::same(3)),
                                    )
                                    .on_hover_text(tr(lang, "log.tab.scroll_left"))
                                    .clicked();

                                // Remaining width is a fixed lane for scrolling tabs.
                                let scroll_w =
                                    tab_lane_w.min((ui.available_width() - 2.0).max(80.0));
                                let scroll_out = ui
                                    .allocate_ui_with_layout(
                                        egui::vec2(scroll_w, TAB_ROW_H),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            egui::ScrollArea::horizontal()
                                                .id_salt("log_file_tabs_bar")
                                                .max_width(scroll_w)
                                                .max_height(TAB_ROW_H)
                                                .auto_shrink([false, false])
                                                .scroll_bar_visibility(
                                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                                )
                                                .animated(false)
                                                .horizontal_scroll_offset(self.tab_scroll_x)
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = 0.0;
                                                        let mut elide_cache =
                                                            std::mem::take(&mut self.tab_elide_cache);
                                                        for (i, tab) in self.tabs.iter().enumerate() {
                                                            let selected = self.active_tab == i;
                                                            let (fill, fg) = if selected {
                                                                (t.surface_bg, t.text_primary)
                                                            } else {
                                                                (t.tab_inactive_bg, t.tab_inactive_text)
                                                            };
                                                            let stroke = if selected {
                                                                Stroke::new(2.0_f32, t.accent)
                                                            } else {
                                                                Stroke::new(1.0_f32, t.border)
                                                            };

                                                            // One exact rect per tab. Nested Buttons
                                                            // negotiated their own natural heights
                                                            // after a file was loaded, so the title
                                                            // and close segment no longer shared a
                                                            // visually consistent row.
                                                            let tab_w = TAB_TITLE_W + TAB_CLOSE_W;
                                                            let (tab_rect, _) = ui.allocate_exact_size(
                                                                egui::vec2(tab_w, TAB_CTRL_H),
                                                                egui::Sense::hover(),
                                                            );
                                                            let close_rect = egui::Rect::from_min_max(
                                                                egui::pos2(
                                                                    tab_rect.right() - TAB_CLOSE_W,
                                                                    tab_rect.top(),
                                                                ),
                                                                tab_rect.max,
                                                            );
                                                            let title_rect = egui::Rect::from_min_max(
                                                                tab_rect.min,
                                                                egui::pos2(close_rect.left(), tab_rect.bottom()),
                                                            );
                                                            ui.painter().rect_filled(
                                                                tab_rect,
                                                                CornerRadius::same(4),
                                                                fill,
                                                            );
                                                            ui.painter().rect_stroke(
                                                                tab_rect,
                                                                CornerRadius::same(4),
                                                                stroke,
                                                                egui::StrokeKind::Inside,
                                                            );
                                                            if i > 0 {
                                                                ui.painter().vline(
                                                                    close_rect.left(),
                                                                    close_rect.top()..=close_rect.bottom(),
                                                                    Stroke::new(1.0_f32, t.border),
                                                                );
                                                            }
                                                            let title_resp = ui.interact(
                                                                title_rect,
                                                                ui.id().with(("tab_title", i)),
                                                                egui::Sense::click(),
                                                            );
                                                            if title_resp.clicked() {
                                                                activate_idx = Some(i);
                                                            }
                                                            if title_resp.hovered() {
                                                                let wheel_y = ui.input(|input| {
                                                                    if input.smooth_scroll_delta.y.abs() > 0.0 {
                                                                        input.smooth_scroll_delta.y
                                                                    } else {
                                                                        input.raw_scroll_delta.y
                                                                    }
                                                                });
                                                                if wheel_y.abs() > 0.5 {
                                                                    wheel_tab_direction =
                                                                        if wheel_y > 0.0 { -1 } else { 1 };
                                                                    wheel_event_time =
                                                                        ui.input(|input| input.time);
                                                                }
                                                            }
                                                            ui.painter().with_clip_rect(title_rect).text(
                                                                egui::pos2(
                                                                    title_rect.left() + 4.0,
                                                                    title_rect.center().y,
                                                                ),
                                                                egui::Align2::LEFT_CENTER,
                                                                elide_tab_title_to_width(
                                                                    ui,
                                                                    &tab.display_title(),
                                                                    title_rect.width() - 8.0,
                                                                    egui::FontId::proportional(TAB_FONT),
                                                                    &mut elide_cache,
                                                                ),
                                                                egui::FontId::proportional(TAB_FONT),
                                                                fg,
                                                            );
                                                            let close_resp = if i > 0 {
                                                                let cr = ui
                                                                    .interact(
                                                                        close_rect,
                                                                        ui.id().with(("tab_close", i)),
                                                                        egui::Sense::click(),
                                                                    )
                                                                    .on_hover_text(tr(lang, "log.tab.close"));
                                                                ui.painter().text(
                                                                    close_rect.center(),
                                                                    egui::Align2::CENTER_CENTER,
                                                                    "×",
                                                                    egui::FontId::proportional(TAB_FONT + 1.0),
                                                                    fg,
                                                                );
                                                                if cr.clicked() {
                                                                    close_idx = Some(i);
                                                                }
                                                                Some(cr)
                                                            } else {
                                                                None
                                                            };
                                                            if selected {
                                                                ui.painter().hline(
                                                                    tab_rect.left()..=tab_rect.right(),
                                                                    tab_rect.bottom() - 1.0,
                                                                    Stroke::new(2.0_f32, t.accent),
                                                                );
                                                            }
                                                            let tab_count = self.tabs.len();
                                                            let add_tab_menu =
                                                                |ui: &mut egui::Ui,
                                                                 menu_action: &mut Option<
                                                                    TabMenuAction,
                                                                >| {
                                                                    ui.set_min_width(140.0);
                                                                    let can_close_current = i > 0;
                                                                    let can_close_left = i > 1;
                                                                    let can_close_right =
                                                                        i + 1 < tab_count;
                                                                    let can_close_all = tab_count > 1;

                                                                    if ui
                                                                        .add_enabled(
                                                                            can_close_current,
                                                                            egui::Button::new(tr(
                                                                                lang,
                                                                                "log.tab.close_current",
                                                                            )),
                                                                        )
                                                                        .clicked()
                                                                    {
                                                                        *menu_action = Some(
                                                                            TabMenuAction::CloseCurrent(
                                                                                i,
                                                                            ),
                                                                        );
                                                                        ui.close_menu();
                                                                    }
                                                                    if ui
                                                                        .add_enabled(
                                                                            can_close_left,
                                                                            egui::Button::new(tr(
                                                                                lang,
                                                                                "log.tab.close_left",
                                                                            )),
                                                                        )
                                                                        .clicked()
                                                                    {
                                                                        *menu_action = Some(
                                                                            TabMenuAction::CloseLeft(i),
                                                                        );
                                                                        ui.close_menu();
                                                                    }
                                                                    if ui
                                                                        .add_enabled(
                                                                            can_close_right,
                                                                            egui::Button::new(tr(
                                                                                lang,
                                                                                "log.tab.close_right",
                                                                            )),
                                                                        )
                                                                        .clicked()
                                                                    {
                                                                        *menu_action = Some(
                                                                            TabMenuAction::CloseRight(
                                                                                i,
                                                                            ),
                                                                        );
                                                                        ui.close_menu();
                                                                    }
                                                                    if ui
                                                                        .add_enabled(
                                                                            can_close_all,
                                                                            egui::Button::new(tr(
                                                                                lang,
                                                                                "log.tab.close_all",
                                                                            )),
                                                                        )
                                                                        .clicked()
                                                                    {
                                                                        *menu_action =
                                                                            Some(TabMenuAction::CloseAll);
                                                                        ui.close_menu();
                                                                    }
                                                                };
                                                            title_resp.context_menu(|ui| {
                                                                add_tab_menu(ui, &mut menu_action);
                                                            });
                                                            if let Some(cr) = close_resp {
                                                                cr.context_menu(|ui| {
                                                                    add_tab_menu(ui, &mut menu_action);
                                                                });
                                                            }

                                                            ui.add_space(4.0);
                                                        }
                                                        self.tab_elide_cache = elide_cache;
                                                    });
                                                })
                                        },
                                    )
                                    .inner;

                                self.tab_scroll_x = scroll_out.state.offset.x;
                                let max_scroll =
                                    (scroll_out.content_size.x - scroll_w).max(0.0);
                                if left_clicked {
                                    self.tab_scroll_x =
                                        (self.tab_scroll_x - scroll_step).max(0.0);
                                }
                                if right_clicked {
                                    self.tab_scroll_x =
                                        (self.tab_scroll_x + scroll_step).min(max_scroll);
                                }
                                self.tab_scroll_x = self.tab_scroll_x.clamp(0.0, max_scroll);
                            });
                            if let Some(i) = close_idx {
                                self.close_tab(i);
                            }
                            if let Some(i) = activate_idx {
                                self.activate_tab(i);
                            }
                            if wheel_tab_direction != 0
                                && !self.tabs.is_empty()
                                && tab_wheel_switch_allowed(
                                    self.tab_wheel_last_switch_at,
                                    wheel_event_time,
                                )
                            {
                                let next = adjacent_tab_index(
                                    self.active_tab,
                                    self.tabs.len(),
                                    wheel_tab_direction,
                                );
                                self.activate_tab(next);
                                self.tab_scroll_x = reveal_tab_scroll_offset(
                                    self.tab_scroll_x,
                                    tab_lane_w,
                                    tab_stride,
                                    TAB_TITLE_W + TAB_CLOSE_W,
                                    next,
                                    self.tabs.len(),
                                );
                                self.tab_wheel_last_switch_at = Some(wheel_event_time);
                            }
                            if let Some(action) = menu_action {
                                match action {
                                    TabMenuAction::CloseCurrent(i) => self.close_tab(i),
                                    TabMenuAction::CloseLeft(i) => self.close_tabs_left_of(i),
                                    TabMenuAction::CloseRight(i) => self.close_tabs_right_of(i),
                                    TabMenuAction::CloseAll => self.close_all_file_tabs(),
                                }
                            }

                            ui.add_space(4.0);

                            let idx =
                                self.active_tab.min(self.tabs.len().saturating_sub(1));
                            if let Some(page) = self.tabs.get_mut(idx) {
                                page.ui(ui, lang, t);
                            }
                        });
                },
            );
        });
    }

    pub fn on_exit(&mut self) {
        self.stop_monitor(self.last_lang);
        self.persist_open_log_files();
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }
}

fn open_live_writer(path: &Path) -> std::io::Result<BufWriter<File>> {
    let f = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(BufWriter::with_capacity(64 * 1024, f))
}

/// Draw sidebar card border with Inside stroke so the right edge stays visible.
fn paint_sidebar_card_border(ui: &egui::Ui, rect: egui::Rect, card_w: f32, border: Color32) {
    let mut r = rect;
    r.set_width(card_w);
    ui.painter().rect_stroke(
        r,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );
}

/// One-line, left-aligned, truncated file row with full-name tooltip.
fn log_browser_file_row(
    ui: &mut egui::Ui,
    name: &str,
    row_w: f32,
    color: Color32,
) -> egui::Response {
    let row_h = 18.0_f32;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(row_w.max(1.0), row_h), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(3),
                Color32::from_rgba_unmultiplied(0x3B, 0x82, 0xF6, 40),
            );
        }
        // Left-aligned single-line label; truncate overflow with ellipsis.
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.set_min_width(rect.width());
                ui.set_max_width(rect.width());
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name).size(12.0).color(color),
                    )
                    .truncate()
                    .selectable(false),
                );
            },
        );
    }
    resp.on_hover_text(name)
}

fn elide_tab_title(title: &str, max_chars: usize) -> String {
    let count = title.chars().count();
    if count <= max_chars {
        return title.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", title.chars().take(keep).collect::<String>())
}

fn elide_tab_title_to_width(
    ui: &egui::Ui,
    title: &str,
    max_width: f32,
    font: egui::FontId,
    cache: &mut HashMap<(String, i32), String>,
) -> String {
    let key = (title.to_string(), max_width.round() as i32);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let result = if max_width <= 4.0 {
        "…".to_string()
    } else {
        let measure = |text: &str| -> f32 {
            ui.fonts(|f| {
                f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
        };
        if measure(title) <= max_width {
            title.to_string()
        } else {
            let mut chars: Vec<char> = title.chars().collect();
            let mut out = "…".to_string();
            while !chars.is_empty() {
                let trial: String = chars.iter().collect();
                if measure(&format!("{trial}…")) <= max_width {
                    out = format!("{trial}…");
                    break;
                }
                chars.pop();
            }
            out
        }
    };
    cache.insert(key, result.clone());
    result
}

fn normalized_log_name(raw: &str) -> Result<String, ()> {
    let name = raw.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        Err(())
    } else {
        Ok(name.to_string())
    }
}

fn adjacent_tab_index(current: usize, count: usize, direction: i8) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    if direction < 0 {
        current.saturating_sub(1)
    } else if direction > 0 {
        current.saturating_add(1).min(count - 1)
    } else {
        current
    }
}

fn tab_wheel_switch_allowed(last_switch: Option<f64>, now: f64) -> bool {
    const TAB_WHEEL_COOLDOWN_SECONDS: f64 = 0.24;
    last_switch.is_none_or(|last| now - last >= TAB_WHEEL_COOLDOWN_SECONDS)
}

fn reveal_tab_scroll_offset(
    current: f32,
    viewport_width: f32,
    tab_stride: f32,
    tab_width: f32,
    active_index: usize,
    tab_count: usize,
) -> f32 {
    if tab_count == 0 || viewport_width <= 0.0 {
        return 0.0;
    }
    let content_width = tab_count as f32 * tab_stride;
    let max_scroll = (content_width - viewport_width).max(0.0);
    let tab_left = active_index.min(tab_count - 1) as f32 * tab_stride;
    let tab_right = tab_left + tab_width;
    let next = if tab_left < current {
        tab_left
    } else if tab_right > current + viewport_width {
        tab_right - viewport_width
    } else {
        current
    };
    next.clamp(0.0, max_scroll)
}

fn rename_log_path(old_path: &Path, new_path: &Path) -> std::io::Result<()> {
    if old_path == new_path || !old_path.exists() {
        return Ok(());
    }
    if new_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "target log file already exists",
        ));
    }
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(old_path, new_path)
}

/// True when the live capture has no realtime lines and the on-disk file is
/// missing or only contains a UTF-8 BOM (created by [`SerialToolPanel::ensure_live_file`]).
fn live_capture_is_empty(live_line_count: usize, live_file: Option<&Path>) -> bool {
    if live_line_count > 0 {
        return false;
    }
    match live_file {
        None => true,
        Some(path) if !path.exists() => true,
        Some(path) => match fs::metadata(path) {
            Ok(meta) => meta.len() <= 3, // `\xEF\xBB\xBF` only
            Err(_) => true,
        },
    }
}

fn serial_worker(
    port: String,
    baud: u32,
    stop: Arc<AtomicBool>,
    tx: Sender<SerialEvent>,
    write_rx: Receiver<Vec<u8>>,
) {
    let mut session = match SerialSession::open(&port, baud) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(SerialEvent::Error(e.to_string()));
            let _ = tx.send(SerialEvent::Stopped);
            return;
        }
    };
    let _ = tx.send(SerialEvent::Status(format!("open {port}")));
    let mut last_stamp = Local::now().naive_local() - chrono::Duration::milliseconds(1);
    while !stop.load(Ordering::SeqCst) {
        while let Ok(bytes) = write_rx.try_recv() {
            if let Err(e) = session.write_bytes(&bytes) {
                let _ = tx.send(SerialEvent::Error(e.to_string()));
            }
        }
        match session.poll_events() {
            Ok(events) => {
                for ev in events {
                    match ev {
                        CapturedEvent::Metrics(m) => {
                            let payload = format!(
                                "AA55:{:.0}:{:.0}:{:.0}:{:.0}:{:.0}:{:.0}:{}:{}:EDED  # Vin={:.2}V Ibat={:.2}A Eff={:.1}%",
                                m.v_in * 1000.0,
                                m.i_in * 1000.0,
                                m.v_out * 1000.0,
                                m.i_out * 1000.0,
                                m.v_bat * 1000.0,
                                m.i_bat * 1000.0,
                                m.t,
                                m.b,
                                m.v_in,
                                m.i_bat,
                                m.eff
                            );
                            let line =
                                format!("{} {}", next_strict_timestamp(&mut last_stamp), payload);
                            if tx.send(SerialEvent::Line(line)).is_err() {
                                return;
                            }
                        }
                        CapturedEvent::Log { line, .. } => {
                            let payload = strip_tx_prefix(&line);
                            let stamped =
                                format!("{} {}", next_strict_timestamp(&mut last_stamp), payload);
                            if tx.send(SerialEvent::Line(stamped)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(SerialEvent::Error(e.to_string()));
                break;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = tx.send(SerialEvent::Stopped);
}

/// Python `SerialWorker.get_strict_timestamp` parity: `[HH:MM:SS.mmm]`, strictly increasing.
fn next_strict_timestamp(last: &mut NaiveDateTime) -> String {
    let mut now = Local::now().naive_local();
    if now <= *last {
        now = *last + chrono::Duration::milliseconds(1);
    }
    *last = now;
    format!(
        "[{}.{:03}]",
        now.format("%H:%M:%S"),
        now.nanosecond() / 1_000_000
    )
}

fn strip_tx_prefix(line: &str) -> &str {
    let t = line.trim_start();
    for p in ["TX0:", "TX1:", "TX0：", "TX1："] {
        if let Some(rest) = t.strip_prefix(p) {
            return rest.trim_start();
        }
    }
    t
}

fn normalize_path(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.replace('/', "\\"))
}

fn same_log_path(left: &str, right: &str) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_name_validation_rejects_windows_reserved_characters() {
        assert_eq!(normalized_log_name("  capture  "), Ok("capture".into()));
        assert!(normalized_log_name("").is_err());
        assert!(normalized_log_name("bad:name").is_err());
        assert!(normalized_log_name("bad.").is_err());
    }

    #[test]
    fn existing_log_file_is_renamed() {
        let dir = std::env::temp_dir().join(format!("wiparse_rename_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let old_path = dir.join("old.txt");
        let new_path = dir.join("new.txt");
        let _ = fs::remove_file(&old_path);
        let _ = fs::remove_file(&new_path);
        fs::write(&old_path, b"log").unwrap();

        rename_log_path(&old_path, &new_path).unwrap();

        assert!(!old_path.exists());
        assert_eq!(fs::read(&new_path).unwrap(), b"log");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn live_capture_empty_detects_bom_only_and_memory_lines() {
        let dir = std::env::temp_dir().join(format!("wiparse_empty_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let bom_only = dir.join("bom.txt");
        let with_data = dir.join("data.txt");
        fs::write(&bom_only, b"\xEF\xBB\xBF").unwrap();
        fs::write(&with_data, b"\xEF\xBB\xBFline\n").unwrap();

        assert!(live_capture_is_empty(0, None));
        assert!(live_capture_is_empty(0, Some(&bom_only)));
        assert!(!live_capture_is_empty(0, Some(&with_data)));
        assert!(!live_capture_is_empty(3, Some(&bom_only)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_when_empty_moves_file_when_nonempty_keeps_old() {
        let dir = std::env::temp_dir().join(format!("wiparse_split_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);

        // Empty (BOM): rename in place.
        let empty_old = dir.join("empty_old.txt");
        let empty_new = dir.join("empty_new.txt");
        fs::write(&empty_old, b"\xEF\xBB\xBF").unwrap();
        assert!(live_capture_is_empty(0, Some(&empty_old)));
        rename_log_path(&empty_old, &empty_new).unwrap();
        assert!(!empty_old.exists());
        assert!(empty_new.exists());

        // Non-empty: keep old file; create a separate new file (split semantics).
        let data_old = dir.join("data_old.txt");
        let data_new = dir.join("data_new.txt");
        fs::write(&data_old, b"\xEF\xBB\xBF[12:00:00.000] hello\n").unwrap();
        assert!(!live_capture_is_empty(0, Some(&data_old)));
        fs::write(&data_new, b"\xEF\xBB\xBF").unwrap();
        assert!(data_old.exists());
        assert!(data_new.exists());
        assert_ne!(
            fs::read(&data_old).unwrap(),
            fs::read(&data_new).unwrap()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn live_timestamp_uses_canonical_millisecond_separator() {
        let mut last = Local::now().naive_local() - chrono::Duration::seconds(1);
        let timestamp = next_strict_timestamp(&mut last);
        assert_eq!(timestamp.as_bytes().get(9), Some(&b'.'));
        assert_eq!(timestamp.len(), 14);
    }

    #[test]
    fn mouse_wheel_tab_switching_stops_at_ends() {
        assert_eq!(adjacent_tab_index(0, 4, -1), 0);
        assert_eq!(adjacent_tab_index(1, 4, -1), 0);
        assert_eq!(adjacent_tab_index(1, 4, 1), 2);
        assert_eq!(adjacent_tab_index(3, 4, 1), 3);
        assert_eq!(adjacent_tab_index(7, 4, 0), 3);
        assert!(tab_wheel_switch_allowed(None, 1.0));
        assert!(!tab_wheel_switch_allowed(Some(1.0), 1.1));
        assert!(tab_wheel_switch_allowed(Some(1.0), 1.24));
    }

    #[test]
    fn active_tab_scrolls_into_the_visible_lane() {
        let stride = 158.0;
        let width = 154.0;
        let viewport = 320.0;
        assert_eq!(
            reveal_tab_scroll_offset(0.0, viewport, stride, width, 0, 6),
            0.0
        );
        assert_eq!(
            reveal_tab_scroll_offset(0.0, viewport, stride, width, 2, 6),
            150.0
        );
        assert_eq!(
            reveal_tab_scroll_offset(300.0, viewport, stride, width, 1, 6),
            158.0
        );
        assert!(
            reveal_tab_scroll_offset(0.0, viewport, stride, width, 5, 6) <= 6.0 * stride - viewport
        );
    }

    #[test]
    fn open_file_identity_accepts_equivalent_windows_paths() {
        assert!(same_log_path(r"D:\logs\capture.txt", "d:/logs/capture.txt"));
        assert!(!same_log_path(r"D:\logs\capture.txt", r"D:\logs\other.txt"));
    }
}
