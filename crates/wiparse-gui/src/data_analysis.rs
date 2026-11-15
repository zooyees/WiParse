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

pub(crate) mod ai_extract;
mod source_location;
mod retained_rows;
mod history;
mod reviewed_data;
pub(crate) use reviewed_data::ReviewedDataSnapshot;
use history::Points as HistoryPoints;
use wiparse_core::extract::{Record, Selector};
use crate::theme::{self as ui_theme, Tokens};
use crate::waveform_analysis::tek_channel_color;
use crate::data_extract_cache::{
    self, downsample_minmax, file_content_fp, list_cached_extracts, live_cache_path,
    live_flush_interval, load_wida, WidaPayload,
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Vec2b};
use egui_plot::{HLine, Line, LineStyle, Plot, PlotBounds, PlotPoints, Points};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, tr_fmt, Lang};
use wiparse_core::paths::project_path;

const TOOLBAR_H: f32 = 40.0;
const SIDE_W: f32 = 248.0;
const PANEL_GAP: f32 = ui_theme::PANEL_GAP;
const CARD_MARGIN_X: i8 = 6;
use wiparse_core::extract::MAX_SERIES;
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
    connect: bool,
}

/// Background open/parse job (keeps UI responsive with a spinner).
struct PendingDataLoad {
    rx: Receiver<LoadEvent>,
    rules_revision: u64,
}

struct CacheJob {
    session_id: String,
    series_count: usize,
    handle: thread::JoinHandle<Result<Vec<HistoryPoints>, String>>,
    rebuilding: bool,
    cancel: Arc<std::sync::atomic::AtomicBool>,
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
    field_catalog: Option<wiparse_core::extract::FieldCatalog>,
    path: Option<PathBuf>,
    file_text: Option<Arc<String>>,
    frame_count: usize,
    series: Vec<SeriesDraft>,
    from_cache: bool,
    frame_header: Option<String>,
    filters: Option<Vec<String>>,
    kinds: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataKind {
    Voltage,
    Current,
    Temperature,
    Power,
    Frequency,
    Scalar,
}

impl DataKind {
    fn unit(self) -> &'static str {
        match self {
            Self::Voltage => "mV",
            Self::Current => "mA",
            Self::Temperature => "\u{00B0}C",
            Self::Power=>"W", Self::Frequency=>"Hz", Self::Scalar=>"—",
        }
    }

    /// Full-scale native span for default display mapping (≈10 V / 5 A / 50 °C).
    fn ref_span(self) -> f64 {
        match self {
            Self::Voltage => 10_000.0, // mV
            Self::Current => 5_000.0,  // mA
            Self::Temperature => 50.0,
            Self::Power=>100., Self::Frequency=>100_000., Self::Scalar=>1.,
        }
    }

    fn as_config_str(self) -> &'static str {
        match self {
            Self::Voltage => "voltage",
            Self::Current => "current",
            Self::Temperature => "temperature",
            Self::Power=>"power", Self::Frequency=>"frequency", Self::Scalar=>"scalar",
        }
    }

    fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "power"=>Self::Power,"frequency"=>Self::Frequency, "scalar"=>Self::Scalar,
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
            Self::Power=>"功率 / Power".into(),Self::Frequency=>"频率 / Frequency".into(),Self::Scalar=>"原数值 / Raw".into(),
        }
    }

    /// Threshold dashed-line color: mV yellow / mA blue / °C orange.
    fn threshold_color(self) -> Color32 {
        match self {
            Self::Voltage => Color32::from_rgb(0xF7, 0xD6, 0x18),
            Self::Current => Color32::from_rgb(0x3B, 0x82, 0xF6),
            Self::Temperature => Color32::from_rgb(0xFF, 0x9F, 0x0A),
            Self::Power=>Color32::from_rgb(150,95,220),Self::Frequency=>Color32::from_rgb(30,170,150),Self::Scalar=>Color32::GRAY,
        }
    }

    fn short_axis(self) -> &'static str {
        match self {
            Self::Voltage => "mV",
            Self::Current => "mA",
            Self::Temperature => "\u{00B0}C",
            Self::Power=>"W", Self::Frequency=>"Hz", Self::Scalar=>"—",
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
    unit_override: Option<String>,
    label: String,
    color: Color32,
    kind: DataKind,
    /// Samples as (frame_index, native value) — at most one point per frame.
    points: HistoryPoints,
    y_scale: f64,
    y_offset: f64,
    /// Native Y extents (raw values).
    y_min: f64,
    y_max: f64,
    /// Display-space cache: `[x, display_y]` rebuilt when scale/offset/kind/points dirty.
    mapped: Vec<[f64; 2]>,
    disp_dirty: bool,
}

/// Immutable metadata and field catalogue captured at one publication boundary.
/// Raw samples remain local and are not represented by this snapshot.
pub(crate) struct PublishedDataSnapshot {
    id: String,
    summary: Value,
    fields: Arc<Vec<(String, String)>>,
}

impl PublishedDataSnapshot {
    pub(crate) fn review_ui_state(&self)->Value {
        json!({"source_mode":self.summary["source"],"series_slots":self.summary["series_count"],
            "view":self.summary.get("view").cloned().unwrap_or_else(||json!("source")),
            "live_view":self.summary["live_view"],
            "scope":"published UI state only; raw samples, source contents, fields, measurements and external file integrity are not included"})
    }
    pub(crate) fn page(&self, offset: usize, limit: usize) -> Value {
        let mut page = self.summary.clone();
        page["snapshot_id"] = json!(self.id);
        let end = offset.saturating_add(limit).min(self.fields.len());
        page["field_candidates"] = json!(&self.fields[offset.min(end)..end]);
        page["next_offset"] = json!((end < self.fields.len()).then_some(end));
        page
    }
}

pub struct DataAnalysisPanel {
    // The primary panel continues to own acquisition and file jobs while this plot is visible.
    result_plot: Option<Box<DataAnalysisPanel>>,
    showing_result: bool,
    source_view: Option<source_location::Viewer>,
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
    agent_instance: uuid::Uuid,
    agent_generation: u64,
    load_rules_revision: u64,
    source_lease: wiparse_core::ai::source_lease::SourceLease,
    data_review_lease: wiparse_core::ai::source_lease::SourceLease,
    published_ai: Option<Arc<PublishedDataSnapshot>>,
    extract_cache: Arc<std::sync::Mutex<ai_extract::ExtractCache>>,
    imported_query: Option<String>,
    imported_quality: Option<Value>,
    frame_count: usize,
    selected: Option<usize>,
    dragging_offset: Option<usize>,
    dragging_scale: Option<usize>,

    source: DataSource,
    live_cursor: usize,
    discovered_fields: Vec<(String,String)>,
    field_catalog_truncated: bool,
    field_search: String,
    live_rules: Option<(Vec<String>, Result<Vec<Option<Selector>>,String>)>,
    live_tail: String,
    live_auto_fit: bool,
    recent_view: bool,
    export_job:Option<thread::JoinHandle<Result<String,String>>>,
    export_message:Option<String>,
    manual_y: bool,
    history_error: Option<String>,
    cache_quarantined: bool,
    cache_rebuilding: bool,
    live_source_generation: Option<u64>,
    gap_notice: Option<String>,
    cache_job: Option<CacheJob>,
    last_checkpoint: std::time::Instant,
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

fn agent_display_unit(unit:&str)->Result<(DataKind,f64),String>{match unit{"V"=>Ok((DataKind::Voltage,1000.)),"mV"=>Ok((DataKind::Voltage,1.)),"A"=>Ok((DataKind::Current,1000.)),"mA"=>Ok((DataKind::Current,1.)),"C"|"°C"=>Ok((DataKind::Temperature,1.)),"W"=>Ok((DataKind::Power,1.)),"mW"=>Ok((DataKind::Power,0.001)),"Hz"=>Ok((DataKind::Frequency,1.)),"kHz"=>Ok((DataKind::Frequency,1000.)),_ if unit.len()<=32 && !unit.chars().any(char::is_control)=>Ok((DataKind::Scalar,1.)),_=>Err(format!("DATA.INVALID_UNIT: {}",unit))}}

impl DataAnalysisPanel {
    pub(crate) fn set_source_lease(&mut self, lease: wiparse_core::ai::source_lease::SourceLease) { self.source_lease = lease; }
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
        let live_session_id = uuid::Uuid::new_v4().to_string();
        let mut panel = Self {
            status: String::new(),
            browser_dir,
            browser_scanned_dir: String::new(),
            browser_folders: Vec::new(),
            last_open_path: None,
            result_plot: None, showing_result: false,
            source_view:None,
            file_text: None,
            frame_header: da.frame_header.clone(),
            series_count,
            filters,
            kinds,
            series: Vec::new(),
            agent_instance: uuid::Uuid::new_v4(),
            agent_generation: 0,
            load_rules_revision: 0,
            source_lease: Default::default(),
            data_review_lease: Default::default(),
            published_ai: None,
            extract_cache: Default::default(),
            imported_query: None,
            imported_quality:None,
            frame_count: 0,
            selected: None,
            dragging_offset: None,
            dragging_scale: None,
            source,
            live_cursor: 0,
            discovered_fields:Vec::new(), field_catalog_truncated:false, field_search:String::new(),
            live_rules:None,
            live_tail: String::new(),
            recent_view: false,
            export_job:None,
            export_message:None,
            manual_y: false,
            history_error: None,
            cache_quarantined: false,
            cache_rebuilding: false,
            live_source_generation: None,
            gap_notice: None,
            cache_job: None,
            last_checkpoint:std::time::Instant::now(),
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
        panel.publish_ai_snapshot();
        panel
    }

    pub fn status_text(&self) -> &str {
        self.visible_result().map_or(&self.status, |view| &view.status)
    }

    fn visible_result(&self)->Option<&Self>{
        self.result_plot.as_deref().filter(|_|self.showing_result)
    }

    pub fn needs_repaint(&self) -> bool {
        if self.pending_load.is_some() || self.cache_job.is_some() || self.export_job.is_some() {
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

    pub fn prepare_live_poll(
        &mut self,
        total: usize,
        dropped: usize,
        source: (u64, usize),
        lang: Lang,
    ) -> bool {
        let may_change=self.wants_live_serial() && (self.live_source_generation!=Some(source.0)
            || self.live_cursor>total || self.live_cursor<dropped
            || (self.last_checkpoint.elapsed()>=std::time::Duration::from_secs(1) && self.live_frames_since_flush>0));
        let update=may_change.then(||self.source_lease.begin_update());
        let result=self.prepare_live_poll_inner(total,dropped,source,lang);
        if update.is_some() { self.publish_ai_snapshot(); }
        drop(update);result
    }
    fn prepare_live_poll_inner(&mut self,total:usize,dropped:usize,source:(u64,usize),lang:Lang)->bool {
        if !self.wants_live_serial() {return false;}
        if self.last_checkpoint.elapsed() >= std::time::Duration::from_secs(1) && self.live_frames_since_flush>0 {
            self.checkpoint_live(false);
        }
        let changed = self
            .live_source_generation
            .is_some_and(|old| old != source.0);
        if changed || self.live_cursor > total || self.live_cursor < dropped {
            let lost = dropped.saturating_sub(self.live_cursor);
            let cursor = self.live_cursor;
            self.clear_data(lang);
            if self.history_error.is_some(){return false;}
            self.live_cursor = dropped.max(source.1);
            self.gap_notice = Some(if lost > 0 {
                format!(
                    "串口日志行 {}–{} 已移出接收缓冲区，本段从第 {} 行开始",
                    cursor + 1,
                    dropped,
                    dropped + 1
                )
            } else {
                "串口来源或日志已重置，已开始新的分析会话".into()
            });
        }
        if self.live_source_generation.is_none() {
            self.live_cursor = self.live_cursor.max(source.1);
        }
        self.live_source_generation = Some(source.0);
        true
    }

    /// Append serial RX lines; `next_cursor` becomes the new live cursor after ingest.
    pub fn ingest_live_lines(&mut self, lines: &[String], next_cursor: usize, lang: Lang) {
        if !matches!(self.source,DataSource::SerialLive){return;}
        let update=(!lines.is_empty() || self.live_cursor!=next_cursor).then(||self.source_lease.begin_update());
        self.ingest_live_lines_inner(lines,next_cursor,lang);
        if update.is_some() { self.publish_ai_snapshot(); }
        drop(update);
    }
    fn ingest_live_lines_inner(&mut self, lines: &[String], next_cursor: usize, lang: Lang) {
        if !matches!(self.source, DataSource::SerialLive) {
            return;
        }
        if lines.is_empty() {
            self.live_cursor = next_cursor;
            return;
        }

        if self.cache_job.as_ref().is_some_and(|job|job.handle.is_finished()){self.finish_checkpoint();}
        let header = self.frame_header.trim().to_owned();
        if !header.is_empty(){
        if !self.live_tail.is_empty() && !self.live_tail.ends_with('\n') {
            self.live_tail.push('\n');
        }
        for line in lines {
            self.live_tail.push_str(line);
            self.live_tail.push('\n');
        }
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

        if self.live_rules.as_ref().is_none_or(|(rules,_)|rules!=&filters) {
            let compiled=compile_filters(&filters.iter().map(String::as_str).collect::<Vec<_>>());
            self.live_rules=Some((filters,compiled));
        }
        let selectors=match &self.live_rules.as_ref().unwrap().1 {
            Ok(v)=>v, Err(error)=>{self.status=error.clone();self.live_cursor=next_cursor;return;}
        };
        let frames;
        let (complete, remain) = if header.is_empty() {
            frames = lines.iter().map(String::as_str).collect::<Vec<_>>();
            (frames.as_slice(), "")
        } else {
            frames = split_frames(&self.live_tail, &header);
            let Some((last, complete)) = frames.split_last() else {
                self.live_tail.clear();
                self.live_cursor = next_cursor;
                return;
            };
            (complete, *last)
        };
        let base_fi = self.frame_count;


        let mut updates = vec![Vec::new(); selectors.len()];
        for (rel, frame) in complete.iter().enumerate() {
            let x = (base_fi + rel) as f64;
            let record = Record::new(frame);
            for (si, selector) in selectors.iter().enumerate() {
                if let Some(value) = selected_number(selector.as_ref(), &record) {
                    updates[si].push([x, value]);
                }
            }
        }
        let added = complete.len();
        self.source_lease.clone().mutate_always(|| {
            for (si, points) in updates.into_iter().enumerate() {
                let s = &mut self.series[si];
                for [x, value] in points {
                    if s.points.is_empty() {
                        s.y_min = value;
                        s.y_max = value;
                    } else {
                        s.y_min = s.y_min.min(value);
                        s.y_max = s.y_max.max(value);
                    }
                    s.points.push([x, value]);

                    s.disp_dirty = true;
                }
            }
            for series in &mut self.series { series.points.seal(); }
            self.frame_count = base_fi + added;
        });
        const MAX_PARTIAL_FRAME:usize=256*1024;
        if remain.len()>MAX_PARTIAL_FRAME {
            self.gap_notice=Some(format!("串口日志第 {next_cursor} 行存在超长未结束帧，已跳过该帧"));
            self.live_tail.clear();
        } else {self.live_tail = remain.to_owned();}
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
        if self.live_frames_since_flush >= live_flush_interval() && self.last_checkpoint.elapsed()>=std::time::Duration::from_millis(250) {
            self.checkpoint_live(false);
        }

        let points: usize = self.series.iter().map(|s| s.points.len()).sum();
        if self.frame_count == 0 {
            self.status = tr(lang, "data.status_live_wait");
        } else {
            self.status = status_with_counts(lang, "data.status_live", self.frame_count, points);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let may_change=ui.input(|input|!input.events.is_empty()) || self.pending_load.is_some()
            || self.cache_job.as_ref().is_some_and(|job|job.handle.is_finished())
            || self.export_job.as_ref().is_some_and(|job|job.is_finished());
        let update=may_change.then(||self.source_lease.begin_update());
        self.ui_inner(ui,lang,tokens);
        if update.is_some() { self.publish_ai_snapshot(); }
        drop(update);
    }
    fn ui_inner(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        if let Some(view)=&mut self.source_view {if !view.show(ui.ctx()) {self.source_view=None;}}
        if self.cache_job.as_ref().is_some_and(|job|job.handle.is_finished()) {self.finish_checkpoint();}
        if self.export_job.as_ref().is_some_and(|job|job.is_finished()) {
            self.export_message=Some(self.export_job.take().unwrap().join().unwrap_or_else(|_|Err("导出线程退出".into())).unwrap_or_else(|e|e));
        }
        self.poll_pending_load(lang);
        if self.pending_load.is_some() {
            // Keep spinner animating even if this frame started before the job was queued.
            ui.ctx().request_repaint();
        }
        if self.result_plot.is_some(){
            ui.horizontal(|ui|{
                let mut showing = self.showing_result;
                ui.selectable_value(&mut showing,false,if lang==Lang::Zh{"当前数据源"}else{"Data source"});
                ui.selectable_value(&mut showing,true,if lang==Lang::Zh{"AI 提取结果"}else{"AI result"});
                if showing != self.showing_result { wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| self.showing_result=showing); }
                if self.showing_result && ui.button(tr(lang,"data.fit")).clicked(){
                    if let Some(view)=self.result_plot.as_mut(){view.manual_y=false;view.fit_request=true;view.pending_bounds=None;}
                }
                if ui.small_button("×").on_hover_text(if lang==Lang::Zh{"关闭提取结果"}else{"Close result"}).clicked(){
                    let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {self.showing_result=false;self.result_plot.take()});
                    drop(old);
                }
            });
            if self.showing_result{
                if let Some(view)=&mut self.result_plot {
                    ui.push_id("ai-result-plot",|ui|view.plot_area(ui,lang,tokens));return;
                }
            }
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
        let body = egui::Rect::from_min_max(egui::pos2(full.left(), body_top), full.max);
        let side_w = ui_theme::resizable_sidebar_width(ui, "data_analysis", body,
            SIDE_W, ui_theme::SIDEBAR_MIN_W, 200.0, PANEL_GAP, tokens);
        let plot_w = (full.width() - side_w - PANEL_GAP).max(0.0);
        let side_rect =
            egui::Rect::from_min_size(egui::pos2(full.min.x, body_top), egui::vec2(side_w, body_h));
        let plot_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, body_top),
            egui::vec2(plot_w, body_h),
        );

        let browser_h = (body_h * 0.25).clamp(120.0, 220.0).min((body_h * 0.45).max(1.0));
        let rules_h = (body_h - browser_h - PANEL_GAP).max(1.0);
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
            .inner_margin(Margin::symmetric(6, 6))
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
                        if self.wants_live_serial() { if lang==Lang::Zh {"全程".into()}else{"Full".into()} } else {tr(lang, "data.fit")},
                        egui::vec2(64.0, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.manual_y = false;
                        self.recent_view = false;
                        self.live_auto_fit = self.wants_live_serial();
                        self.fit_request = true;
                        self.pending_bounds = if self.live_auto_fit {self.live_fit_bounds()} else {None};
                    }
                    if self.wants_live_serial() && ui.selectable_label(self.live_auto_fit && self.recent_view, if lang==Lang::Zh {"最近 80 帧"}else{"Last 80"}).clicked() {
                        self.live_auto_fit = true;
                        self.recent_view = true;
                        self.pending_bounds = self.live_fit_bounds();
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
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 6))
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
                if ui.add_enabled(self.frame_count>0 && self.export_job.is_none(),egui::Button::new(if lang==Lang::Zh {"导出全程 CSV"}else{"Export full CSV"})).clicked(){
                    if let Some(path)=rfd::FileDialog::new().set_file_name("capture.csv").add_filter("CSV",&["csv"]).save_file(){
                        let curves=self.series.iter().map(|s|(s.label.clone(),s.unit_override.as_deref().unwrap_or(s.kind.unit()).to_string(),s.points.clone())).collect::<Vec<_>>();
                        self.export_message=None;
                        self.export_job=Some(thread::spawn(move||history::export_csv(&path,&curves).map(|_|format!("已导出 {}",path.display())).map_err(|e|format!("导出失败：{e}"))));
                    }
                }
                if let Some(message)=&self.export_message {ui.label(message);}
                if let Some(notice)=&self.gap_notice {ui.label(notice);}
                if let Some(error)=&self.history_error { ui.colored_label(tokens.text_primary,error); }
                if self.cache_quarantined && ui.button(if lang==Lang::Zh {"保留数据并重建历史缓存"} else {"Rebuild history cache without clearing data"}).clicked() {
                    self.rebuild_quarantined_cache();
                }
                if self.cache_rebuilding {
                    ui.weak(if lang==Lang::Zh {"历史缓存重建中；停止后已开始的写入可能仍会完成。"} else {"Rebuilding history cache; an in-progress write may still finish after cancellation."});
                    if ui.button(if lang==Lang::Zh {"停止重建"} else {"Stop rebuild"}).clicked() {
                        if let Some(job)=self.cache_job.as_ref().filter(|job|job.rebuilding) { job.cancel.store(true,std::sync::atomic::Ordering::Release); }
                    }
                }
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

    fn rules_panel(&mut self,ui:&mut egui::Ui,lang:Lang,tokens:&Tokens){
        let mut apply_now=false;let mut source_changed=false;
        Frame::NONE.fill(tokens.surface_bg).stroke(Stroke::new(1.,tokens.divider)).corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD)).inner_margin(8).show(ui,|ui|{
            ui.set_min_height(ui.available_height());
            let width=ui.available_width();
            ui.horizontal(|ui|{ui.label(RichText::new(tr(lang,"data.rules")).size(ui_theme::FONT_TITLE).strong());ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{if ui_theme::primary_btn_sized(ui,tokens,tr(lang,"data.apply"),egui::vec2(56.,28.)).clicked(){apply_now=true;}});});
            ui.separator();
            egui::ScrollArea::vertical().id_salt("data-rules-scroll").auto_shrink([false,false]).show(ui,|ui|{
                ui.set_width(width);ui.spacing_mut().item_spacing=egui::vec2(6.,6.);
                ui_theme::form_row(ui,&tr(lang,"data.source"),|ui|{
                    let before=self.source;
                    let mut next=before;
                    egui::ComboBox::from_id_salt("data_source").selected_text(tr(lang,if self.source==DataSource::File{"data.source_file"}else{"data.source_serial"})).width(ui.available_width()).show_ui(ui,|ui|{ui.selectable_value(&mut next,DataSource::File,tr(lang,"data.source_file"));ui.selectable_value(&mut next,DataSource::SerialLive,tr(lang,"data.source_serial"));});
                    if before!=next {wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.source=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});source_changed=true;}
                });
                ui_theme::form_row(ui,&tr(lang,"data.frame_header"),|ui|{let mut next=self.frame_header.clone();let r=ui.add_sized([ui.available_width(),ui_theme::CTRL_H],egui::TextEdit::singleline(&mut next).hint_text(if lang==Lang::Zh{"留空按行"}else{"Blank: one row per line"}));apply_now|=r.lost_focus();if r.changed(){wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.frame_header=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});}r.on_hover_text(tr(lang,"data.frame_header_hint"));});
                ui_theme::form_row(ui,&tr(lang,"data.series_count"),|ui|{let before=self.series_count;let mut next=before;egui::ComboBox::from_id_salt("data_series_count").selected_text(before.to_string()).width(ui.available_width()).show_ui(ui,|ui|{for n in 1..=MAX_SERIES{ui.selectable_value(&mut next,n,n.to_string());}});if before!=next{wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.series_count=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});apply_now=true;}});
                ui.add_space(2.);ui.separator();
                for i in 0..self.series_count{
                    ui.push_id(i,|ui|{
                        ui.horizontal(|ui|{
                            let(rect,_)=ui.allocate_exact_size(egui::vec2(8.,8.),egui::Sense::hover());ui.painter().rect_filled(rect,2,series_color(i));
                            ui.label(RichText::new(format!("{} {}",if lang==Lang::Zh{"数据"}else{"Series"},i+1)).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{let before=self.kinds[i];let mut next=before;egui::ComboBox::from_id_salt("data_kind").width(68.).selected_text(before.short_axis()).show_ui(ui,|ui|{for k in [DataKind::Voltage,DataKind::Current,DataKind::Temperature,DataKind::Power,DataKind::Frequency,DataKind::Scalar]{ui.selectable_value(&mut next,k,format!("{} ({})",k.label(lang),k.short_axis()));}});if before!=next{wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.kinds[i]=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});apply_now=true;}});
                        });
                        if self.source == DataSource::File && self.file_text.is_some() && !self.discovered_fields.is_empty() {
                            ui.menu_button(if lang==Lang::Zh{"选择已识别字段"}else{"Detected fields"},|ui|{
                                ui.set_min_width(200.);
                                ui.add(egui::TextEdit::singleline(&mut self.field_search).hint_text(if lang==Lang::Zh{"搜索字段或选择器"}else{"Search fields or selectors"}));
                                let query=self.field_search.trim().to_lowercase();
                                let visible: Vec<_> = self.discovered_fields.iter().filter(|(label,sel)|query.is_empty() || label.to_lowercase().contains(&query) || sel.to_lowercase().contains(&query)).collect();
                                ui.small(format!("{} / {}{}",visible.len(),self.discovered_fields.len(),if self.field_catalog_truncated {"+"} else {""}));
                                let height=ui.spacing().interact_size.y;
                                egui::ScrollArea::vertical().max_height(320.).show_rows(ui,height,visible.len(),|ui,rows|{
                                    for row in rows {
                                        let (label,selector)=visible[row];
                                        if ui.selectable_label(self.filters[i]==*selector,format!("{label}  ·  {selector}")).clicked(){
                                            let next=selector.clone();wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.filters[i]=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});apply_now=true;ui.close_menu();
                                        }
                                    }
                                });
                            });
                        }
                        let mut next=self.filters[i].clone();let edit=ui.add_sized([ui.available_width(),ui_theme::CTRL_H],egui::TextEdit::singleline(&mut next).hint_text(if lang==Lang::Zh{"提取字段，如 TEMP"}else{"Field, e.g. TEMP"}));apply_now|=edit.lost_focus();if edit.changed(){wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.filters[i]=next;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});}edit.on_hover_text(tr(lang,"data.filter_hint"));
                    });
                }
                ui.separator();ui.label(RichText::new(tr(lang,"data.thresholds")).strong()).on_hover_text(tr(lang,"data.threshold_hint"));
                let mut voltage=(self.thr_voltage_on,self.thr_voltage);
                let mut current=(self.thr_current_on,self.thr_current);
                let mut temperature=(self.thr_temperature_on,self.thr_temperature);
                let mut dirty=threshold_row(ui,tokens,lang,DataKind::Voltage,&mut voltage.0,&mut voltage.1);
                dirty|=threshold_row(ui,tokens,lang,DataKind::Current,&mut current.0,&mut current.1);
                dirty|=threshold_row(ui,tokens,lang,DataKind::Temperature,&mut temperature.0,&mut temperature.1);
                self.commit_thresholds(voltage,current,temperature);
                if dirty{self.persist_rules();}
            });
        });
        if source_changed{self.on_source_changed(lang);self.persist_rules();}else if apply_now{self.persist_rules();self.apply_rules(lang);}
    }

    fn commit_thresholds(&mut self, voltage:(bool,f64), current:(bool,f64), temperature:(bool,f64)) {
        let old=[(self.thr_voltage_on,self.thr_voltage.to_bits()),(self.thr_current_on,self.thr_current.to_bits()),(self.thr_temperature_on,self.thr_temperature.to_bits())];
        let next=[(voltage.0,voltage.1.to_bits()),(current.0,current.1.to_bits()),(temperature.0,temperature.1.to_bits())];
        if old != next { wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
            (self.thr_voltage_on,self.thr_voltage)=voltage;
            (self.thr_current_on,self.thr_current)=current;
            (self.thr_temperature_on,self.thr_temperature)=temperature;
        }); }
    }

    fn browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 6))
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
        let x_label = if self.imported_seconds(){"时间 / s".into()}else{tr(lang, "data.axis_x")};
        let y_label = self.y_axis_label_for_plot(lang);
        let thresholds = self.threshold_display_lines();
        let selected = self.selected;
        let live = matches!(self.source, DataSource::SerialLive);

        let total_w = ui.available_width();
        let nch = self.series.iter().filter(|s| !s.points.is_empty()).count();
        let strip_w = if nch > 0 { 74.0 } else { 0.0 };
        let plot_w = (total_w - strip_w - 3.0).max(80.0);
        let plot_width_px = plot_w.max(64.0);

        // Build draw lists outside the plot closure (no per-frame series.clone).
        let (x0_hint, x1_hint) = pending_clamped.or(fit_bounds).map(|b|(b.min()[0],b.max()[0])).or(self.last_x_range).unwrap_or_else(|| {
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
                    let plot_tokens=if tokens.scope_dark{*tokens}else{Tokens{plot_bg:tokens.surface_bg,plot_fg:tokens.text_primary,..*tokens}};
                    ui_theme::with_plot_well_visuals(ui, &plot_tokens, |ui| {
                        let response = Plot::new("data_analysis_plot").grid_spacing(egui::emath::Rangef::new(28.,140.))
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
                                    if item.connect {plot_ui.line(
                                        Line::new(pts)
                                            .name(item.label.clone())
                                            .color(ui_theme::readable_on(item.color,plot_tokens.plot_bg))
                                            .width(width),
                                    );}
                                    if item.draw_dots {
                                        let dots =
                                            PlotPoints::from_iter(item.line.iter().copied());
                                        plot_ui.points(
                                            Points::new(dots).name(if item.connect {String::new()}else{item.label.clone()}).color(ui_theme::readable_on(item.color,plot_tokens.plot_bg)).radius(2.4_f32),
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
                                            .color(ui_theme::readable_on(kind.threshold_color(),plot_tokens.plot_bg))
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
                        let actual_bounds=response.transform.bounds();
                        next_x_range=Some((actual_bounds.min()[0],actual_bounds.max()[0]));
                        next_y_range=Some((actual_bounds.min()[1],actual_bounds.max()[1]));
                        plot_hovered = response.response.hovered();
                        plot_clicked = response.response.clicked();
                        if live && (response.response.dragged() && ui.input(|i|i.pointer.delta().x != 0.0)
                            || response.response.dragged_by(egui::PointerButton::Secondary)
                            || plot_hovered && ui.input(|i| (ctrl && !shift && i.zoom_delta()!=1.0) || (!ctrl && i.smooth_scroll_delta.x!=0.0))) {
                            self.live_auto_fit = false;
                        }
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
            let val = if let Some(x) = cursor_x {
                value_at_x(&s.points, x)
            } else {
                s.points.last().map(|p| p[1])
            };
            let text = match val {
                Some(v) => format!("{}{}", format_plain(v), s.unit_override.as_deref().unwrap_or(s.kind.unit())),
                None => "—".into(),
            };
            let label = if s.label.is_empty() {
                format!("{}", i + 1)
            } else {
                s.label.clone()
            };
            rows.push((i, ui_theme::readable_on(s.color,tokens.surface_bg), label, text));
        }
        if rows.is_empty() {
            return;
        }

        let mode = if live && cursor_x.is_none() {
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
            tokens.surface_bg,
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
                (s.kind, ui_theme::readable_on(s.color,tokens.surface_bg), s.y_scale, s.y_offset)
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
        let header_h = 30.0_f32;
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
            egui::pos2(rect.center().x, rect.top() + 17.0),
            egui::Align2::CENTER_TOP,
            format!("{}{}/div", format_eng_short(per_div), y_unit),
            egui::FontId::monospace(10.0),
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
                    egui::FontId::monospace(10.0),
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
            let color = ui_theme::readable_on(s.color,tokens.surface_bg);
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
                        egui::FontId::monospace(10.0),
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
                    self.manual_y = true;
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
                self.manual_y = true;
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
                    self.manual_y = true;
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

    fn start_live_session(&mut self) {
        self.live_session_id = uuid::Uuid::new_v4().to_string();
        self.cache_quarantined = false;
        self.cache_rebuilding = false;
        self.recent_view = false;
        self.gap_notice = None;
        self.live_source_generation = None;
        self.manual_y = false;
    }

    fn clear_data(&mut self, lang: Lang) {
        self.data_review_lease.mutate_always(||());
        if matches!(self.source, DataSource::SerialLive) && self.frame_count > 0 {
            self.flush_live_cache();
            if self.history_error.is_some(){return;}
        }
        let (old_series,old_text,old_load) = wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
            self.imported_query=None;self.imported_quality=None;
            self.last_open_path = None;
            self.agent_generation = self.agent_generation.wrapping_add(1);
            self.load_rules_revision = self.load_rules_revision.saturating_add(1);
            self.frame_count = 0;
            (std::mem::take(&mut self.series), self.file_text.take(), self.pending_load.take())
        });
        drop((old_series,old_text,old_load));
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
        self.status = tr(lang, "data.status_cleared");
        if matches!(self.source, DataSource::SerialLive) {
            self.start_live_session();
            self.live_auto_fit = true;
            self.status = tr(lang, "data.status_live_wait");
        }
    }

    fn on_source_changed(&mut self, lang: Lang) {
        match self.source {
            DataSource::SerialLive => {
                let (old_points,old_text,old_load)=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
                    self.start_live_session();
                    let old_points:Vec<_>=self.series.iter_mut().map(|s| {
                        let old=(std::mem::take(&mut s.points),std::mem::take(&mut s.mapped));
                        s.disp_dirty=true;s.y_min=0.0;s.y_max=0.0;s.y_scale=1.0;s.y_offset=0.0;old
                    }).collect();
                    self.agent_generation=self.agent_generation.wrapping_add(1);
                    self.load_rules_revision=self.load_rules_revision.saturating_add(1);
                    self.frame_count=0;
                    (old_points,self.file_text.take(),self.pending_load.take())
                });
                drop((old_points,old_text,old_load));
                self.live_tail.clear();
                self.live_cursor = 0;
                self.live_auto_fit = true;
                self.live_plot_dirty = false;
                self.live_frames_since_flush = 0;
                self.cached_extent = None;
                self.pending_bounds = None;
                self.fit_request = false;
                self.invalidate_lod();
                self.status = tr(lang, "data.status_live_wait");
            }
            DataSource::File => {
                if self.frame_count > 0 {
                    self.flush_live_cache();
                    if self.history_error.is_some(){wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{self.source=DataSource::SerialLive;self.load_rules_revision=self.load_rules_revision.saturating_add(1);});return;}
                }
                self.refresh_data_browser();
                self.live_auto_fit = false;
                self.live_tail.clear();
                self.live_cursor = 0;
                self.live_plot_dirty = false;
                let old_load=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||self.pending_load.take());drop(old_load);
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
                    if self.history_error.is_some(){return;}
                }
                let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{
                    let old=std::mem::take(&mut self.series);
                    self.start_live_session();
                    self.agent_generation=self.agent_generation.wrapping_add(1);
                    self.load_rules_revision=self.load_rules_revision.saturating_add(1);
                    self.frame_count=0;old
                });
                drop(old);
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
            .add_filter("Text", &["txt", "csv", "tsv", "jsonl", "ndjson", "log"])
            .add_filter("Extract", &["wida","widalive"])
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
        if self.wants_live_serial() {self.flush_live_cache();if self.history_error.is_some(){return;}}
        let header = self.frame_header.trim().to_owned();
        let filters = self.rules_filter_vec();
        let kinds = self.rules_kind_vec();
        let (tx, rx) = unbounded();
        let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
            self.imported_query=None;
            let old_quality=self.imported_quality.take();
            self.source=DataSource::File;self.live_auto_fit=false;self.live_tail.clear();self.live_cursor=0;
            self.last_open_path=Some(path.clone());
            self.load_rules_revision=self.load_rules_revision.saturating_add(1);
            self.pending_load=Some(PendingDataLoad{rx,rules_revision:self.load_rules_revision});
            self.agent_generation=self.agent_generation.wrapping_add(1);self.frame_count=0;
            (std::mem::take(&mut self.series),old_quality)
        });
        drop(old);
        self.load_progress = 0;
        self.load_progress_target = 0;
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
        if self.imported_query.is_some(){self.status="当前为AI提取数据集，请重新提取后导入，不能用帧筛选规则重解析。".into();return;}
        let Some(text) = self.file_text.clone() else {
            return;
        };
        if self.pending_load.is_some() {
            self.status = tr(lang, "data.status_loading_busy");
            return;
        }
        let header = self.frame_header.trim().to_owned();
        let filters = self.rules_filter_vec();
        if filters.iter().all(|f| f.is_empty()) {
            let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
                self.agent_generation=self.agent_generation.wrapping_add(1);self.frame_count=0;
                self.load_rules_revision=self.load_rules_revision.saturating_add(1);
                std::mem::take(&mut self.series)
            });
            drop(old);
            self.cached_extent = None;
            self.invalidate_lod();
            self.status = tr(lang, "data.status_need_filter");
            return;
        }
        let kinds = self.rules_kind_vec();
        let path = self.last_open_path.clone();
        let (tx, rx) = unbounded();
        wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||self.pending_load=Some(PendingDataLoad { rx, rules_revision:self.load_rules_revision }));
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
                        let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||self.pending_load.take());drop(old);
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
            let stale = self.pending_load.as_ref().is_none_or(|pending| pending.rules_revision == u64::MAX || pending.rules_revision != self.load_rules_revision);
            if stale {
                let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||self.pending_load.take());drop(old);
                self.load_progress = 0;
                self.load_progress_target = 0;
                self.status = if lang == Lang::Zh {"规则或来源已变化，旧加载结果未应用，请重新加载。"} else {"Rules or source changed; stale load discarded. Reload to apply."}.into();
                return;
            }
            // Drain any trailing progress that arrived with Done.
            if let Some(pending) = self.pending_load.as_ref() {
                while let Ok(LoadEvent::Progress(p)) = pending.rx.try_recv() {
                    self.load_progress_target = self.load_progress_target.max(p.min(99));
                }
            }
            let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||self.pending_load.take());drop(old);
            self.load_progress = 100;
            self.load_progress_target = 100;
            match msg {
                Ok(ok) => self.apply_load_ok(ok, lang),
                Err(e) => {
                    let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
                        self.agent_generation=self.agent_generation.wrapping_add(1);self.frame_count=0;
                        (std::mem::take(&mut self.series),self.file_text.take())
                    });
                    drop(old);
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
        let next_series = ok
            .series
            .into_iter()
            .enumerate()
            .map(|(i, d)| DataSeries {
                unit_override: None,
                label: d.label,
                color: series_color(i),
                kind: d.kind,
                points: d.points.into(),
                y_scale: d.y_scale,
                y_offset: d.y_offset,
                y_min: d.y_min,
                y_max: d.y_max,
                mapped: d.mapped,
                disp_dirty: false,
            })
            .collect();
        let old = wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
            self.imported_query=None;
            let old_quality=self.imported_quality.take();
            self.source = DataSource::File;
            self.live_auto_fit = false;
            if let Some(path) = ok.path { self.last_open_path = Some(path); }
            if let Some(catalog) = ok.field_catalog {
                self.discovered_fields = catalog.fields;
                self.field_catalog_truncated = catalog.truncated;
                self.field_search.clear();
            }
            let old_text=std::mem::replace(&mut self.file_text,ok.file_text);
            if let Some(h) = ok.frame_header {self.frame_header = h;}
            if let Some(filters) = ok.filters {
                while self.filters.len() < MAX_SERIES { self.filters.push(String::new()); }
                for (i, f) in filters.into_iter().enumerate().take(MAX_SERIES) { self.filters[i] = f; }
            }
            if let Some(kinds) = ok.kinds {
                for (i, k) in kinds.into_iter().enumerate().take(MAX_SERIES) { self.kinds[i] = DataKind::from_config_str(&k); }
            }
            self.frame_count = ok.frame_count;
            self.agent_generation=self.agent_generation.wrapping_add(1);
            let old_series=std::mem::replace(&mut self.series, next_series);
            self.series_count = self.series.len().clamp(1, MAX_SERIES);
            (old_series,old_text,old_quality)
        });
        drop(old);
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
            for p in s.points.iter() {
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
        if !self.wants_live_serial() { self.ensure_display_mapped(); }
        let cols = ((pixel_w * LOD_POINTS_PER_PX).ceil() as usize).max(64);
        let mut lines = Vec::with_capacity(self.series.len());
        for s in &self.series {
            if self.wants_live_serial() {
                let mut points=s.points.visible(x0,x1,cols);
                for p in &mut points {p[1]=display_y(p[1],s.y_scale,s.y_offset,s.kind);}
                lines.push(points);
                continue;
            }
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
            let use_lod = self.wants_live_serial() || s.mapped.len() > SKIP_DOTS_ABOVE
                || s.mapped.len() as f32 > pixel_w * LOD_POINTS_PER_PX;
            let line = if use_lod {
                self.lod_lines
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| s.mapped.clone())
            } else {
                s.mapped.clone()
            };
            let draw_dots = line.len() <= SKIP_DOTS_ABOVE && (!use_lod || self.wants_live_serial());
            out.push(PlotDrawItem {
                index: i,
                label: s.label.clone(),
                color: s.color,
                line,
                draw_dots:draw_dots||self.imported_quality.as_ref().is_some_and(|q|q["quality_complete"]!=true),
                connect:self.imported_quality.as_ref().is_none_or(|q|q["quality_complete"]==true),
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

    fn flush_live_cache(&mut self) {
        self.checkpoint_live(true);
    }

    fn checkpoint_live(&mut self, wait: bool) {
        if self
            .cache_job
            .as_ref()
            .is_some_and(|j| !wait && !j.handle.is_finished())
        {
            return;
        }
        self.finish_checkpoint();
        if self.cache_quarantined { return; }
        if self.frame_count == 0 || self.live_frames_since_flush == 0 {
            return;
        }
        self.last_checkpoint=std::time::Instant::now();
        let path = live_cache_path(&self.live_session_id);
        let header = data_extract_cache::WidaHeader {
            source_key: "serial:live".into(),
            content_fp: self.live_session_id.clone(),
            header: self.frame_header.clone(),
            filters: self.rules_filter_vec(),
            kinds: self.rules_kind_vec(),
            frame_count: self.frame_count as u32,
            series: self
                .series
                .iter()
                .map(|s| data_extract_cache::WidaSeriesMeta {
                    label: s.label.clone(),
                    kind: s.kind.as_config_str().into(),
                    y_scale: s.y_scale,
                    y_offset: s.y_offset,
                    y_min: s.y_min,
                    y_max: s.y_max,
                    n: s.points.len() as u32,
                })
                .collect(),
        };
        let mut points = self
            .series
            .iter()
            .map(|s| s.points.clone())
            .collect::<Vec<_>>();
        let session_id=self.live_session_id.clone();
        let series_count=points.len();
        let rebuilding=self.cache_rebuilding;
        let cancel=Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel=cancel.clone();
        let handle=thread::spawn(move || {
            for series in &mut points {
                if worker_cancel.load(std::sync::atomic::Ordering::Acquire) { return Err("已请求停止；新缓存可能已部分写入 / Cancelled; new cache may be partial".into()); }
                series
                    .page_out()
                    .map_err(|e| format!("历史分块写入失败：{e}"))?;
            }
            if worker_cancel.load(std::sync::atomic::Ordering::Acquire) { return Err("已请求停止；新缓存可能已部分写入 / Cancelled; new cache may be partial".into()); }
            history::checkpoint(&path, &header, &points)
                .map_err(|e| format!("历史缓存写入失败：{e}"))?;
            if worker_cancel.load(std::sync::atomic::Ordering::Acquire) { return Err("已请求停止；写入可能已经完成，请检查新缓存 / Cancelled; write may have completed, inspect the new cache".into()); }
            Ok(points)
        });
        self.cache_job = Some(CacheJob { session_id, series_count, handle, rebuilding, cancel });
        self.live_frames_since_flush = 0;
        if wait {
            self.finish_checkpoint();
        }
    }

    fn finish_checkpoint(&mut self) {
        let Some(job) = self.cache_job.take() else {
            return;
        };
        let same_session=job.session_id==self.live_session_id;
        let is_current=same_session && job.series_count==self.series.len();
        let rebuilding=job.rebuilding;
        let cancel=job.cancel.clone();
        let mut result=job.handle
            .join()
            .unwrap_or_else(|_| Err("历史写入线程退出".into()));
        if rebuilding && cancel.load(std::sync::atomic::Ordering::Acquire) {
            result=Err("已停止等待；写入可能已经完成，新缓存需检查 / Cancelled; write may have completed, inspect the new cache".into());
        }
        if !is_current {
            if same_session {
                self.data_review_lease.mutate_always(||());
                self.cache_rebuilding=false;self.cache_quarantined=true;self.live_frames_since_flush=self.frame_count;
                self.history_error=Some("曲线集合已变化，历史缓存已隔离 / Series changed; history cache quarantined".into());
            } else if let Err(error)=result { self.status=format!("上一会话历史缓存失败 / Previous session cache failed: {error}"); }
            return;
        }
        if rebuilding { self.cache_rebuilding=false; }
        match result {
            Ok(snapshot) => {
                if snapshot.len()!=self.series.len() { self.data_review_lease.mutate_always(||());self.cache_quarantined=true;self.history_error=Some("历史快照通道数不匹配，缓存已隔离 / History series count mismatch; cache quarantined".into());self.live_frames_since_flush=self.frame_count;return; }
                let adopted=self.source_lease.clone().mutate_always(|| {
                    if !self.series.iter().zip(&snapshot).all(|(s,points)|s.points.can_adopt_pages(points)) {return false;}
                    for (s, points) in self.series.iter_mut().zip(snapshot) { let _=s.points.adopt_pages(points); }
                    true
                });
                if adopted { self.history_error = None;if rebuilding {self.status="历史缓存重建完成 / History cache rebuilt".into();} }
                else {
                    self.data_review_lease.mutate_always(||());
                    self.cache_quarantined=true;
                    self.history_error=Some("历史缓存快照与当前数据不一致；保留内存数据，请检查并重新开始采集 / Cache snapshot differs; memory data retained, inspect and restart acquisition".into());
                    self.live_frames_since_flush=self.frame_count;
                }
            }
            Err(error) => {
                self.data_review_lease.mutate_always(||());
                self.live_frames_since_flush = self.frame_count;
                if rebuilding { self.cache_quarantined=true; }
                self.history_error = Some(error);
            }
        }
    }

    fn rebuild_quarantined_cache(&mut self) {
        if !self.cache_quarantined || self.cache_job.is_some() { return; }
        let next_id=uuid::Uuid::new_v4().to_string();
        wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
            self.live_session_id=next_id;
            self.agent_generation=self.agent_generation.wrapping_add(1);
            self.cache_quarantined=false;
            self.cache_rebuilding=self.frame_count>0;
            self.history_error=None;
            self.live_frames_since_flush=self.frame_count;
        });
        self.status=if self.cache_rebuilding {"已保留当前数据，将在新的缓存文件中重建 / Data retained; rebuilding into a new cache file"} else {"当前没有数据，已清除缓存隔离状态 / No data; cache quarantine cleared"}.into();
        if self.cache_rebuilding { self.checkpoint_live(false); }
    }

    fn ensure_series_shells(&mut self, lang: Lang) {
        let filters: Vec<&str> = self.filters[..self.series_count]
            .iter()
            .map(|s| s.trim())
            .collect();
        if self.series.len() != self.series_count {
            self.series.resize_with(self.series_count, || DataSeries {
                unit_override: None,
                label: String::new(),
                color: Color32::WHITE,
                kind: DataKind::Voltage,
                points: HistoryPoints::default(),
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
        let mut groups: [Vec<usize>; 6] = Default::default();
        for (i, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let gi = match s.kind {
                DataKind::Voltage => 0,
                DataKind::Current => 1,
                DataKind::Temperature => 2,DataKind::Power=>3,DataKind::Frequency=>4,DataKind::Scalar=>5,
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
            if let Err(error) = save_config(&cfg) {
                tracing::warn!("failed to save data-analysis browser config: {error}");
            }
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
            if let Err(error) = save_config(&cfg) {
                tracing::warn!("failed to save data-analysis thresholds: {error}");
            }
        }
    }

    fn active_kinds(&self) -> Vec<DataKind> {
        let mut out = Vec::new();
        for kind in [DataKind::Voltage, DataKind::Current, DataKind::Temperature,DataKind::Power,DataKind::Frequency,DataKind::Scalar] {
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
        if zoom_x { self.live_auto_fit = false; } else { self.manual_y = true; self.last_y_range=Some((ny0,ny1)); }
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
                    self.manual_y = true;
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
        let x0 = if self.recent_view {
            n.saturating_sub(window) as f64
        } else {
            0.0
        };
        let (x0, x1) = if n == 1 { (-0.5, 0.5) } else { (x0, x1) };
        let (y0, y1) = self
            .last_y_range
            .filter(|_| self.manual_y)
            .unwrap_or((ext.min()[1], ext.max()[1]));
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
        if xmin.is_finite() && self.wants_live_serial() { xmin=0.; xmax=self.frame_count.saturating_sub(1) as f64; }
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
        self.snapshot_fields(0,usize::MAX)
    }
    fn snapshot_fields(&self, offset:usize, limit:usize) -> Value {
        if let Some(view)=self.visible_result(){
            let mut value=view.snapshot_fields(offset,limit);value["view"]=json!("ai_result");value["background_source"]=json!(self.source.as_config_str());return value;
        }
        let fields=if self.source==DataSource::File && self.file_text.is_some() {self.discovered_fields.as_slice()} else {&[]};
        let end=offset.saturating_add(limit).min(fields.len());
        json!({
            "field_candidates_total":fields.len(),
            "field_catalog_truncated":!fields.is_empty() && self.field_catalog_truncated,
            "next_offset":(end<fields.len()).then_some(end),
            "imported_query":self.imported_query,
            "imported_quality":self.imported_quality,
            "browser_dir": self.browser_dir,
            "frame_header": self.frame_header,
            "field_candidates": &fields[offset.min(end)..end],
            "extract_rules": wiparse_core::extract::HELP,
            "series_count": self.series_count,
            "filters": self.filters[..self.series_count],
            "kinds": self.kinds[..self.series_count].iter().map(|k| k.as_config_str()).collect::<Vec<_>>(),
            "source": self.source.as_config_str(),
            "frames": self.frame_count,
            "live_cursor": self.live_cursor,
            "history_error":self.history_error,
            "capture_boundary":self.gap_notice,
            "live_view":if !self.live_auto_fit {"manual"}else if self.recent_view {"recent"}else{"full"},
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

    /// Series, scales and thresholds. Local paths and raw samples stay on this machine.
    pub(crate) fn ai_summary(&self) -> Value {
        self.ai_field_page(0,32)
    }
    pub(crate) fn ai_field_page(&self, offset:usize, limit:usize) -> Value {
        self.published_ai_snapshot().map(|snapshot| snapshot.page(offset,limit))
            .unwrap_or_else(||json!({"snapshot_available":false}))
    }
    fn build_ai_field_page(&self, offset:usize, limit:usize) -> Value {
        let mut summary = self.snapshot_fields(offset,limit);
        if let Some(object) = summary.as_object_mut() {
            object.remove("browser_dir");
            object.remove("last_open");
            object.insert(
                "status".into(),
                json!(self.status_text().chars().take(240).collect::<String>()),
            );
        }
        summary
    }

    fn published_field_source(&self) -> &[(String, String)] {
        if let Some(view) = self.visible_result() {
            return view.published_field_source();
        }
        if self.source == DataSource::File && self.file_text.is_some() {
            &self.discovered_fields
        } else {
            &[]
        }
    }

    /// Publish only after the current mutation has completed, while its update guard is active.
    fn publish_ai_snapshot(&mut self) {
        let mut summary = self.build_ai_field_page(0, 0);
        if let Some(object) = summary.as_object_mut() {
            object.remove("field_candidates");
            object.remove("next_offset");
            object.insert("snapshot_kind".into(), json!("immutable_published_metadata"));
            object.insert("raw_samples_included".into(), json!(false));
            object.insert("pagination_consistency".into(), json!("pass snapshot_id on subsequent pages; restart if changed"));
        }
        let fields = self.published_field_source();
        if self.published_ai.as_ref().is_some_and(|old| old.summary == summary && old.fields.as_slice() == fields) {
            return;
        }
        let catalog = match &self.published_ai {
            Some(old) if old.fields.as_slice() == fields => old.fields.clone(),
            _ => Arc::new(fields.to_vec()),
        };
        let next = Arc::new(PublishedDataSnapshot { id: uuid::Uuid::new_v4().to_string(), summary, fields: catalog });
        let lease = self.source_lease.clone();
        let previous = lease.mutate_always(|| self.published_ai.replace(next));
        drop(previous);
    }

    pub(crate) fn published_ai_snapshot(&self) -> Option<Arc<PublishedDataSnapshot>> {
        self.published_ai.clone()
    }
    pub(crate) fn capture_review_source(&self)->Option<wiparse_core::ai::ReviewedSource<Arc<PublishedDataSnapshot>>> {
        let snapshot=self.published_ai.as_ref()?;
        wiparse_core::ai::ReviewedSource::capture(&self.source_lease,||snapshot.clone())
    }
}

fn paint_loading_in_rect(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    lang: Lang,
    rect: egui::Rect,
    progress: u8,
) {
    let bar_w = 220.0_f32.min(rect.width().max(1.0));
    let content = egui::Rect::from_center_size(rect.center(), egui::vec2(bar_w, 96.0));
    crate::theme::scope_in_allocated_rect(ui,
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
    let mut groups: [Vec<usize>; 6] = Default::default();
    for (i, s) in series.iter().enumerate() {
        if s.points.is_empty() {
            continue;
        }
        let gi = match s.kind {
            DataKind::Voltage => 0,
            DataKind::Current => 1,
            DataKind::Temperature => 2,DataKind::Power=>3,DataKind::Frequency=>4,DataKind::Scalar=>5,
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
    field_catalog: Option<wiparse_core::extract::FieldCatalog>,
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
        field_catalog,
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
        .is_some_and(|e| e.eq_ignore_ascii_case("wida") || e.eq_ignore_ascii_case("widalive"))
    {
        send_progress(tx, 20);
        match load_wida(&path) {
            Ok(payload) => {
                send_progress(tx, 55);
                send_done(Ok(wida_to_load_ok(path, payload, Some(Default::default()), None, Some(tx))));
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
    let detect = filters.iter().all(|s|s.trim().is_empty());
    let catalog = wiparse_core::extract::discover_catalog(&text);
    let candidates = &catalog.fields;
    let (filters,kinds) = if !detect || candidates.is_empty() { (filters,kinds) } else {
        let selectors:Vec<_> = candidates.iter().take(MAX_SERIES).map(|(_,s)|s.clone()).collect();
        let kinds=vec!["scalar".into();selectors.len()];
        (selectors,kinds)
    };
    let fp = file_content_fp(&path);
    let cache_path = data_extract_cache::cache_path_for(&fp, &header, &filters, &kinds);
    if let Ok(payload) = load_wida(&cache_path) {
        if payload.meta.content_fp == fp && payload.meta.header == header {
            send_progress(tx, 55);
            send_done(Ok(wida_to_load_ok(
                path,
                payload,
                Some(catalog),
                Some(Arc::clone(&text)),
                Some(tx),
            )));
            return;
        }
    }

    let filter_refs: Vec<&str> = filters.iter().map(|s| s.as_str()).collect();
    send_progress(tx, 15);
    let parsed = match parse_data_file_progress(&text, &header, &filter_refs, tx) { Ok(v) => v, Err(e) => { send_done(Err(e)); return; } };
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
        field_catalog: Some(catalog),
        path: Some(path),
        file_text: Some(text),
        frame_count,
        series,
        from_cache: false,
        frame_header: None,
        filters: Some(filters),
        kinds: Some(kinds),
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
                    None,
                    Some(Arc::clone(&text)),
                    Some(tx),
                )));
                return;
            }
        }
    }
    send_progress(tx, 15);
    let parsed = match parse_data_file_progress(text.as_str(), &header, &filter_refs, tx) { Ok(v) => v, Err(e) => { send_done(Err(e)); return; } };
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
        field_catalog: None,
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

fn compile_filters(filters: &[&str]) -> Result<Vec<Option<Selector>>, String> {
    filters.iter().map(|filter| {
        if filter.is_empty() { Ok(None) } else { Selector::compile(filter, true, true).map(Some) }
    }).collect()
}

fn selected_number(selector: Option<&Selector>, record: &Record<'_>) -> Option<f64> {
    wiparse_core::extract::number(&selector?.first(record)?)
}

fn parse_data_file_progress(
    text: &str,
    header: &str,
    filters: &[&str],
    tx: &Sender<LoadEvent>,
) -> Result<ParsedData, String> {
    let selectors = compile_filters(filters)?;
    // 15–45%: locate frames; 45–78%: extract series values.
    let frames = split_frames_progress(text, header, tx, 15, 45);
    let n = frames.len().max(1);
    let mut series: Vec<Vec<[f64; 2]>> = (0..filters.len()).map(|_| Vec::new()).collect();
    let step = (n / 40).max(1);
    for (fi, frame) in frames.iter().enumerate() {
        let x = fi as f64;
        let record = Record::new(frame);
        for (si, selector) in selectors.iter().enumerate() {
            if let Some(value) = selected_number(selector.as_ref(), &record) {
                series[si].push([x, value]);
            }
        }
        if fi % step == 0 || fi + 1 == frames.len() {
            let pct = 45 + ((fi as u64 * 33) / n as u64) as u8;
            send_progress(tx, pct.min(78));
        }
    }
    Ok(ParsedData {
        frame_count: frames.len(),
        series,
    })
}

struct ParsedData {
    frame_count: usize,
    series: Vec<Vec<[f64; 2]>>,
}

fn parse_data_file(text: &str, header: &str, filters: &[&str]) -> ParsedData {
    let (tx, _rx) = unbounded::<LoadEvent>();
    parse_data_file_progress(text, header, filters, &tx).unwrap()
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
        return text.lines().collect();
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

#[cfg(test)]
fn extract_first_filter_value(frame: &str, filter: &str) -> Option<f64> {
    selected_number(Some(&Selector::compile(filter,true,true).ok()?), &Record::new(frame))
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
fn threshold_row(ui:&mut egui::Ui,tokens:&Tokens,lang:Lang,kind:DataKind,on:&mut bool,value:&mut f64)->bool{
    let mut dirty=false;
    ui_theme::form_row(ui,&kind.label(lang),|ui|{
        dirty|=ui.checkbox(on,"").on_hover_text(tr(lang,"data.threshold_hint")).changed();
        let width=ui.available_width();
        let response=ui.add_sized([width,ui_theme::CTRL_H],egui::DragValue::new(value).speed(0.01).max_decimals(4).min_decimals(2).suffix(format!(" {}",kind.unit())));
        dirty|=response.drag_stopped()||response.lost_focus();
    });
    let _=tokens;dirty
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
fn value_at_x(points: &HistoryPoints, x: f64) -> Option<f64> {
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
        Some("txt" | "csv" | "tsv" | "jsonl" | "ndjson" | "log")
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
    crate::theme::scope_in_allocated_rect(ui,
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
    fn separate_data_panel_instances_have_distinct_source_identity() {
        let a=DataAnalysisPanel::new(&AppConfig::default());
        let b=DataAnalysisPanel::new(&AppConfig::default());
        assert_ne!(a.agent_identity(),b.agent_identity());
    }
    #[test]
    fn review_snapshot_excludes_external_data_and_old_permit_expires() {
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());
        panel.source=DataSource::File;
        panel.file_text=Some(Arc::new("sensitive sample".into()));
        panel.discovered_fields.push(("sensitive field".into(),"key:secret".into()));
        panel.imported_query=Some("private-query".into());
        panel.publish_ai_snapshot();
        let snapshot=panel.published_ai_snapshot().unwrap();
        let review=snapshot.review_ui_state().to_string();
        assert!(!review.contains("sensitive"));assert!(!review.contains("private-query"));assert!(!review.contains("key:secret"));
        let (_,permit)=panel.source_lease.snapshot(||()).unwrap();
        let update=panel.source_lease.begin_update();
        assert!(panel.capture_review_source().is_none());
        panel.series_count=2;panel.publish_ai_snapshot();drop(update);
        assert!(permit.recapture(||()).is_none());
    }

    #[test]
    fn published_catalog_pages_keep_the_same_immutable_boundary() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::File;
        panel.file_text = Some(Arc::new("fixture".into()));
        panel.discovered_fields = (0..70).map(|i|(format!("f{i}"), format!("key:f{i}"))).collect();
        panel.publish_ai_snapshot();
        let old = panel.published_ai_snapshot().unwrap();
        let first = old.page(0, 32);
        panel.discovered_fields.remove(0);
        panel.publish_ai_snapshot();
        let later = old.page(32, 32);
        let current = panel.published_ai_snapshot().unwrap().page(0, 32);
        assert_eq!(first["snapshot_id"], later["snapshot_id"]);
        assert_ne!(first["snapshot_id"], current["snapshot_id"]);
        assert_eq!(later["field_candidates"][0][0], "f32");
        assert_eq!(later["field_candidates_total"], 70);
        assert_eq!(current["field_candidates_total"], 69);
        assert_eq!(first["raw_samples_included"], false);
    }

    #[test]
    fn publication_changes_invalidate_permits_and_unchanged_publication_does_not() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        let lease = panel.source_lease.clone();
        let (_, unchanged) = lease.snapshot(|| ()).unwrap();
        panel.publish_ai_snapshot();
        assert!(unchanged.recapture(|| ()).is_some());
        let (_, previous) = lease.snapshot(|| ()).unwrap();
        panel.status = "changed".into();
        panel.publish_ai_snapshot();
        assert!(previous.recapture(|| ()).is_none());
    }

    #[test]
    fn file_catalog_is_paged_independently_of_plot_rules_and_survives_reparse() {
        fn next_result(rx: &Receiver<LoadEvent>) -> DataLoadOk {
            rx.try_iter().find_map(|e|match e {LoadEvent::Done(r)=>Some(r.unwrap()), _=>None}).unwrap()
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many-fields.log");
        let text = (0..43).map(|i|format!("field{i}={i}\n")).collect::<String>();
        std::fs::write(&path,&text).unwrap();
        let (tx, rx) = unbounded();
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());
        load_file_worker(path, String::new(), vec!["key:field4".into()], vec!["scalar".into()], &tx);
        panel.apply_load_ok(next_result(&rx), Lang::Zh);
        panel.publish_ai_snapshot();
        assert_eq!(panel.filters[0],"key:field4");
        assert_eq!(panel.ai_summary()["field_candidates_total"],43);
        assert_eq!(panel.ai_summary()["next_offset"],32);
        let page=panel.ai_field_page(32,32);
        assert_eq!(page["field_candidates"][10][1],"key:field42");
        assert!(page["next_offset"].is_null());
        reparse_text_worker(Arc::new(text),None,String::new(),vec!["key:field42".into()],vec!["scalar".into()],&tx);
        panel.apply_load_ok(next_result(&rx),Lang::Zh);
        panel.publish_ai_snapshot();
        assert_eq!(panel.discovered_fields.len(),43);
        assert_eq!(panel.series[0].points.last().unwrap()[1],42.);
    }
    #[test]
    fn file_and_live_extract_identical_rows_without_a_marker() {
        let lines: Vec<String>=["id,note,VIN","1,\"a,b\",5.1","2,missing,","3,ok,5.3"].iter().map(|s|s.to_string()).collect();
        let parsed=parse_data_file(&lines.join("\n"),"", &["csv:3"]);
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());
        panel.source=DataSource::SerialLive;panel.frame_header.clear();panel.filters[0]="csv:3".into();panel.series_count=1;
        panel.ingest_live_lines(&lines[..2],2,Lang::Zh);
        panel.ingest_live_lines(&lines[2..],4,Lang::Zh);
        assert_eq!(parsed.series[0],vec![[1.,5.1],[3.,5.3]]);
        assert_eq!(panel.series[0].points,parsed.series[0]);assert_eq!(panel.frame_count,4);
    }
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

#[derive(Default)]
struct AgentReducer{count:u64,invalid:u64,min:Option<f64>,max:Option<f64>,mean:f64,scale:f64,ssq:f64,above:u64}
pub(crate) struct AgentDataStats{signature:String,cutoffs:Vec<usize>,identities:Vec<Arc<()>>,series:usize,offset:usize,reducers:Vec<AgentReducer>}
impl DataAnalysisPanel{
 fn imported_seconds(&self)->bool {self.imported_query.is_some() && !self.imported_quality.as_ref().is_some_and(|q| q["provenance"]["x_axis"]=="record_index")}
 pub(crate) fn agent_identity(&self)->String{if let Some(view)=self.visible_result(){return format!("result:{}:{:?}:{}",self.agent_instance,view.imported_query,view.agent_identity());}format!("{}:{}:{:?}:{}:{:?}:{:?}",self.agent_instance,self.source==DataSource::SerialLive,self.last_open_path,self.series_count,self.filters,(self.agent_generation,&self.kinds))}
 pub(crate) fn agent_statistics(&self,job:&mut Option<AgentDataStats>,begin:bool)->Value{
  if self.showing_result {if let Some(view)=&self.result_plot{return view.agent_statistics(job,begin);}}
  let signature=format!("{:?}:{}:{:?}:{:?}",self.imported_query,self.agent_identity(),self.series.iter().map(|s|(&s.label,s.kind)).collect::<Vec<_>>(),[(self.thr_voltage_on,self.thr_voltage),(self.thr_current_on,self.thr_current),(self.thr_temperature_on,self.thr_temperature)]);
  if begin{*job=Some(AgentDataStats{signature:signature.clone(),cutoffs:self.series.iter().map(|s|s.points.len()).collect(),identities:self.series.iter().map(|s|s.points.identity()).collect(),series:0,offset:0,reducers:self.series.iter().map(|_|AgentReducer::default()).collect()});}
  let Some(job)=job.as_mut()else{return json!({"ok":false,"error":"数据统计任务不存在"})};if signature!=job.signature||self.series.len()!=job.cutoffs.len()||self.series.iter().enumerate().any(|(i,s)|s.points.len()<job.cutoffs[i]||!s.points.has_identity(&job.identities[i])){return json!({"ok":false,"error":"数据源在统计过程中发生变化，未报告全量结论；实时采集请先保存快照"});}
  let at=std::time::Instant::now();let mut processed=0;
  while job.series<self.series.len()&&processed<20000&&at.elapsed()<std::time::Duration::from_millis(4){
   let series=&self.series[job.series];if job.offset>=job.cutoffs[job.series]{job.series+=1;job.offset=0;continue;}
   let value=series.points[job.offset][1];job.offset+=1;processed+=1;let stats=&mut job.reducers[job.series];if !value.is_finite(){stats.invalid+=1;continue;}
   stats.count+=1;stats.min=Some(stats.min.map_or(value,|v|v.min(value)));stats.max=Some(stats.max.map_or(value,|v|v.max(value)));stats.mean=stats.mean*(1.-1./stats.count as f64)+value/stats.count as f64;
   let magnitude=value.abs();if magnitude>0.{if stats.scale<magnitude{stats.ssq=1.+stats.ssq*(stats.scale/magnitude).powi(2);stats.scale=magnitude;}else{stats.ssq+=(magnitude/stats.scale).powi(2);}}
   let threshold=match series.kind{DataKind::Voltage=>self.thr_voltage_on.then_some(self.thr_voltage),DataKind::Current=>self.thr_current_on.then_some(self.thr_current),DataKind::Temperature=>self.thr_temperature_on.then_some(self.thr_temperature),_=>None};if threshold.is_some_and(|limit|value>limit){stats.above+=1;}
  }
  if job.series<self.series.len(){return json!({"_agent_wait":{"method":"data.statistics.result","params":{},"interval_ms":16}});}
  json!({"ok":job.reducers.iter().any(|s|s.count>0),"data":{"coverage":"immutable cutoff of each curve at query start; appended samples excluded","raw_samples_uploaded":false,"imported_quality":self.imported_quality,"x_axis":if self.imported_seconds(){"seconds from source timestamps; separate sources are not synchronized"}else{"frame index, not synchronized physical time"},"series":self.series.iter().zip(&job.reducers).enumerate().map(|(i,(series,stats))|json!({"index":i,"label":series.label,"unit":series.unit_override.as_deref().unwrap_or(series.kind.unit()),"sample_count":stats.count,"invalid_samples":stats.invalid,"quality_complete":stats.count>0&&stats.invalid==0&&self.imported_quality.as_ref().is_none_or(|q|q["quality_complete"]==true),"min":stats.min,"max":stats.max,"mean":(stats.count>0&&stats.mean.is_finite()).then_some(stats.mean),"rms":if stats.count>0{Some(stats.scale*(stats.ssq/stats.count as f64).sqrt())}else{None},"points":job.cutoffs[i],"new_points_excluded":series.points.len().saturating_sub(job.cutoffs[i]),"first_frame":series.points.first().map(|p|p[0]),"last_frame":job.cutoffs[i].checked_sub(1).and_then(|n|series.points.get(n)).map(|p|p[0]),"above_configured_upper_limit":match series.kind{DataKind::Voltage=>self.thr_voltage_on.then_some(stats.above),DataKind::Current=>self.thr_current_on.then_some(stats.above),DataKind::Temperature=>self.thr_temperature_on.then_some(stats.above),_=>None}})).collect::<Vec<_>>()}})
 }
}

#[cfg(test)]mod agent_statistics_tests{
 use super::*;
 #[test]fn all_points_are_reduced_once_and_missing_values_are_explicit(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=2;panel.ensure_series_shells(Lang::Zh);panel.series[0].points=vec![[0.,1.],[1.,2.],[2.,3.],[3.,f64::NAN]].into();panel.thr_voltage_on=true;panel.thr_voltage=2.;let mut job=None;let result=panel.agent_statistics(&mut job,true);let s=&result["data"]["series"][0];assert_eq!(s["sample_count"],3);assert_eq!(s["invalid_samples"],1);assert_eq!(s["mean"],2.);assert!((s["rms"].as_f64().unwrap()-(14f64/3.).sqrt()).abs()<1e-12);assert_eq!(s["above_configured_upper_limit"],1);assert_eq!(s["quality_complete"],false);let empty=&result["data"]["series"][1];assert!(empty["min"].is_null());assert!(empty["mean"].is_null());}
 #[test]fn incremental_statistics_reject_replaced_sources(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=2;panel.ensure_series_shells(Lang::Zh);panel.series[0].points=(0..50000).map(|i|[i as f64,5.]).collect();let mut job=None;let result=panel.agent_statistics(&mut job,true);assert!(result.get("_agent_wait").is_some());panel.series[0].points[0]=[0.,99.];assert_eq!(panel.agent_statistics(&mut job,false)["ok"],false);}
 #[test]fn incremental_statistics_reject_middle_edit_and_equal_length_clone_replacement(){
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=1;panel.ensure_series_shells(Lang::Zh);
  panel.series[0].points=(0..50000).map(|i|[i as f64,5.]).collect();let mut job=None;
  assert!(panel.agent_statistics(&mut job,true).get("_agent_wait").is_some());
  panel.series[0].points[25000]=[25000.,99.];assert_eq!(panel.agent_statistics(&mut job,false)["ok"],false);
  assert!(panel.agent_statistics(&mut job,true).get("_agent_wait").is_some());
  panel.series[0].points=panel.series[0].points.clone();assert_eq!(panel.agent_statistics(&mut job,false)["ok"],false);
 }
}

#[cfg(test)]impl DataAnalysisPanel{pub(crate) fn prepare_style_audit(&mut self){self.series_count=3;self.filters[0]="VIN".into();self.filters[1]="IOUT".into();self.filters[2]="TEMP".into();self.kinds[0]=DataKind::Voltage;self.kinds[1]=DataKind::Current;self.kinds[2]=DataKind::Temperature;self.ensure_series_shells(Lang::Zh);for(i,series)in self.series.iter_mut().enumerate(){series.points=(0..240).map(|n|[n as f64,match i{0=>5000.+(n as f64*0.05).sin()*100.,1=>1000.+(n as f64*0.08).cos()*50.,_=>25.+n as f64*0.04}]).collect();series.y_min=series.points.iter().map(|p|p[1]).fold(f64::INFINITY,f64::min);series.y_max=series.points.iter().map(|p|p[1]).fold(f64::NEG_INFINITY,f64::max);series.disp_dirty=true;}self.frame_count=240;self.thr_voltage=5400.;self.thr_voltage_on=true;self.thr_current=1500.;self.thr_current_on=true;self.thr_temperature=45.;self.thr_temperature_on=true;self.fit_request=true;}}

#[cfg(test)]mod remediation_snapshot_tests{
 use super::*;
 #[test]fn quarantined_cache_does_not_start_another_writer(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.cache_quarantined=true;panel.frame_count=100;panel.live_frames_since_flush=100;panel.checkpoint_live(false);assert!(panel.cache_job.is_none());assert_eq!(panel.frame_count,100);}
 #[test]fn old_checkpoint_error_does_not_modify_new_session(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.live_session_id="new".into();panel.frame_count=17;panel.history_error=None;panel.cache_job=Some(CacheJob{session_id:"old".into(),series_count:panel.series.len(),handle:std::thread::spawn(||Err("old failure".into())),rebuilding:false,cancel:Arc::new(std::sync::atomic::AtomicBool::new(false))});panel.finish_checkpoint();assert!(panel.history_error.is_none());assert_eq!(panel.frame_count,17);}
 #[test]fn cancelled_rebuild_is_quarantined_and_keeps_current_data(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.frame_count=17;panel.cache_rebuilding=true;panel.cache_job=Some(CacheJob{session_id:panel.live_session_id.clone(),series_count:panel.series.len(),handle:std::thread::spawn(||Err("cancelled".into())),rebuilding:true,cancel:Arc::new(std::sync::atomic::AtomicBool::new(true))});panel.finish_checkpoint();assert!(panel.cache_quarantined);assert!(!panel.cache_rebuilding);assert_eq!(panel.frame_count,17);assert!(panel.history_error.as_deref().is_some_and(|error|error.contains("写入可能")));}
 #[test]fn same_session_changed_series_quarantines_old_job(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.frame_count=17;panel.cache_rebuilding=true;panel.cache_job=Some(CacheJob{session_id:panel.live_session_id.clone(),series_count:panel.series.len()+1,handle:std::thread::spawn(||Ok(Vec::new())),rebuilding:true,cancel:Arc::new(std::sync::atomic::AtomicBool::new(false))});panel.finish_checkpoint();assert!(panel.cache_quarantined);assert!(!panel.cache_rebuilding);assert_eq!(panel.frame_count,17);}
 #[test]fn stale_worker_result_cannot_clear_new_view(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());let(tx,rx)=crossbeam_channel::unbounded();panel.pending_load=Some(PendingDataLoad{rx,rules_revision:3});panel.load_rules_revision=4;panel.imported_query=Some("preserved".into());tx.send(LoadEvent::Done(Err("stale worker".into()))).unwrap();panel.poll_pending_load(Lang::Zh);assert_eq!(panel.imported_query.as_deref(),Some("preserved"));assert!(panel.status.contains("旧加载结果未应用"));}
 #[test]fn live_append_reallocation_does_not_change_snapshot_cutoff(){let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::SerialLive;panel.series_count=1;panel.ensure_series_shells(Lang::Zh);panel.series[0].points=(0..60000).map(|i|[i as f64,2.]).collect();let identity=panel.agent_identity();let mut job=None;assert!(panel.agent_statistics(&mut job,true).get("_agent_wait").is_some());panel.series[0].points.extend((60000..90000).map(|i|[i as f64,100.]));assert_eq!(identity,panel.agent_identity());let result=loop{let r=panel.agent_statistics(&mut job,false);if r.get("_agent_wait").is_none(){break r;}};assert_eq!(result["data"]["series"][0]["sample_count"],60000);assert_eq!(result["data"]["series"][0]["mean"],2.);assert_eq!(result["data"]["series"][0]["new_points_excluded"],30000);}
 #[test]fn empty_curves_are_not_complete_measurements(){let panel=DataAnalysisPanel::new(&AppConfig::default());let mut job=None;let r=panel.agent_statistics(&mut job,true);assert_eq!(r["ok"],false);for series in r["data"]["series"].as_array().unwrap(){assert_eq!(series["quality_complete"],false);}}
}

impl DataAnalysisPanel {
 pub(crate) fn agent_present(&mut self,d:&wiparse_core::log_analysis::timeline::Dataset,query:&str,lang:Lang)->Result<Value,String>{
  let update=self.source_lease.begin_update();let review_update=self.data_review_lease.begin_update();let result=self.agent_present_inner(d,query,lang);self.publish_ai_snapshot();drop(review_update);drop(update);result
 }
 fn agent_present_inner(&mut self,d:&wiparse_core::log_analysis::timeline::Dataset,query:&str,lang:Lang)->Result<Value,String>{
  if self.pending_load.is_some() || self.source==DataSource::SerialLive || self.result_plot.is_some() {
      let mut view=Self::new(&AppConfig::default());
      view.set_source_lease(self.source_lease.clone());
      view.data_review_lease=self.data_review_lease.clone();
      view.source=DataSource::File;
      view.extract_cache=self.extract_cache.clone();
      let mut receipt=view.import_dataset(d,query,lang)?;
      receipt["view"]=json!("ai_result");
      let old = wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),|| {
          self.showing_result=true;
          self.result_plot.replace(Box::new(view))
      });
      drop(old);
      return Ok(receipt);
  }
  self.import_dataset(d,query,lang)
 }
 fn import_dataset(&mut self,d:&wiparse_core::log_analysis::timeline::Dataset,query:&str,lang:Lang)->Result<Value,String>{
  if d.rows.is_empty()||d.omitted>0{return Err("DATA.EMPTY_OR_TRUNCATED_DATASET".into());}
  if d.rows.iter().any(|row|row.3.len()!=d.fields.len()){return Err("DATA.INVALID_ROW_WIDTH".into());}
  let units=d.fields.iter().map(|f|agent_display_unit(&f.unit)).collect::<Result<Vec<_>,_>>()?;
  let full=self.extract_cache.lock().ok().and_then(|cache|cache.full_curves(query,d));

  // Count populated source/field pairs. Different record types often have disjoint fields.
  let mut sources = Vec::new();
  let mut by_source = std::collections::HashMap::new();
  let mut present = Vec::<Vec<bool>>::new();
  for (id, _, _, row) in &d.rows {
      let index = *by_source.entry(id.as_str()).or_insert_with(|| {
          sources.push(id.as_str());
          present.push(vec![false; d.fields.len()]);
          sources.len() - 1
      });
      for (found, value) in present[index].iter_mut().zip(row) {
          *found |= value.is_some();
      }
  }
  let total = present.iter().flatten().filter(|&&found| found).count();
  if total > MAX_SERIES {
      return Err(format!("DATA.TOO_MANY_SERIES: {total} populated source/field curves; this view supports {MAX_SERIES}. Select fewer sources or fields."));
  }
  let mut series = Vec::with_capacity(total);
  let mut curves = Vec::with_capacity(total);
  for (source_index, &id) in sources.iter().enumerate() {
      for (i, field) in d.fields.iter().enumerate() {
          if !present[source_index][i] { continue; }
          let points: Vec<_> = d.rows.iter().filter(|r| r.0 == id)
              .filter_map(|r| r.3[i].map(|y| [r.1, y * units[i].1])).collect();
          if points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
              return Err("DATA.NONFINITE_CONVERSION".into());
          }
          // The file overview preserves each bucket's extrema. The plotted history retains every sample.
          let low = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
          let high = points.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
          let points = full.as_ref().map(|cached| cached[i].clone()).unwrap_or_else(|| points.into());
          let unit_override = (units[i].0 == DataKind::Scalar).then(|| field.unit.clone());
          let unit = unit_override.as_deref().unwrap_or(units[i].0.unit());
          curves.push(json!({"field":field.name,"source_id":id,"unit":unit,"sample_count":points.len()}));
          let suffix = if units[i].0 == DataKind::Scalar && !field.unit.is_empty() {format!(" [{}]", field.unit)} else {String::new()};
          series.push(DataSeries {unit_override, label:format!("{}{suffix} · {}", field.name, id.chars().take(8).collect::<String>()),
              color:series_color(series.len()), kind:units[i].0, points, y_scale:1., y_offset:0.,
              y_min:low, y_max:high, mapped:vec![], disp_dirty:true});
      }
  }
  let missing_fields: Vec<_> = d.fields.iter().enumerate().filter(|(i,_)| !present.iter().any(|p| p[*i]))
      .map(|(_,f)| &f.name).collect();
  if series.is_empty(){return Err("DATA.NO_VALID_VALUES".into());}
  let indexed=d.provenance["x_axis"]=="record_index";
  let file=(d.provenance["producer"]=="data.extract").then(||(self.file_text.clone(),self.last_open_path.clone()));
  self.clear_data(lang);
  let missing=d.rows.iter().map(|r|r.3.iter().filter(|v|v.is_none()).count()).sum::<usize>();
  let missing=d.provenance["missing_plotted_fields"].as_u64().unwrap_or(missing as u64);
  let quality=json!({"curves":curves,"missing_fields":missing_fields,"missing_field_values":missing,"provenance":d.provenance,"quality_complete":missing_fields.is_empty()&&missing==0&&(indexed||d.provenance["sources"].as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(|s|s["missing_timestamp_rows"]==0&&s["backward_rows"]==0)))});
  let records=d.provenance["plottable_rows"].as_u64().and_then(|n|usize::try_from(n).ok()).unwrap_or(d.rows.len());
  let labels:Vec<_>=series.iter().map(|s|(s.label.clone(),s.kind)).collect();
  let old=wiparse_core::ai::source_lease::SourceLease::mutate_pair_always(&self.source_lease.clone(),&self.data_review_lease.clone(),||{
      self.series_count=series.len();
      let old=std::mem::replace(&mut self.series,series);
      for (i,(label,kind)) in labels.into_iter().enumerate(){self.filters[i]=label;self.kinds[i]=kind;}
      self.load_rules_revision=self.load_rules_revision.saturating_add(1);
      self.frame_count=records;self.file_text=None;self.last_open_path=None;if let Some((text,path))=file{self.file_text=text;self.last_open_path=path;}self.imported_query=Some(query.into());self.imported_quality=Some(quality);
      self.thr_voltage_on=false;self.thr_current_on=false;self.thr_temperature_on=false;
      old
  });
  drop(old);
  self.auto_layout_scales();self.fit_request=true;self.pending_bounds=None;self.cached_extent=None;self.lod_key=None;self.lod_lines.clear();
  self.status=format!("AI提取 · {} 条记录 · {} 条曲线 · 横轴 {} · 缺失值未补齐",records,self.series.len(),if indexed{"记录序号"}else{"时间 / s"});
  Ok(json!({"query_id":query,"presented":true,"records":records,"report_overview_rows":d.rows.len(),"series":self.series.len(),"curves":curves,"missing_fields":missing_fields,"raw_samples_uploaded":false,"x_unit":if indexed{"record_index"}else{"s"},"missing_values":"omitted, never filled; incomplete datasets use scatter rather than connecting gaps","source_ids":sources,"display_units":self.series.iter().map(|s|s.unit_override.as_deref().unwrap_or(s.kind.unit())).collect::<Vec<_>>()}))
 }
}

#[cfg(test)]
mod ai_dataset_flow_tests {
 use super::*;
 use wiparse_core::log_analysis::{Inventory,Source,Query,Scan};
 use std::sync::atomic::AtomicBool;
 #[test]fn live_cursor_update_invalidates_old_source_permit(){
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::SerialLive;
  let (_,old)=panel.source_lease.snapshot(||()).unwrap();
  panel.ingest_live_lines(&[],8,Lang::Zh);
  assert_eq!(panel.live_cursor,8);assert!(old.commit_prepared(||()).is_none());
  let (_,unchanged)=panel.source_lease.snapshot(||()).unwrap();panel.ingest_live_lines(&[],8,Lang::Zh);
  assert_eq!(unchanged.commit_prepared(||()),Some(()));
 }
 #[test]fn threshold_change_invalidates_source_permit_but_unchanged_values_do_not(){
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());
  let voltage=(panel.thr_voltage_on,panel.thr_voltage);let current=(panel.thr_current_on,panel.thr_current);let temperature=(panel.thr_temperature_on,panel.thr_temperature);
  let (_,unchanged)=panel.source_lease.snapshot(||()).unwrap();
  panel.commit_thresholds(voltage,current,temperature);
  assert_eq!(unchanged.commit_prepared(||()),Some(()));
  let (_,old)=panel.source_lease.snapshot(||()).unwrap();
  panel.commit_thresholds((!voltage.0,voltage.1),current,temperature);
  assert!(old.commit_prepared(||()).is_none());
 }
 #[test]fn programmatic_import_invalidates_prior_source_permit(){
  use wiparse_core::ai::{AiConfig,AiRequest,AiService};
  let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::File;
  let(request,permit)=panel.source_lease.snapshot(||AiRequest{purpose:"fixture".into(),text:"fixture".into()}).unwrap();
  let service=AiService::new(AiConfig{enabled:true,base_url:format!("http://{}",listener.local_addr().unwrap()),model:"fixture".into(),..Default::default()});
  let task=service.submit_conversation_reviewed_guarded(request,vec![],permit).unwrap();let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
  while task.outbound_preview().is_none(){assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(5));}
  panel.agent_present(&dataset(),"query",Lang::Zh).unwrap();assert!(task.approve_outbound());
  assert!(matches!(task.result.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),Err(wiparse_core::ai::AiError::Config(_))));
  assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
 }
 fn dataset()->wiparse_core::log_analysis::timeline::Dataset {
  let lines=(0..1000).map(|i|Arc::<str>::from(format!("t={i} VIN=5 IOUT=2"))).collect();
  let source=Source::memory("synthetic",lines,1,0);
  let query:Query=serde_json::from_value(json!({"samples":0,"timeline":{"timestamp_pattern":"t=(\\d+)","timestamp_unit":"ms","fields":[{"name":"VIN","pattern":"VIN=([0-9.]+)","unit":"V"},{"name":"IOUT","pattern":"IOUT=([0-9.]+)","unit":"A"}],"power_pair":["VIN","IOUT"],"max_points":8,"retain_dataset":true}})).unwrap();
  let mut scan=Scan::new(Inventory{sources:vec![source],incomplete:false,issues:vec![]},query).unwrap();
  assert!(scan.dataset().is_err());
  scan.run_chunk(1024*1024,std::time::Duration::from_secs(5),&AtomicBool::new(false)).unwrap();
  assert!(scan.report()["timeline_analysis"]["sources"][0]["points"].as_array().unwrap().len()<=8);
  scan.dataset().unwrap()
 }
 #[test]fn actual_local_rows_not_model_summary_are_imported_with_physical_units(){
  let d=dataset();assert_eq!(d.rows.len(),1000);
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::File;
  let response=panel.agent_present(&d,"query",Lang::Zh).unwrap();assert_eq!(response["records"],1000);assert_eq!(response["series"],3);
  assert_eq!(panel.series[0].points[999],[0.999,5000.]);assert_eq!(panel.series[1].points[999],[0.999,2000.]);assert_eq!(panel.series[2].points[999],[0.999,10.]);assert_eq!(panel.series[2].kind.unit(),"W");
  let stats=panel.agent_statistics(&mut None,true);assert_eq!(stats["data"]["series"][2]["sample_count"],1000);assert!((stats["data"]["series"][2]["mean"].as_f64().unwrap()-10.).abs()<1e-10);
 }
 #[test]
 fn serial_timeline_plots_four_named_fields_and_reports_absent_values() {
     use wiparse_core::ai::tools::{host_functions, normalized, ToolCall};
     let fields = json!([
         {"name":"VIN","pattern":"key:VIN","unit":"mV"},
         {"name":"VBRG","pattern":"key:VBRG","unit":"mV"},
         {"name":"IBRG","pattern":"key:IBRG","unit":"mA"},
         {"name":"T","pattern":r"\bPH\s+\d+\s+T\s+(\d+)$","unit":"°C"}
     ]);
     let tool = host_functions(vec!["log.analyze"]).into_iter().find(|f|f.name=="wiparse_log_timeline").unwrap();
     assert_eq!(tool.parameters["properties"]["fields"]["maxItems"],MAX_SERIES);
     let call = ToolCall{id:"call".into(), name:tool.name, arguments:json!({"timestamp_pattern":r"(?:\d\d:\d\d:\d\d)",
         "fields":fields,"retain_dataset":true}).to_string()};
     let (_,p)=normalized(&call).unwrap();
     let mut panel=DataAnalysisPanel::new(&AppConfig::default());
     panel.source=DataSource::File;
     for (name,lines,want) in [
         ("all",vec!["12:00:00 ASK VIN 5 VBRG 6 IBRG 7 PH 0 T 27","12:00:01 STAT 81301"],4),
         ("missing",vec!["12:00:00 ASK VIN 5 VBRG 6 IBRG 7","12:00:01 STAT 81301"],3)
     ]{
         let source=Source::memory("fixture",lines.into_iter().map(Arc::<str>::from).collect(),1,0);
         let mut scan=Scan::new(Inventory{sources:vec![source],incomplete:false,issues:vec![]},serde_json::from_value(p.clone()).unwrap()).unwrap();
         scan.run_chunk(65536,std::time::Duration::from_secs(2),&AtomicBool::new(false)).unwrap();
         let receipt=panel.agent_present(&scan.dataset().unwrap(),name,Lang::Zh).unwrap();
         assert_eq!(receipt["series"],want);
         assert_eq!(receipt["curves"].as_array().unwrap().len(),want);
         assert_eq!(receipt["curves"][0]["sample_count"],1);
         if want==4 {assert_eq!(receipt["curves"][3]["field"],"T"); assert_eq!(panel.series[3].points[0][1],27.);}
         else {assert_eq!(receipt["missing_fields"],json!(["T"]));}
     }
 }
 #[test]
 fn sparse_sources_count_actual_series_and_overflow_keeps_previous_view() {
     let mut data=dataset();
     data.rows=(0..8).map(|i|(format!("source{i}"),0.,1,vec![Some(1.),None,None])).collect();
     let mut panel=DataAnalysisPanel::new(&AppConfig::default()); panel.source=DataSource::File;
     let receipt=panel.agent_present(&data,"eight",Lang::Zh).unwrap();
     assert_eq!(receipt["series"],8);
     data.rows[0].3[1]=Some(2.);
     assert!(panel.agent_present(&data,"nine",Lang::Zh).unwrap_err().contains("9 populated"));
     assert_eq!(panel.imported_query.as_deref(),Some("eight"));
 }
 #[test]
 fn absent_requested_field_cannot_be_marked_complete_by_provenance_count() {
  let mut data=dataset();
  for row in &mut data.rows { row.3[1]=None; }
  data.provenance["missing_plotted_fields"]=json!(0);
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());
  panel.source=DataSource::File;
  let receipt=panel.agent_present(&data,"missing-field",Lang::Zh).unwrap();
  assert!(receipt["missing_fields"].as_array().unwrap().iter().any(|v|v=="IOUT"));
  assert_eq!(panel.imported_quality.as_ref().unwrap()["quality_complete"],false);
 }
 #[test]fn loading_file_keeps_its_worker_and_ai_plot(){
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::File;
  let (tx,rx)=crossbeam_channel::unbounded();panel.pending_load=Some(PendingDataLoad{rx,rules_revision:panel.load_rules_revision});
  panel.agent_present(&dataset(),"q",Lang::Zh).unwrap();tx.send(LoadEvent::Progress(37)).unwrap();
  panel.poll_pending_load(Lang::Zh);assert!(panel.is_loading());assert_eq!(panel.load_progress_target,37);
  let mut bad=dataset();bad.rows[0].3.clear();assert!(panel.agent_present(&bad,"invalid",Lang::Zh).is_err());
  assert_eq!(panel.api_snapshot()["imported_query"],"q");drop(tx);panel.poll_pending_load(Lang::Zh);
  assert!(!panel.is_loading());assert_eq!(panel.api_snapshot()["imported_query"],"q");
 }
 #[test]fn live_destination_and_malformed_rows_preserve_previous_curves(){
  let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::SerialLive;panel.live_cursor=123;
  assert!(panel.agent_present(&dataset(),"q",Lang::Zh).is_ok());
  assert!(panel.wants_live_serial());assert_eq!(panel.live_line_cursor(),123);assert_eq!(panel.frame_count,0);
  assert_eq!(panel.api_snapshot()["imported_query"],"q");assert_eq!(panel.agent_statistics(&mut None,true)["data"]["series"][0]["sample_count"],1000);
  panel.showing_result=false;assert_eq!(panel.api_snapshot()["source"],"serial");
  panel.result_plot=None;panel.source=DataSource::File;panel.agent_present(&dataset(),"old",Lang::Zh).unwrap();
  let mut bad=dataset();bad.rows[0].3.clear();assert!(panel.agent_present(&bad,"new",Lang::Zh).is_err());assert_eq!(panel.imported_query.as_deref(),Some("old"));assert_eq!(panel.series[0].points.len(),1000);
 }
}

impl Drop for DataAnalysisPanel {
    fn drop(&mut self) {
        if self.wants_live_serial() {
            self.flush_live_cache();
        }
        if let Some(job) = self.export_job.take() {
            let _ = job.join();
        }
    }
}
#[cfg(test)]
mod live_history_tests {
    use super::*;

    #[test]
    fn serial_generation_and_fifo_gaps_begin_separate_sessions() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::SerialLive;
        panel.prepare_live_poll(4000, 0, (10, 0), Lang::Zh);
        assert_eq!(panel.live_cursor, 0);
        panel.live_cursor = 20;
        let session = panel.live_session_id.clone();
        panel.prepare_live_poll(4010, 100, (10, 0), Lang::Zh);
        assert_ne!(session, panel.live_session_id);
        assert_eq!(panel.live_cursor, 100);
        assert!(panel.gap_notice.as_deref().unwrap().contains("21–100"));
        panel.prepare_live_poll(4020, 100, (11, 4010), Lang::Zh);
        assert_eq!(panel.live_cursor, 4010);
    }

    #[test]
    fn full_recent_and_manual_views_with_sparse_data() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::SerialLive;
        panel.series_count = 1;
        panel.live_auto_fit = true;
        panel.ensure_series_shells(Lang::Zh);
        panel.series[0].points.push([0., 2.]);
        for n in [1, 79, 80, 81, 1000] {
            panel.frame_count = n;
            panel.refresh_cached_extent();
            let bounds = panel.live_fit_bounds().unwrap();
            assert!(bounds.min()[0] <= 0. && bounds.max()[0] >= (n - 1) as f64);
            assert!(bounds.width() > 0.);
        }
        panel.recent_view = true;
        let bounds = panel.live_fit_bounds().unwrap();
        assert_eq!(bounds.min()[0], 920.);
        assert_eq!(bounds.max()[0], 999.);
        panel.last_x_range = Some((5., 15.));
        panel.request_zoom_axis(true, 0.5);
        assert!(!panel.live_auto_fit);
        let frozen = panel.pending_bounds.unwrap();
        panel.frame_header.clear();
        panel.filters[0] = "csv:1".into();
        panel.ingest_live_lines(&["7".into()], 1, Lang::Zh);
        assert_eq!(panel.pending_bounds.unwrap().min(), frozen.min());
        assert_eq!(panel.pending_bounds.unwrap().max(), frozen.max());
        panel.live_frames_since_flush = 0;
    }

    #[test]
    fn eight_series_preserve_early_spikes_across_checkpoint_and_resume() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::SerialLive;
        panel.series_count = 8;
        panel.ensure_series_shells(Lang::Zh);
        let n = 200_117;
        for s in &mut panel.series {
            s.points = (0..n)
                .map(|i| [i as f64, if i == 17 { 900. } else { 2. }])
                .collect();
            s.points.seal();
        }
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("session.widalive");
        let mut points = panel
            .series
            .iter()
            .map(|s| s.points.clone())
            .collect::<Vec<_>>();
        for s in &mut points {
            s.page_out().unwrap();
        }
        let mut header = data_extract_cache::WidaHeader {
            source_key: "serial:live".into(),
            content_fp: "test".into(),
            header: String::new(),
            filters: vec![],
            kinds: vec![],
            frame_count: n as u32,
            series: (0..8)
                .map(|_| data_extract_cache::WidaSeriesMeta {
                    label: "v".into(),
                    kind: "voltage".into(),
                    y_scale: 1.,
                    y_offset: 0.,
                    y_min: 2.,
                    y_max: 900.,
                    n: n as u32,
                })
                .collect(),
        };
        history::checkpoint(&file, &header, &points).unwrap();
        let csv = root.path().join("capture.csv");
        history::export_csv(&csv, &[("VIN".into(), "mV".into(), points[0].clone())]).unwrap();
        let csv = fs::read_to_string(csv).unwrap();
        assert_eq!(csv.lines().count(), n + 1);
        assert_eq!(csv.lines().nth(18), Some("17,\"VIN\",\"mV\",900"));
        let mut job = None;
        let mut stats = panel.agent_statistics(&mut job, true);
        while stats.get("_agent_wait").is_some() {
            stats = panel.agent_statistics(&mut job, false);
        }
        for series in stats["data"]["series"].as_array().unwrap() {
            assert_eq!(series["sample_count"], n);
            assert_eq!(series["max"], 900.0);
        }
        for (i, p) in points.iter_mut().enumerate() {
            let overview = p.visible(0., n as f64, 900);
            assert!(overview.contains(&[17., 900.]));
            assert!(overview.windows(2).all(|w| w[0][0] < w[1][0]));
            assert_eq!(value_at_x(p, 17.), Some(900.));
            assert!(p.visible(15., 20., 900).contains(&[17., 900.]));
            p.push([n as f64, 13.]);
            header.series[i].n += 1;
        }
        header.frame_count += 1;
        history::checkpoint(&file, &header, &points).unwrap();
        let restored = load_wida(&file).unwrap();
        assert_eq!(restored.meta.frame_count as usize, n + 1);
        for s in restored.series {
            assert_eq!(s.points.len(), n + 1);
            assert_eq!(s.points[17], [17., 900.]);
            assert_eq!(s.points.last().unwrap(), &[n as f64, 13.]);
        }
    }
}
