//! Serial tool panel — left connect sidebar + `log_file_tabs` (Python `log_panel` parity).

use crate::log_tab::LogTabPage;
use crate::theme::{self as ui_theme, Tokens};
use chrono::{Local, NaiveDateTime, Timelike};
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, tr_fmt, tr_monitoring, Lang};
use wiparse_core::log::{build_file_store_worker, FileBuildEvent, LogStore};
use wiparse_core::serial::{list_ports, CapturedEvent, SerialSession};

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
    live_file: Option<PathBuf>,
    last_lang: Lang,
    tab_scroll_x: f32,
    tab_wheel_last_switch_at: Option<f64>,
    pending_loads: Vec<PendingFileLoad>,
    /// Leftover RX lines when drain hits per-frame cap.
    pending_lines: Vec<String>,
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
            live_file: None,
            last_lang: lang,
            tab_scroll_x: 0.0,
            tab_wheel_last_switch_at: None,
            pending_loads: Vec::new(),
            pending_lines: Vec::new(),
        };
        panel.restore_open_log_files(&cfg.log_monitor.open_log_files);
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
                let _ = save_config(&cfg);
            }
            Err(_) => {
                let mut cfg = AppConfig::default();
                cfg.log_monitor.default_filename = self.committed_live_name.clone();
                cfg.log_monitor.save_dir = self.live_dir.clone();
                cfg.log_monitor.open_log_files = paths;
                cfg.log_monitor.last_open_dir = self.open_dir.clone();
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
        self.live_name = name;
        let new_path = self.live_path();
        if let Some(old_path) = self.live_file.as_ref() {
            if old_path != &new_path && old_path.exists() {
                if let Err(err) = rename_log_path(old_path, &new_path) {
                    self.live_name = self.committed_live_name.clone();
                    self.status = if err.kind() == std::io::ErrorKind::AlreadyExists {
                        tr(lang, "status.rename_exists")
                    } else {
                        format!("{}: {err}", tr(lang, "status.rename_failed"))
                    };
                    return false;
                }
            }
            self.live_file = Some(new_path);
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
    }

    fn persist_live_log_name(&self) {
        self.persist_open_log_files();
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
        self.live_file = Some(path);
        Ok(())
    }

    fn append_to_live_file(&self, line: &str) {
        let Some(path) = &self.live_file else {
            return;
        };
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }

    fn start_monitor(&mut self, lang: Lang) {
        if self.monitoring {
            return;
        }
        if self.live_name.trim() != self.committed_live_name && !self.commit_live_name(lang) {
            return;
        }
        let Some(port) = self.ports.get(self.selected_port).cloned() else {
            self.status = tr(lang, "status.port_none");
            return;
        };
        let baud: u32 = match self.baud.parse() {
            Ok(v) if v > 0 => v,
            _ => {
                self.status = tr(lang, "status.baud_invalid");
                return;
            }
        };
        if let Err(e) = self.ensure_live_file() {
            self.status = format!("{}: {e}", tr(lang, "status.create_failed"));
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded();
        let stop_c = Arc::clone(&stop);
        let port_for_thread = port.clone();
        thread::spawn(move || serial_worker(port_for_thread, baud, stop_c, tx));
        self.stop_flag = Some(stop);
        self.rx = Some(rx);
        self.monitoring = true;
        self.status = tr_monitoring(lang, &port, baud);
        if let Some(live) = self.tabs.get_mut(0) {
            live.title = self.live_name.clone();
        }
        self.active_tab = 0;
    }

    fn stop_monitor(&mut self, lang: Lang) {
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
        self.rx = None;
        self.monitoring = false;
        self.status = tr(lang, "status.stopped");
    }

    fn new_live_log(&mut self, lang: Lang) {
        // Archive current live content as a named file tab if it has data
        if let Some(live) = self.tabs.first() {
            if !live.title.is_empty() {
                // Keep existing live as archived file tab copy is expensive — just reset live
                let _ = live;
            }
        }
        let stamp = Local::now().format("%Y%m%d_%H%M%S");
        self.live_name = format!("{} {}", tr(lang, "log.default_filename"), stamp);
        self.committed_live_name = self.live_name.clone();
        if let Some(live) = self.tabs.get_mut(0) {
            live.clear_display();
            live.title = self.live_name.clone();
        }
        self.live_file = None;
        let _ = self.ensure_live_file();
        self.active_tab = 0;
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

    pub fn drain_events(&mut self) {
        for tab in &mut self.tabs {
            tab.poll_background_tasks();
        }
        self.poll_file_loads();

        let mut budget = DRAIN_LINES_PER_FRAME;
        let mut batch = Vec::new();

        while budget > 0 && !self.pending_lines.is_empty() {
            batch.push(self.pending_lines.remove(0));
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
                    SerialEvent::Status(s) => self.status = s,
                    SerialEvent::Error(e) => {
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
            self.pending_lines.remove(0);
        }

        if stop_seen {
            self.monitoring = false;
            self.stop_flag = None;
            self.rx = None;
            self.status = tr(self.last_lang, "status.stopped");
        }

        for line in batch {
            self.append_to_live_file(&line);
            if let Some(live) = self.tabs.get_mut(0) {
                live.append_lines([line]);
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

    fn activate_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
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

        // Python log_panel: fixed left sidebar + content (must force top-down
        // layouts — parent horizontal_top would otherwise stack every control in a row).
        let sidebar_w = 172.0;
        let avail_h = ui.available_height();
        let avail_w = ui.available_width();

        ui.horizontal_top(|ui| {
            // ── Left connect sidebar ───────────────────────────────────
            ui.allocate_ui_with_layout(
                egui::vec2(sidebar_w, avail_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    Frame::NONE
                        .fill(t.surface_bg)
                        .inner_margin(Margin::symmetric(8, 6))
                        .corner_radius(CornerRadius::same(6))
                        .stroke(Stroke::new(1.0_f32, t.border))
                        .show(ui, |ui| {
                            ui.set_min_width(sidebar_w - 4.0);
                            ui.set_max_width(sidebar_w - 4.0);
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);

                            // All controls must use the same *actual* content
                            // width. A width derived from `sidebar_w` diverges
                            // from button width after Frame margins and DPI
                            // scaling, making ComboBoxes visibly narrower.
                            let ctrl_w = ui.available_width();

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
                                        "—"
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
                                            .selectable_label(!self.baud_custom && self.baud == b, &b)
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
                                        if self
                                            .baud_options
                                            .iter()
                                            .any(|b| b == &self.baud)
                                        {
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
                                    self.baud = self.baud.chars().filter(|c| c.is_ascii_digit()).collect();
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

                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.new")).clicked()
                            {
                                self.new_live_log(lang);
                            }
                            if ui_theme::secondary_button(ui, t, tr(lang, "btn.clear")).clicked()
                            {
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
                },
            );

            ui.add_space(8.0);

            // ── Right: file tabs + LogTabPage ─────────────────────────
            let content_w = (avail_w - sidebar_w - 16.0).max(200.0);
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
) -> String {
    if max_width <= 4.0 {
        return "…".to_string();
    }
    let measure = |text: &str| -> f32 {
        ui.fonts(|f| {
            f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if measure(title) <= max_width {
        return title.to_string();
    }
    let mut chars: Vec<char> = title.chars().collect();
    while !chars.is_empty() {
        let trial: String = chars.iter().collect();
        if measure(&format!("{trial}…")) <= max_width {
            return format!("{trial}…");
        }
        chars.pop();
    }
    "…".to_string()
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

fn serial_worker(port: String, baud: u32, stop: Arc<AtomicBool>, tx: Sender<SerialEvent>) {
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
                            // Python parity: live log shows raw timestamped ASK/FSK only;
                            // full decode is hover tooltip (Auto Parse), not an inline "| …".
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
