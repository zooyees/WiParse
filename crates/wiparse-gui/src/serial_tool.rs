//! Serial tool panel — left connect sidebar + `log_file_tabs` (Python `log_panel` parity).

use crate::log_tab::LogTabPage;
use crate::theme::{self as ui_theme, Tokens};
use chrono::{Local, NaiveDateTime, Timelike};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use serde_json::json;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use wiparse_core::brief::{IngestResult, LiveBrief};
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::evidence::{evidence_root, EvidencePack};
use wiparse_core::i18n::{tr, tr_fmt, tr_monitoring, Lang};
use wiparse_core::log::{build_file_store_worker, live_abs_page, FileBuildEvent, LogStore};
use wiparse_core::paths::project_path;
use wiparse_core::serial::{
    kick_wifi_uart_discovery, list_ports, refresh_ports, wifi_uart_revision, CapturedEvent,
    PortInfo, SerialSession,
};
use wiparse_core::testrun::{decode_hex, RunVerdict, TestDriver, TestPlan, TickAction};

mod ai_chat;
mod sidebar;

/// One subfolder under the log browser root that contains `.txt` files.
struct LogBrowserFolder {
    name: String,
    files: Vec<(String, PathBuf)>,
}

/// Cap RX UI apply per frame so a post-dialog backlog cannot freeze the UI.
const DRAIN_LINES_PER_FRAME: usize = 400;
const SERIAL_EVENT_CAPACITY: usize = 8_192;
const DRAIN_TIME_PER_FRAME: Duration = Duration::from_millis(4);
/// Flush the live log writer at this interval when save-to-disk is on.
const LIVE_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

enum SerialEvent {
    Opened { device: String, baud: u32 },
    Line(String),
    Status(String),
    Error(String),
    Stopped,
}

struct PendingFileLoad {
    tab_idx: usize,
    rx: Receiver<FileBuildEvent>,
}

struct ExtraLink {
    device: String,
    stop: Arc<AtomicBool>,
    rx: Receiver<SerialEvent>,
    #[allow(dead_code)]
    write_tx: Sender<Vec<u8>>,
    tab: usize,
}

pub struct SerialToolPanel {
    repaint: Option<egui::Context>,
    repaint_visible: Arc<AtomicBool>,
    ports: Vec<PortInfo>,
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
    save_live_to_disk: bool,
    recording_settings_open: bool,
    last_live_flush: Instant,
    last_lang: Lang,
    tab_scroll_x: f32,
    tab_wheel_last_switch_at: Option<f64>,
    pending_loads: Vec<PendingFileLoad>,
    live_rows_received: u64,
    recording_error: Option<String>,
    live_write_failed: bool,
    /// Cached elided tab titles: (source title, width_px) -> elided text.
    tab_elide_cache: HashMap<(String, i32), String>,
    /// Configurable root for the sidebar folder/txt browser.
    browser_dir: String,
    /// Last path that was scanned into `browser_folders`.
    browser_scanned_dir: String,
    browser_folders: Vec<LogBrowserFolder>,
    live_brief: LiveBrief,
    live_test: Option<LiveTest>,
    last_pack: Option<serde_json::Value>,
    pending_scope: Vec<String>,
    /// Last observed WiFi UART discovery thread state (merge when it finishes).
    wifi_scan_pending: bool,
    wifi_rev: u64,
    /// Device address of the primary live session (`tabs[0]`).
    primary_device: Option<String>,
    /// Additional open sessions. Each has its own live tab and TCP socket.
    extras: Vec<ExtraLink>,
    manual_tcp: String,
    ai_chat: ai_chat::SerialAiChat,
    ai_confirmed: HashMap<String, u32>,
    ai_serial_error: HashMap<String, String>,
    ai_epoch: u64,
    ai_test_locked: bool,
    ai_settings_requested: bool,
}

struct LiveTest {
    driver: TestDriver,
    pack: EvidencePack,
    scope_queue: Vec<String>,
    instrument_jobs: Vec<TestInstrumentJob>,
    finished: bool,
}

#[derive(Debug, Clone)]
pub enum TestInstrumentJob {
    Capture {
        tag: String,
        save: bool,
        device_id: Option<u64>,
    },
    WaveformSource {
        device_id: Option<u64>,
        dir: String,
        filename: Option<String>,
        overwrite: bool,
    },
    Command {
        device_id: Option<u64>,
        command: serde_json::Value,
    },
}

impl SerialToolPanel {
    pub fn new(cfg: &AppConfig, lang: Lang) -> Self {
        let ports = list_ports().unwrap_or_default();
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
            save_live_to_disk: cfg.log_monitor.save_live_to_disk,
            recording_settings_open: false,
            last_live_flush: Instant::now(),
            last_lang: lang,
            repaint: None,
            repaint_visible: Arc::new(AtomicBool::new(true)),
            tab_scroll_x: 0.0,
            tab_wheel_last_switch_at: None,
            pending_loads: Vec::new(),
            live_rows_received: 0,
            recording_error: None,
            live_write_failed: false,
            tab_elide_cache: HashMap::new(),
            browser_dir,
            browser_scanned_dir: String::new(),
            browser_folders: Vec::new(),
            live_brief: LiveBrief::new(),
            live_test: None,
            last_pack: None,
            pending_scope: Vec::new(),
            wifi_scan_pending: false,
            wifi_rev: 0,
            primary_device: None,
            extras: Vec::new(),
            manual_tcp: String::new(),
            ai_chat: Default::default(),
            ai_confirmed: HashMap::new(),
            ai_serial_error: HashMap::new(),
            ai_epoch: 0,
            ai_test_locked: false,
            ai_settings_requested: false,
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
                cfg.log_monitor.save_live_to_disk = self.save_live_to_disk;
                if let Err(error) = save_config(&cfg) {
                    tracing::warn!("failed to save serial-tool config: {error}");
                }
            }
            Err(_) => {
                let mut cfg = AppConfig::default();
                cfg.log_monitor.default_filename = self.committed_live_name.clone();
                cfg.log_monitor.save_dir = self.live_dir.clone();
                cfg.log_monitor.open_log_files = paths;
                cfg.log_monitor.last_open_dir = self.open_dir.clone();
                cfg.log_monitor.log_browser_dir = self.browser_dir.clone();
                cfg.log_monitor.save_live_to_disk = self.save_live_to_disk;
                if let Err(error) = save_config(&cfg) {
                    tracing::warn!("failed to save serial-tool config: {error}");
                }
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
    pub(crate) fn open_path(&mut self, path: PathBuf) -> Option<usize> {
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

    /// Open a log file and switch to its tab (report jump / API).
    pub(crate) fn open_path_and_show(&mut self, path: PathBuf) -> Option<usize> {
        let tab_idx = self.open_path(path)?;
        self.activate_tab(tab_idx);
        self.persist_open_log_files();
        Some(tab_idx)
    }

    fn refresh_ports(&mut self, lang: Lang) {
        let current = self.ports.get(self.selected_port).map(|p| p.device.clone());
        self.ports = refresh_ports().unwrap_or_default();
        if let Some(cur) = current {
            if let Some(idx) = self.ports.iter().position(|p| p.device == cur) {
                self.selected_port = idx;
            } else if self.monitoring {
                self.ports.insert(0, PortInfo::from_device(cur));
                self.selected_port = 0;
            } else {
                self.selected_port = 0;
            }
        } else if self.selected_port >= self.ports.len() {
            self.selected_port = 0;
        }
        self.wifi_rev = wifi_uart_revision();
        self.status = if self.monitoring {
            let port = self
                .ports
                .get(self.selected_port)
                .map(|p| p.compact_label())
                .unwrap_or_else(|| "?".into());
            let baud: u32 = self.baud.parse().unwrap_or(0);
            tr_monitoring(lang, &port, baud)
        } else if self.ports.is_empty() {
            tr(lang, "status.port_none")
        } else {
            tr_fmt(lang, "status.ports_count", self.ports.len())
        };
    }

    fn merge_discovered_ports(&mut self) {
        let current = self.ports.get(self.selected_port).map(|p| p.device.clone());
        let fresh = list_ports().unwrap_or_default();
        let same = fresh.len() == self.ports.len()
            && fresh
                .iter()
                .zip(self.ports.iter())
                .all(|(a, b)| a.device == b.device && a.label == b.label);
        if same {
            return;
        }
        self.ports = fresh;
        if let Some(cur) = current {
            if let Some(idx) = self.ports.iter().position(|p| p.device == cur) {
                self.selected_port = idx;
            } else if self.selected_port >= self.ports.len() {
                self.selected_port = 0;
            }
        } else if self.selected_port >= self.ports.len() {
            self.selected_port = 0;
        }
    }

    fn poll_wifi_ports(&mut self, ctx: &egui::Context) {
        kick_wifi_uart_discovery();
        let rev = wifi_uart_revision();
        if rev != self.wifi_rev {
            self.wifi_rev = rev;
            self.merge_discovered_ports();
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn ensure_listed_device(&mut self, device: &str) {
        if let Some(idx) = self.ports.iter().position(|p| p.device == device) {
            self.selected_port = idx;
        } else {
            self.ports
                .insert(0, PortInfo::from_device(device.to_owned()));
            self.selected_port = 0;
        }
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

        if !self.save_live_to_disk {
            self.live_name = name;
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
                    if let Err(err) = self.close_live_writer() {
                        self.recording_error = Some(err.to_string());
                        self.live_name = old_name;
                        self.status = format!("{}: {err}", tr(lang, "status.rename_failed"));
                        return false;
                    }
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
            if let Err(err) = self.close_live_writer() {
                self.recording_error = Some(err.to_string());
                self.live_name = old_name;
                self.status = format!("{}: {err}", tr(lang, "status.rename_failed"));
                return false;
            }
            // old_path file is left as-is (already flushed).
            self.live_file = None;
            self.live_writer = None;
            self.live_rows_received = 0;
            self.recording_error = None;
            self.live_write_failed = false;
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

    fn flush_live_writer(&mut self) -> std::io::Result<()> {
        if let Some(w) = &mut self.live_writer {
            w.flush()?;
        }
        Ok(())
    }

    fn close_live_writer(&mut self) -> std::io::Result<()> {
        if let Some(mut w) = self.live_writer.take() {
            w.flush()?;
        }
        Ok(())
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
            self.close_live_writer()?;
            self.live_writer = Some(open_live_writer(&path)?);
        }
        self.live_file = Some(path);
        Ok(())
    }

    fn append_to_live_file(&mut self, line: &str) -> std::io::Result<()> {
        if !self.save_live_to_disk {
            return Ok(());
        }
        if self.live_writer.is_none() {
            self.ensure_live_file()?;
        }
        if let Some(w) = &mut self.live_writer {
            writeln!(w, "{line}")?;
        }
        Ok(())
    }

    fn set_save_live_to_disk(&mut self, on: bool) {
        if self.save_live_to_disk == on {
            return;
        }
        self.save_live_to_disk = on;
        if on {
            self.live_write_failed = false;
            if self.monitoring {
                if let Err(e) = self.ensure_live_file() {
                    self.recording_error = Some(e.to_string());
                } else {
                    if let Some(live) = self.tabs.first() {
                        let lines = live.lines_slice(0, live.line_count());
                        if let Some(w) = &mut self.live_writer {
                            for line in lines {
                                if let Err(e) = writeln!(w, "{line}") {
                                    self.recording_error = Some(e.to_string());
                                    break;
                                }
                            }
                        }
                    }
                    if let Err(e) = self.flush_live_writer() {
                        self.recording_error = Some(e.to_string());
                    }
                    self.last_live_flush = Instant::now();
                }
            }
        } else {
            if let Err(e) = self.close_live_writer() {
                self.recording_error = Some(e.to_string());
            }
        }
        self.persist_open_log_files();
    }

    pub fn api_apply_default_baudrates(&mut self,values:&[String]) {
        // Changing the menu must never restart or reconfigure a running link.
        self.baud_options=values.to_vec();
        self.baud_custom=!self.baud_options.iter().any(|value|value==&self.baud);
    }

    fn recording_settings(&self)->serde_json::Value {
        serde_json::json!({"enabled":self.save_live_to_disk,"file_name":self.committed_live_name,"directory":self.live_dir})
    }

    pub fn api_recording_status(&self)->serde_json::Value {
        use std::hash::{Hash,Hasher};let settings=self.recording_settings();let mut hash=std::collections::hash_map::DefaultHasher::new();settings.to_string().hash(&mut hash);
        serde_json::json!({"enabled":self.save_live_to_disk,"active":self.live_writer.is_some(),"error":self.recording_error,"file_name":self.live_file.as_ref().and_then(|p|p.file_name()).and_then(|n|n.to_str()),"file_path":self.live_file.as_ref().map(|p|p.display().to_string()),"settings":settings,"revision":format!("{:016x}",hash.finish())})
    }

    pub fn api_recording_settings_safe(&mut self,cfg:&mut AppConfig,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.recording_settings_safe";
        if self.monitoring||!self.extras.is_empty()||self.is_test_running(){return err(method,"SERIAL.MONITOR_IN_PROGRESS");}
        let status=self.api_recording_status();if params["revision"].as_str()!=status["revision"].as_str(){return err(method,"SERIAL.SETTINGS_STALE");}
        let Some(enabled)=params["enabled"].as_bool() else{return err(method,"SERIAL.INVALID_REQUEST")};
        let Some(raw)=params["file_name"].as_str().filter(|value|value.len()<=120) else{return err(method,"SERIAL.INVALID_NAME")};
        let Ok(name)=normalized_log_name(raw) else{return err(method,"SERIAL.INVALID_NAME")};
        let stem=name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(),"CON"|"PRN"|"AUX"|"NUL")||(stem.len()==4&&(stem.starts_with("COM")||stem.starts_with("LPT"))&&matches!(stem.as_bytes()[3],b'1'..=b'9')){return err(method,"SERIAL.INVALID_NAME");}
        let Some(directory)=params["directory"].as_str().filter(|value|!value.trim().is_empty()&&value.len()<=4096&&!value.chars().any(char::is_control)) else{return err(method,"SERIAL.INVALID_DIRECTORY")};
        // Closing the existing writer is the only action on the previous file.
        // Editing defaults must never rename, truncate or delete captured data.
        if let Err(error)=self.close_live_writer(){self.recording_error=Some(error.to_string());return err(method,"SERIAL.FLUSH_FAILED");}
        let mut next=cfg.clone();next.log_monitor.default_filename=name.clone();next.log_monitor.save_dir=directory.to_string();next.log_monitor.save_live_to_disk=enabled;
        if let Err(error)=wiparse_core::config::save_config(&next){return err(method,&format!("SERIAL.SAVE_FAILED: {error}"));}
        self.live_name=name.clone();self.committed_live_name=name;self.live_dir=directory.to_string();self.live_file=None;self.save_live_to_disk=enabled;self.live_write_failed=false;self.recording_error=None;
        if let Some(tab)=self.tabs.first_mut(){tab.title=self.committed_live_name.clone();}
        *cfg=next;ok(method,self.api_recording_status())
    }

    pub fn api_set_recording_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};
        let Some(enabled)=params.get("enabled").and_then(|v|v.as_bool()) else{return err("ui.serial.recording_safe","SERIAL.INVALID_REQUEST")};
        if self.monitoring||!self.extras.is_empty(){return err("ui.serial.recording_safe","SERIAL.MONITOR_IN_PROGRESS");}
        self.set_save_live_to_disk(enabled);
        ok("ui.serial.recording_safe",self.api_recording_status())
    }

    fn release_writer_if_target_is_being_edited(&mut self) {
        let Some(live) = self.live_file.clone() else {
            return;
        };
        let live_s = normalize_path(&live.to_string_lossy());
        let editing = self.tabs.iter().any(|tab| {
            tab.is_editing()
                && tab
                    .filepath
                    .as_deref()
                    .is_some_and(|p| same_log_path(p, &live_s))
        });
        if editing {
            if let Err(e) = self.close_live_writer() {
                self.recording_error = Some(e.to_string());
            }
        }
    }

    fn start_monitor(&mut self, lang: Lang) {
        let _ = self.api_start_monitor(lang, None, None);
    }

    fn apply_port_baud(&mut self, port_override: Option<String>, baud_override: Option<u32>) {
        if let Some(ref p) = port_override {
            self.ensure_listed_device(p);
        }
        if let Some(b) = baud_override {
            self.baud = b.to_string();
            self.baud_custom = !self.baud_options.iter().any(|x| x == &self.baud);
        }
    }

    /// Select port/baud in the UI without opening the serial device.
    pub fn api_select_port(
        &mut self,
        _lang: Lang,
        port: Option<String>,
        baud: Option<u32>,
    ) -> Result<(Option<String>, u32), String> {
        self.apply_port_baud(port, baud);
        Ok(self.current_port_baud())
    }
    pub fn api_start_monitor(
        &mut self,
        lang: Lang,
        port_override: Option<String>,
        baud_override: Option<u32>,
    ) -> Result<(String, u32), String> {
        if let Some(ref p) = port_override {
            self.ensure_listed_device(p);
        }
        if let Some(b) = baud_override {
            self.baud = b.to_string();
            self.baud_custom = !self.baud_options.iter().any(|x| x == &self.baud);
        }
        let Some(port) = self.ports.get(self.selected_port).map(|p| p.device.clone()) else {
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
        if self.primary_device.as_deref() == Some(port.as_str())
            || self.extras.iter().any(|link| link.device == port)
        {
            return Ok((port, baud));
        }
        if self.monitoring {
            return self.start_extra(lang, port, baud);
        }
        if self.live_name.trim() != self.committed_live_name && !self.commit_live_name(lang) {
            return Err(tr(lang, "status.create_failed").into());
        }
        if self.save_live_to_disk {
            if let Err(e) = self.ensure_live_file() {
                self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
                return Err(self.status.clone());
            }
            self.last_live_flush = Instant::now();
        } else {
            if let Err(e) = self.close_live_writer() {
                self.recording_error = Some(e.to_string());
                self.status = format!("recording error: {e}");
                return Err(self.status.clone());
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = bounded(SERIAL_EVENT_CAPACITY);
        let (write_tx, write_rx) = unbounded();
        let stop_c = Arc::clone(&stop);
        let port_for_thread = port.clone();
        let repaint = self.repaint.clone();
        let visible = Arc::clone(&self.repaint_visible);
        thread::spawn(move || {
            serial_worker(
                port_for_thread,
                baud,
                stop_c,
                tx,
                write_rx,
                repaint,
                visible,
            )
        });
        self.stop_flag = Some(stop);
        self.rx = Some(rx);
        self.write_tx = Some(write_tx);
        self.monitoring = true;
        self.ai_epoch = self.ai_epoch.wrapping_add(1);
        self.ai_serial_error.remove(&port);
        self.primary_device = Some(port.clone());
        let status_port = self
            .ports
            .get(self.selected_port)
            .map(|p| p.compact_label())
            .unwrap_or_else(|| port.clone());
        self.status = tr_monitoring(lang, &status_port, baud);
        if let Some(live) = self.tabs.get_mut(0) {
            live.title = self.live_name.clone();
        }
        self.active_tab = 0;
        self.live_brief.reset();
        self.live_rows_received = 0;
        self.recording_error = None;
        self.live_write_failed = false;
        Ok((port, baud))
    }

    fn start_extra(
        &mut self,
        lang: Lang,
        port: String,
        baud: u32,
    ) -> Result<(String, u32), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = bounded(SERIAL_EVENT_CAPACITY);
        let (write_tx, write_rx) = unbounded();
        let stop_c = Arc::clone(&stop);
        let port_for_thread = port.clone();
        let repaint = self.repaint.clone();
        let visible = Arc::clone(&self.repaint_visible);
        thread::spawn(move || {
            serial_worker(
                port_for_thread,
                baud,
                stop_c,
                tx,
                write_rx,
                repaint,
                visible,
            )
        });
        let title = self
            .ports
            .iter()
            .find(|p| p.device == port)
            .map(|p| p.compact_label())
            .unwrap_or_else(|| port.clone());
        let mut tab = LogTabPage::live_tab(lang);
        tab.title = title.clone();
        self.tabs.push(tab);
        let tab_idx = self.tabs.len() - 1;
        self.extras.push(ExtraLink {
            device: port.clone(),
            stop,
            rx,
            write_tx,
            tab: tab_idx,
        });
        self.active_tab = tab_idx;
        self.monitoring = true;
        self.status = tr_monitoring(lang, &title, baud);
        Ok((port, baud))
    }

    pub fn api_stop_device(&mut self, lang: Lang, device: &str) {
        if self.primary_device.as_deref() == Some(device) {
            self.api_stop_monitor(lang);
            return;
        }
        self.stop_extra(device);
        self.monitoring = self.primary_device.is_some() || !self.extras.is_empty();
    }

    fn stop_extra(&mut self, device: &str) {
        let Some(idx) = self.extras.iter().position(|link| link.device == device) else {
            return;
        };
        self.ai_epoch = self.ai_epoch.wrapping_add(1);
        self.ai_confirmed.remove(device);
        let link = self.extras.remove(idx);
        link.stop.store(true, Ordering::SeqCst);
        let tab = link.tab;
        if tab > 0 && self.tabs.get(tab).is_some_and(|t| t.live) {
            self.tabs.remove(tab);
            for extra in &mut self.extras {
                if extra.tab > tab {
                    extra.tab -= 1;
                }
            }
            for load in &mut self.pending_loads {
                if load.tab_idx > tab {
                    load.tab_idx -= 1;
                }
            }
            if self.active_tab >= self.tabs.len() {
                self.active_tab = 0;
            }
        }
    }

    fn drain_extras(&mut self, update_live_filters: bool) {
        let mut dead = Vec::new();
        let n = self.extras.len();
        for i in 0..n {
            let mut batch = Vec::new();
            let mut stopped = false;
            let tab = self.extras[i].tab;
            let started = Instant::now();
            let mut remaining = DRAIN_LINES_PER_FRAME;
            while remaining > 0 && started.elapsed() < DRAIN_TIME_PER_FRAME {
                let Ok(ev) = self.extras[i].rx.try_recv() else {
                    break;
                };
                remaining -= 1;
                match ev {
                    SerialEvent::Opened { device, baud } => { self.ai_confirmed.insert(device, baud); }
                    SerialEvent::Line(line) => batch.push(line),
                    SerialEvent::Error(e) => { self.ai_serial_error.insert(self.extras[i].device.clone(), e.clone()); batch.push(format!("[ERR] {e}")); }
                    SerialEvent::Stopped => {
                        stopped = true;
                        break;
                    }
                    SerialEvent::Status(_) => {}
                }
            }
            if !batch.is_empty() {
                if let Some(page) = self.tabs.get_mut(tab) {
                    self.ai_chat.observe(page.ai_view_id(), &batch);
                    page.append_lines(batch, update_live_filters);
                }
            }
            if stopped {
                dead.push(self.extras[i].device.clone());
            }
        }
        for device in dead.into_iter().rev() {
            self.stop_extra(&device);
        }
    }

    pub(crate) fn selected_device(&self) -> Option<String> {
        self.ports.get(self.selected_port).map(|p| p.device.clone())
    }

    fn device_is_open(&self, device: &str) -> bool {
        self.primary_device.as_deref() == Some(device)
            || self.extras.iter().any(|link| link.device == device)
    }

    pub fn api_stop_monitor(&mut self, lang: Lang) {
        if self.live_test.is_some() {
            self.finish_test_and_record(RunVerdict::Aborted, "monitor stopped");
        }
        self.stop_monitor(lang);
    }

    pub fn take_write_sender(&mut self) -> Option<Sender<Vec<u8>>> {
        self.write_tx.clone()
    }

    /// Enqueue bytes for the explicitly named, confirmed open port. This does
    /// not imply that the device has received them; the worker performs I/O.
    pub fn api_queue_device_bytes(&self,device:&str,bytes:Vec<u8>)->Result<(),String>{
        if bytes.is_empty()||bytes.len()>4096{return Err("SERIAL.INVALID_DATA".into());}
        if !self.ai_confirmed.keys().any(|port|Self::serial_ports_equal(port,device)){return Err("SERIAL.NOT_CONFIRMED".into());}
        let sender=if self.primary_device.as_deref().is_some_and(|port|Self::serial_ports_equal(port,device)) {
            self.write_tx.as_ref()
        }else{self.extras.iter().find(|link|Self::serial_ports_equal(&link.device,device)).map(|link|&link.write_tx)};
        sender.ok_or("SERIAL.NOT_MONITORING")?.send(bytes).map_err(|_|"SERIAL.WRITE_QUEUE_CLOSED".into())
    }

    pub fn log_versions(&self)->Vec<(u64,u64,usize,bool,Option<String>)>{
        let mut versions:Vec<_>=self.tabs.iter().map(|tab|(tab.ai_view_id(),tab.content_epoch(),tab.dropped().saturating_add(tab.line_count()),tab.loading,tab.load_error.clone())).collect();
        versions.push((u64::MAX,self.active_tab as u64,0,false,None));versions
    }
    pub fn publish_log_changes(&self,before:&[(u64,u64,usize,bool,Option<String>)],api:&crate::backend::ApiBridge){
        if before!=self.log_versions(){api.events.publish("serial.log.changed",serde_json::json!({}),None);}
    }

    pub fn api_tabs_list(&self) -> serde_json::Value {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(id, tab)| {
                serde_json::json!({
                    "tab_id": id,
                    "view_id": tab.ai_view_id(),
                    "content_epoch": tab.content_epoch(),
                    "web_editing":tab.has_web_editor(),
                    "title": tab.title,
                    "live": tab.live,
                    "path": tab.filepath.clone(),
                    "lines": tab.line_count(),
                    "total": tab.dropped().saturating_add(tab.line_count()),
                    "dropped": tab.dropped(),
                    "loading": tab.loading,
                    "index_progress": tab.index_progress,
                    "load_error": tab.load_error.as_ref().map(|error|error.chars().take(1024).collect::<String>()),
                })
            })
            .collect();
        serde_json::json!({ "tabs": tabs, "active_tab": self.active_tab })
    }

    fn serial_ports_equal(left: &str, right: &str) -> bool {
        left.eq_ignore_ascii_case(right)
    }

    /// Live monitor tab for an open COM / Dongle UART session.
    pub fn tab_for_port(&self, port: &str) -> Option<usize> {
        if self
            .primary_device
            .as_deref()
            .is_some_and(|device| Self::serial_ports_equal(device, port))
        {
            return Some(0);
        }
        self.extras
            .iter()
            .find(|link| Self::serial_ports_equal(&link.device, port))
            .map(|link| link.tab)
    }

    pub fn port_for_tab(&self, tab_id: usize) -> Option<&str> {
        if tab_id == 0 {
            return self.primary_device.as_deref();
        }
        self.extras
            .iter()
            .find(|link| link.tab == tab_id)
            .map(|link| link.device.as_str())
    }

    pub fn baud_presets(&self) -> Vec<String> {
        let values = if self.baud_options.is_empty() {
            vec!["115200".into(), "1000000".into(), "2000000".into()]
        } else {
            self.baud_options.clone()
        };
        let mut unique = Vec::new();
        for value in values {
            let Ok(rate) = value.parse::<u32>() else { continue };
            if (1..=12_000_000).contains(&rate) && !unique.iter().any(|seen| seen == &value) {
                unique.push(value);
            }
            if unique.len() == 32 { break; }
        }
        if unique.is_empty() { vec!["115200".into(), "1000000".into(), "2000000".into()] } else { unique }
    }

    pub fn monitor_sessions(&self) -> Vec<serde_json::Value> {
        let mut sessions = Vec::new();
        if let Some(port) = &self.primary_device {
            let confirmed=self.ai_confirmed.iter().find(|(key,_)|Self::serial_ports_equal(key,port)).map(|(_,baud)|*baud);
            let error=self.ai_serial_error.iter().find(|(key,_)|Self::serial_ports_equal(key,port)).map(|(_,message)|message);
            sessions.push(serde_json::json!({ "port": port, "tab_id": 0, "confirmed": confirmed.is_some(), "baud":confirmed, "error":error }));
        }
        for link in &self.extras {
            let confirmed=self.ai_confirmed.iter().find(|(key,_)|Self::serial_ports_equal(key,&link.device)).map(|(_,baud)|*baud);
            let error=self.ai_serial_error.iter().find(|(key,_)|Self::serial_ports_equal(key,&link.device)).map(|(_,message)|message);
            sessions.push(serde_json::json!({ "port": link.device, "tab_id": link.tab, "confirmed":confirmed.is_some(), "baud":confirmed, "error":error }));
        }
        sessions
    }
    pub fn monitor_errors(&self) -> &HashMap<String,String> { &self.ai_serial_error }

    pub fn api_lines_get(&self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let port = params
            .get("port")
            .and_then(|v| v.as_str())
            .or_else(|| params.get("device").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty());
        let tab_id = if let Some(port) = port {
            match self.tab_for_port(port) {
                Some(id) => id,
                None => return err("log.lines.get", "port is not being monitored"),
            }
        } else {
            params.get("tab_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize
        };
        let from = params.get("from_row").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        // Cap page size to avoid accidental DoS / huge JSON payloads.
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(100)
            .min(2000) as usize;
        let Some(tab) = self.tabs.get(tab_id) else {
            return err("log.lines.get", "tab not found");
        };
        if params.get("view_id").is_some_and(|expected|expected.as_u64()!=Some(tab.ai_view_id())) {
            return err("log.lines.get", "LOG.VIEW_CHANGED");
        }
        if params.get("content_epoch").is_some_and(|expected|expected.as_u64()!=Some(tab.content_epoch())) {
            return err("log.lines.get", "LOG.CONTENT_CHANGED");
        }
        let dropped = tab.dropped();
        let window = tab.line_count();
        let page = live_abs_page(dropped, window, from, limit);
        let lines = if let Some(value)=params.get("max_bytes") {
            let Some(max)=value.as_u64().filter(|n|(1024..=2_097_152).contains(n)) else{return err("log.lines.get","LOG.INVALID_BYTE_LIMIT")};
            match tab.lines_slice_bounded(page.window_from,page.take,max as usize){Ok(lines)=>lines,Err(())=>return err("log.lines.get","LOG.LINE_TOO_LARGE")}
        } else {tab.lines_slice(page.window_from, page.take)};
        let next=if lines.len()==page.take {page.next_abs} else {dropped.saturating_add(page.window_from).saturating_add(lines.len())};
        ok(
            "log.lines.get",
            serde_json::json!({
                "tab_id": tab_id,
                "view_id": tab.ai_view_id(),
                "content_epoch": tab.content_epoch(),
                "port": self.port_for_tab(tab_id),
                "from_row": from,
                "lines": lines,
                "count": lines.len(),
                "total": page.total_abs,
                "dropped": dropped,
                "window": window,
                "next_row": next,
                "truncated": page.truncated,
            }),
        )
    }

    pub fn api_recent_live_lines(&self, limit: usize) -> Vec<String> {
        self.tabs
            .first()
            .map(|t| t.recent_lines(limit))
            .unwrap_or_default()
    }

    /// Total lines in the live tab (tab 0) — current FIFO window length.
    pub fn live_line_count(&self) -> usize {
        self.tabs.first().map(|t| t.line_count()).unwrap_or(0)
    }

    /// Live FIFO evictions since last clear.
    pub fn live_dropped(&self) -> usize {
        self.tabs.first().map(|t| t.dropped()).unwrap_or(0)
    }

    /// Next absolute live sequence (`dropped + window`).
    pub fn live_abs_len(&self) -> usize {
        self.live_dropped().saturating_add(self.live_line_count())
    }

    /// Live tab lines from an absolute session row.
    pub fn live_lines_since(&self, from: usize, limit: usize) -> Vec<String> {
        let Some(tab) = self.tabs.first() else {
            return Vec::new();
        };
        let page = live_abs_page(tab.dropped(), tab.line_count(), from, limit);
        tab.lines_slice(page.window_from, page.take)
    }

    pub fn current_port_baud(&self) -> (Option<String>, u32) {
        let port = self.ports.get(self.selected_port).map(|p| p.device.clone());
        let baud = self.baud.parse().unwrap_or(0);
        (port, baud)
    }

    pub fn api_brief(&self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let since = params
            .get("since_row")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let detail = params.get("detail").and_then(|v| v.as_str());
        let mut data = self.live_brief.snapshot_json(since, detail);
        let (port, baud) = self.current_port_baud();
        data["monitoring"] = serde_json::json!(self.monitoring);
        data["port"] = serde_json::json!(port);
        data["baud"] = serde_json::json!(baud);
        if let Some(t) = &self.live_test {
            data["run"] = serde_json::json!({
                "id": t.pack.run_id,
                "plan": t.driver.plan.id,
                "verdict": t.driver.verdict,
                "reason": t.driver.reason,
                "step": t.driver.current_label(),
                "dir": t.pack.dir.display().to_string(),
            });
        }
        if self.tabs.is_empty() {
            return err("log.brief", "no live tab");
        }
        ok("log.brief", data)
    }

    pub fn api_test_start(
        &mut self,
        lang: Lang,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if self.is_test_running() {
            return Err("a test is already running".into());
        }
        let plan_v = params
            .get("plan")
            .cloned()
            .ok_or_else(|| "missing plan".to_string())?;
        let plan = TestPlan::from_json(&plan_v)?;
        let port_ov = params
            .get("port")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let baud_ov = params
            .get("baud")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        if self.monitoring {
            let (cur_port, cur_baud) = self.current_port_baud();
            let port_differs = port_ov
                .as_ref()
                .is_some_and(|p| cur_port.as_deref() != Some(p.as_str()));
            let baud_differs = baud_ov.is_some_and(|b| b != cur_baud);
            if port_differs || baud_differs {
                self.api_stop_monitor(lang);
                self.api_start_monitor(lang, port_ov, baud_ov)?;
            }
        } else {
            self.api_start_monitor(lang, port_ov, baud_ov)?;
        }
        let (port, baud) = self.current_port_baud();
        let port = port.unwrap_or_default();
        let pack = EvidencePack::create(&evidence_root(), plan.clone(), &port, baud)?;
        let dir = pack.dir.display().to_string();
        let run_id = pack.run_id.clone();
        self.live_test = Some(LiveTest {
            driver: TestDriver::new(plan),
            pack,
            scope_queue: Vec::new(),
            instrument_jobs: Vec::new(),
            finished: false,
        });
        self.status = format!("test {run_id}");
        self.tick_test();
        Ok(serde_json::json!({
            "run_id": run_id,
            "dir": dir,
            "monitoring": self.monitoring,
            "port": port,
            "baud": baud,
        }))
    }

    pub fn api_test_status(&self) -> serde_json::Value {
        if let Some(t) = &self.live_test {
            serde_json::json!({
                "running": t.driver.is_running(),
                "verdict": t.driver.verdict,
                "reason": t.driver.reason,
                "step": t.driver.current_label(),
                "elapsed_s": t.driver.elapsed_s(),
                "run_id": t.pack.run_id,
                "dir": t.pack.dir.display().to_string(),
                "steps": t.driver.step_log,
                "recording_error": self.recording_error,
            })
        } else {
            serde_json::json!({
                "running": false,
                "verdict": null,
                "last": self.last_pack,
                "recording_error": self.recording_error,
            })
        }
    }

    pub fn api_test_abort(&mut self, reason: &str) -> Result<serde_json::Value, String> {
        let (verdict, why, act) = {
            let t = self.live_test.as_mut().ok_or("no test running")?;
            let act = t.driver.abort(reason);
            (t.driver.verdict, t.driver.reason.clone(), act)
        };
        self.apply_tick_action(act);
        self.finish_test(verdict, &why)
    }

    pub fn api_test_pack(&mut self) -> Result<serde_json::Value, String> {
        if self.live_test.is_none() {
            return self
                .last_pack
                .clone()
                .ok_or_else(|| "no test pack (run test.start first)".into());
        }
        let now = self.live_brief.elapsed_s();
        let due = self
            .live_test
            .as_mut()
            .map(|t| t.pack.take_due_events(now))
            .unwrap_or_default();
        let rows: Vec<wiparse_core::evidence::CorrelateRow> = due
            .into_iter()
            .map(|ev| wiparse_core::evidence::CorrelateRow {
                t: ev.t,
                ev: ev.k,
                reason: ev.x,
                m: self.live_brief.metrics_window(ev.t - 0.05, ev.t + 0.10),
                s: None,
            })
            .collect();
        if let Some(t) = self.live_test.as_mut() {
            for row in rows {
                t.pack.push_correlate(row);
            }
        }
        let finished = self
            .live_test
            .as_ref()
            .is_some_and(|t| !t.driver.is_running() && !t.finished);
        if finished {
            let t = self.live_test.as_ref().unwrap();
            let v = t.driver.verdict;
            let r = t.driver.reason.clone();
            return self.finish_test(v, &r);
        }
        let brief = self.live_brief.snapshot_json(0, None);
        let t = self.live_test.as_ref().unwrap();
        Ok(t.pack.pack_summary(
            t.driver.verdict,
            &t.driver.reason,
            &t.driver.step_log,
            &brief,
        ))
    }

    fn ingest_live_batch(&mut self, start_row: u64, lines: &[String]) -> Result<(), String> {
        let mut first_error = None;
        for (i, line) in lines.iter().enumerate() {
            let row = start_row + i as u64;
            let result = self.live_brief.ingest_line(row, line);
            if let Some(t) = self.live_test.as_mut() {
                if let Err(e) = t.pack.append_serial_line(line) {
                    first_error.get_or_insert(e);
                }
                match &result {
                    IngestResult::Event(ev) => {
                        if let Err(e) = t.pack.on_event(ev) {
                            first_error.get_or_insert(e);
                        }
                    }
                    IngestResult::Metric { t: ts, sample } => {
                        if let Err(e) = t.pack.on_metric(*ts, sample) {
                            first_error.get_or_insert(e);
                        }
                    }
                    IngestResult::None => {}
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn tick_test(&mut self) {
        if !self.live_test.as_ref().is_some_and(|t| !t.finished) {
            return;
        }
        let now = self.live_brief.elapsed_s();
        let due = self
            .live_test
            .as_mut()
            .map(|t| t.pack.take_due_events(now))
            .unwrap_or_default();
        let rows: Vec<wiparse_core::evidence::CorrelateRow> = due
            .into_iter()
            .map(|ev| wiparse_core::evidence::CorrelateRow {
                t: ev.t,
                ev: ev.k,
                reason: ev.x,
                m: self.live_brief.metrics_window(ev.t - 0.05, ev.t + 0.10),
                s: None,
            })
            .collect();
        let lines = self.api_recent_live_lines(5000);
        let act = {
            let SerialToolPanel {
                live_brief,
                live_test,
                ..
            } = self;
            if let Some(t) = live_test.as_mut() {
                for row in rows {
                    t.pack.push_correlate(row);
                }
                t.driver.tick(live_brief, &lines)
            } else {
                TickAction::None
            }
        };
        self.apply_tick_action(act);
        if let Some(t) = self.live_test.as_ref() {
            if !t.driver.is_running() && !t.finished {
                let v = t.driver.verdict;
                let r = t.driver.reason.clone();
                self.finish_test_and_record(v, &r);
            }
        }
    }

    fn apply_tick_action(&mut self, act: TickAction) {
        match act {
            TickAction::None => {}
            TickAction::Send { name, hex } => {
                self.live_brief.note_send(&hex);
                let send_err = match decode_hex(&hex) {
                    Err(e) => Some(e),
                    Ok(bytes) => match self.write_tx.as_ref() {
                        None => Some("no serial write channel".into()),
                        Some(tx) => tx
                            .send(bytes)
                            .err()
                            .map(|_| "serial write channel closed".to_string()),
                    },
                };
                if let Some(e) = send_err {
                    if let Some(t) = self.live_test.as_mut() {
                        let fail_act = t.driver.fail(&e);
                        match fail_act {
                            TickAction::Capture {
                                tag,
                                save,
                                device_id,
                            } => {
                                self.queue_capture(tag, save, device_id);
                            }
                            other => self.apply_tick_action(other),
                        }
                    }
                    return;
                }
                self.status = format!("test send {name}");
            }
            TickAction::Capture {
                tag,
                save,
                device_id,
            } => {
                self.queue_capture(tag, save, device_id);
            }
            TickAction::WaveformSource {
                device_id,
                dir,
                filename,
                overwrite,
            } => {
                if let Some(t) = self.live_test.as_mut() {
                    t.instrument_jobs.push(TestInstrumentJob::WaveformSource {
                        device_id,
                        dir,
                        filename,
                        overwrite,
                    });
                }
            }
            TickAction::Command { device_id, command } => {
                if let Some(t) = self.live_test.as_mut() {
                    t.instrument_jobs
                        .push(TestInstrumentJob::Command { device_id, command });
                }
            }
            TickAction::Done => {}
        }
    }

    fn queue_capture(&mut self, tag: String, save: bool, device_id: Option<u64>) {
        if save {
            if let Some(t) = self.live_test.as_mut() {
                t.instrument_jobs.push(TestInstrumentJob::Capture {
                    tag,
                    save,
                    device_id,
                });
            }
        } else {
            self.pending_scope.push(tag.clone());
            if let Some(t) = self.live_test.as_mut() {
                t.scope_queue.push(tag);
            }
        }
    }

    pub fn take_test_instrument_jobs(&mut self) -> Vec<TestInstrumentJob> {
        self.live_test
            .as_mut()
            .map(|t| std::mem::take(&mut t.instrument_jobs))
            .unwrap_or_default()
    }

    pub fn evidence_dir(&self) -> Option<PathBuf> {
        self.live_test.as_ref().map(|t| t.pack.dir.clone())
    }

    pub fn notify_test_job(&mut self, result: &crate::instrument_control::InstrumentJobResult) {
        if !self.live_test.as_ref().is_some_and(|t| !t.finished) {
            return;
        }
        let t_s = self.live_brief.elapsed_s();
        if let Some(t) = self.live_test.as_mut() {
            if let Some(path) = result.detail.get("path").and_then(|v| v.as_str()) {
                t.pack.note_artifact(&result.kind, path, t_s);
            }
            if result.kind == "capture" {
                t.pack.note_scope(
                    result
                        .detail
                        .get("tag")
                        .and_then(|v| v.as_str())
                        .unwrap_or("scope"),
                    result.device_id,
                    result.ok,
                    t_s,
                );
            }
        }
        let detail = if result.ok {
            result
                .detail
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("ok")
                .to_string()
        } else {
            result
                .detail
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("instrument job failed")
                .to_string()
        };
        let act = if let Some(t) = self.live_test.as_mut() {
            t.driver.notify_job(result.ok, &detail)
        } else {
            TickAction::None
        };
        self.apply_tick_action(act);
        self.tick_test();
        if let Some(t) = self.live_test.as_ref() {
            if !t.driver.is_running() && !t.finished {
                let v = t.driver.verdict;
                let r = t.driver.reason.clone();
                self.finish_test_and_record(v, &r);
            }
        }
    }

    fn finish_test(
        &mut self,
        verdict: RunVerdict,
        reason: &str,
    ) -> Result<serde_json::Value, String> {
        let Some(mut t) = self.live_test.take() else {
            return Err("no test".into());
        };
        t.finished = true;
        let evidence_error = self.recording_error.clone();
        let final_verdict = if evidence_error.is_some() {
            RunVerdict::Failed
        } else {
            verdict
        };
        let final_reason = evidence_error
            .as_ref()
            .map(|e| format!("evidence incomplete: {e}"))
            .unwrap_or_else(|| reason.to_string());
        let live = self.live_file.clone();
        let hit = t.driver.last_hit_line.clone();
        let mut summary = t.pack.finalize(
            &self.live_brief,
            final_verdict,
            &final_reason,
            &t.driver.step_log,
            live.as_deref(),
        )?;
        if let Some(hit) = hit {
            summary["last_hit_line"] = serde_json::json!(hit);
        }
        self.status = format!("test {:?} ({final_reason})", final_verdict);
        self.last_pack = Some(summary.clone());
        Ok(summary)
    }

    fn finish_test_and_record(&mut self, verdict: RunVerdict, reason: &str) {
        if let Err(error) = self.finish_test(verdict, reason) {
            self.recording_error = Some(error.clone());
            self.status = format!("evidence finalization failed: {error}");
        }
    }

    pub fn take_scope_captures(&mut self) -> Vec<String> {
        let mut out = std::mem::take(&mut self.pending_scope);
        if let Some(t) = self.live_test.as_mut() {
            out.extend(std::mem::take(&mut t.scope_queue));
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn note_scope_capture(&mut self, tag: &str, device_id: Option<u64>, ok: bool) {
        let t_s = self.live_brief.elapsed_s();
        if let Some(t) = self.live_test.as_mut() {
            t.pack.note_scope(tag, device_id, ok, t_s);
        }
    }

    fn stop_monitor(&mut self, lang: Lang) {
        self.ai_epoch = self.ai_epoch.wrapping_add(1);
        if let Some(port) = &self.primary_device { self.ai_confirmed.remove(port); }
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
        self.rx = None;
        self.write_tx = None;
        self.primary_device = None;
        self.monitoring = !self.extras.is_empty();
        if let Err(e) = self.close_live_writer() {
            self.recording_error = Some(e.to_string());
        }
        self.status = tr(lang, "status.stopped");
    }

    fn new_live_log(&mut self, lang: Lang) -> bool {
        if !self.save_live_to_disk {
            self.clear_live_display();
            let stamp = Local::now().format("%Y%m%d_%H%M%S");
            self.live_name = format!("{} {}", tr(lang, "log.default_filename"), stamp);
            self.committed_live_name = self.live_name.clone();
            if let Some(live) = self.tabs.get_mut(0) {
                live.title = self.live_name.clone();
            }
            self.persist_live_log_name();
            self.status = format!("new live log: {}", self.live_name);
            self.ai_epoch=self.ai_epoch.wrapping_add(1);
            return true;
        }
        // Flush current live to disk, then push it right as a file tab and
        // insert a fresh live tab at index 0 (newest live stays leftmost).
        if let Err(e) = self.ensure_live_file() {
            self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
            return false;
        }
        if let Err(e) = self.close_live_writer() {
            self.recording_error = Some(e.to_string());
            return false;
        }
        let Some(archived_path) = self.live_file.take() else {
            self.status = tr(lang, "status.create_failed").into();
            return false;
        };
        self.live_writer = None;

        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        self.live_name = format!("{} {}", tr(lang, "log.default_filename"), stamp);
        let unique=wiparse_core::paths::unique_file_path(self.live_path().parent().unwrap_or(Path::new("log")),&format!("{}.txt",self.live_name),false);
        if let Some(stem)=unique.file_stem().and_then(|value|value.to_str()){self.live_name=stem.to_string();}
        self.committed_live_name = self.live_name.clone();

        let mut new_live = LogTabPage::live_tab(lang);
        new_live.title = self.live_name.clone();
        self.tabs.insert(0, new_live);
        for load in &mut self.pending_loads {
            load.tab_idx += 1;
        }
        for link in &mut self.extras {link.tab+=1;}

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
            return false;
        }
        self.active_tab = 0;
        self.persist_open_log_files();
        self.status = format!("new live log: {}", self.live_name);
        self.live_rows_received=0;self.ai_epoch=self.ai_epoch.wrapping_add(1);
        true
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
        folders.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
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

    fn close_tab(&mut self,idx:usize){self.close_tab_indices(vec![idx]);}

    fn close_tab_indices(&mut self,indices:Vec<usize>){
        let removed=indices.into_iter().filter(|index|*index>0&&*index<self.tabs.len()&&!self.tabs[*index].is_editing()&&!self.extras.iter().any(|link|link.tab==*index)).collect::<std::collections::HashSet<_>>();
        if removed.is_empty(){return;}
        let mut shift=0;let mapping=(0..self.tabs.len()).map(|index|{let mapped=index-shift;if removed.contains(&index){shift+=1;}mapped}).collect::<Vec<_>>();
        let remap=|index:usize|mapping.get(index).copied().unwrap_or(index);
        let active=if removed.contains(&self.active_tab){(0..self.active_tab).rev().find(|index|!removed.contains(index)).unwrap_or(0)}else{self.active_tab};
        self.active_tab=remap(active);
        self.tabs=std::mem::take(&mut self.tabs).into_iter().enumerate().filter_map(|(index,tab)|(!removed.contains(&index)).then_some(tab)).collect();
        self.pending_loads.retain_mut(|load|{if removed.contains(&load.tab_idx){false}else{load.tab_idx=remap(load.tab_idx);true}});
        for link in &mut self.extras{link.tab=remap(link.tab);}
        self.persist_open_log_files();
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
                            if tab.is_editing() {
                                drop(store);
                            } else {
                                let total = store.line_count();
                                tab.set_file_store(store);
                                self.status = format!("opened {} ({total} lines)", tab.title);
                            }
                        }
                        finished = true;
                        break;
                    }
                    Ok(FileBuildEvent::Err(e)) => {
                        self.status = format!("open failed: {e}");
                        if let Some(tab) = self.tabs.get_mut(load.tab_idx) {
                            tab.load_error=Some(e);
                            tab.finish_loading();
                        }
                        finished = true;
                        break;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        let message="日志读取任务异常结束".to_owned();
                        self.status=message.clone();
                        if let Some(tab)=self.tabs.get_mut(load.tab_idx){tab.load_error=Some(message);tab.finish_loading();}
                        finished=true;
                        break;
                    },
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
        self.release_writer_if_target_is_being_edited();
        for tab in &mut self.tabs {
            tab.poll_background_tasks();
        }
        self.poll_file_loads();

        let mut budget = DRAIN_LINES_PER_FRAME;
        let mut batch = Vec::new();
        let mut stop_seen = false;
        let drain_started = Instant::now();
        if let Some(rx) = &self.rx {
            while budget > 0 && drain_started.elapsed() < DRAIN_TIME_PER_FRAME {
                let Ok(ev) = rx.try_recv() else {
                    break;
                };
                budget -= 1;
                match ev {
                    SerialEvent::Opened { device, baud } => { self.ai_confirmed.insert(device, baud); }
                    SerialEvent::Line(line) => {
                        batch.push(line);
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
                        if let Some(port) = &self.primary_device { self.ai_serial_error.insert(port.clone(), e.clone()); }
                        self.status = e.clone();
                        batch.push(format!("[ERR] {e}"));
                    }
                    SerialEvent::Stopped => {
                        stop_seen = true;
                        break;
                    }
                }
            }
        }

        if !batch.is_empty() {
            let start_row = self.live_rows_received;
            self.live_rows_received += batch.len() as u64;
            if let Err(e) = self.ingest_live_batch(start_row, &batch) {
                self.recording_error.get_or_insert(e);
            }
            if self.save_live_to_disk {
                for line in &batch {
                    if let Some(api) = api {
                        api.publish_serial_line(line);
                    }
                    if !self.live_write_failed {
                        if let Err(e) = self.append_to_live_file(line) {
                            self.recording_error = Some(e.to_string());
                            self.live_write_failed = true;
                            self.live_writer = None;
                        }
                    }
                }
            } else {
                for line in &batch {
                    if let Some(api) = api {
                        api.publish_serial_line(line);
                    }
                }
            }
            if let Some(live) = self.tabs.get_mut(0) {
                self.ai_chat.observe(live.ai_view_id(), &batch);
                live.append_lines(batch, update_live_filters);
            }
        }
        if stop_seen {
            self.ai_epoch = self.ai_epoch.wrapping_add(1);
            if let Some(port) = &self.primary_device { self.ai_confirmed.remove(port); }
            self.primary_device = None;
            self.monitoring = !self.extras.is_empty();
            self.stop_flag = None;
            self.rx = None;
            self.write_tx = None;
            if let Err(e) = self.close_live_writer() {
                self.recording_error = Some(e.to_string());
            }
            self.status = tr(self.last_lang, "status.stopped");
            if let Some(api) = api {
                api.set_monitoring(false, None, None);
                *api.serial_write.lock().unwrap_or_else(|e| e.into_inner()) = None;
                api.publish_serial_status("stopped");
            }
        }

        if stop_seen && self.live_test.is_some() {
            self.finish_test_and_record(RunVerdict::Aborted, "monitor stopped");
        }

        if let Some(error) = self.recording_error.as_ref() {
            self.status = format!("recording error: {error}");
        }
        self.drain_extras(update_live_filters);
        self.tick_test();
        if self.save_live_to_disk
            && self.live_writer.is_some()
            && self.last_live_flush.elapsed() >= LIVE_FLUSH_INTERVAL
        {
            if let Err(e) = self.flush_live_writer() {
                self.recording_error = Some(e.to_string());
                self.live_write_failed = true;
                self.live_writer = None;
            }
            self.last_live_flush = Instant::now();
        }
    }

    pub fn needs_repaint(&self) -> bool {
        self.monitoring
            || !self.pending_loads.is_empty()
            || self.tabs.iter().any(LogTabPage::has_background_filter)
            || self.is_test_running()
    }

    /// Background file load / RX backlog — repaint even when another main tab is visible.
    pub fn has_background_io(&self) -> bool {
        !self.pending_loads.is_empty()
            || self.tabs.iter().any(LogTabPage::has_background_filter)
            || self.is_test_running()
    }

    pub fn is_test_running(&self) -> bool {
        self.live_test
            .as_ref()
            .is_some_and(|t| t.driver.is_running() && !t.finished)
    }

    pub(crate) fn runtime_busy(&self)->bool{self.is_monitoring()||self.is_test_running()||self.has_pending_log_edits()||self.ai_chat.monitoring_active()}

    pub fn is_monitoring(&self) -> bool {
        self.monitoring
    }

    pub fn attach_repaint_context(&mut self, ctx: &egui::Context) {
        self.repaint = Some(ctx.clone());
    }

    pub fn set_live_view_visible(&self, visible: bool) {
        self.repaint_visible.store(visible || self.ai_chat.monitoring_active(), Ordering::Relaxed);
    }

    pub fn has_pending_rx(&self) -> bool {
        self.rx.as_ref().is_some_and(|rx| !rx.is_empty())
            || self.extras.iter().any(|source| !source.rx.is_empty())
            || (self.monitoring && self.repaint.is_none())
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

    fn close_tabs_left_of(&mut self,idx:usize){self.close_tab_indices((1..idx.min(self.tabs.len())).collect());}
    fn close_tabs_right_of(&mut self,idx:usize){self.close_tab_indices((idx.saturating_add(1)..self.tabs.len()).collect());}
    fn close_all_file_tabs(&mut self){self.close_tab_indices((1..self.tabs.len()).collect());}

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        self.last_lang = lang;
        self.poll_wifi_ports(ui.ctx());
        // Events are drained from app::update so backlog clears even when this tab is hidden.
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.handle_dropped_files(&dropped);
        }

        // Independent left sidebar + content (must force top-down
        // layouts). Independent max_rects so ComboBox selected_text cannot leak
        // width into the parent and open a gap beside the card.
        const SIDE_MAIN_GAP: f32 = ui_theme::PANEL_GAP;

        let overall = ui.available_rect_before_wrap();
        let _ = ui.allocate_exact_size(overall.size(), egui::Sense::hover());
        let (full, ai_rect) = self.ai_chat.split(ui, overall, t);
        let side_w = ui_theme::resizable_sidebar_width(
            ui,
            "serial_tool",
            full,
            ui_theme::SIDEBAR_W,
            ui_theme::SIDEBAR_MIN_W,
            160.0,
            SIDE_MAIN_GAP,
            t,
        );
        let side_rect = egui::Rect::from_min_size(full.min, egui::vec2(side_w, full.height()));
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(
                (side_rect.max.x + SIDE_MAIN_GAP).min(full.max.x),
                full.min.y,
            ),
            full.max,
        );
        {
            // 1px left/right padding so card borders are never clipped by the allocate edge.
            const SIDE_PAD: f32 = 1.0;
            let side_inner = egui::Rect::from_min_max(
                egui::pos2(side_rect.min.x + SIDE_PAD, side_rect.min.y),
                egui::pos2(side_rect.max.x - SIDE_PAD, side_rect.max.y),
            );
            crate::theme::scope_in_allocated_rect(
                ui,
                egui::UiBuilder::new()
                    .max_rect(side_inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    let card_w = side_inner.width();
                    ui.set_max_width(card_w);
                    ui.set_min_width(card_w);
                    ui.set_clip_rect(side_rect.intersect(ui.clip_rect()));
                    egui::ScrollArea::vertical().id_salt("serial-controls-scroll").max_height(side_inner.height()).auto_shrink([false, false]).show(ui, |ui| {
                    let card_w = ui.available_width();
                    const CARD_MARGIN_X: i8 = 8;
                    let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(1.0);

                    // Group 1: serial port controls — fill + Inside stroke (no Frame stroke clip).
                    ui.set_width(card_w);
                    let g1 = Frame::NONE
                        .fill(t.surface_bg)
                        .inner_margin(Margin::symmetric(CARD_MARGIN_X, 8))
                        .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                        .show(ui, |ui| {
                            ui.set_width(inner_w);
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                            ui.label(egui::RichText::new(if matches!(lang, Lang::Zh) { "连接与采集" } else { "Connection & capture" }).size(ui_theme::FONT_SECTION).strong());
                            ui.add_space(6.0);
                            let ctrl_w = inner_w;

                            let port_text = self
                                .ports
                                .get(self.selected_port)
                                .map(|p| p.compact_label())
                                .unwrap_or_else(|| tr(lang, "status.port_none"));
                            let port_hover = self
                                .ports
                                .get(self.selected_port)
                                .map(|p| p.display_label().to_owned())
                                .unwrap_or_else(|| port_text.clone());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
                                let refresh_w = ui_theme::CTRL_H;
                                let port_w = (ctrl_w - refresh_w - ui_theme::SPACE_SM).max(1.0);
                                egui::ComboBox::from_id_salt("serial_port")
                                    .width(port_w)
                                    .truncate()
                                    .selected_text(port_text)
                                    .show_ui(ui, |ui| {
                                        ui.set_min_width(port_w);
                                        if self.ports.is_empty() {
                                            ui.label(tr(lang, "status.port_none"));
                                        }
                                        for (i, p) in self.ports.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut self.selected_port,
                                                i,
                                                p.display_label(),
                                            );
                                        }
                                    })
                                    .response
                                    .on_hover_text(port_hover);
                                if ui_theme::secondary_btn_sized(
                                    ui,
                                    t,
                                    "⟳",
                                    egui::vec2(refresh_w, ui_theme::CTRL_H),
                                )
                                .on_hover_text(tr(lang, "btn.refresh_ports"))
                                .clicked()
                                {
                                    self.refresh_ports(lang);
                                }
                            });
                            let manual = ui.add_sized(
                                [ctrl_w, ui_theme::CTRL_H],
                                ui_theme::singleline_sized(&mut self.manual_tcp, ctrl_w)
                                    .hint_text("tcp://192.168.1.89:1234"),
                            );
                            if manual.lost_focus() && !self.manual_tcp.trim().is_empty() {
                                let raw = self.manual_tcp.trim().to_owned();
                                let device =
                                    if raw.starts_with("tcp://") || raw.starts_with("TCP://") {
                                        raw
                                    } else {
                                        format!("tcp://{raw}")
                                    };
                                self.ensure_listed_device(&device);
                            }

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
                                .truncate()
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
                                let baud_edit = ui.add_sized(
                                    [ctrl_w, ui_theme::CTRL_H],
                                    ui_theme::singleline_sized(&mut self.baud, ctrl_w)
                                        .hint_text(tr(lang, "serial.baud_custom_hint")),
                                );
                                if baud_edit.changed() {
                                    self.baud =
                                        self.baud.chars().filter(|c| c.is_ascii_digit()).collect();
                                }
                            }

                            let selected_open = self
                                .selected_device()
                                .is_some_and(|device| self.device_is_open(&device));
                            if selected_open {
                                let port = self
                                    .ports
                                    .get(self.selected_port)
                                    .map(|p| p.compact_label())
                                    .unwrap_or_else(|| "-".into());
                                let live = format!(
                                    "●  {port}  ·  {}  ·  {}",
                                    self.baud,
                                    tr(lang, "status.live")
                                );
                                ui.add_sized(
                                    [ctrl_w, ui_theme::CTRL_H],
                                    egui::Label::new(egui::RichText::new(&live)
                                        .size(ui_theme::FONT_CAPTION).color(t.accent)).truncate(),
                                ).on_hover_text(live);
                            }

                            let start_resp = if selected_open {
                                ui_theme::stop_button(ui, t, tr(lang, "btn.stop"))
                            } else {
                                ui_theme::accent_button(ui, t, tr(lang, "btn.start"))
                            };
                            if start_resp.clicked() {
                                if let Some(device) = self.selected_device() {
                                    if self.device_is_open(&device) {
                                        self.api_stop_device(lang, &device);
                                    } else {
                                        self.start_monitor(lang);
                                    }
                                }
                            }

                            let (new, clear) = sidebar::action_pair(
                                ui, t, ctrl_w, &tr(lang, "btn.new"), &tr(lang, "btn.clear"),
                                self.tabs.first().is_some_and(|tab| tab.line_count() > 0),
                            );
                            if new.clicked() { self.new_live_log(lang); }
                            if clear.clicked() { self.clear_live_display(); }

                            let summary = format!(
                                "{} · {}",
                                self.live_name,
                                if self.save_live_to_disk {
                                    tr(lang, "log.save_data")
                                } else {
                                    tr(lang, "status.ready")
                                }
                            );
                            let record_label = if self.recording_settings_open {
                                if lang == Lang::Zh {
                                    "记录设置  ▾"
                                } else {
                                    "Recording  ▾"
                                }
                            } else if lang == Lang::Zh {
                                "记录设置  ▸"
                            } else {
                                "Recording  ▸"
                            };
                            if ui_theme::secondary_btn_sized(
                                ui,
                                t,
                                record_label,
                                egui::vec2(ctrl_w, ui_theme::CTRL_H),
                            )
                            .on_hover_text(summary)
                            .clicked()
                            {
                                self.recording_settings_open = !self.recording_settings_open;
                            }
                            if self.recording_settings_open {
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(tr(lang, "log.filename"))
                                        .size(12.0)
                                        .color(t.text_muted),
                                );
                                let name_edit = ui.add_sized(
                                    [ctrl_w, ui_theme::CTRL_H],
                                    ui_theme::singleline_sized(&mut self.live_name, ctrl_w),
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
                                if (name_edit.has_focus() || name_edit.lost_focus())
                                    && enter_pressed
                                {
                                    self.commit_live_name(lang);
                                }

                                let mut save_data = self.save_live_to_disk;
                                let save_chk = ui
                                    .checkbox(
                                        &mut save_data,
                                        egui::RichText::new(tr(lang, "log.save_data"))
                                            .size(12.0)
                                            .color(t.text_muted),
                                    )
                                    .on_hover_text(tr(lang, "log.save_data_hint"));
                                if save_chk.changed() {
                                    self.set_save_live_to_disk(save_data);
                                }

                                ui.label(
                                    egui::RichText::new(tr(lang, "log.save_dir"))
                                        .size(12.0)
                                        .color(t.text_muted),
                                );
                                let _ = ui.add_sized(
                                    [ctrl_w, ui_theme::CTRL_H],
                                    ui_theme::singleline_sized(&mut self.live_dir, ctrl_w),
                                );

                                let (browse, open) = sidebar::action_pair(
                                    ui, t, ctrl_w, &tr(lang, "btn.browse_dir"), &tr(lang, "btn.open_log"), true,
                                );
                                if browse.clicked() { self.browse_dir(); }
                                if open.clicked() { self.open_files(); }
                            }
                        });
                    paint_sidebar_card_border(ui, g1.response.rect, card_w, t.border);

                    // Egui adds the surrounding item spacing as well; keep the
                    // visible card-to-card gap at the shared panel spacing.
                    ui.add_space((ui_theme::PANEL_GAP - ui.spacing().item_spacing.y).max(0.0));

                    // Group 2: log directory browser — same fixed width as Group 1.
                    let browser_h = (side_inner.bottom() - ui.cursor().min.y).max(160.0);
                    let browser_rect =
                        egui::Rect::from_min_size(ui.cursor().min, egui::vec2(card_w, browser_h));
                    ui.advance_cursor_after_rect(browser_rect);
                    crate::theme::scope_in_allocated_rect(
                        ui,
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
                                CornerRadius::same(ui_theme::RADIUS_CARD),
                                t.surface_bg,
                            );
                            ui.painter().rect_stroke(
                                browser_rect,
                                CornerRadius::same(ui_theme::RADIUS_CTRL),
                                Stroke::new(1.0_f32, t.border),
                                egui::StrokeKind::Inside,
                            );
                            let content = browser_rect
                                .shrink2(egui::vec2(1.0 + f32::from(CARD_MARGIN_X), 1.0 + 6.0));
                            crate::theme::scope_in_allocated_rect(
                                ui,
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

                                    ui.label(
                                        egui::RichText::new(tr(lang, "log.browser_dir"))
                                            .size(ui_theme::FONT_SECTION)
                                            .strong()
                                            .color(t.text_primary),
                                    );
                                    let browser_edit = ui.add_sized(
                                        [ctrl_w, ui_theme::CTRL_H],
                                        ui_theme::singleline_sized(&mut self.browser_dir, ctrl_w)
                                            .hint_text(tr(lang, "log.browser_hint")),
                                    );
                                    if browser_edit.lost_focus() {
                                        self.refresh_log_browser();
                                        self.persist_open_log_files();
                                    }

                                    let (browse, refresh) = sidebar::action_pair(
                                        ui, t, ctrl_w, &tr(lang, "btn.browse_dir"), &tr(lang, "btn.refresh_browser"), true,
                                    );
                                    if browse.clicked() { self.browse_log_browser_dir(); }
                                    if refresh.clicked() { self.refresh_log_browser(); }

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
                                        let mut open_path: Option<PathBuf> = None;
                                        // Solid bar: floating overlay is easy to miss on this
                                        // narrow card, so overflow looked like clipped names.
                                        ui.spacing_mut().scroll.floating = false;
                                        egui::ScrollArea::vertical()
                                            .id_salt("log_browser_tree")
                                            .max_height(list_h)
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                let list_w = ui.available_width().max(1.0);
                                                ui.set_min_width(list_w);
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
                                                            .strong()
                                                            .color(t.text_primary),
                                                    )
                                                    .id_salt((
                                                        "log-browser-folder",
                                                        folder.name.as_str(),
                                                    ))
                                                    .default_open(false)
                                                    .show(ui, |ui| {
                                                        let row_w = ui
                                                            .available_width()
                                                            .min(list_w)
                                                            .max(1.0);
                                                        ui.set_min_width(row_w);
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
                                                // Keep the last row above the card clip edge.
                                                ui.add_space(8.0);
                                            });
                                        if let Some(path) = open_path {
                                            self.open_browser_file(path);
                                        }
                                    }
                                },
                            ); // end browser content
                        },
                    ); // end browser card scope
                    }); // scroll controls on short windows
                },
            ); // end sidebar scope_builder

            // ── Right: file tabs + LogTabPage ─────────────────────────
            crate::theme::scope_in_allocated_rect(
                ui,
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_max_width(content_rect.width());
                    ui.set_clip_rect(content_rect.intersect(ui.clip_rect()));
                    Frame::NONE
                        .fill(t.surface_bg)
                        .inner_margin(Margin::same(6))
                        .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                        .stroke(Stroke::new(1.0_f32, t.border))
                        .show(ui, |ui| {
                            ui.set_min_height(ui.available_height().max(1.0));
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
                            const TAB_ROW_H: f32 = ui_theme::CTRL_H;
                            const TAB_CTRL_H: f32 = ui_theme::CTRL_H;
                            const TAB_FONT: f32 = ui_theme::FONT_CAPTION;
                            const NAV_W: f32 = ui_theme::CTRL_H;
                            const TAB_TITLE_W: f32 = 132.0;
                            const TAB_CLOSE_W: f32 = 22.0;
                            let row_w = ui.available_width();
                            let tab_piece = TAB_TITLE_W + TAB_CLOSE_W;
                            let tab_stride = tab_piece + ui_theme::SPACE_SM;
                            let content_w = if self.tabs.is_empty() {
                                0.0
                            } else {
                                self.tabs.len() as f32 * tab_piece
                                    + (self.tabs.len().saturating_sub(1) as f32) * ui_theme::SPACE_SM
                            };
                            let overflow = content_w > row_w + 0.5;
                            let nav_reserve = if overflow {
                                NAV_W * 2.0 + ui_theme::SPACE_SM * 2.0
                            } else {
                                0.0
                            };
                            let tab_lane_w = (row_w - nav_reserve).max(80.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(row_w, TAB_ROW_H),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                ui.set_min_height(TAB_ROW_H);
                                ui.set_max_height(TAB_ROW_H);
                                ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
                                let scroll_step = 180.0_f32;

                                // Arrows only when tabs overflow. right-to-left draws ▶ then ◀.
                                let mut right_clicked = false;
                                let mut left_clicked = false;
                                if overflow {
                                    right_clicked = ui
                                        .add_sized(
                                            [NAV_W, TAB_CTRL_H],
                                            egui::Button::new(
                                                egui::RichText::new("▶")
                                                    .size(ui_theme::FONT_CAPTION)
                                                    .color(t.text_primary),
                                            )
                                            .fill(t.button_bg)
                                            .stroke(Stroke::new(1.0_f32, t.border))
                                            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CTRL)),
                                        )
                                        .on_hover_text(tr(lang, "log.tab.scroll_right"))
                                        .clicked();
                                    left_clicked = ui
                                        .add_sized(
                                            [NAV_W, TAB_CTRL_H],
                                            egui::Button::new(
                                                egui::RichText::new("◀")
                                                    .size(ui_theme::FONT_CAPTION)
                                                    .color(t.text_primary),
                                            )
                                            .fill(t.button_bg)
                                            .stroke(Stroke::new(1.0_f32, t.border))
                                            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CTRL)),
                                        )
                                        .on_hover_text(tr(lang, "log.tab.scroll_left"))
                                        .clicked();
                                }

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
                                                                CornerRadius::same(ui_theme::RADIUS_CTRL),
                                                                fill,
                                                            );
                                                            ui.painter().rect_stroke(
                                                                tab_rect,
                                                                CornerRadius::same(ui_theme::RADIUS_CTRL),
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
                                page.set_ai_visible(self.ai_chat.open);
                                page.ui(ui, lang, t);
                                if page.take_ai_toggle() {
                                    if self.ai_chat.open { self.ai_chat.close(); } else { self.ai_chat.open = true; }
                                    ui.ctx().request_repaint();
                                }
                            }
                        });
                },
            );
        }
        if let Some(rect) = ai_rect { self.paint_ai_sidebar(ui, rect, lang, t); }
    }

    pub fn on_exit(&mut self) {
        self.ai_chat.close();
        self.stop_monitor(self.last_lang);
        self.persist_open_log_files();
    }

    /// Explicit AI-context capture; bounded to the active tab's latest 50 lines.
    pub(crate) fn ai_summary(&self) -> serde_json::Value {
        let tab = self.tabs.get(self.active_tab);
        let lines: Vec<String> = tab.map(|tab| tab.recent_lines(50).into_iter().map(|line| line.chars().take(400).collect()).collect()).unwrap_or_default();
        serde_json::json!({"monitor_status": self.monitor_status_text(), "active_tab": tab.map(|t| t.display_title()), "total_lines": tab.map(|t| t.line_count()), "sample": "latest 50 lines, each limited to 400 characters", "recent_lines": lines})
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// Monitor-only status for API (not file-open / indexing UI strings).
    pub fn monitor_status_text(&self) -> String {
        if let Some(error) = self.recording_error.as_ref() {
            return format!("recording error: {error}");
        }
        if self.monitoring {
            let (port, baud) = self.current_port_baud();
            match port {
                Some(p) => format!("open {p} @ {baud}"),
                None => format!("open @ {baud}"),
            }
        } else {
            "stopped".into()
        }
    }

    pub fn api_open_log(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
            return err("ui.serial.open", "missing path");
        };
        let pb = PathBuf::from(path);
        if !pb.is_file() {
            return err("ui.serial.open", &format!("not a file: {path}"));
        }
        match self.open_path(pb) {
            Some(tab_id) => {
                self.activate_tab(tab_id);
                self.persist_open_log_files();
                ok("ui.serial.open", json!({ "tab_id": tab_id, "path": path }))
            }
            None => err("ui.serial.open", &format!("could not open {path}")),
        }
    }

    pub fn api_open_log_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.open_safe";
        let Some(path)=params.get("path").and_then(|v|v.as_str()).filter(|s|!s.is_empty()&&s.len()<=4096&&!s.chars().any(char::is_control)) else{return err(method,"LOG.INVALID_PATH")};
        let path=PathBuf::from(path);if !path.is_absolute()||!path.is_file(){return err(method,"LOG.NOT_FOUND");}
        let reply=self.api_open_log(&json!({"path":path}));if !reply.ok{return reply;}
        let Some(id)=reply.data["tab_id"].as_u64().and_then(|id|usize::try_from(id).ok()) else{return err(method,"LOG.OPEN_UNKNOWN")};
        let Some(tab)=self.tabs.get(id) else{return err(method,"LOG.OPEN_UNKNOWN")};
        ok(method,json!({"tab_id":id,"view_id":tab.ai_view_id()}))
    }

    pub fn api_close_tab_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.close_safe";
        let Some(id)=params.get("tab_id").and_then(|v|v.as_u64()).and_then(|id|usize::try_from(id).ok()) else{return err(method,"LOG.INVALID_REQUEST")};
        let Some(view)=params.get("view_id").and_then(|v|v.as_u64()) else{return err(method,"LOG.INVALID_REQUEST")};
        let Some(tab)=self.tabs.get(id) else{return err(method,"LOG.VIEW_CHANGED")};
        if tab.ai_view_id()!=view{return err(method,"LOG.VIEW_CHANGED");}
        if id==0||self.extras.iter().any(|session|session.tab==id){return err(method,"LOG.LIVE_SESSION");}
        if tab.is_editing(){return err(method,"LOG.EDIT_IN_PROGRESS");}
        self.close_tab(id);ok(method,self.api_tabs_list())
    }

    pub fn api_close_tab(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let tab_id = params.get("tab_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if tab_id == 0 {
            return err("ui.serial.close", "cannot close live tab");
        }
        if tab_id >= self.tabs.len() {
            return err("ui.serial.close", "tab not found");
        }
        self.close_tab(tab_id);
        ok("ui.serial.close", self.api_tabs_list())
    }

    pub fn api_clear(&mut self) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        self.clear_live_display();
        ok("ui.serial.clear", json!({ "cleared": true }))
    }

    pub fn api_new_live_log_safe(&mut self,lang:Lang,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.new_log_safe";
        if params["consent"]!=true{return err(method,"SERIAL.CONSENT_REQUIRED");}
        if self.monitoring||!self.extras.is_empty()||self.is_test_running(){return err(method,"SERIAL.MONITOR_IN_PROGRESS");}
        let Some(tab)=self.tabs.first() else{return err(method,"LOG.VIEW_CHANGED")};
        if params["view_id"].as_u64()!=Some(tab.ai_view_id())||params["content_epoch"].as_u64()!=Some(tab.content_epoch()){return err(method,"LOG.CONTENT_CHANGED");}
        if !self.new_live_log(lang){return err(method,&format!("SERIAL.NEW_LOG_FAILED: {}",self.status));}
        ok(method,self.api_tabs_list())
    }

    pub fn api_clear_live_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.clear_live_safe";
        let Some(view)=params["view_id"].as_u64() else{return err(method,"LOG.INVALID_REQUEST")};
        let Some(epoch)=params["content_epoch"].as_u64() else{return err(method,"LOG.INVALID_REQUEST")};
        let Some(tab)=self.tabs.first() else{return err(method,"LOG.VIEW_CHANGED")};
        if tab.ai_view_id()!=view||tab.content_epoch()!=epoch{return err(method,"LOG.VIEW_CHANGED");}
        if tab.is_editing(){return err(method,"LOG.EDIT_IN_PROGRESS");}
        if self.is_test_running(){return err(method,"RUN.STATION_BUSY");}
        self.clear_live_display();ok(method,self.api_tabs_list())
    }

    pub fn api_filter(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tab_id = params.get("tab_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let pane = params.get("pane").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let Some(tab) = self.tabs.get_mut(tab_id) else {
            return err("ui.serial.filter", "tab not found");
        };
        if let Err(e) = tab.set_pane_filter(pane, &query) {
            return err("ui.serial.filter", &e);
        }
        ok(
            "ui.serial.filter",
            json!({ "tab_id": tab_id, "pane": pane, "query": query }),
        )
    }

    pub(crate) fn has_pending_log_edits(&self)->bool{self.tabs.iter().any(|tab|tab.is_editing())}
    pub fn api_edit_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.edit_safe";
        let Some(view)=params["view_id"].as_u64() else{return err(method,"LOG.INVALID_REQUEST")};let action=params["action"].as_str().unwrap_or("");
        let Some(index)=self.tabs.iter().position(|tab|tab.ai_view_id()==view) else{return err(method,"LOG.VIEW_CHANGED")};
        if action=="begin"&&params["tab_id"].as_u64()!=Some(index as u64){return err(method,"LOG.VIEW_CHANGED");}
        if self.tabs[index].live||self.extras.iter().any(|link|link.tab==index){return err(method,"LOG.LIVE_SESSION");}
        if matches!(action,"begin"|"save"|"save_as") {
            let target=if action=="save_as"{params["path"].as_str()}else{self.tabs[index].filepath.as_deref()}.and_then(|path|Path::new(path).canonicalize().ok());
            if self.monitoring&&target.is_some()&&self.live_file.as_ref().and_then(|path|path.canonicalize().ok())==target{return err(method,"LOG.LIVE_RECORDING");}
            if target.is_some()&&self.tabs.iter().enumerate().any(|(other,tab)|other!=index&&tab.is_editing()&&tab.filepath.as_deref().and_then(|path|Path::new(path).canonicalize().ok())==target){return err(method,"LOG.TARGET_IN_EDIT");}
        }
        if action=="begin"{self.activate_tab(index);}
        match self.tabs[index].web_edit_action(action,params){Ok(mut value)=>{value["tab_id"]=json!(index);value["view_id"]=json!(view);ok(method,value)},Err(error)=>err(method,&error)}
    }

    pub fn api_select_log_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.select_log_safe";
        let Some(id)=params["tab_id"].as_u64().and_then(|value|usize::try_from(value).ok()) else{return err(method,"LOG.INVALID_TAB")};
        let Some(view)=params["view_id"].as_u64() else{return err(method,"LOG.VIEW_CHANGED")};
        if self.tabs.get(id).is_none_or(|tab|tab.ai_view_id()!=view){return err(method,"LOG.VIEW_CHANGED");}
        self.activate_tab(id);ok(method,json!({"tab_id":id,"view_id":view}))
    }

    pub fn api_activate_tab(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let tab_id = params.get("tab_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if tab_id >= self.tabs.len() {
            return err("ui.serial.tab", "tab not found");
        }
        self.activate_tab(tab_id);
        ok("ui.serial.tab", json!({ "tab_id": tab_id }))
    }

    pub fn api_set_live_name(
        &mut self,
        lang: Lang,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
            return err("ui.serial.name", "missing name");
        };
        self.live_name = name.to_string();
        if !self.commit_live_name(lang) {
            return err("ui.serial.name", "invalid live log name");
        }
        ok(
            "ui.serial.name",
            json!({ "name": self.committed_live_name }),
        )
    }

    pub fn api_browser_root_safe(&mut self,cfg:&mut AppConfig,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.serial.browser_root_safe";
        if let Some(value)=params.get("directory"){
            let Some(directory)=value.as_str().filter(|value|!value.trim().is_empty()&&value.len()<=4096&&!value.chars().any(char::is_control)) else{return err(method,"LOG.INVALID_DIRECTORY")};
            let Ok(canonical)=project_path(directory).canonicalize() else{return err(method,"LOG.DIRECTORY_NOT_FOUND")};if !canonical.is_dir(){return err(method,"LOG.INVALID_DIRECTORY");}
            let root=wiparse_core::paths::app_root().canonicalize().unwrap_or_else(|_|wiparse_core::paths::app_root());
            let saved=if canonical==root{".".to_string()}else{canonical.strip_prefix(&root).unwrap_or(&canonical).to_string_lossy().into_owned()};
            let mut next=cfg.clone();next.log_monitor.log_browser_dir=saved.clone();
            if let Err(error)=save_config(&next){return err(method,&format!("LOG.SAVE_FAILED: {error}"));}
            self.browser_dir=saved;self.browser_scanned_dir.clear();*cfg=next;
        }
        ok(method,json!({"directory":self.browser_dir,"resolved":if self.browser_dir.is_empty(){None}else{Some(project_path(&self.browser_dir))}}))
    }

    pub fn api_set_browser_dir(
        &mut self,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(dir) = params.get("dir").and_then(|v| v.as_str()) else {
            return err("ui.serial.browser", "missing dir");
        };
        self.browser_dir = dir.to_string();
        self.refresh_log_browser();
        ok("ui.serial.browser", json!({ "dir": self.browser_dir }))
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
        crate::theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                ui.set_min_width(rect.width());
                ui.set_max_width(rect.width());
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(name).size(12.0).color(color))
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
    repaint: Option<egui::Context>,
    visible: Arc<AtomicBool>,
) {
    let mut session = match SerialSession::open(&port, baud) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(SerialEvent::Error(e.to_string()));
            let _ = tx.send(SerialEvent::Stopped);
            if let Some(ctx) = &repaint {
                ctx.request_repaint();
            }
            return;
        }
    };
    let _ = tx.send(SerialEvent::Opened { device: session.device.clone(), baud });
    let _ = tx.send(SerialEvent::Status(format!("open {}", session.device)));
    if let Some(ctx) = &repaint {
        ctx.request_repaint();
    }
    let mut last_stamp = Local::now().naive_local() - chrono::Duration::milliseconds(1);
    while !stop.load(Ordering::SeqCst) {
        while let Ok(bytes) = write_rx.try_recv() {
            if let Err(e) = session.write_bytes(&bytes) {
                let _ = tx.send(SerialEvent::Error(e.to_string()));
            }
        }
        match session.poll_events() {
            Ok(events) => {
                let idle = events.is_empty();
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
                if !idle {
                    if let Some(ctx) = &repaint {
                        ctx.request_repaint_after(Duration::from_millis(
                            if visible.load(Ordering::Relaxed) {
                                33
                            } else {
                                500
                            },
                        ));
                    }
                } else {
                    // Some USB transports return an empty packet without waiting.
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Err(e) => {
                let _ = tx.send(SerialEvent::Error(e.to_string()));
                break;
            }
        }
        // The transport already blocks with a read timeout when idle. Sleeping
        // here limits a continuous stream to 2048 bytes per 20 ms at best.
        thread::yield_now();
    }
    let _ = tx.send(SerialEvent::Stopped);
    if let Some(ctx) = &repaint {
        ctx.request_repaint();
    }
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
    fn log_change_events_are_commit_based_and_cover_extra_tabs_and_idle(){
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        let api=crate::backend::ApiBridge::new();let rx=api.events.subscribe(0).unwrap();rx.recv().unwrap();
        let before=panel.log_versions();panel.publish_log_changes(&before,&api);assert!(rx.try_recv().is_err());
        panel.tabs[0].append_lines(["first".into()],false);panel.publish_log_changes(&before,&api);
        let event:serde_json::Value=serde_json::from_str(&rx.recv().unwrap()).unwrap();assert_eq!(event["type"],"serial.log.changed");
        let current=panel.log_versions();panel.publish_log_changes(&current,&api);assert!(rx.try_recv().is_err());
        panel.tabs.push(LogTabPage::live_tab(Lang::En));let before=panel.log_versions();
        panel.tabs[1].append_lines(["extra".into()],false);panel.publish_log_changes(&before,&api);assert!(rx.recv().unwrap().contains("serial.log.changed"));
        let before=panel.log_versions();panel.active_tab=1;panel.publish_log_changes(&before,&api);assert!(rx.recv().unwrap().contains("serial.log.changed"));
        let before=panel.log_versions();panel.clear_live_display();panel.publish_log_changes(&before,&api);assert!(rx.recv().unwrap().contains("serial.log.changed"));
    }

    #[test]
    fn final_serial_batch_is_flushed_before_the_writer_closes() {
        let dir=std::env::temp_dir().join(format!("wiparse-serial-stop-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut cfg=AppConfig::default();cfg.log_monitor.save_dir=dir.display().to_string();cfg.log_monitor.default_filename="capture".into();cfg.log_monitor.save_live_to_disk=true;
        let mut panel=SerialToolPanel::new(&cfg,Lang::En);panel.ensure_live_file().unwrap();panel.monitoring=true;panel.primary_device=Some("COM_TEST".into());
        let (tx,rx)=bounded(4);panel.rx=Some(rx);
        tx.send(SerialEvent::Line("last-line-before-stop".into())).unwrap();tx.send(SerialEvent::Stopped).unwrap();
        panel.drain_events(false);
        assert!(!panel.monitoring);assert!(panel.live_writer.is_none());assert!(panel.recording_error.is_none());
        let path=dir.join("capture.txt");let content=std::fs::read(&path).unwrap();assert!(String::from_utf8_lossy(&content).contains("last-line-before-stop"));
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn recording_setting_cannot_change_during_a_monitor_session() {
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        let before=panel.save_live_to_disk;panel.monitoring=true;
        let response=panel.api_set_recording_safe(&serde_json::json!({"enabled":!before}));
        assert_eq!(response.error.as_deref(),Some("SERIAL.MONITOR_IN_PROGRESS"));
        assert_eq!(panel.save_live_to_disk,before);
    }

    #[test]
    fn guarded_live_clear_checks_view_and_invalidates_old_snapshot() {
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        panel.tabs[0].append_lines(vec!["unrecorded line".into()],true);
        let view=panel.tabs[0].ai_view_id();let epoch=panel.tabs[0].content_epoch();
        assert_eq!(panel.api_clear_live_safe(&json!({"view_id":view+1,"content_epoch":epoch})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
        assert_eq!(panel.api_clear_live_safe(&json!({"view_id":view,"content_epoch":epoch+1})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
        assert_eq!(panel.api_lines_get(&json!({"tab_id":0,"view_id":view,"from_row":0,"limit":10})).data["lines"],json!(["unrecorded line"]));
        assert!(panel.api_clear_live_safe(&json!({"view_id":view,"content_epoch":epoch})).ok);
        assert_ne!(panel.tabs[0].ai_view_id(),view);
        assert_eq!(panel.api_lines_get(&json!({"tab_id":0,"view_id":view,"from_row":0,"limit":10})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
    }

    #[test]
    fn guarded_log_open_and_close_reject_wrong_or_live_tabs() {
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        assert_eq!(panel.api_open_log_safe(&json!({"path":"relative.txt"})).error.as_deref(),Some("LOG.NOT_FOUND"));
        let live=panel.tabs[0].ai_view_id();
        assert_eq!(panel.api_close_tab_safe(&json!({"tab_id":0,"view_id":live})).error.as_deref(),Some("LOG.LIVE_SESSION"));
        panel.tabs.push(LogTabPage::live_tab(Lang::En));let tab=panel.tabs.len()-1;let view=panel.tabs[tab].ai_view_id();
        assert_eq!(panel.api_close_tab_safe(&json!({"tab_id":tab,"view_id":view+1})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
        let (_tx,rx)=bounded(1);let (write_tx,_write_rx)=unbounded();
        panel.extras.push(ExtraLink{device:"COM7".into(),stop:Arc::new(AtomicBool::new(false)),rx,write_tx,tab});
        assert_eq!(panel.api_close_tab_safe(&json!({"tab_id":tab,"view_id":view})).error.as_deref(),Some("LOG.LIVE_SESSION"));
        assert_eq!(panel.tabs.len(),tab+1);
    }

    #[test]
    fn log_index_failure_is_visible_on_its_own_tab() {
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
        panel.tabs.push(LogTabPage::file_tab_empty("fixture.log".into()));let tab=panel.tabs.len()-1;
        let (tx,rx)=unbounded();panel.pending_loads.push(PendingFileLoad{tab_idx:tab,rx});
        tx.send(FileBuildEvent::Err("fixture read failure".into())).unwrap();panel.poll_file_loads();
        let rows=panel.api_tabs_list();assert_eq!(rows["tabs"][tab]["loading"],false);assert_eq!(rows["tabs"][tab]["load_error"],"fixture read failure");
        panel.tabs.push(LogTabPage::file_tab_empty("disconnected.log".into()));let disconnected=panel.tabs.len()-1;
        let (sender,rx)=unbounded();drop(sender);panel.pending_loads.push(PendingFileLoad{tab_idx:disconnected,rx});panel.poll_file_loads();
        let rows=panel.api_tabs_list();assert_eq!(rows["tabs"][disconnected]["loading"],false);assert!(rows["tabs"][disconnected]["load_error"].as_str().is_some_and(|error|error.contains("异常结束")));
    }

    #[test]
    fn guarded_log_file_round_trip_keeps_original_file() {
        if let Ok(root)=std::env::var("WIPARSE_LOG_IO_TEST_ROOT") {
            let root=PathBuf::from(root);let file=root.join("source.log");let data=b"first line\nsecond line\n";std::fs::write(&file,data).unwrap();
            let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::En);
            let reply=panel.api_open_log_safe(&json!({"path":file}));assert!(reply.ok,"{:?}",reply.error);
            let id=reply.data["tab_id"].as_u64().unwrap() as usize;let view=reply.data["view_id"].as_u64().unwrap();
            let deadline=Instant::now()+Duration::from_secs(5);
            while panel.tabs[id].loading&&Instant::now()<deadline {panel.poll_file_loads();std::thread::sleep(Duration::from_millis(10));}
            assert!(!panel.tabs[id].loading);assert_eq!(panel.api_lines_get(&json!({"tab_id":id,"view_id":view,"from_row":0,"limit":10})).data["lines"],json!(["first line","second line"]));
            assert!(panel.api_close_tab_safe(&json!({"tab_id":id,"view_id":view})).ok);assert_eq!(std::fs::read(&file).unwrap(),data);
            return;
        }
        let root=std::env::temp_dir().join(format!("wiparse-log-io-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","serial_tool::tests::guarded_log_file_round_trip_keeps_original_file","--nocapture"])
            .env("WIPARSE_LOG_IO_TEST_ROOT",&root).env("WIPARSE_CONFIG",root.join("config.json")).env_remove("WCM_CONFIG").output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stdout));
        let temp=std::env::temp_dir().canonicalize().unwrap();assert!(root.canonicalize().unwrap().starts_with(temp));
        for entry in std::fs::read_dir(&root).unwrap(){let entry=entry.unwrap();assert!(entry.file_type().unwrap().is_file());std::fs::remove_file(entry.path()).unwrap();}
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn safe_send_targets_only_the_named_confirmed_session() {
        let mut panel=SerialToolPanel::new(&AppConfig::default(),Lang::Zh);
        let (primary,primary_rx)=unbounded();let (extra,extra_rx)=unbounded();let (_event_tx,event_rx)=bounded(1);
        panel.primary_device=Some("COM3".into());panel.write_tx=Some(primary);
        panel.extras.push(ExtraLink{device:"COM4".into(),stop:Arc::new(AtomicBool::new(false)),rx:event_rx,write_tx:extra,tab:1});
        assert!(panel.api_queue_device_bytes("COM4",vec![1]).is_err());
        panel.ai_confirmed.insert("COM3".into(),115200);panel.ai_confirmed.insert("COM4".into(),115200);
        panel.api_queue_device_bytes("com4",vec![0x41,0x42]).unwrap();
        assert_eq!(extra_rx.try_recv().unwrap(),vec![0x41,0x42]);assert!(primary_rx.try_recv().is_err());
        assert!(panel.api_queue_device_bytes("COM9",vec![1]).is_err());assert!(panel.api_queue_device_bytes("COM3",vec![]).is_err());
        panel.api_queue_device_bytes("COM3",vec![7]).unwrap();assert_eq!(primary_rx.try_recv().unwrap(),vec![7]);
    }

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
        assert_ne!(fs::read(&data_old).unwrap(), fs::read(&data_new).unwrap());

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

    #[test]
    fn log_lines_get_resolves_leased_port_instead_of_tab_zero() {
        let mut panel = SerialToolPanel::new(&AppConfig::default(), Lang::En);
        panel.primary_device = Some("COM3".into());
        panel.monitoring = true;
        let (_tx, rx) = bounded(1);
        let (write_tx, _write_rx) = unbounded();
        panel.tabs.push(LogTabPage::live_tab(Lang::En));
        let tab = panel.tabs.len() - 1;
        panel.extras.push(ExtraLink {
            device: "COM7".into(),
            stop: Arc::new(AtomicBool::new(false)),
            rx,
            write_tx,
            tab,
        });
        assert_eq!(panel.tab_for_port("com7"), Some(tab));
        let reply = panel.api_lines_get(&json!({ "port": "COM7", "from_row": 0, "limit": 10 }));
        assert!(reply.ok, "{:?}", reply.error);
        assert_eq!(reply.data["tab_id"], json!(tab));
        assert_eq!(reply.data["port"], json!("COM7"));
        let view_id=reply.data["view_id"].as_u64().unwrap();
        assert!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":view_id})).ok);
        assert_eq!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":view_id+1})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
        let epoch=reply.data["content_epoch"].as_u64().unwrap();
        assert_eq!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":view_id,"content_epoch":epoch+1})).error.as_deref(),Some("LOG.CONTENT_CHANGED"));
        panel.tabs[tab].clear_display();
        assert_eq!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":view_id})).error.as_deref(),Some("LOG.VIEW_CHANGED"));
        let new_view=panel.tabs[tab].ai_view_id();
        assert_eq!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":new_view,"content_epoch":epoch})).error.as_deref(),Some("LOG.CONTENT_CHANGED"));
        panel.tabs[tab].append_lines(["a".repeat(800),"b".repeat(800),"c".repeat(1100)],false);
        let first=panel.api_lines_get(&json!({"tab_id":tab,"view_id":new_view,"from_row":0,"limit":3,"max_bytes":1024}));
        assert!(first.ok);assert_eq!(first.data["count"],1);assert_eq!(first.data["next_row"],1);
        let second=panel.api_lines_get(&json!({"tab_id":tab,"view_id":new_view,"from_row":1,"limit":3,"max_bytes":1024}));
        assert!(second.ok);assert_eq!(second.data["next_row"],2);
        assert_eq!(panel.api_lines_get(&json!({"tab_id":tab,"view_id":new_view,"from_row":2,"max_bytes":1024})).error.as_deref(),Some("LOG.LINE_TOO_LARGE"));
        let missing = panel.api_lines_get(&json!({ "port": "COM9", "from_row": 0 }));
        assert!(!missing.ok);
    }
}
