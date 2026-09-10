//! Data Analysis — frame-header + filter extraction, multi-series plot.
//!
//! Rules:
//! 1. Split file into frames by a user-defined frame header.
//! 2. Up to 8 series; each has a filter token. After a match, the number following
//!    whitespace is the sample (first match per filter per frame).
//! 3. Frames share one X axis (frame index 0..N-1, left→right).
//! 4. Per-series DataKind (mV / mA / °C) maps native values into shared display Y
//!    via `y_plot = raw * (y_scale / kind.ref_span()) + y_offset`.
//! 5. Open file / Apply rules → parse & plot. Serial Live ingests RX lines.

use crate::theme::{self as ui_theme, Tokens};
use crate::waveform_analysis::tek_channel_color;
use crate::data_extract_cache::{
    self, downsample_minmax, file_content_fp, list_cached_extracts, live_cache_path,
    live_flush_interval, load_wida, save_wida, WidaPayload, WidaSeriesData,
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Vec2b};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotBounds, PlotPoints, Points};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, tr_fmt, Lang};
use wiparse_core::paths::project_path;

const TOOLBAR_H: f32 = 40.0;
const SIDE_W: f32 = 280.0;
const PANEL_GAP: f32 = 8.0;
const CARD_MARGIN_X: i8 = 10;
const MAX_SERIES: usize = 8;
const VIEW_MIN_SPAN_ABS: f64 = 1e-12;
const VIEW_MAX_PAD: f64 = 8.0;
const VIEW_MAX_PAD_Y: f64 = 20.0;
const VIEW_MAX_ZOOM_X: f64 = 1e6;
const AXIS_HANDLE_RADIUS: f32 = 5.5;
const AXIS_TICK_LEN: f32 = 4.0;
const BAND_SPACING: f64 = 1.2;
const LIVE_FIT_FRAMES: usize = 80;
const LOD_POINTS_PER_PX: f32 = 3.0;
const SKIP_DOTS_ABOVE: usize = 2000;

/// One first-level subfolder under the browser root that contains data files.
struct DataBrowserFolder {
    name: String,
    files: Vec<(String, PathBuf)>,
}

struct PlotDrawItem {
    index: usize,
    label: String,
    color: Color32,
    line: Vec<[f64; 2]>,
    draw_dots: bool,
}

/// Background open/parse job (keeps UI responsive with a spinner).
struct PendingDataLoad {
    rx: Receiver<LoadEvent>,
}

enum LoadEvent {
    Progress(u8),
    Done(Result<DataLoadOk, String>),
}

struct SeriesDraft {
    label: String,
    kind: DataKind,
    points: Vec<[f64; 2]>,
    y_scale: f64,
    y_offset: f64,
    y_min: f64,
    y_max: f64,
    mapped: Vec<[f64; 2]>,
}

struct DataLoadOk {
    path: Option<PathBuf>,
    file_text: Option<Arc<String>>,
    frame_count: usize,
    series: Vec<SeriesDraft>,
    from_cache: bool,
    frame_header: Option<String>,
    filters: Option<Vec<String>>,
    kinds: Option<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataKind {
    Voltage,
    Current,
    Temperature,
}

impl DataKind {
    fn unit(self) -> &'static str {
        match self {
            Self::Voltage => "mV",
            Self::Current => "mA",
            Self::Temperature => "\u{00B0}C",
        }
    }

    /// Full-scale native span for default display mapping (≈10 V / 5 A / 50 °C).
    fn ref_span(self) -> f64 {
        match self {
            Self::Voltage => 10_000.0, // mV
            Self::Current => 5_000.0,  // mA
            Self::Temperature => 50.0,
        }
    }

    fn as_config_str(self) -> &'static str {
        match self {
            Self::Voltage => "voltage",
            Self::Current => "current",
            Self::Temperature => "temperature",
        }
    }

    fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "current" => Self::Current,
            "temperature" | "temp" => Self::Temperature,
            _ => Self::Voltage,
        }
    }

    fn label(self, lang: Lang) -> String {
        match self {
            Self::Voltage => tr(lang, "data.kind_voltage"),
            Self::Current => tr(lang, "data.kind_current"),
            Self::Temperature => tr(lang, "data.kind_temperature"),
        }
    }

    /// Threshold dashed-line color: mV yellow / mA blue / °C orange.
    fn threshold_color(self) -> Color32 {
        match self {
            Self::Voltage => Color32::from_rgb(0xF7, 0xD6, 0x18),
            Self::Current => Color32::from_rgb(0x3B, 0x82, 0xF6),
            Self::Temperature => Color32::from_rgb(0xFF, 0x9F, 0x0A),
        }
    }

    fn short_axis(self) -> &'static str {
        match self {
            Self::Voltage => "mV",
            Self::Current => "mA",
            Self::Temperature => "\u{00B0}C",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataSource {
    File,
    SerialLive,
}

impl DataSource {
    fn as_config_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::SerialLive => "serial",
        }
    }

    fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "serial" | "serial_live" | "live" => Self::SerialLive,
            _ => Self::File,
        }
    }
}

#[derive(Clone)]
struct DataSeries {
    label: String,
    color: Color32,
    kind: DataKind,
    /// Samples as (frame_index, native value) — at most one point per frame.
    points: Vec<[f64; 2]>,
    y_scale: f64,
    y_offset: f64,
    /// Native Y extents (raw values).
    y_min: f64,
    y_max: f64,
    /// Display-space cache: `[x, display_y]` rebuilt when scale/offset/kind/points dirty.
    mapped: Vec<[f64; 2]>,
    disp_dirty: bool,
}

pub struct DataAnalysisPanel {
    status: String,
    browser_dir: String,
    browser_scanned_dir: String,
    browser_folders: Vec<DataBrowserFolder>,
    last_open_path: Option<PathBuf>,
    /// Raw file text kept for re-parse when rules change (file mode).
    file_text: Option<Arc<String>>,

    frame_header: String,
    series_count: usize,
    filters: Vec<String>,
    kinds: Vec<DataKind>,

    series: Vec<DataSeries>,
    frame_count: usize,
    selected: Option<usize>,
    dragging_offset: Option<usize>,
    dragging_scale: Option<usize>,

    source: DataSource,
    live_cursor: usize,
    live_tail: String,
    live_auto_fit: bool,
    /// Set when live ingest added frames; cleared after UI observes it.
    live_plot_dirty: bool,
    live_session_id: String,
    live_frames_since_flush: usize,

    /// Native thresholds per physical kind + enable flags.
    thr_voltage: f64,
    thr_voltage_on: bool,
    thr_current: f64,
    thr_current_on: bool,
    thr_temperature: f64,
    thr_temperature_on: bool,

    fit_request: bool,
    pending_bounds: Option<PlotBounds>,
    last_x_range: Option<(f64, f64)>,
    last_y_range: Option<(f64, f64)>,
    /// Display-space extent (after kind/scale/offset mapping).
    cached_extent: Option<(f64, f64, f64, f64)>,
    /// Viewport LOD key: (x0, x1, pixel_w_rounded) + per-series envelopes.
    lod_key: Option<(i64, i64, i32)>,
    lod_lines: Vec<Vec<[f64; 2]>>,
    /// Background file parse / cache load.
    pending_load: Option<PendingDataLoad>,
    /// Displayed 0–100 while `pending_load` is active (eases toward target).
    load_progress: u8,
    /// Latest progress reported by the worker.
    load_progress_target: u8,
}

impl DataAnalysisPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let da = &cfg.apps.data_analysis;
        let browser_dir = da.browser_dir.clone();
        let series_count = (da.series_count as usize).clamp(1, MAX_SERIES);
        let mut filters = da.filters.clone();
        while filters.len() < MAX_SERIES {
            filters.push(String::new());
        }
        filters.truncate(MAX_SERIES);

        let mut kinds = Vec::with_capacity(MAX_SERIES);
        for i in 0..MAX_SERIES {
            let s = da.kinds.get(i).map(|s| s.as_str()).unwrap_or("voltage");
            kinds.push(DataKind::from_config_str(s));
        }

        let source = DataSource::from_config_str(&da.source);
        let live_session_id = format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        let mut panel = Self {
            status: String::new(),
            browser_dir,
            browser_scanned_dir: String::new(),
            browser_folders: Vec::new(),
            last_open_path: None,
            file_text: None,
            frame_header: da.frame_header.clone(),
            series_count,
            filters,
            kinds,
            series: Vec::new(),
            frame_count: 0,
            selected: None,
            dragging_offset: None,
            dragging_scale: None,
            source,
            live_cursor: 0,
            live_tail: String::new(),
            live_auto_fit: matches!(source, DataSource::SerialLive),
            live_plot_dirty: false,
            live_session_id,
            live_frames_since_flush: 0,
            thr_voltage: da.threshold_voltage,
            thr_voltage_on: da.threshold_voltage_on,
            thr_current: da.threshold_current,
            thr_current_on: da.threshold_current_on,
            thr_temperature: da.threshold_temperature,
            thr_temperature_on: da.threshold_temperature_on,
            fit_request: false,
            pending_bounds: None,
            last_x_range: None,
            last_y_range: None,
            cached_extent: None,
            lod_key: None,
            lod_lines: Vec::new(),
            pending_load: None,
            load_progress: 0,
            load_progress_target: 0,
        };
        panel.refresh_data_browser();
        panel
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn needs_repaint(&self) -> bool {
        if self.pending_load.is_some() {
            return true;
        }
        matches!(self.source, DataSource::SerialLive)
            && (self.live_plot_dirty
                || self.fit_request
                || self.pending_bounds.is_some()
                || self.dragging_offset.is_some()
                || self.dragging_scale.is_some())
    }

    pub fn is_loading(&self) -> bool {
        self.pending_load.is_some()
    }

    /// Call after UI frame so idle live mode stops pumping at 33ms.
    pub fn clear_live_plot_dirty(&mut self) {
        self.live_plot_dirty = false;
    }

    pub fn wants_live_serial(&self) -> bool {
        matches!(self.source, DataSource::SerialLive)
    }

    pub fn live_line_cursor(&self) -> usize {
        self.live_cursor
    }

    /// Cap initial catch-up so enabling Serial Live does not re-parse a huge log.
    pub fn prepare_live_poll(&mut self, total_lines: usize) {
        if !matches!(self.source, DataSource::SerialLive) {
            return;
        }
        const MAX_CATCHUP: usize = 2000;
        if self.live_cursor == 0 && total_lines > MAX_CATCHUP {
            self.live_cursor = total_lines - MAX_CATCHUP;
        }
    }

    /// Append serial RX lines; `next_cursor` becomes the new live cursor after ingest.
    pub fn ingest_live_lines(&mut self, lines: &[String], next_cursor: usize, lang: Lang) {
        if !matches!(self.source, DataSource::SerialLive) {
            return;
        }
        if lines.is_empty() {
            self.live_cursor = next_cursor;
            return;
        }

        if !self.live_tail.is_empty() && !self.live_tail.ends_with('\n') {
            self.live_tail.push('\n');
        }
        for line in lines {
            self.live_tail.push_str(line);
            self.live_tail.push('\n');
        }
        const MAX_LIVE_TAIL: usize = 256 * 1024;
        if self.live_tail.len() > MAX_LIVE_TAIL {
            let mut cut = self.live_tail.len() - MAX_LIVE_TAIL / 2;
            while cut < self.live_tail.len() && !self.live_tail.is_char_boundary(cut) {
                cut += 1;
            }
            self.live_tail = self.live_tail[cut..].to_owned();
        }

        let header = self.frame_header.trim().to_owned();
        if header.is_empty() {
            self.live_cursor = next_cursor;
            self.status = tr(lang, "data.status_need_header");
            return;
        }

        let filters: Vec<String> = self.filters[..self.series_count]
            .iter()
            .map(|s| s.trim().to_owned())
            .collect();
        let any_filter = filters.iter().any(|f| !f.is_empty());
        if !any_filter {
            self.live_cursor = next_cursor;
            self.status = tr(lang, "data.status_need_filter");
            return;
        }

        self.ensure_series_shells(lang);

        let frames = split_frames(&self.live_tail, &header);
        if frames.len() < 2 {
            self.live_cursor = next_cursor;
            if self.frame_count == 0 {
                self.status = tr(lang, "data.status_live_wait");
            }
            return;
        }

        let complete = &frames[..frames.len() - 1];
        let remain = frames.last().copied().unwrap_or("");
        let base_fi = self.frame_count;
        const MAX_LIVE_POINTS: usize = 200_000;

        for (rel, frame) in complete.iter().enumerate() {
            let x = (base_fi + rel) as f64;
            for (si, filter) in filters.iter().enumerate() {
                if filter.is_empty() {
                    continue;
                }
                if let Some(value) = extract_first_filter_value(frame, filter) {
                    let s = &mut self.series[si];
                    if s.points.is_empty() {
                        s.y_min = value;
                        s.y_max = value;
                    } else {
                        s.y_min = s.y_min.min(value);
                        s.y_max = s.y_max.max(value);
                    }
                    s.points.push([x, value]);
                    if s.points.len() > MAX_LIVE_POINTS {
                        let drop_n = s.points.len() - MAX_LIVE_POINTS;
                        s.points.drain(..drop_n);
                    }
                    s.disp_dirty = true;
                }
            }
        }

        let added = complete.len();
        self.frame_count = base_fi + added;
        self.live_tail = remain.to_owned();
        self.live_cursor = next_cursor;
        self.live_plot_dirty = true;
        self.invalidate_lod();
        if base_fi == 0 {
            self.auto_layout_scales();
        }
        self.refresh_cached_extent();

        if self.live_auto_fit {
            if let Some(bounds) = self.live_fit_bounds() {
                self.pending_bounds = Some(bounds);
                self.fit_request = false;
            }
        }

        self.live_frames_since_flush += added;
        if self.live_frames_since_flush >= live_flush_interval() {
            self.flush_live_cache();
        }

        let points: usize = self.series.iter().map(|s| s.points.len()).sum();
        if self.frame_count == 0 {
            self.status = tr(lang, "data.status_live_wait");
        } else {
            self.status = status_with_counts(lang, "data.status_live", self.frame_count, points);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        self.poll_pending_load(lang);
        if self.pending_load.is_some() {
            // Keep spinner animating even if this frame started before the job was queued.
            ui.ctx().request_repaint();
        }
        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        if !full.is_positive() {
            return;
        }

        let toolbar_rect =
            egui::Rect::from_min_size(full.min, egui::vec2(full.width(), TOOLBAR_H));
        let body_top = full.min.y + TOOLBAR_H + PANEL_GAP;
        let body_h = (full.max.y - body_top).max(1.0);
        let side_w = SIDE_W
            .min(full.width() * 0.36)
            .max(240.0)
            .min(full.width() - 140.0);
        let plot_w = (full.width() - side_w - PANEL_GAP).max(120.0);
        let side_rect =
            egui::Rect::from_min_size(egui::pos2(full.min.x, body_top), egui::vec2(side_w, body_h));
        let plot_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, body_top),
            egui::vec2(plot_w, body_h),
        );

        let browser_h = (body_h * 0.42).clamp(140.0, body_h - 160.0);
        let rules_h = (body_h - browser_h - PANEL_GAP).max(140.0);
        let browser_rect =
            egui::Rect::from_min_size(side_rect.min, egui::vec2(side_w, browser_h));
        let rules_rect = egui::Rect::from_min_size(
            egui::pos2(side_rect.min.x, side_rect.min.y + browser_h + PANEL_GAP),
            egui::vec2(side_w, rules_h),
        );

        panel_in_rect(ui, toolbar_rect, |ui| self.toolbar(ui, lang, tokens));
        panel_in_rect(ui, browser_rect, |ui| {
            if matches!(self.source, DataSource::SerialLive) {
                self.live_status_panel(ui, lang, tokens);
            } else {
                self.browser_panel(ui, lang, tokens);
            }
        });
        panel_in_rect(ui, rules_rect, |ui| self.rules_panel(ui, lang, tokens));
        card_in_rect(ui, plot_rect, tokens, &tr(lang, "data.plot_title"), |ui| {
            self.plot_area(ui, lang, tokens);
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
                    if ui_theme::primary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "data.open_file"),
                        egui::vec2(100.0, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.pick_open_file(lang);
                    }
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "data.clear"),
                        egui::vec2(64.0, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.clear_data(lang);
                    }
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "data.fit"),
                        egui::vec2(64.0, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.fit_request = true;
                        self.live_auto_fit = false;
                    }
                    let z = egui::vec2(36.0, ui_theme::CTRL_H);
                    if ui_theme::ghost_btn_sized(ui, tokens, "X+", z, false).clicked() {
                        self.request_zoom_axis(true, 0.5);
                    }
                    if ui_theme::ghost_btn_sized(ui, tokens, "X−", z, false).clicked() {
                        self.request_zoom_axis(true, 2.0);
                    }
                    if ui_theme::ghost_btn_sized(ui, tokens, "Y+", z, false).clicked() {
                        self.apply_y_zoom_factor(2.0);
                    }
                    if ui_theme::ghost_btn_sized(ui, tokens, "Y−", z, false).clicked() {
                        self.apply_y_zoom_factor(0.5);
                    }
                    match self.source {
                        DataSource::SerialLive => {
                            ui.label(
                                RichText::new(tr(lang, "data.source_serial"))
                                    .size(ui_theme::FONT_CAPTION)
                                    .strong()
                                    .color(tokens.accent),
                            );
                        }
                        DataSource::File => {
                            if let Some(path) = &self.last_open_path {
                                let name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(name)
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                )
                                .on_hover_text(path.display().to_string());
                            }
                        }
                    }
                });
            });
    }

    fn live_status_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 8))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                ui.label(
                    RichText::new(tr(lang, "data.source_serial"))
                        .size(ui_theme::FONT_TITLE)
                        .strong()
                        .color(tokens.text_primary),
                );
                ui.label(
                    RichText::new(tr(lang, "data.status_live_wait"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );
                ui.label(
                    RichText::new(tr(lang, "data.axis_hint"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );
                let points: usize = self.series.iter().map(|s| s.points.len()).sum();
                if self.frame_count > 0 {
                    ui.label(
                        RichText::new(status_with_counts(
                            lang,
                            "data.status_live",
                            self.frame_count,
                            points,
                        ))
                        .size(ui_theme::FONT_BODY)
                        .color(tokens.text_primary),
                    );
                }
            });
    }

    fn rules_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        let mut apply_now = false;
        let mut count_changed = false;
        let mut source_changed = false;
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 8))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 5.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(lang, "data.rules"))
                            .size(ui_theme::FONT_TITLE)
                            .strong()
                            .color(tokens.text_primary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui_theme::primary_btn_sized(
                            ui,
                            tokens,
                            tr(lang, "data.apply"),
                            egui::vec2(72.0, 24.0),
                        )
                        .clicked()
                        {
                            apply_now = true;
                        }
                    });
                });

                ui.label(
                    RichText::new(tr(lang, "data.rules_hint"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(lang, "data.source"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    let mut src = self.source;
                    let src_label = match src {
                        DataSource::File => tr(lang, "data.source_file"),
                        DataSource::SerialLive => tr(lang, "data.source_serial"),
                    };
                    egui::ComboBox::from_id_salt("data_source")
                        .selected_text(src_label)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut src,
                                DataSource::File,
                                tr(lang, "data.source_file"),
                            );
                            ui.selectable_value(
                                &mut src,
                                DataSource::SerialLive,
                                tr(lang, "data.source_serial"),
                            );
                        });
                    if src != self.source {
                        self.source = src;
                        source_changed = true;
                    }
                });

                ui.label(
                    RichText::new(tr(lang, "data.frame_header"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );
                let header_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.frame_header)
                        .desired_width(inner_w)
                        .hint_text(tr(lang, "data.frame_header_hint"))
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if header_edit.lost_focus() {
                    apply_now = true;
                }

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(lang, "data.series_count"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    let mut count = self.series_count;
                    egui::ComboBox::from_id_salt("data_series_count")
                        .selected_text(count.to_string())
                        .width(56.0)
                        .show_ui(ui, |ui| {
                            for n in 1..=MAX_SERIES {
                                ui.selectable_value(&mut count, n, n.to_string());
                            }
                        });
                    if count != self.series_count {
                        self.series_count = count;
                        count_changed = true;
                        apply_now = true;
                    }
                });

                let list_h = (ui.available_height() - 90.0).max(48.0);
                egui::ScrollArea::vertical()
                    .id_salt("data_filters_scroll")
                    .max_height(list_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(inner_w);
                        for i in 0..self.series_count {
                            let color = series_color(i);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let (swatch, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 10.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(swatch, CornerRadius::same(2), color);
                                ui.label(
                                    RichText::new(format!("{}", i + 1))
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                );
                                let filter_w = (inner_w - 100.0).max(80.0);
                                let edit = ui.add(
                                    egui::TextEdit::singleline(&mut self.filters[i])
                                        .desired_width(filter_w)
                                        .hint_text(tr(lang, "data.filter_hint"))
                                        .margin(egui::vec2(4.0, 3.0)),
                                );
                                if edit.lost_focus() {
                                    apply_now = true;
                                }
                                let mut kind = self.kinds[i];
                                egui::ComboBox::from_id_salt(("data_kind", i))
                                    .selected_text(kind.short_axis())
                                    .width(56.0)
                                    .show_ui(ui, |ui| {
                                        for k in [
                                            DataKind::Voltage,
                                            DataKind::Current,
                                            DataKind::Temperature,
                                        ] {
                                            ui.selectable_value(
                                                &mut kind,
                                                k,
                                                format!("{} ({})", k.label(lang), k.short_axis()),
                                            );
                                        }
                                    });
                                if kind != self.kinds[i] {
                                    self.kinds[i] = kind;
                                    apply_now = true;
                                }
                            });
                            ui.add_space(2.0);
                        }
                    });

                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr(lang, "data.thresholds"))
                        .size(ui_theme::FONT_CAPTION)
                        .strong()
                        .color(tokens.text_primary),
                );
                ui.label(
                    RichText::new(tr(lang, "data.threshold_hint"))
                        .size(10.0)
                        .color(tokens.text_muted),
                );
                let mut thr_dirty = false;
                thr_dirty |= threshold_row(
                    ui,
                    tokens,
                    lang,
                    DataKind::Voltage,
                    &mut self.thr_voltage_on,
                    &mut self.thr_voltage,
                );
                thr_dirty |= threshold_row(
                    ui,
                    tokens,
                    lang,
                    DataKind::Current,
                    &mut self.thr_current_on,
                    &mut self.thr_current,
                );
                thr_dirty |= threshold_row(
                    ui,
                    tokens,
                    lang,
                    DataKind::Temperature,
                    &mut self.thr_temperature_on,
                    &mut self.thr_temperature,
                );
                if thr_dirty {
                    self.persist_rules();
                }
            });

        if source_changed {
            self.on_source_changed(lang);
            self.persist_rules();
        } else if apply_now || count_changed {
            self.persist_rules();
            self.apply_rules(lang);
        }
    }

    fn browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 8))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = inner_w;

                ui.label(
                    RichText::new(tr(lang, "data.browser_dir"))
                        .size(ui_theme::FONT_TITLE)
                        .strong()
                        .color(tokens.text_primary),
                );
                let browser_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.browser_dir)
                        .desired_width(ctrl_w)
                        .hint_text(tr(lang, "data.browser_hint"))
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if browser_edit.lost_focus() {
                    self.refresh_data_browser();
                    self.persist_browser_dir();
                }

                let gap = 6.0;
                let btn_w = ((ctrl_w - gap) * 0.5).max(48.0);
                ui.horizontal(|ui| {
                    ui.set_max_width(ctrl_w);
                    ui.spacing_mut().item_spacing.x = gap;
                    if ui
                        .add_sized(
                            egui::vec2(btn_w, ui_theme::CTRL_H),
                            egui::Button::new(tr(lang, "btn.browse_dir")),
                        )
                        .clicked()
                    {
                        self.pick_browser_dir();
                    }
                    if ui
                        .add_sized(
                            egui::vec2(btn_w, ui_theme::CTRL_H),
                            egui::Button::new(tr(lang, "btn.refresh_browser")),
                        )
                        .clicked()
                    {
                        self.refresh_data_browser();
                    }
                });

                self.ensure_data_browser_fresh();

                if self.browser_dir.trim().is_empty() && self.browser_folders.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "data.browser_hint"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                } else if self.browser_folders.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "data.browser_empty"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                } else {
                    let list_h = ui.available_height().max(72.0);
                    let selected_path = self.last_open_path.clone();
                    let mut open_path: Option<PathBuf> = None;
                    ui.spacing_mut().scroll.floating = false;
                    egui::ScrollArea::vertical()
                        .id_salt("data_browser_tree")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ctrl_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for folder in &self.browser_folders {
                                let header =
                                    format!("{}  ({})", folder.name, folder.files.len());
                                egui::CollapsingHeader::new(
                                    RichText::new(header)
                                        .size(ui_theme::FONT_BODY)
                                        .strong()
                                        .color(tokens.text_primary),
                                )
                                .id_salt(("data-browser-folder", folder.name.as_str()))
                                .default_open(false)
                                .show(ui, |ui| {
                                    ui.set_max_width(ctrl_w);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    for (name, path) in &folder.files {
                                        let selected = selected_path
                                            .as_ref()
                                            .is_some_and(|p| same_path(p, path));
                                        let resp = ui.selectable_label(
                                            selected,
                                            RichText::new(name)
                                                .size(ui_theme::FONT_BODY)
                                                .color(tokens.text_primary),
                                        );
                                        if resp.clicked() {
                                            open_path = Some(path.clone());
                                        }
                                    }
                                });
                            }
                            ui.add_space(8.0);
                        });
                    if let Some(path) = open_path {
                        self.open_file(path, lang);
                    }
                }
            });
    }

    fn plot_area(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let height = ui.available_height().max(80.0);
        let loading = self.is_loading();
        let has_points = self.series.iter().any(|s| !s.points.is_empty());
        if loading && !has_points {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), height),
                egui::Sense::hover(),
            );
            paint_loading_in_rect(ui, tokens, lang, rect, self.load_progress);
            return;
        }
        if !has_points {
            let hint = self.empty_plot_hint(lang);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(RichText::new(hint).color(tokens.text_muted));
                },
            );
            return;
        }

        let (ctrl, shift) = ui.input(|i| {
            (
                i.modifiers.ctrl || i.modifiers.command || i.modifiers.mac_cmd,
                i.modifiers.shift,
            )
        });
        let allow_zoom = if ctrl && !shift {
            Vec2b::new(true, false)
        } else {
            Vec2b::FALSE
        };
        let y_wheel = if ctrl && shift {
            ctrl_shift_wheel_scroll(ui)
        } else {
            0.0
        };

        self.ensure_display_mapped();

        let fit = self.fit_request;
        let pending = self.pending_bounds.take();
        let fit_bounds = if fit {
            self.data_extent().map(|ext| self.clamp_view_bounds(ext))
        } else {
            None
        };
        let pending_clamped = pending.map(|b| self.clamp_view_bounds(b));
        let extent_for_clamp = self.data_extent();

        let mut next_x_range = None;
        let mut next_y_range = None;
        let x_label = tr(lang, "data.axis_x");
        let y_label = self.y_axis_label_for_plot(lang);
        let thresholds = self.threshold_display_lines();
        let selected = self.selected;
        let live = matches!(self.source, DataSource::SerialLive);

        let total_w = ui.available_width();
        let nch = self.series.iter().filter(|s| !s.points.is_empty()).count();
        let strip_w = if nch > 0 { 56.0 } else { 0.0 };
        let plot_w = (total_w - strip_w - 3.0).max(80.0);
        let plot_width_px = plot_w.max(64.0);

        // Build draw lists outside the plot closure (no per-frame series.clone).
        let (x0_hint, x1_hint) = self.last_x_range.unwrap_or_else(|| {
            self.cached_extent
                .map(|(a, b, _, _)| (a, b))
                .unwrap_or((0.0, 1.0))
        });
        let draw_items = self.build_plot_draw_items(x0_hint, x1_hint, plot_width_px);

        let mut y_for_axes = self.last_y_range;
        let mut pointer_xy: Option<(f64, f64)> = None;
        let mut plot_hovered = false;
        let mut plot_clicked = false;
        let mut plot_rect: Option<egui::Rect> = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.allocate_ui_with_layout(
                egui::vec2(plot_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(plot_w);
                    ui.set_max_width(plot_w);
                    plot_rect = Some(ui.max_rect());
                    ui_theme::with_plot_well_visuals(ui, tokens, |ui| {
                        let response = Plot::new("data_analysis_plot")
                            .height(height)
                            .allow_zoom(allow_zoom)
                            .allow_drag(true)
                            .allow_scroll(!ctrl)
                            .allow_boxed_zoom(true)
                            .x_axis_label(x_label)
                            .y_axis_label(y_label)
                            .show(ui, |plot_ui| {
                                let mut applied: Option<PlotBounds> = None;
                                if let Some(clamped) = fit_bounds {
                                    plot_ui.set_plot_bounds(clamped);
                                    applied = Some(clamped);
                                } else if fit {
                                    plot_ui.set_auto_bounds(Vec2b::TRUE);
                                }
                                if let Some(clamped) = pending_clamped {
                                    plot_ui.set_plot_bounds(clamped);
                                    applied = Some(clamped);
                                }

                                let bounds = applied.unwrap_or_else(|| plot_ui.plot_bounds());
                                next_x_range = Some((bounds.min()[0], bounds.max()[0]));
                                next_y_range = Some((bounds.min()[1], bounds.max()[1]));

                                if let Some(p) = plot_ui.pointer_coordinate() {
                                    pointer_xy = Some((p.x, p.y));
                                }

                                for item in &draw_items {
                                    let width = if selected == Some(item.index) {
                                        2.2_f32
                                    } else {
                                        1.5_f32
                                    };
                                    let pts = PlotPoints::from_iter(item.line.iter().copied());
                                    plot_ui.line(
                                        Line::new(pts)
                                            .name(item.label.clone())
                                            .color(item.color)
                                            .width(width),
                                    );
                                    if item.draw_dots {
                                        let dots =
                                            PlotPoints::from_iter(item.line.iter().copied());
                                        plot_ui.points(
                                            Points::new(dots).color(item.color).radius(2.4_f32),
                                        );
                                    }
                                }

                                for (kind, y_plot, native) in &thresholds {
                                    let name = format!(
                                        "{} {}{}",
                                        kind.short_axis(),
                                        format_eng_short(*native),
                                        kind.unit()
                                    );
                                    plot_ui.hline(
                                        HLine::new(*y_plot)
                                            .color(kind.threshold_color())
                                            .width(1.6_f32)
                                            .style(LineStyle::dashed_dense())
                                            .name(name),
                                    );
                                }

                                if applied.is_none() {
                                    if let Some(extent) = extent_for_clamp {
                                        let live_b = plot_ui.plot_bounds();
                                        let clamped =
                                            clamp_view_bounds_static(extent, live_b);
                                        if (clamped.min()[0] - live_b.min()[0]).abs() > 1e-12
                                            || (clamped.max()[0] - live_b.max()[0]).abs() > 1e-12
                                            || (clamped.min()[1] - live_b.min()[1]).abs() > 1e-12
                                            || (clamped.max()[1] - live_b.max()[1]).abs() > 1e-12
                                        {
                                            plot_ui.set_plot_bounds(clamped);
                                            next_x_range =
                                                Some((clamped.min()[0], clamped.max()[0]));
                                            next_y_range =
                                                Some((clamped.min()[1], clamped.max()[1]));
                                        }
                                    }
                                }
                            });
                        plot_hovered = response.response.hovered();
                        plot_clicked = response.response.clicked();
                        plot_rect = Some(response.response.rect);
                    });
                },
            );

            y_for_axes = next_y_range.or(y_for_axes);
            if strip_w > 0.0 {
                let (y0, y1) = y_for_axes.unwrap_or((-1.0, 1.0));
                ui.allocate_ui_with_layout(
                    egui::vec2(strip_w, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(strip_w);
                        ui.set_max_width(strip_w);
                        self.series_offset_axes(ui, tokens, lang, y0, y1);
                    },
                );
            }
        });

        if fit {
            self.fit_request = false;
        }
        self.last_x_range = next_x_range.or(self.last_x_range);
        self.last_y_range = next_y_range.or(self.last_y_range);

        // Rebuild LOD for the settled viewport if it moved (next frame uses it).
        if let Some((x0, x1)) = self.last_x_range {
            self.ensure_viewport_lod(x0, x1, plot_width_px);
        }

        if plot_clicked {
            if let Some((px, py)) = pointer_xy {
                if let Some(i) = self.nearest_series_at(px, py) {
                    self.selected = Some(i);
                }
            }
        }

        if y_wheel != 0.0 && plot_hovered {
            let zoom_speed = ui.ctx().options(|o| o.scroll_zoom_speed);
            let zoom = (zoom_speed * y_wheel).exp() as f64;
            self.apply_y_zoom_factor(zoom);
        }

        if let Some(rect) = plot_rect {
            let cursor_x = pointer_xy.map(|(x, _)| x).filter(|_| plot_hovered);
            self.paint_series_readout(ui, tokens, lang, rect, cursor_x, live);
            if loading {
                paint_loading_overlay(ui, tokens, lang, rect, self.load_progress);
            }
        }
    }

    /// Top-right readout: color + label + value (latest for live, cursor-X for file).
    fn paint_series_readout(
        &self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        lang: Lang,
        plot_rect: egui::Rect,
        cursor_x: Option<f64>,
        live: bool,
    ) {
        let mut rows: Vec<(usize, Color32, String, String)> = Vec::new();
        for (i, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let val = if live {
                s.points.last().map(|p| p[1])
            } else if let Some(x) = cursor_x {
                value_at_x(&s.points, x)
            } else {
                s.points.last().map(|p| p[1])
            };
            let text = match val {
                Some(v) => format!("{}{}", format_plain(v), s.kind.unit()),
                None => "—".into(),
            };
            let label = if s.label.is_empty() {
                format!("{}", i + 1)
            } else {
                s.label.clone()
            };
            rows.push((i, s.color, label, text));
        }
        if rows.is_empty() {
            return;
        }

        let mode = if live {
            tr(lang, "data.readout_live")
        } else if cursor_x.is_some() {
            tr(lang, "data.readout_cursor")
        } else {
            tr(lang, "data.readout_latest")
        };

        let panel_w = 152.0_f32;
        let row_h = 16.0_f32;
        let panel_h = 18.0 + rows.len() as f32 * row_h + 8.0;
        let origin = egui::pos2(
            plot_rect.right() - panel_w - 8.0,
            plot_rect.top() + 8.0,
        );
        let panel = egui::Rect::from_min_size(origin, egui::vec2(panel_w, panel_h));
        let painter = ui.painter();
        painter.rect_filled(
            panel,
            CornerRadius::same(4),
            tokens.surface_bg.gamma_multiply(0.92),
        );
        painter.rect_stroke(
            panel,
            CornerRadius::same(4),
            Stroke::new(1.0_f32, tokens.divider),
            egui::StrokeKind::Inside,
        );
        painter.text(
            egui::pos2(panel.left() + 8.0, panel.top() + 4.0),
            egui::Align2::LEFT_TOP,
            mode,
            egui::FontId::proportional(10.0),
            tokens.text_muted,
        );
        let mut y = panel.top() + 18.0;
        for (idx, color, label, text) in &rows {
            if self.selected == Some(*idx) {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(panel.left() + 2.0, y),
                        egui::vec2(panel.width() - 4.0, row_h),
                    ),
                    CornerRadius::same(2),
                    color.gamma_multiply(0.18),
                );
            }
            let sw = egui::Rect::from_center_size(
                egui::pos2(panel.left() + 12.0, y + row_h * 0.5),
                egui::vec2(8.0, 8.0),
            );
            painter.rect_filled(sw, CornerRadius::same(2), *color);
            let name = if label.chars().count() > 10 {
                format!("{}…", label.chars().take(9).collect::<String>())
            } else {
                label.clone()
            };
            painter.text(
                egui::pos2(panel.left() + 20.0, y + row_h * 0.5),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(11.0),
                tokens.text_primary,
            );
            painter.text(
                egui::pos2(panel.right() - 6.0, y + row_h * 0.5),
                egui::Align2::RIGHT_CENTER,
                text,
                egui::FontId::monospace(11.0),
                *color,
            );
            y += row_h;
        }
    }

    fn nearest_series_at(&self, x: f64, y_plot: f64) -> Option<usize> {
        let mut best = None;
        let mut best_d = f64::INFINITY;
        for (i, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let Some(native) = value_at_x(&s.points, x) else {
                continue;
            };
            let dy = (display_y(native, s.y_scale, s.y_offset, s.kind) - y_plot).abs();
            if dy < best_d {
                best_d = dy;
                best = Some(i);
            }
        }
        best
    }

    /// Right-side axis strip: handles at y_offset (ground), select / drag / wheel.
    fn series_offset_axes(
        &mut self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        lang: Lang,
        y0: f64,
        y1: f64,
    ) {
        let n = self.series.len();
        if n == 0 || !self.series.iter().any(|s| !s.points.is_empty()) {
            return;
        }
        let (y_lo, y_hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let y_span = (y_hi - y_lo).max(1e-30);
        let strip_h = ui.available_height();
        let strip_w = ui.available_width();
        let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command || i.modifiers.mac_cmd);
        let shift = ui.input(|i| i.modifiers.shift);

        let sel_idx = self
            .selected
            .filter(|&i| i < n && !self.series[i].points.is_empty())
            .or_else(|| {
                self.series
                    .iter()
                    .position(|s| !s.points.is_empty())
            });
        let (sel_kind, sel_color, sel_scale, sel_offset) = sel_idx
            .map(|i| {
                let s = &self.series[i];
                (s.kind, s.color, s.y_scale, s.y_offset)
            })
            .unwrap_or((DataKind::Voltage, tokens.text_muted, 1.0, 0.0));
        let y_unit = sel_kind.unit();

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(strip_w, strip_h),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(2), tokens.surface_bg);
        painter.line_segment(
            [rect.left_top(), rect.left_bottom()],
            Stroke::new(1.0_f32, tokens.border),
        );

        // Header: selected series unit + per-division (native units of that series).
        let header_h = 22.0_f32;
        let native_lo = plot_y_to_native(y_lo, sel_scale, sel_offset, sel_kind);
        let native_hi = plot_y_to_native(y_hi, sel_scale, sel_offset, sel_kind);
        let per_div = ((native_hi - native_lo).abs() / 8.0).max(0.0);
        painter.text(
            egui::pos2(rect.center().x, rect.top() + 4.0),
            egui::Align2::CENTER_TOP,
            sel_kind.short_axis(),
            egui::FontId::monospace(10.0),
            sel_color,
        );
        painter.text(
            egui::pos2(rect.center().x, rect.top() + 14.0),
            egui::Align2::CENTER_TOP,
            format!("{}{}/div", format_eng_short(per_div), y_unit),
            egui::FontId::monospace(7.5),
            tokens.text_muted,
        );

        // Nice ticks in native units of the selected (or first) series.
        let tick_x0 = rect.left() + 1.0;
        let tick_x1 = tick_x0 + AXIS_TICK_LEN;
        let axis_top = rect.top() + header_h;
        let axis_bot = rect.bottom() - 2.0;
        if axis_bot > axis_top + 8.0 {
            let (n0, n1) = if native_lo <= native_hi {
                (native_lo, native_hi)
            } else {
                (native_hi, native_lo)
            };
            let ticks = nice_axis_ticks(n0, n1, 5);
            for (i, nv) in ticks.iter().enumerate() {
                let y_plot = display_y(*nv, sel_scale, sel_offset, sel_kind);
                let frac = ((y_plot - y_lo) / y_span).clamp(0.0, 1.0);
                let y = egui::lerp(axis_bot..=axis_top, frac as f32);
                painter.line_segment(
                    [egui::pos2(tick_x0, y), egui::pos2(tick_x1, y)],
                    Stroke::new(0.6_f32, tokens.border.gamma_multiply(0.75)),
                );
                let with_unit = i == 0 || i + 1 == ticks.len();
                painter.text(
                    egui::pos2(rect.right() - 2.0, y),
                    egui::Align2::RIGHT_CENTER,
                    format_axis_tick(*nv, y_unit, with_unit),
                    egui::FontId::monospace(7.5),
                    if self.selected.is_some() {
                        sel_color.gamma_multiply(0.9)
                    } else {
                        tokens.text_muted
                    },
                );
            }
        }

        let axis_cx = rect.center().x;
        let handle_y = |offset: f64| -> (f32, bool) {
            series_gnd_screen_y(offset, y_lo, y_span, rect)
        };
        let pick_at = |pos: egui::Pos2| -> Option<usize> {
            let mut best = None;
            let mut best_d2 = 14.0_f32 * 14.0_f32;
            for i in 0..n {
                if self.series[i].points.is_empty() {
                    continue;
                }
                let (hy, _) = handle_y(self.series[i].y_offset);
                let d2 = pos.distance_sq(egui::pos2(axis_cx, hy));
                if d2 < best_d2 {
                    best_d2 = d2;
                    best = Some(i);
                }
            }
            best
        };
        let hover_ch = resp.interact_pointer_pos().and_then(pick_at);

        for i in 0..n {
            let s = &self.series[i];
            if s.points.is_empty() {
                continue;
            }
            let color = s.color;
            let (hy, on_screen) = handle_y(s.y_offset);
            let selected = self.selected == Some(i);
            let active =
                self.dragging_offset == Some(i) || self.dragging_scale == Some(i);
            let highlighted = selected || active || hover_ch == Some(i);
            let radius = if highlighted {
                AXIS_HANDLE_RADIUS + 1.0
            } else {
                AXIS_HANDLE_RADIUS
            };
            paint_series_gnd_handle(
                &painter,
                rect,
                axis_cx,
                hy,
                on_screen,
                radius,
                color,
                highlighted,
            );
            if highlighted {
                painter.text(
                    egui::pos2(axis_cx, hy - radius - 1.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("{}", i + 1),
                    egui::FontId::proportional(9.0),
                    color,
                );
                if (s.y_scale - 1.0).abs() > 1e-6 {
                    painter.text(
                        egui::pos2(axis_cx, hy + radius + 1.0),
                        egui::Align2::CENTER_TOP,
                        format_scale(s.y_scale),
                        egui::FontId::monospace(8.0),
                        color.gamma_multiply(0.85),
                    );
                }
            }
        }

        let active_ch = self
            .dragging_offset
            .or(self.dragging_scale)
            .or(hover_ch)
            .or(self.selected.filter(|&i| i < n && !self.series[i].points.is_empty()));

        let mut extent_dirty = false;
        if resp.hovered() {
            if let Some(scroll) = {
                let s = ui.input(|inp| {
                    inp.smooth_scroll_delta.y as f64 + inp.raw_scroll_delta.y as f64
                });
                if s.abs() > 0.0 {
                    Some(s)
                } else {
                    None
                }
            } {
                if let Some(i) = active_ch {
                    if shift {
                        self.series[i].y_offset += -(scroll / 120.0) * y_span * 0.08;
                    } else {
                        let factor = (scroll * 0.002).exp();
                        self.series[i].y_scale =
                            (self.series[i].y_scale * factor).clamp(1e-12, 1e12);
                    }
                    self.selected = Some(i);
                    self.live_auto_fit = false;
                    extent_dirty = true;
                }
            }
        }

        if resp.drag_started() {
            if let Some(i) = hover_ch {
                self.selected = Some(i);
                if ctrl {
                    self.dragging_scale = Some(i);
                } else {
                    self.dragging_offset = Some(i);
                }
                self.live_auto_fit = false;
            }
        }
        if let Some(i) = self.dragging_scale {
            if resp.dragged() {
                let dy = resp.drag_delta().y as f64;
                let factor = (-dy * 0.008).exp();
                self.series[i].y_scale =
                    (self.series[i].y_scale * factor).clamp(1e-12, 1e12);
                extent_dirty = true;
            }
        } else if let Some(i) = self.dragging_offset {
            if resp.dragged() {
                let dy = resp.drag_delta().y;
                self.series[i].y_offset += -(dy as f64 / rect.height() as f64) * y_span;
                extent_dirty = true;
            }
        }
        if resp.double_clicked() {
            if let Some(i) = hover_ch.or(self.selected) {
                if i < n {
                    if ctrl {
                        self.series[i].y_scale = 1.0;
                    } else {
                        self.series[i].y_offset = 0.0;
                    }
                    self.selected = Some(i);
                    self.live_auto_fit = false;
                    extent_dirty = true;
                }
            }
        }
        if resp.clicked() && !resp.dragged() {
            if let Some(i) = hover_ch {
                self.selected = Some(i);
            }
        }
        if resp.hovered() {
            if let Some(i) = active_ch {
                let tip = format!(
                    "{}\n{}",
                    self.series[i].label,
                    tr(lang, "data.axis_hint")
                );
                resp.clone().on_hover_text(tip);
            }
        }

        if !ui.input(|i| i.pointer.primary_down()) {
            self.dragging_offset = None;
            self.dragging_scale = None;
        }
        if extent_dirty {
            if let Some(i) = self
                .dragging_offset
                .or(self.dragging_scale)
                .or(self.selected)
            {
                if let Some(s) = self.series.get_mut(i) {
                    s.disp_dirty = true;
                }
            } else {
                for s in &mut self.series {
                    s.disp_dirty = true;
                }
            }
            self.invalidate_lod();
            self.refresh_cached_extent();
        }
    }

    fn empty_plot_hint(&self, lang: Lang) -> String {
        if matches!(self.source, DataSource::SerialLive) {
            if self.frame_header.trim().is_empty() {
                return tr(lang, "data.status_need_header");
            }
            let any_filter = self.filters[..self.series_count]
                .iter()
                .any(|f| !f.trim().is_empty());
            if !any_filter {
                return tr(lang, "data.status_need_filter");
            }
            return tr(lang, "data.status_live_wait");
        }
        if self.file_text.is_none() {
            return tr(lang, "data.plot_stub");
        }
        if self.frame_header.trim().is_empty() {
            return tr(lang, "data.status_need_header");
        }
        let any_filter = self.filters[..self.series_count]
            .iter()
            .any(|f| !f.trim().is_empty());
        if !any_filter {
            return tr(lang, "data.status_need_filter");
        }
        if self.frame_count == 0 {
            return tr(lang, "data.status_no_frames");
        }
        tr(lang, "data.status_no_points")
    }

    fn clear_data(&mut self, lang: Lang) {
        if matches!(self.source, DataSource::SerialLive) && self.frame_count > 0 {
            self.flush_live_cache();
        }
        self.last_open_path = None;
        self.file_text = None;
        self.series.clear();
        self.frame_count = 0;
        self.cached_extent = None;
        self.last_x_range = None;
        self.last_y_range = None;
        self.pending_bounds = None;
        self.fit_request = false;
        self.selected = None;
        self.dragging_offset = None;
        self.dragging_scale = None;
        self.live_tail.clear();
        self.live_cursor = 0;
        self.live_plot_dirty = false;
        self.live_frames_since_flush = 0;
        self.invalidate_lod();
        self.pending_load = None;
        self.status = tr(lang, "data.status_cleared");
        if matches!(self.source, DataSource::SerialLive) {
            self.live_auto_fit = true;
            self.status = tr(lang, "data.status_live_wait");
        }
    }

    fn on_source_changed(&mut self, lang: Lang) {
        match self.source {
            DataSource::SerialLive => {
                self.live_session_id = format!(
                    "{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                );
                for s in &mut self.series {
                    s.points.clear();
                    s.mapped.clear();
                    s.disp_dirty = true;
                    s.y_min = 0.0;
                    s.y_max = 0.0;
                    s.y_scale = 1.0;
                    s.y_offset = 0.0;
                }
                self.frame_count = 0;
                self.live_tail.clear();
                self.live_cursor = 0;
                self.live_auto_fit = true;
                self.live_plot_dirty = false;
                self.live_frames_since_flush = 0;
                self.cached_extent = None;
                self.pending_bounds = None;
                self.fit_request = false;
                self.file_text = None;
                self.pending_load = None;
                self.invalidate_lod();
                self.status = tr(lang, "data.status_live_wait");
            }
            DataSource::File => {
                if self.frame_count > 0 {
                    self.flush_live_cache();
                }
                self.live_auto_fit = false;
                self.live_tail.clear();
                self.live_cursor = 0;
                self.live_plot_dirty = false;
                self.pending_load = None;
                if self.file_text.is_some() {
                    self.reparse(lang);
                } else {
                    self.status = tr(lang, "data.plot_stub");
                }
            }
        }
    }

    fn apply_rules(&mut self, lang: Lang) {
        match self.source {
            DataSource::File => {
                self.reparse(lang);
            }
            DataSource::SerialLive => {
                if self.frame_count > 0 {
                    self.flush_live_cache();
                }
                for s in &mut self.series {
                    s.points.clear();
                    s.mapped.clear();
                    s.disp_dirty = true;
                    s.y_min = 0.0;
                    s.y_max = 0.0;
                }
                self.series.clear();
                self.frame_count = 0;
                self.live_tail.clear();
                self.live_cursor = 0;
                self.live_auto_fit = true;
                self.live_plot_dirty = false;
                self.live_frames_since_flush = 0;
                self.cached_extent = None;
                self.pending_bounds = None;
                self.invalidate_lod();
                self.ensure_series_shells(lang);
                self.status = tr(lang, "data.status_live_wait");
            }
        }
    }

    fn pick_open_file(&mut self, lang: Lang) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Text", &["txt", "csv", "log"])
            .add_filter("Extract", &["wida"])
            .add_filter("All", &["*"]);
        if let Some(root) = self.resolve_browser_root() {
            dialog = dialog.set_directory(root);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_file(path, lang);
        }
    }

    fn pick_browser_dir(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        let start = if !self.browser_dir.trim().is_empty() {
            project_path(&self.browser_dir)
        } else {
            project_path(".")
        };
        if start.is_dir() {
            dialog = dialog.set_directory(start);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.browser_dir = dir.to_string_lossy().into_owned();
            self.refresh_data_browser();
            self.persist_browser_dir();
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

    fn refresh_data_browser(&mut self) {
        let scanned_key = self.browser_dir.trim().to_owned();
        self.browser_scanned_dir = scanned_key;
        self.browser_folders.clear();
        let mut folders = Vec::new();
        if let Some(root) = self.resolve_browser_root() {
            if let Ok(entries) = fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let Ok(files) = fs::read_dir(&path) else {
                        continue;
                    };
                    let mut data_files = Vec::new();
                    for file in files.flatten() {
                        let file_path = file.path();
                        if !file_path.is_file() {
                            continue;
                        }
                        if !is_data_source_ext(&file_path) {
                            continue;
                        }
                        let name = file_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| file_path.display().to_string());
                        data_files.push((name, file_path));
                    }
                    if data_files.is_empty() {
                        continue;
                    }
                    data_files
                        .sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    folders.push(DataBrowserFolder {
                        name,
                        files: data_files,
                    });
                }
            }
        }
        folders.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        let extracts = list_cached_extracts();
        if !extracts.is_empty() {
            folders.insert(
                0,
                DataBrowserFolder {
                    name: "提取缓存 / Extract Cache".into(),
                    files: extracts,
                },
            );
        }
        self.browser_folders = folders;
    }

    fn ensure_data_browser_fresh(&mut self) {
        if self.browser_dir.trim() != self.browser_scanned_dir.trim() {
            self.refresh_data_browser();
        }
    }

    fn open_file(&mut self, path: PathBuf, lang: Lang) {
        self.start_open_file(path, lang);
    }

    fn start_open_file(&mut self, path: PathBuf, lang: Lang) {
        if self.pending_load.is_some() {
            self.status = tr(lang, "data.status_loading_busy");
            return;
        }
        self.source = DataSource::File;
        self.live_auto_fit = false;
        self.live_tail.clear();
        self.live_cursor = 0;
        self.last_open_path = Some(path.clone());

        let header = self.frame_header.trim().to_owned();
        let filters = self.rules_filter_vec();
        let kinds = self.rules_kind_vec();
        let (tx, rx) = unbounded();
        self.pending_load = Some(PendingDataLoad { rx });
        self.load_progress = 0;
        self.load_progress_target = 0;
        self.series.clear();
        self.frame_count = 0;
        self.cached_extent = None;
        self.invalidate_lod();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.status = format!("{} {name}…", tr(lang, "data.status_loading"));

        thread::spawn(move || {
            load_file_worker(path, header, filters, kinds, &tx);
        });
    }

    fn start_reparse_job(&mut self, lang: Lang) {
        let Some(text) = self.file_text.clone() else {
            return;
        };
        if self.pending_load.is_some() {
            self.status = tr(lang, "data.status_loading_busy");
            return;
        }
        let header = self.frame_header.trim().to_owned();
        if header.is_empty() {
            self.series.clear();
            self.frame_count = 0;
            self.cached_extent = None;
            self.invalidate_lod();
            self.status = tr(lang, "data.status_need_header");
            return;
        }
        let filters = self.rules_filter_vec();
        if filters.iter().all(|f| f.is_empty()) {
            self.series.clear();
            self.frame_count = 0;
            self.cached_extent = None;
            self.invalidate_lod();
            self.status = tr(lang, "data.status_need_filter");
            return;
        }
        let kinds = self.rules_kind_vec();
        let path = self.last_open_path.clone();
        let (tx, rx) = unbounded();
        self.pending_load = Some(PendingDataLoad { rx });
        self.load_progress = 0;
        self.load_progress_target = 0;
        self.status = tr(lang, "data.status_loading");
        thread::spawn(move || {
            reparse_text_worker(text, path, header, filters, kinds, &tx);
        });
    }

    fn poll_pending_load(&mut self, lang: Lang) {
        let mut done_msg = None;
        loop {
            let event = {
                let Some(pending) = self.pending_load.as_ref() else {
                    return;
                };
                match pending.rx.try_recv() {
                    Ok(ev) => ev,
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        self.pending_load = None;
                        self.load_progress = 0;
                        self.load_progress_target = 0;
                        self.status = tr(lang, "data.status_read_err");
                        return;
                    }
                }
            };
            match event {
                LoadEvent::Progress(p) => {
                    self.load_progress_target = self.load_progress_target.max(p.min(99));
                }
                LoadEvent::Done(msg) => {
                    done_msg = Some(msg);
                    break;
                }
            }
        }

        if let Some(msg) = done_msg {
            // Drain any trailing progress that arrived with Done.
            if let Some(pending) = self.pending_load.as_ref() {
                while let Ok(LoadEvent::Progress(p)) = pending.rx.try_recv() {
                    self.load_progress_target = self.load_progress_target.max(p.min(99));
                }
            }
            self.pending_load = None;
            self.load_progress = 100;
            self.load_progress_target = 100;
            match msg {
                Ok(ok) => self.apply_load_ok(ok, lang),
                Err(e) => {
                    self.series.clear();
                    self.file_text = None;
                    self.frame_count = 0;
                    self.cached_extent = None;
                    self.invalidate_lod();
                    self.load_progress = 0;
                    self.load_progress_target = 0;
                    self.status = format!("{}: {e}", tr(lang, "data.status_read_err"));
                }
            }
            return;
        }

        // Ease displayed % toward worker target so a burst of events does not jump to 90%+.
        if self.pending_load.is_some() && self.load_progress < self.load_progress_target {
            let gap = self.load_progress_target - self.load_progress;
            let step = ((gap / 4).max(1)).min(6);
            self.load_progress = (self.load_progress + step).min(self.load_progress_target);
            self.status = format!(
                "{} {}%",
                tr(lang, "data.status_loading"),
                self.load_progress.min(99)
            );
        }
    }

    fn apply_load_ok(&mut self, ok: DataLoadOk, lang: Lang) {
        self.source = DataSource::File;
        self.live_auto_fit = false;
        if let Some(path) = ok.path {
            self.last_open_path = Some(path);
        }
        self.file_text = ok.file_text;
        if let Some(h) = ok.frame_header {
            if !h.is_empty() {
                self.frame_header = h;
            }
        }
        if let Some(filters) = ok.filters {
            while self.filters.len() < MAX_SERIES {
                self.filters.push(String::new());
            }
            for (i, f) in filters.into_iter().enumerate().take(MAX_SERIES) {
                self.filters[i] = f;
            }
        }
        if let Some(kinds) = ok.kinds {
            for (i, k) in kinds.into_iter().enumerate().take(MAX_SERIES) {
                self.kinds[i] = DataKind::from_config_str(&k);
            }
        }
        self.frame_count = ok.frame_count;
        self.series = ok
            .series
            .into_iter()
            .enumerate()
            .map(|(i, d)| DataSeries {
                label: d.label,
                color: series_color(i),
                kind: d.kind,
                points: d.points,
                y_scale: d.y_scale,
                y_offset: d.y_offset,
                y_min: d.y_min,
                y_max: d.y_max,
                mapped: d.mapped,
                disp_dirty: false,
            })
            .collect();
        self.series_count = self.series.len().clamp(1, MAX_SERIES);
        self.invalidate_lod();
        self.refresh_cached_extent();
        self.fit_request = true;
        self.pending_bounds = None;
        self.load_progress = 0;
        self.load_progress_target = 0;
        let points: usize = self.series.iter().map(|s| s.points.len()).sum();
        if self.frame_count == 0 {
            self.status = tr(lang, "data.status_no_frames");
        } else if points == 0 {
            self.status = tr(lang, "data.status_no_points");
        } else if ok.from_cache {
            self.status =
                status_with_counts(lang, "data.status_cache_hit", self.frame_count, points);
        } else {
            self.status = status_with_counts(lang, "data.status_loaded", self.frame_count, points);
        }
        self.persist_rules();
    }

    fn invalidate_lod(&mut self) {
        self.lod_key = None;
        self.lod_lines.clear();
    }

    fn ensure_display_mapped(&mut self) {
        for s in &mut self.series {
            if !s.disp_dirty && s.mapped.len() == s.points.len() {
                continue;
            }
            s.mapped.clear();
            s.mapped.reserve(s.points.len());
            for p in &s.points {
                s.mapped
                    .push([p[0], display_y(p[1], s.y_scale, s.y_offset, s.kind)]);
            }
            s.disp_dirty = false;
        }
    }

    fn lod_key_of(x0: f64, x1: f64, pixel_w: f32) -> (i64, i64, i32) {
        (
            (x0 * 1000.0).round() as i64,
            (x1 * 1000.0).round() as i64,
            pixel_w.round() as i32,
        )
    }

    fn ensure_viewport_lod(&mut self, x0: f64, x1: f64, pixel_w: f32) {
        let key = Self::lod_key_of(x0, x1, pixel_w);
        if self.lod_key == Some(key) && self.lod_lines.len() == self.series.len() {
            return;
        }
        self.ensure_display_mapped();
        let cols = ((pixel_w * LOD_POINTS_PER_PX).ceil() as usize).max(64);
        let mut lines = Vec::with_capacity(self.series.len());
        for s in &self.series {
            if s.mapped.is_empty() {
                lines.push(Vec::new());
                continue;
            }
            lines.push(downsample_minmax(&s.mapped, x0, x1, cols));
        }
        self.lod_key = Some(key);
        self.lod_lines = lines;
    }

    fn build_plot_draw_items(
        &mut self,
        x0: f64,
        x1: f64,
        pixel_w: f32,
    ) -> Vec<PlotDrawItem> {
        self.ensure_viewport_lod(x0, x1, pixel_w);
        let mut out = Vec::new();
        for (i, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let use_lod = s.mapped.len() > SKIP_DOTS_ABOVE
                || s.mapped.len() as f32 > pixel_w * LOD_POINTS_PER_PX;
            let line = if use_lod {
                self.lod_lines
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| s.mapped.clone())
            } else {
                s.mapped.clone()
            };
            let draw_dots = !use_lod && s.mapped.len() <= SKIP_DOTS_ABOVE;
            out.push(PlotDrawItem {
                index: i,
                label: s.label.clone(),
                color: s.color,
                line,
                draw_dots,
            });
        }
        out
    }

    fn rules_filter_vec(&self) -> Vec<String> {
        self.filters[..self.series_count]
            .iter()
            .map(|s| s.trim().to_owned())
            .collect()
    }

    fn rules_kind_vec(&self) -> Vec<String> {
        self.kinds[..self.series_count]
            .iter()
            .map(|k| k.as_config_str().to_string())
            .collect()
    }

    fn build_wida_payload(&self, source_key: &str, content_fp: &str) -> WidaPayload {
        let series: Vec<WidaSeriesData> = self
            .series
            .iter()
            .map(|s| WidaSeriesData {
                label: s.label.clone(),
                kind: s.kind.as_config_str().to_string(),
                y_scale: s.y_scale,
                y_offset: s.y_offset,
                y_min: s.y_min,
                y_max: s.y_max,
                points: s.points.clone(),
            })
            .collect();
        WidaPayload {
            meta: data_extract_cache::WidaHeader {
                source_key: source_key.to_owned(),
                content_fp: content_fp.to_owned(),
                header: self.frame_header.trim().to_owned(),
                filters: self.rules_filter_vec(),
                kinds: self.rules_kind_vec(),
                frame_count: self.frame_count as u32,
                series: Vec::new(),
            },
            series,
        }
    }

    fn flush_live_cache(&mut self) {
        if self.frame_count == 0 || self.series.iter().all(|s| s.points.is_empty()) {
            self.live_frames_since_flush = 0;
            return;
        }
        let path = live_cache_path(&self.live_session_id);
        let payload = self.build_wida_payload("serial:live", &self.live_session_id);
        if save_wida(&path, &payload).is_ok() {
            self.live_frames_since_flush = 0;
            // Refresh browser so the live extract appears under cache folder.
            self.refresh_data_browser();
        }
    }

    fn ensure_series_shells(&mut self, lang: Lang) {
        let filters: Vec<&str> = self.filters[..self.series_count]
            .iter()
            .map(|s| s.trim())
            .collect();
        if self.series.len() != self.series_count {
            self.series.resize_with(self.series_count, || DataSeries {
                label: String::new(),
                color: Color32::WHITE,
                kind: DataKind::Voltage,
                points: Vec::new(),
                y_scale: 1.0,
                y_offset: 0.0,
                y_min: 0.0,
                y_max: 0.0,
                mapped: Vec::new(),
                disp_dirty: true,
            });
        }
        for i in 0..self.series_count {
            let label = if filters[i].is_empty() {
                tr_fmt(lang, "data.filter_n", i + 1)
            } else {
                filters[i].to_string()
            };
            let s = &mut self.series[i];
            s.label = label;
            s.color = series_color(i);
            if s.kind != self.kinds[i] {
                s.kind = self.kinds[i];
                s.disp_dirty = true;
            }
        }
    }

    fn reparse(&mut self, lang: Lang) {
        self.start_reparse_job(lang);
    }

    /// Stagger display offsets by kind (and within kind). Call after file reparse only.
    fn auto_layout_scales(&mut self) {
        let mut groups: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (i, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let gi = match s.kind {
                DataKind::Voltage => 0,
                DataKind::Current => 1,
                DataKind::Temperature => 2,
            };
            groups[gi].push(i);
        }

        let mut band = 0.0_f64;
        for group in &groups {
            if group.is_empty() {
                continue;
            }
            for (k, &i) in group.iter().enumerate() {
                let s = &mut self.series[i];
                s.y_scale = 1.0;
                s.y_offset = band + k as f64 * BAND_SPACING;
                s.disp_dirty = true;
            }
            band += group.len() as f64 * BAND_SPACING + BAND_SPACING;
        }
    }

    fn persist_browser_dir(&self) {
        if let Ok(mut cfg) = load_config() {
            cfg.apps.data_analysis.browser_dir = self.browser_dir.clone();
            let _ = save_config(&cfg);
        }
    }

    fn persist_rules(&self) {
        if let Ok(mut cfg) = load_config() {
            cfg.apps.data_analysis.frame_header = self.frame_header.clone();
            cfg.apps.data_analysis.series_count = self.series_count as u8;
            cfg.apps.data_analysis.filters = self.filters[..self.series_count].to_vec();
            cfg.apps.data_analysis.kinds = self.kinds[..self.series_count]
                .iter()
                .map(|k| k.as_config_str().to_string())
                .collect();
            cfg.apps.data_analysis.source = self.source.as_config_str().to_string();
            cfg.apps.data_analysis.browser_dir = self.browser_dir.clone();
            cfg.apps.data_analysis.threshold_voltage = self.thr_voltage;
            cfg.apps.data_analysis.threshold_voltage_on = self.thr_voltage_on;
            cfg.apps.data_analysis.threshold_current = self.thr_current;
            cfg.apps.data_analysis.threshold_current_on = self.thr_current_on;
            cfg.apps.data_analysis.threshold_temperature = self.thr_temperature;
            cfg.apps.data_analysis.threshold_temperature_on = self.thr_temperature_on;
            let _ = save_config(&cfg);
        }
    }

    fn active_kinds(&self) -> Vec<DataKind> {
        let mut out = Vec::new();
        for kind in [DataKind::Voltage, DataKind::Current, DataKind::Temperature] {
            if self
                .series
                .iter()
                .any(|s| s.kind == kind && !s.points.is_empty())
            {
                out.push(kind);
            }
        }
        out
    }

    fn y_axis_label_for_plot(&self, lang: Lang) -> String {
        let kinds = self.active_kinds();
        if kinds.is_empty() {
            return tr(lang, "data.axis_y");
        }
        kinds
            .iter()
            .map(|k| k.short_axis())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Map enabled native thresholds into display-Y using a representative series.
    fn threshold_display_lines(&self) -> Vec<(DataKind, f64, f64)> {
        let specs = [
            (DataKind::Voltage, self.thr_voltage_on, self.thr_voltage),
            (DataKind::Current, self.thr_current_on, self.thr_current),
            (
                DataKind::Temperature,
                self.thr_temperature_on,
                self.thr_temperature,
            ),
        ];
        let mut out = Vec::new();
        for (kind, on, native) in specs {
            if !on || !native.is_finite() {
                continue;
            }
            let Some(s) = self.series_for_kind(kind) else {
                continue;
            };
            let y = display_y(native, s.y_scale, s.y_offset, kind);
            out.push((kind, y, native));
        }
        out
    }

    fn series_for_kind(&self, kind: DataKind) -> Option<&DataSeries> {
        if let Some(i) = self.selected {
            if let Some(s) = self.series.get(i) {
                if s.kind == kind && !s.points.is_empty() {
                    return Some(s);
                }
            }
        }
        self.series
            .iter()
            .find(|s| s.kind == kind && !s.points.is_empty())
    }

    fn request_zoom_axis(&mut self, zoom_x: bool, factor: f64) {
        let Some(extent) = self.data_extent() else {
            return;
        };
        let (x0, x1) = self
            .last_x_range
            .unwrap_or((extent.min()[0], extent.max()[0]));
        let (y0, y1) = self
            .last_y_range
            .unwrap_or((extent.min()[1], extent.max()[1]));
        let (nx0, nx1, ny0, ny1) = if zoom_x {
            let mid = 0.5 * (x0 + x1);
            let half = 0.5 * (x1 - x0) * factor;
            if !half.is_finite() || half <= 0.0 {
                return;
            }
            (mid - half, mid + half, y0, y1)
        } else {
            let mid = 0.5 * (y0 + y1);
            let half = 0.5 * (y1 - y0) * factor;
            if !half.is_finite() || half <= 0.0 {
                return;
            }
            (x0, x1, mid - half, mid + half)
        };
        let bounds = PlotBounds::from_min_max([nx0, ny0], [nx1, ny1]);
        self.pending_bounds = Some(self.clamp_view_bounds(bounds));
        self.fit_request = false;
        self.live_auto_fit = false;
    }

    /// Y zoom: selected series scale (like waveform), else global plot Y via request_zoom_axis.
    fn apply_y_zoom_factor(&mut self, zoom: f64) {
        if !zoom.is_finite() || (zoom - 1.0).abs() <= 1e-6 {
            return;
        }
        if let Some(i) = self.selected {
            if let Some(s) = self.series.get_mut(i) {
                if !s.points.is_empty() {
                    s.y_scale = (s.y_scale * zoom).clamp(1e-12, 1e12);
                    s.disp_dirty = true;
                    self.live_auto_fit = false;
                    self.invalidate_lod();
                    self.refresh_cached_extent();
                    return;
                }
            }
        }
        self.request_zoom_axis(false, 1.0 / zoom);
    }

    fn data_extent(&self) -> Option<PlotBounds> {
        let (xmin, xmax, ymin, ymax) = self.cached_extent?;
        let xpad = ((xmax - xmin).abs() * 0.02).max(0.5);
        let ypad = ((ymax - ymin).abs() * 0.08).max(VIEW_MIN_SPAN_ABS);
        Some(PlotBounds::from_min_max(
            [xmin - xpad, ymin - ypad],
            [xmax + xpad, ymax + ypad],
        ))
    }

    fn live_fit_bounds(&self) -> Option<PlotBounds> {
        let ext = self.data_extent()?;
        let n = self.frame_count;
        if n == 0 {
            return Some(ext);
        }
        let window = LIVE_FIT_FRAMES.min(n);
        let x1 = (n as f64 - 1.0).max(0.0);
        let x0 = (n.saturating_sub(window)) as f64;
        let y0 = ext.min()[1];
        let y1 = ext.max()[1];
        Some(self.clamp_view_bounds(PlotBounds::from_min_max([x0, y0], [x1, y1])))
    }

    fn refresh_cached_extent(&mut self) {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for s in &self.series {
            if s.points.is_empty() {
                continue;
            }
            // X from first/last sample (points are frame-ordered).
            xmin = xmin.min(s.points[0][0]).min(s.points[s.points.len() - 1][0]);
            xmax = xmax.max(s.points[0][0]).max(s.points[s.points.len() - 1][0]);
            // Y: affine map of native extents (O(1) per series).
            let y0 = display_y(s.y_min, s.y_scale, s.y_offset, s.kind);
            let y1 = display_y(s.y_max, s.y_scale, s.y_offset, s.kind);
            ymin = ymin.min(y0).min(y1);
            ymax = ymax.max(y0).max(y1);
        }
        if !xmin.is_finite() {
            self.cached_extent = None;
            return;
        }
        if (xmax - xmin).abs() < VIEW_MIN_SPAN_ABS {
            xmax = xmin + 1.0;
        }
        if (ymax - ymin).abs() < VIEW_MIN_SPAN_ABS {
            ymax = ymin + 1.0;
            ymin -= 0.5;
        }
        self.cached_extent = Some((xmin, xmax, ymin, ymax));
    }

    fn clamp_view_bounds(&self, bounds: PlotBounds) -> PlotBounds {
        let Some(extent) = self.data_extent() else {
            return bounds;
        };
        clamp_view_bounds_static(extent, bounds)
    }

    pub fn api_snapshot(&self) -> Value {
        json!({
            "browser_dir": self.browser_dir,
            "frame_header": self.frame_header,
            "series_count": self.series_count,
            "filters": self.filters[..self.series_count],
            "kinds": self.kinds[..self.series_count].iter().map(|k| k.as_config_str()).collect::<Vec<_>>(),
            "source": self.source.as_config_str(),
            "frames": self.frame_count,
            "live_cursor": self.live_cursor,
            "thresholds": {
                "voltage": { "on": self.thr_voltage_on, "value": self.thr_voltage },
                "current": { "on": self.thr_current_on, "value": self.thr_current },
                "temperature": { "on": self.thr_temperature_on, "value": self.thr_temperature },
            },
            "series": self.series.iter().map(|s| json!({
                "label": s.label,
                "kind": s.kind.as_config_str(),
                "points": s.points.len(),
                "y_scale": s.y_scale,
                "y_offset": s.y_offset,
            })).collect::<Vec<_>>(),
            "last_open": self.last_open_path.as_ref().map(|p| p.display().to_string()),
        })
    }
}

fn paint_loading_in_rect(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    lang: Lang,
    rect: egui::Rect,
    progress: u8,
) {
    let bar_w = 220.0_f32;
    let content = egui::Rect::from_center_size(rect.center(), egui::vec2(bar_w, 96.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.set_min_size(egui::vec2(bar_w, 96.0));
            ui.add(egui::Spinner::new().size(36.0));
            ui.add_space(8.0);
            let label = if progress > 0 {
                format!("{} {}%", tr(lang, "data.status_loading"), progress.min(99))
            } else {
                tr(lang, "data.status_loading")
            };
            ui.label(
                RichText::new(label)
                    .size(ui_theme::FONT_BODY)
                    .color(tokens.text_muted),
            );
            ui.add_space(8.0);
            let (bar_rect, _) =
                ui.allocate_exact_size(egui::vec2(bar_w, 8.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(bar_rect, CornerRadius::same(4), tokens.divider);
            let fill_w = bar_rect.width() * (progress.min(100) as f32 / 100.0);
            if fill_w > 0.5 {
                let fill =
                    egui::Rect::from_min_size(bar_rect.min, egui::vec2(fill_w, bar_rect.height()));
                ui.painter()
                    .rect_filled(fill, CornerRadius::same(4), tokens.accent);
            }
        },
    );
}

fn paint_loading_overlay(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    lang: Lang,
    rect: egui::Rect,
    progress: u8,
) {
    ui.painter().rect_filled(
        rect,
        CornerRadius::ZERO,
        tokens.surface_bg.gamma_multiply(0.55),
    );
    paint_loading_in_rect(ui, tokens, lang, rect, progress);
}

fn send_progress(tx: &Sender<LoadEvent>, pct: u8) {
    let _ = tx.send(LoadEvent::Progress(pct.min(99)));
}

fn layout_and_map_drafts(series: &mut [SeriesDraft]) {
    layout_and_map_drafts_progress(series, None, 0, 0);
}

fn layout_and_map_drafts_progress(
    series: &mut [SeriesDraft],
    tx: Option<&Sender<LoadEvent>>,
    lo: u8,
    hi: u8,
) {
    let mut groups: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (i, s) in series.iter().enumerate() {
        if s.points.is_empty() {
            continue;
        }
        let gi = match s.kind {
            DataKind::Voltage => 0,
            DataKind::Current => 1,
            DataKind::Temperature => 2,
        };
        groups[gi].push(i);
    }
    let mut band = 0.0_f64;
    for group in &groups {
        if group.is_empty() {
            continue;
        }
        for (k, &i) in group.iter().enumerate() {
            let s = &mut series[i];
            s.y_scale = 1.0;
            s.y_offset = band + k as f64 * BAND_SPACING;
        }
        band += group.len() as f64 * BAND_SPACING + BAND_SPACING;
    }
    map_drafts_inplace_progress(series, tx, lo, hi);
}

fn map_drafts_inplace(series: &mut [SeriesDraft]) {
    map_drafts_inplace_progress(series, None, 0, 0);
}

fn map_drafts_inplace_progress(
    series: &mut [SeriesDraft],
    tx: Option<&Sender<LoadEvent>>,
    lo: u8,
    hi: u8,
) {
    let total: usize = series.iter().map(|s| s.points.len()).sum::<usize>().max(1);
    let report_every = (total / 40).max(4096);
    let mut done = 0usize;
    for s in series.iter_mut() {
        s.mapped.clear();
        s.mapped.reserve(s.points.len());
        for p in &s.points {
            s.mapped
                .push([p[0], display_y(p[1], s.y_scale, s.y_offset, s.kind)]);
            done += 1;
            if let Some(tx) = tx {
                if done % report_every == 0 || done == total {
                    let pct = lo as u64 + (done as u64 * (hi - lo) as u64) / total as u64;
                    send_progress(tx, pct as u8);
                }
            }
        }
    }
}

fn save_extract_cache_worker(
    path: &Path,
    header: &str,
    filters: &[String],
    kinds: &[String],
    frame_count: usize,
    series: &[SeriesDraft],
    tx: Option<&Sender<LoadEvent>>,
    lo: u8,
    hi: u8,
) {
    let fp = file_content_fp(path);
    let cache_path = data_extract_cache::cache_path_for(&fp, header, filters, kinds);
    let meta = data_extract_cache::WidaHeader {
        source_key: path.to_string_lossy().into_owned(),
        content_fp: fp,
        header: header.to_owned(),
        filters: filters.to_vec(),
        kinds: kinds.to_vec(),
        frame_count: frame_count as u32,
        series: Vec::new(),
    };
    let parts: Vec<(String, String, f64, f64, f64, f64, &[[f64; 2]])> = series
        .iter()
        .map(|s| {
            (
                s.label.clone(),
                s.kind.as_config_str().to_string(),
                s.y_scale,
                s.y_offset,
                s.y_min,
                s.y_max,
                s.points.as_slice(),
            )
        })
        .collect();
    let _ = data_extract_cache::save_wida_from_parts(&cache_path, meta, &parts, |frac| {
        if let Some(tx) = tx {
            let pct = lo as f32 + (hi - lo) as f32 * frac;
            send_progress(tx, pct.round().clamp(0.0, 99.0) as u8);
        }
    });
}

fn wida_to_load_ok(
    path: PathBuf,
    payload: WidaPayload,
    file_text: Option<Arc<String>>,
    tx: Option<&Sender<LoadEvent>>,
) -> DataLoadOk {
    let mut series: Vec<SeriesDraft> = payload
        .series
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let kind = DataKind::from_config_str(&s.kind);
            let label = if s.label.is_empty() {
                format!("{}", i + 1)
            } else {
                s.label
            };
            SeriesDraft {
                label,
                kind,
                points: s.points,
                y_scale: s.y_scale,
                y_offset: s.y_offset,
                y_min: s.y_min,
                y_max: s.y_max,
                mapped: Vec::new(),
            }
        })
        .collect();
    map_drafts_inplace_progress(&mut series, tx, 70, 95);
    DataLoadOk {
        path: Some(path),
        file_text,
        frame_count: payload.meta.frame_count as usize,
        series,
        from_cache: true,
        frame_header: Some(payload.meta.header),
        filters: Some(payload.meta.filters),
        kinds: Some(payload.meta.kinds),
    }
}

fn load_file_worker(
    path: PathBuf,
    header: String,
    filters: Vec<String>,
    kinds: Vec<String>,
    tx: &Sender<LoadEvent>,
) {
    let send_done = |r: Result<DataLoadOk, String>| {
        let _ = tx.send(LoadEvent::Done(r));
    };

    send_progress(tx, 2);
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wida"))
    {
        send_progress(tx, 20);
        match load_wida(&path) {
            Ok(payload) => {
                send_progress(tx, 55);
                send_done(Ok(wida_to_load_ok(path, payload, None, Some(tx))));
            }
            Err(e) => send_done(Err(e)),
        }
        return;
    }

    send_progress(tx, 5);
    let text = match read_text_file(&path) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            send_done(Err(e));
            return;
        }
    };
    send_progress(tx, 12);
    let fp = file_content_fp(&path);
    let cache_path = data_extract_cache::cache_path_for(&fp, &header, &filters, &kinds);
    if let Ok(payload) = load_wida(&cache_path) {
        if payload.meta.content_fp == fp && payload.meta.header == header {
            send_progress(tx, 55);
            send_done(Ok(wida_to_load_ok(
                path,
                payload,
                Some(Arc::clone(&text)),
                Some(tx),
            )));
            return;
        }
    }

    if header.trim().is_empty() {
        send_done(Err("empty frame header".into()));
        return;
    }
    let filter_refs: Vec<&str> = filters.iter().map(|s| s.as_str()).collect();
    if filter_refs.iter().all(|f| f.is_empty()) {
        send_done(Err("empty filters".into()));
        return;
    }
    send_progress(tx, 15);
    let parsed = parse_data_file_progress(&text, &header, &filter_refs, tx);
    send_progress(tx, 78);
    let frame_count = parsed.frame_count;
    let mut series: Vec<SeriesDraft> = parsed
        .series
        .into_iter()
        .enumerate()
        .map(|(i, points)| {
            let label = if filters.get(i).map(|s| s.is_empty()).unwrap_or(true) {
                format!("{}", i + 1)
            } else {
                filters[i].clone()
            };
            let kind = kinds
                .get(i)
                .map(|s| DataKind::from_config_str(s))
                .unwrap_or(DataKind::Voltage);
            let (y_min, y_max) = native_extent(&points);
            SeriesDraft {
                label,
                kind,
                points,
                y_scale: 1.0,
                y_offset: 0.0,
                y_min,
                y_max,
                mapped: Vec::new(),
            }
        })
        .collect();
    layout_and_map_drafts_progress(&mut series, Some(tx), 78, 88);
    send_progress(tx, 88);
    save_extract_cache_worker(
        &path,
        &header,
        &filters,
        &kinds,
        frame_count,
        &series,
        Some(tx),
        88,
        97,
    );
    send_progress(tx, 98);
    send_done(Ok(DataLoadOk {
        path: Some(path),
        file_text: Some(text),
        frame_count,
        series,
        from_cache: false,
        frame_header: None,
        filters: None,
        kinds: None,
    }));
}

fn reparse_text_worker(
    text: Arc<String>,
    path: Option<PathBuf>,
    header: String,
    filters: Vec<String>,
    kinds: Vec<String>,
    tx: &Sender<LoadEvent>,
) {
    let send_done = |r: Result<DataLoadOk, String>| {
        let _ = tx.send(LoadEvent::Done(r));
    };
    send_progress(tx, 3);
    if header.trim().is_empty() {
        send_done(Err("empty frame header".into()));
        return;
    }
    let filter_refs: Vec<&str> = filters.iter().map(|s| s.as_str()).collect();
    if filter_refs.iter().all(|f| f.is_empty()) {
        send_done(Err("empty filters".into()));
        return;
    }
    if let Some(ref p) = path {
        send_progress(tx, 10);
        let fp = file_content_fp(p);
        let cache_path = data_extract_cache::cache_path_for(&fp, &header, &filters, &kinds);
        if let Ok(payload) = load_wida(&cache_path) {
            if payload.meta.content_fp == fp && payload.meta.header == header {
                send_progress(tx, 55);
                send_done(Ok(wida_to_load_ok(
                    p.clone(),
                    payload,
                    Some(Arc::clone(&text)),
                    Some(tx),
                )));
                return;
            }
        }
    }
    send_progress(tx, 15);
    let parsed = parse_data_file_progress(text.as_str(), &header, &filter_refs, tx);
    send_progress(tx, 78);
    let frame_count = parsed.frame_count;
    let mut series: Vec<SeriesDraft> = parsed
        .series
        .into_iter()
        .enumerate()
        .map(|(i, points)| {
            let label = if filters.get(i).map(|s| s.is_empty()).unwrap_or(true) {
                format!("{}", i + 1)
            } else {
                filters[i].clone()
            };
            let kind = kinds
                .get(i)
                .map(|s| DataKind::from_config_str(s))
                .unwrap_or(DataKind::Voltage);
            let (y_min, y_max) = native_extent(&points);
            SeriesDraft {
                label,
                kind,
                points,
                y_scale: 1.0,
                y_offset: 0.0,
                y_min,
                y_max,
                mapped: Vec::new(),
            }
        })
        .collect();
    layout_and_map_drafts_progress(&mut series, Some(tx), 78, 88);
    send_progress(tx, 88);
    if let Some(ref p) = path {
        save_extract_cache_worker(
            p,
            &header,
            &filters,
            &kinds,
            frame_count,
            &series,
            Some(tx),
            88,
            97,
        );
    }
    send_progress(tx, 98);
    send_done(Ok(DataLoadOk {
        path,
        file_text: Some(text),
        frame_count,
        series,
        from_cache: false,
        frame_header: None,
        filters: None,
        kinds: None,
    }));
}

fn parse_data_file_progress(
    text: &str,
    header: &str,
    filters: &[&str],
    tx: &Sender<LoadEvent>,
) -> ParsedData {
    // 15–45%: locate frames; 45–78%: extract series values.
    let frames = split_frames_progress(text, header, tx, 15, 45);
    let n = frames.len().max(1);
    let mut series: Vec<Vec<[f64; 2]>> = (0..filters.len()).map(|_| Vec::new()).collect();
    let step = (n / 40).max(1);
    for (fi, frame) in frames.iter().enumerate() {
        let x = fi as f64;
        for (si, filter) in filters.iter().enumerate() {
            if filter.is_empty() {
                continue;
            }
            if let Some(value) = extract_first_filter_value(frame, filter) {
                series[si].push([x, value]);
            }
        }
        if fi % step == 0 || fi + 1 == frames.len() {
            let pct = 45 + ((fi as u64 * 33) / n as u64) as u8;
            send_progress(tx, pct.min(78));
        }
    }
    ParsedData {
        frame_count: frames.len(),
        series,
    }
}

struct ParsedData {
    frame_count: usize,
    series: Vec<Vec<[f64; 2]>>,
}

fn parse_data_file(text: &str, header: &str, filters: &[&str]) -> ParsedData {
    let (tx, _rx) = unbounded::<LoadEvent>();
    parse_data_file_progress(text, header, filters, &tx)
}

fn split_frames<'a>(text: &'a str, header: &str) -> Vec<&'a str> {
    let (tx, _rx) = unbounded::<LoadEvent>();
    split_frames_progress(text, header, &tx, 0, 0)
}

fn split_frames_progress<'a>(
    text: &'a str,
    header: &str,
    tx: &Sender<LoadEvent>,
    lo: u8,
    hi: u8,
) -> Vec<&'a str> {
    if header.is_empty() {
        return Vec::new();
    }
    let total = text.len().max(1);
    let report_every = (total / 50).max(64 * 1024);
    let mut starts = Vec::new();
    let mut search_from = 0;
    let mut last_report = 0usize;
    while let Some(rel) = text[search_from..].find(header) {
        let abs = search_from + rel;
        starts.push(abs);
        search_from = abs + header.len().max(1);
        if hi > lo && search_from.saturating_sub(last_report) >= report_every {
            last_report = search_from;
            let pct = lo as u64 + (search_from as u64 * (hi - lo) as u64) / total as u64;
            send_progress(tx, pct.min(hi as u64) as u8);
        }
        if search_from >= text.len() {
            break;
        }
    }
    if hi > lo {
        send_progress(tx, hi);
    }
    if starts.is_empty() {
        return Vec::new();
    }
    let mut frames = Vec::with_capacity(starts.len());
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        frames.push(&text[start..end]);
    }
    frames
}

/// First `filter` + whitespace + number in `frame`.
/// Token boundary: filter must not be mid-identifier (prev char not alphanumeric/_).
fn extract_first_filter_value(frame: &str, filter: &str) -> Option<f64> {
    let mut search_from = 0;
    while let Some(rel) = frame[search_from..].find(filter) {
        let start = search_from + rel;
        let abs = start + filter.len();
        if !is_token_boundary_before(frame, start) {
            search_from = abs;
            continue;
        }
        let rest = &frame[abs..];
        let trimmed = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');
        if trimmed.len() == rest.len() {
            search_from = abs;
            continue;
        }
        if let Some((value, _)) = parse_leading_number(trimmed) {
            return Some(value);
        }
        search_from = abs;
        if search_from >= frame.len() {
            break;
        }
    }
    None
}

fn is_token_boundary_before(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let Some(prev) = text[..idx].chars().next_back() else {
        return true;
    };
    !(prev.is_ascii_alphanumeric() || prev == '_')
}

fn parse_leading_number(s: &str) -> Option<(f64, usize)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        i = 1;
    }
    let start_digits = i;
    let mut saw_digit = false;
    let mut saw_dot = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                saw_digit = true;
                i += 1;
            }
            b'.' if !saw_dot => {
                saw_dot = true;
                i += 1;
            }
            b'e' | b'E' if saw_digit => {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                let exp_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == exp_start {
                    return None;
                }
                break;
            }
            _ => break,
        }
    }
    if !saw_digit || i == start_digits {
        return None;
    }
    let num = std::str::from_utf8(&bytes[..i]).ok()?;
    let value: f64 = num.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((value, i))
}

fn display_y(raw: f64, y_scale: f64, y_offset: f64, kind: DataKind) -> f64 {
    raw * (y_scale / kind.ref_span()) + y_offset
}

fn clamp_view_bounds_static(extent: PlotBounds, bounds: PlotBounds) -> PlotBounds {
    let ex0 = extent.min()[0];
    let ex1 = extent.max()[0];
    let ey0 = extent.min()[1];
    let ey1 = extent.max()[1];
    let x_span_data = (ex1 - ex0).abs().max(VIEW_MIN_SPAN_ABS);
    let y_span_data = (ey1 - ey0).abs().max(VIEW_MIN_SPAN_ABS);

    let outer_x0 = ex0 - x_span_data * (VIEW_MAX_PAD - 1.0);
    let outer_x1 = ex1 + x_span_data * (VIEW_MAX_PAD - 1.0);
    let outer_y0 = ey0 - y_span_data * (VIEW_MAX_PAD_Y - 1.0);
    let outer_y1 = ey1 + y_span_data * (VIEW_MAX_PAD_Y - 1.0);

    let min_x_span = (x_span_data / VIEW_MAX_ZOOM_X).max(VIEW_MIN_SPAN_ABS);
    let min_y_span = VIEW_MIN_SPAN_ABS;
    let max_x_span = (outer_x1 - outer_x0).abs();

    let mut x0 = bounds.min()[0];
    let mut x1 = bounds.max()[0];
    let mut y0 = bounds.min()[1];
    let mut y1 = bounds.max()[1];
    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }
    if y1 < y0 {
        std::mem::swap(&mut y0, &mut y1);
    }

    let mut x_span = (x1 - x0).max(min_x_span).min(max_x_span);
    let mut y_span = (y1 - y0).max(min_y_span);
    let mut cx = 0.5 * (x0 + x1);
    let mut cy = 0.5 * (y0 + y1);

    let clamp_center = |c: f64, outer0: f64, outer1: f64, span: f64| -> (f64, f64) {
        let half = span * 0.5;
        let lo = outer0 + half;
        let hi = outer1 - half;
        if lo <= hi && lo.is_finite() && hi.is_finite() {
            (c.clamp(lo, hi), span)
        } else {
            (
                (outer0 + outer1) * 0.5,
                (outer1 - outer0).abs().max(VIEW_MIN_SPAN_ABS),
            )
        }
    };
    (cx, x_span) = clamp_center(cx, outer_x0, outer_x1, x_span);
    (cy, y_span) = clamp_center(cy, outer_y0, outer_y1, y_span);

    PlotBounds::from_min_max(
        [cx - x_span * 0.5, cy - y_span * 0.5],
        [cx + x_span * 0.5, cy + y_span * 0.5],
    )
}

fn plot_y_to_native(y_plot: f64, y_scale: f64, y_offset: f64, kind: DataKind) -> f64 {
    let scale = y_scale / kind.ref_span();
    if scale.abs() < f64::EPSILON {
        y_plot
    } else {
        (y_plot - y_offset) / scale
    }
}

fn native_extent(points: &[[f64; 2]]) -> (f64, f64) {
    if points.is_empty() {
        return (0.0, 0.0);
    }
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for p in points {
        ymin = ymin.min(p[1]);
        ymax = ymax.max(p[1]);
    }
    if !ymin.is_finite() {
        (0.0, 0.0)
    } else {
        (ymin, ymax)
    }
}

fn series_gnd_screen_y(y_zero_plot: f64, y_lo: f64, y_span: f64, rect: egui::Rect) -> (f32, bool) {
    let t = ((y_zero_plot - y_lo) / y_span) as f32;
    let hy = egui::lerp(rect.bottom()..=rect.top(), t);
    if hy >= rect.top() && hy <= rect.bottom() {
        (hy, true)
    } else if hy < rect.top() {
        (rect.top() + 7.0, false)
    } else {
        (rect.bottom() - 7.0, false)
    }
}

fn paint_series_gnd_handle(
    painter: &egui::Painter,
    rect: egui::Rect,
    axis_cx: f32,
    hy: f32,
    on_screen: bool,
    radius: f32,
    color: Color32,
    highlighted: bool,
) {
    let origin = egui::pos2(axis_cx, hy);
    if on_screen {
        painter.line_segment(
            [egui::pos2(rect.left(), hy), egui::pos2(axis_cx - radius, hy)],
            Stroke::new(if highlighted { 1.4_f32 } else { 1.0_f32 }, color),
        );
        painter.circle_stroke(
            origin,
            radius,
            Stroke::new(if highlighted { 2.0_f32 } else { 1.2_f32 }, color),
        );
        painter.circle_filled(origin, (radius - 1.2).max(2.0), color);
    } else {
        let up = hy <= rect.top() + 8.0;
        let dy = if up { -6.0 } else { 6.0 };
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(axis_cx - 4.5, hy),
                egui::pos2(axis_cx + 4.5, hy),
                egui::pos2(axis_cx, hy + dy),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

fn format_axis_tick(value: f64, unit: &str, with_unit: bool) -> String {
    let abs = value.abs();
    let text = if abs >= 1000.0 || (abs > 0.0 && abs < 0.01) {
        format!("{value:.2e}")
    } else if abs >= 100.0 {
        format!("{value:.0}")
    } else if abs >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    };
    if with_unit && !unit.is_empty() {
        format!("{text}{unit}")
    } else {
        text
    }
}

fn format_scale(scale: f64) -> String {
    if (scale - 1.0).abs() < 1e-6 {
        "×1".into()
    } else if scale >= 10.0 || scale < 0.1 {
        format!("×{scale:.2e}")
    } else {
        format!("×{scale:.2}")
    }
}

fn status_with_counts(lang: Lang, key: &str, frames: usize, points: usize) -> String {
    tr(lang, key)
        .replace("{frames}", &frames.to_string())
        .replace("{points}", &points.to_string())
}

/// One threshold row: color swatch + checkbox + value + unit. Returns true if changed.
fn threshold_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    lang: Lang,
    kind: DataKind,
    on: &mut bool,
    value: &mut f64,
) -> bool {
    let mut dirty = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (swatch, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(swatch, CornerRadius::same(2), kind.threshold_color());
        if ui
            .checkbox(on, RichText::new(kind.label(lang)).size(ui_theme::FONT_CAPTION))
            .changed()
        {
            dirty = true;
        }
        let edit = ui.add(
            egui::DragValue::new(value)
                .speed(0.01)
                .max_decimals(4)
                .min_decimals(0),
        );
        // Persist on commit (lost focus) to avoid rewriting config every drag frame.
        if edit.lost_focus() {
            dirty = true;
        }
        ui.label(
            RichText::new(kind.unit())
                .size(ui_theme::FONT_CAPTION)
                .color(tokens.text_muted),
        );
    });
    dirty
}

fn format_eng_short(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let a = value.abs();
    if a >= 1000.0 || (a > 0.0 && a < 0.01) {
        format!("{value:.3e}")
    } else if (value - value.round()).abs() < 1e-9 {
        format!("{:.0}", value)
    } else {
        format!("{value:.3}")
    }
}

/// Plain decimal for readout (no scientific notation).
fn format_plain(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let a = value.abs();
    if (value - value.round()).abs() < 1e-9 && a < 1e12 {
        format!("{:.0}", value)
    } else if a >= 1000.0 {
        format!("{value:.1}")
    } else if a >= 1.0 {
        format!("{value:.3}")
    } else if a >= 0.001 {
        format!("{value:.4}")
    } else {
        format!("{value:.6}")
    }
}

/// Sample at X: exact match or linear interpolate between neighboring frames.
fn value_at_x(points: &[[f64; 2]], x: f64) -> Option<f64> {
    if points.is_empty() {
        return None;
    }
    if x <= points[0][0] {
        return Some(points[0][1]);
    }
    let last = points.len() - 1;
    if x >= points[last][0] {
        return Some(points[last][1]);
    }
    let mut lo = 0usize;
    let mut hi = last;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if points[mid][0] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (x0, y0) = (points[lo][0], points[lo][1]);
    let (x1, y1) = (points[hi][0], points[hi][1]);
    if (x1 - x0).abs() < 1e-15 {
        Some(y0)
    } else {
        let t = (x - x0) / (x1 - x0);
        Some(y0 + t * (y1 - y0))
    }
}

/// Nice round tick values covering [lo, hi].
fn nice_axis_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let target = target.max(2);
    let span = (hi - lo).abs();
    if !span.is_finite() || span < 1e-30 {
        return vec![lo];
    }
    let raw = span / (target as f64 - 1.0);
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let frac = raw / base;
    let nice_frac = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice_frac * base;
    let start = (lo / step).floor() * step;
    let mut ticks = Vec::new();
    let mut v = start;
    let end = hi + step * 0.5;
    let mut guard = 0;
    while v <= end && guard < 64 {
        if v >= lo - step * 1e-6 && v <= hi + step * 1e-6 {
            ticks.push(v);
        }
        v += step;
        guard += 1;
    }
    if ticks.is_empty() {
        ticks.push(lo);
        ticks.push(hi);
    }
    ticks
}

/// egui converts Shift+wheel into horizontal scroll (`delta.y = 0`) before building
/// `zoom_delta`, so plot Y-zoom via `allow_zoom(y)` never sees a non-1 factor.
/// Read the original `Event::MouseWheel` and accept horizontal deltas when Shift is held.
fn ctrl_shift_wheel_scroll(ui: &egui::Ui) -> f32 {
    let line_speed = ui.ctx().options(|o| o.line_scroll_speed);
    let screen_h = ui.ctx().screen_rect().height();
    ui.input(|i| {
        let mut scroll = 0.0_f32;
        for ev in &i.events {
            let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
            } = ev
            else {
                continue;
            };
            let zoom_mod = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
            if !(zoom_mod && modifiers.shift) {
                continue;
            }
            let notches = if delta.y.abs() >= delta.x.abs() {
                delta.y
            } else {
                delta.x
            };
            scroll += match unit {
                egui::MouseWheelUnit::Point => notches,
                egui::MouseWheelUnit::Line => notches * line_speed,
                egui::MouseWheelUnit::Page => notches * screen_h,
            };
        }
        scroll
    })
}

fn series_color(index: usize) -> Color32 {
    let ch = format!("CH{}", (index % MAX_SERIES) + 1);
    tek_channel_color(&ch).unwrap_or(Color32::from_rgb(0x3B, 0x82, 0xF6))
}

fn is_data_source_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("txt" | "csv" | "log")
    )
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(aa), Ok(bb)) => aa == bb,
        _ => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

/// Prefer UTF-8; fall back to lossy decode so GBK/ANSI logs still open.
fn read_text_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(e) => Ok(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

fn panel_in_rect(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    if !rect.is_positive() {
        return;
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            add(ui);
        },
    );
}

fn card_in_rect(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tokens: &Tokens,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    panel_in_rect(ui, rect, |ui| {
        ui_theme::section_frame(tokens).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.set_max_size(ui.available_size());
            ui_theme::section_title(ui, tokens, title);
            let body = ui.available_size();
            ui.allocate_ui_with_layout(body, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(body);
                ui.set_max_size(body);
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                add(ui);
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_extract_basic() {
        let text = "\
HDR id=1
TEMP 25.5
VOLT 3.3
HDR id=2
TEMP 26.0
VOLT 3.4
";
        let parsed = parse_data_file(text, "HDR", &["TEMP", "VOLT"]);
        assert_eq!(parsed.frame_count, 2);
        assert_eq!(parsed.series[0], vec![[0.0, 25.5], [1.0, 26.0]]);
        assert_eq!(parsed.series[1], vec![[0.0, 3.3], [1.0, 3.4]]);
    }

    #[test]
    fn requires_whitespace_after_filter() {
        let frame = "TEMP25.5 TEMP 26.1";
        assert_eq!(extract_first_filter_value(frame, "TEMP"), Some(26.1));
    }

    #[test]
    fn first_match_only_per_frame() {
        let frame = "TEMP 1.0 TEMP 2.0";
        assert_eq!(extract_first_filter_value(frame, "TEMP"), Some(1.0));
    }

    #[test]
    fn rejects_mid_token_filter() {
        let frame = "MYTEMP 9.0 TEMP 3.0";
        assert_eq!(extract_first_filter_value(frame, "TEMP"), Some(3.0));
    }

    #[test]
    fn scientific_number() {
        let frame = "GAIN 1.5e-3";
        assert_eq!(extract_first_filter_value(frame, "GAIN"), Some(1.5e-3));
    }

    #[test]
    fn kind_mapping_smoke() {
        assert_eq!(DataKind::from_config_str("voltage").as_config_str(), "voltage");
        assert_eq!(DataKind::from_config_str("current").as_config_str(), "current");
        assert_eq!(
            DataKind::from_config_str("temperature").as_config_str(),
            "temperature"
        );
        assert!((DataKind::Voltage.ref_span() - 10_000.0).abs() < 1e-12);
        assert!((DataKind::Current.ref_span() - 5_000.0).abs() < 1e-12);
        assert!((DataKind::Temperature.ref_span() - 50.0).abs() < 1e-12);
        assert_eq!(DataKind::Voltage.unit(), "mV");
        assert_eq!(DataKind::Current.unit(), "mA");

        let y = display_y(10_000.0, 1.0, 2.0, DataKind::Voltage);
        assert!((y - 3.0).abs() < 1e-12);
        let native = plot_y_to_native(y, 1.0, 2.0, DataKind::Voltage);
        assert!((native - 10_000.0).abs() < 1e-12);

        let y_a = display_y(5_000.0, 1.0, 0.0, DataKind::Current);
        assert!((y_a - 1.0).abs() < 1e-12);
        let y_t = display_y(50.0, 1.0, 0.0, DataKind::Temperature);
        assert!((y_t - 1.0).abs() < 1e-12);

        assert_eq!(DataSource::from_config_str("serial").as_config_str(), "serial");
        assert_eq!(DataSource::from_config_str("file").as_config_str(), "file");
    }
}
