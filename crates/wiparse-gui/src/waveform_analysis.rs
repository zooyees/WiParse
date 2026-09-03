//! Offline waveform analysis — folder browser, open scope sources, zoom/pan, measure.

use crate::plot::{PlotTextLabel, ScopeEnvelopePlotItem, ScopeVectorPlotItem};
use crate::theme::Tokens;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Vec2b};
use egui_plot::{
    CoordinatesFormatter, Corner, GridMark, HLine, Legend, Plot, PlotBounds, PlotPoints, Points,
    VLine,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use crossbeam_channel::{Receiver, Sender, unbounded};
use rayon::prelude::*;
use wiparse_core::bus_decode::{
    compact_bus_decode_indices, decode_bus, BusDecodeResult, BusDecodeSettings, BusKind, I2sFormat,
    SpiMode, SpiWire, UartParity,
};
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::instrument::WaveformTrace;
use wiparse_core::paths::project_path;
use wiparse_core::wave_display::{
    build_load_snapshot, build_viewport_series_ex, quantize_view_cache_key,
    viewport_column_count, WaveViewportSeries,
};
use wiparse_core::waveform_file::{
    load_waveform_file_all, measure_waveform, measure_waveform_range, save_waveform_file,
    WaveformMeasurements,
};

const TOOLBAR_H: f32 = 44.0;
const PANEL_GAP: f32 = 8.0;
const SIDE_W: f32 = 280.0;
/// Compact Y-axis strip width (scope-style, right of plot).
const CHANNEL_AXIS_COL_W: f32 = 28.0;
const AXIS_HANDLE_RADIUS: f32 = 5.5;
const AXIS_TICK_LEN: f32 = 4.0;
const BTN: egui::Vec2 = egui::vec2(108.0, 28.0);
const BTN_SM: egui::Vec2 = egui::vec2(72.0, 28.0);
const CARD_MARGIN_X: i8 = 8;
/// Cursor grab distance as a fraction of visible axis span.
const CURSOR_GRAB_FRAC: f64 = 0.012;
/// Max zoom-out: view may extend this factor beyond full data extent (X axis).
const VIEW_MAX_PAD: f64 = 1.25;
/// Min zoom-in on X: visible span ≥ full data span / this factor.
const VIEW_MAX_ZOOM_X: f64 = 50_000.0;
/// Max zoom-out padding for global Y viewport (per-channel scale handles trace sizing).
const VIEW_MAX_PAD_Y: f64 = 50.0;
const VIEW_MIN_SPAN_ABS: f64 = 1e-15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractMode {
    /// Scroll pan, right-drag box zoom. Clicks do not set cursors.
    Pan,
    /// Click places cursors; drag near a cursor to move it.
    Cursor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CursorAxis {
    /// Vertical cursors X1 / X2 (time).
    X,
    /// Horizontal cursors Y1 / Y2 (voltage).
    Y,
}

/// One first-level subfolder under the browser root that contains waveform sources.
struct WaveBrowserFolder {
    name: String,
    files: Vec<(String, PathBuf)>,
}

struct LoadedWave {
    path: PathBuf,
    label: String,
    trace: WaveformTrace,
    /// Global overview envelope (display-only; measure uses `trace`).
    overview: Arc<Vec<wiparse_core::wave_display::ScopeEnvelopeColumn>>,
    /// Cached full-trace extents (avoids O(N) scans every frame).
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    measures: WaveformMeasurements,
    /// True until tier-2 frequency / robust extent finish.
    measures_pending: bool,
    color: Color32,
    /// Plot-Y of channel ground (native 0). Display y = raw × scale + offset.
    /// Measurements use the raw trace.
    y_offset: f64,
    /// Per-channel vertical scale (1.0 = native); zoom expands around ground.
    y_scale: f64,
    /// Full-trace X is non-decreasing (viewport decimate can skip a second scan).
    x_monotonic: bool,
}

/// Viewport envelope cache: one entry per visible wave when zoomed.
struct ViewLineCache {
    view_key: u64,
    per_wave: HashMap<usize, WaveViewportSeries>,
}

/// Parsed channel payload from a background file load (colors assigned on UI thread).
struct LoadedChannelDraft {
    trace: WaveformTrace,
    measures: WaveformMeasurements,
    overview: Arc<Vec<wiparse_core::wave_display::ScopeEnvelopeColumn>>,
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    label: String,
    x_monotonic: bool,
}

struct PendingWaveLoad {
    path: PathBuf,
    rx: Receiver<WaveLoadEvent>,
    /// Wave indices inserted by tier-1 preview (updated when preview arrives).
    wave_range: Option<(usize, usize)>,
}

enum WaveLoadEvent {
    Preview {
        channels: Vec<LoadedChannelDraft>,
        max_pts: usize,
    },
    Final {
        measures: Vec<WaveformMeasurements>,
        extents: Vec<(f64, f64, f64, f64)>,
    },
    Error(String),
}

struct PendingViewportBuild {
    view_key: u64,
    rx: Receiver<(u64, HashMap<usize, WaveViewportSeries>)>,
}

struct PendingBusDecode {
    generation: u64,
    rx: Receiver<(u64, BusDecodeResult)>,
}

pub struct WaveformAnalysisPanel {
    waves: Vec<LoadedWave>,
    selected: Option<usize>,
    status: String,
    open_dir: PathBuf,
    /// Vertical cursors (time axis).
    x1: Option<f64>,
    x2: Option<f64>,
    /// Horizontal cursors (voltage axis).
    y1: Option<f64>,
    y2: Option<f64>,
    /// Space toggles X ↔ Y cursor measurement.
    cursor_axis: CursorAxis,
    next_color: usize,
    /// Configurable root for the sidebar folder browser.
    browser_dir: String,
    browser_scanned_dir: String,
    browser_folders: Vec<WaveBrowserFolder>,
    mode: InteractMode,
    /// Which cursor is being dragged (`1` or `2` on the active axis).
    dragging_cursor: Option<u8>,
    /// Request auto-fit on next plot frame.
    fit_request: bool,
    /// One-shot explicit X/Y bounds (e.g. zoom-to-cursors).
    pending_bounds: Option<PlotBounds>,
    /// Last frame plot X range — used for viewport-aware resampling.
    last_x_range: Option<(f64, f64)>,
    /// Last frame plot Y range.
    last_y_range: Option<(f64, f64)>,
    /// Cached selected-wave viewport polyline.
    view_line_cache: Option<ViewLineCache>,
    /// Cached X1–X2 gated measurements `(wave_idx, gate_key, measures)`.
    gated_measure_cache: Option<(usize, u64, WaveformMeasurements)>,
    /// Cached union extent of all loaded waves (avoids per-frame scans).
    cached_extent: Option<(f64, f64, f64, f64)>,
    extent_dirty: bool,
    /// Background file parse in progress.
    pending_load: Option<PendingWaveLoad>,
    /// Background viewport LOD rebuild.
    pending_viewport: Option<PendingViewportBuild>,
    /// Latest pan/zoom request while a viewport job is already running.
    queued_viewport: Option<(u64, f64, f64, f32)>,
    /// Dragging a channel position handle on the right axis strip.
    dragging_channel_offset: Option<usize>,
    /// Ctrl+drag on channel strip adjusts vertical scale.
    dragging_channel_scale: Option<usize>,
    /// Bus decode configuration and results.
    bus_settings: BusDecodeSettings,
    bus_result: Option<BusDecodeResult>,
    bus_decode_gen: u64,
    pending_bus_decode: Option<PendingBusDecode>,
    selected_bus_frame: Option<usize>,
    bus_decode_dirty: bool,
    /// Optional manual UART baud entry (empty = auto).
    uart_baud_text: String,
}

impl WaveformAnalysisPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let open_dir = PathBuf::from(&cfg.apps.instruments.save_dir);
        let open_dir = if open_dir.as_os_str().is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            open_dir
        };
        let browser_dir = if cfg.apps.instruments.waveform_browser_dir.trim().is_empty() {
            String::new()
        } else {
            cfg.apps.instruments.waveform_browser_dir.clone()
        };
        let mut panel = Self {
            waves: Vec::new(),
            selected: None,
            status: String::new(),
            open_dir,
            x1: None,
            x2: None,
            y1: None,
            y2: None,
            cursor_axis: CursorAxis::X,
            next_color: 0,
            browser_dir,
            browser_scanned_dir: String::new(),
            view_line_cache: None,
            gated_measure_cache: None,
            browser_folders: Vec::new(),
            mode: InteractMode::Pan,
            dragging_cursor: None,
            fit_request: false,
            pending_bounds: None,
            last_x_range: None,
            last_y_range: None,
            cached_extent: None,
            extent_dirty: true,
            pending_load: None,
            pending_viewport: None,
            queued_viewport: None,
            dragging_channel_offset: None,
            dragging_channel_scale: None,
            bus_settings: BusDecodeSettings::default(),
            bus_result: None,
            bus_decode_gen: 0,
            pending_bus_decode: None,
            selected_bus_frame: None,
            bus_decode_dirty: false,
            uart_baud_text: String::new(),
        };
        panel.refresh_wave_browser();
        panel
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        self.poll_pending_loads(lang);
        self.poll_pending_viewport();
        self.poll_pending_bus_decode();
        if self.bus_decode_dirty && self.schedule_bus_decode() {
            self.bus_decode_dirty = false;
        }
        let typing = ui.ctx().wants_keyboard_input();
        if !typing && ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.selected.is_some() {
            self.selected = None;
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

        panel_in_rect(ui, toolbar_rect, |ui| {
            self.toolbar(ui, lang, tokens);
        });

        // Browser (~45%) + bus decode (~30%) + measurements (rest).
        let body_inner = (body_h - PANEL_GAP * 2.0).max(200.0);
        let browser_h = (body_inner * 0.45).max(140.0);
        let bus_h = (body_inner * 0.30).max(110.0);
        let meas_h = (body_inner - browser_h - bus_h).max(90.0);
        let browser_rect =
            egui::Rect::from_min_size(side_rect.min, egui::vec2(side_w, browser_h));
        let bus_rect = egui::Rect::from_min_size(
            egui::pos2(side_rect.min.x, side_rect.min.y + browser_h + PANEL_GAP),
            egui::vec2(side_w, bus_h),
        );
        let meas_rect = egui::Rect::from_min_size(
            egui::pos2(side_rect.min.x, side_rect.min.y + browser_h + bus_h + PANEL_GAP * 2.0),
            egui::vec2(side_w, meas_h),
        );

        panel_in_rect(ui, browser_rect, |ui| {
            self.browser_panel(ui, lang, tokens);
        });
        card_in_rect(
            ui,
            bus_rect,
            tokens,
            &t(lang, "协议分析", "Bus Decode"),
            |ui| self.bus_decode_panel(ui, lang, tokens),
        );
        card_in_rect(
            ui,
            meas_rect,
            tokens,
            &t(lang, "测量", "Measurements"),
            |ui| self.measures_panel(ui, lang, tokens),
        );
        card_in_rect(
            ui,
            plot_rect,
            tokens,
            &t(lang, "波形图", "Plot"),
            |ui| self.plot_area(ui, lang, tokens),
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.set_max_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if ui
                        .add(
                            egui::Button::new(t(lang, "打开波形…", "Open Waveform…"))
                                .fill(tokens.accent)
                                .min_size(BTN),
                        )
                        .clicked()
                    {
                        self.open_files(lang);
                    }
                    let has = self.selected.is_some();
                    if ui
                        .add_enabled(
                            has,
                            egui::Button::new(t(lang, "关闭", "Close")).min_size(BTN_SM),
                        )
                        .clicked()
                    {
                        if let Some(i) = self.selected {
                            // Multi-channel WFM shares one path — close the whole document.
                            let path = self.waves.get(i).map(|w| w.path.clone());
                            if let Some(path) = path {
                                self.waves.retain(|w| !same_wave_path(&w.path, &path));
                            } else {
                                self.waves.remove(i);
                            }
                            if self.waves.is_empty() {
                                self.selected = None;
                                self.last_x_range = None;
                                self.last_y_range = None;
                                self.pending_bounds = None;
                                self.fit_request = false;
                                self.bus_result = None;
                                self.pending_bus_decode = None;
                                self.selected_bus_frame = None;
                            } else {
                                self.activate_wave(0);
                            }
                            self.status = t(lang, "已关闭波形", "Waveform closed").into();
                            self.mark_extent_dirty();
                        }
                    }
                    if ui
                        .add_enabled(
                            has,
                            egui::Button::new(t(lang, "导出…", "Export…")).min_size(BTN_SM),
                        )
                        .clicked()
                    {
                        self.export_selected(lang);
                    }

                    ui.separator();

                    // Interact mode: Pan vs Cursor (avoids click/drag conflict).
                    let pan_on = self.mode == InteractMode::Pan;
                    if ui
                        .add(
                            egui::Button::new(t(lang, "平移", "Pan"))
                                .selected(pan_on)
                                .min_size(egui::vec2(56.0, 28.0)),
                        )
                        .on_hover_text(t(
                            lang,
                            "滚轮平移 · 左键拖拽 · 右键框选 · Ctrl+滚轮缩 X · Ctrl+Shift+滚轮缩 Y",
                            "Scroll pan · drag · box zoom · Ctrl+wheel X · Ctrl+Shift+wheel Y",
                        ))
                        .clicked()
                    {
                        self.mode = InteractMode::Pan;
                        self.dragging_cursor = None;
                    }
                    let cur_on = self.mode == InteractMode::Cursor;
                    if ui
                        .add(
                            egui::Button::new(t(lang, "光标", "Cursor"))
                                .selected(cur_on)
                                .min_size(egui::vec2(56.0, 28.0)),
                        )
                        .on_hover_text(t(
                            lang,
                            "点击设光标 · 空格切换 X/Y · 拖近光标微调",
                            "Click set cursor · Space toggle X/Y · drag to fine-tune",
                        ))
                        .clicked()
                    {
                        self.mode = InteractMode::Cursor;
                    }

                    let axis_label = match self.cursor_axis {
                        CursorAxis::X => t(lang, "光标:X", "Cursors:X"),
                        CursorAxis::Y => t(lang, "光标:Y", "Cursors:Y"),
                    };
                    if ui
                        .add(
                            egui::Button::new(axis_label)
                                .selected(true)
                                .min_size(egui::vec2(72.0, 28.0)),
                        )
                        .on_hover_text(t(
                            lang,
                            "空格键切换 X1/X2 与 Y1/Y2 测量",
                            "Space toggles X1/X2 vs Y1/Y2 measure",
                        ))
                        .clicked()
                    {
                        self.toggle_cursor_axis();
                    }

                    ui.separator();

                    if ui
                        .add(egui::Button::new(t(lang, "适应", "Fit")).min_size(egui::vec2(52.0, 28.0)))
                        .on_hover_text(t(lang, "显示全部波形", "Show full waveform"))
                        .clicked()
                    {
                        // Fit all channels in the current document (same source path).
                        self.fit_request = true;
                        self.pending_bounds =
                            self.document_extent().or_else(|| self.selected_extent());
                        if let Some(ext) = self.pending_bounds.as_ref() {
                            self.last_x_range = Some((ext.min()[0], ext.max()[0]));
                            self.last_y_range = Some((ext.min()[1], ext.max()[1]));
                        }
                    }
                    let has_pair = match self.cursor_axis {
                        CursorAxis::X => self.x1.is_some() && self.x2.is_some(),
                        CursorAxis::Y => self.y1.is_some() && self.y2.is_some(),
                    };
                    if ui
                        .add_enabled(
                            has_pair,
                            egui::Button::new(t(lang, "缩放到光标", "Zoom Cursors"))
                                .min_size(egui::vec2(96.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.request_zoom_to_cursors();
                    }
                    if ui
                        .add(egui::Button::new("X+").min_size(egui::vec2(36.0, 28.0)))
                        .on_hover_text(t(lang, "横向放大", "Zoom in X"))
                        .clicked()
                    {
                        self.request_zoom_axis(true, 0.5);
                    }
                    if ui
                        .add(egui::Button::new("X−").min_size(egui::vec2(36.0, 28.0)))
                        .on_hover_text(t(lang, "横向缩小", "Zoom out X"))
                        .clicked()
                    {
                        self.request_zoom_axis(true, 2.0);
                    }
                    if ui
                        .add(egui::Button::new("Y+").min_size(egui::vec2(36.0, 28.0)))
                        .on_hover_text(t(
                            lang,
                            if self.selected.is_some() {
                                "纵向放大选中通道"
                            } else {
                                "纵向放大"
                            },
                            if self.selected.is_some() {
                                "Zoom in Y (selected channel scale)"
                            } else {
                                "Zoom in Y"
                            },
                        ))
                        .clicked()
                    {
                        self.apply_y_zoom_factor(2.0);
                    }
                    if ui
                        .add(egui::Button::new("Y−").min_size(egui::vec2(36.0, 28.0)))
                        .on_hover_text(t(
                            lang,
                            if self.selected.is_some() {
                                "纵向缩小选中通道"
                            } else {
                                "纵向缩小"
                            },
                            if self.selected.is_some() {
                                "Zoom out Y (selected channel scale)"
                            } else {
                                "Zoom out Y"
                            },
                        ))
                        .clicked()
                    {
                        self.apply_y_zoom_factor(0.5);
                    }
                    if ui
                        .add_enabled(self.selected.is_some(), {
                            egui::Button::new(t(lang, "复位通道", "Reset CH"))
                                .min_size(egui::vec2(72.0, 28.0))
                        })
                        .on_hover_text(t(
                            lang,
                            "复位选中通道的位移与比例",
                            "Reset offset & scale for selected channel",
                        ))
                        .clicked()
                    {
                        if let Some(i) = self.selected {
                            self.reset_channel_display(i);
                        }
                    }
                    if ui
                        .add_enabled(!self.waves.is_empty(), {
                            egui::Button::new(t(lang, "复位全部", "Reset All"))
                                .min_size(egui::vec2(72.0, 28.0))
                        })
                        .on_hover_text(t(
                            lang,
                            "复位所有通道的位移与比例",
                            "Reset offset & scale for all channels",
                        ))
                        .clicked()
                    {
                        self.reset_all_channel_display();
                    }
                    if ui
                        .add(
                            egui::Button::new(t(lang, "清除光标", "Clear"))
                                .min_size(egui::vec2(72.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.clear_active_cursors();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(t(
                                lang,
                                "Ctrl+滚轮=X · Ctrl+Shift+滚轮=Y · 轴滚轮=比例 · Shift+滚轮=位移",
                                "Ctrl+wheel=X · Ctrl+Shift+wheel=Y · strip wheel=scale · Shift+wheel=offset",
                            ))
                            .small()
                            .color(tokens.text_muted),
                        );
                    });
                });
            });
    }

    fn reset_channel_display(&mut self, index: usize) {
        if let Some(w) = self.waves.get_mut(index) {
            w.y_offset = 0.0;
            w.y_scale = 1.0;
            self.mark_extent_dirty();
        }
    }

    fn reset_all_channel_display(&mut self) {
        for w in &mut self.waves {
            w.y_offset = 0.0;
            w.y_scale = 1.0;
        }
        self.mark_extent_dirty();
    }

    fn toggle_cursor_axis(&mut self) {
        self.cursor_axis = match self.cursor_axis {
            CursorAxis::X => CursorAxis::Y,
            CursorAxis::Y => CursorAxis::X,
        };
        self.mode = InteractMode::Cursor;
        self.dragging_cursor = None;
        self.status = match self.cursor_axis {
            CursorAxis::X => "光标模式: X1/X2 (空格切换 Y)".into(),
            CursorAxis::Y => "光标模式: Y1/Y2 (空格切换 X)".into(),
        };
    }

    fn clear_active_cursors(&mut self) {
        match self.cursor_axis {
            CursorAxis::X => {
                self.x1 = None;
                self.x2 = None;
            }
            CursorAxis::Y => {
                self.y1 = None;
                self.y2 = None;
            }
        }
        self.dragging_cursor = None;
        self.gated_measure_cache = None;
    }

    fn request_zoom_to_cursors(&mut self) {
        let Some(extent) = self.data_extent() else {
            return;
        };
        let bounds = match self.cursor_axis {
            CursorAxis::X => {
                let (Some(a), Some(b)) = (self.x1, self.x2) else {
                    return;
                };
                let (lo, hi) = ordered(a, b);
                if (hi - lo).abs() < f64::EPSILON {
                    return;
                }
                let pad = (hi - lo) * 0.05;
                let x0 = lo - pad;
                let x1 = hi + pad;
                let (y0, y1) = self.y_range_in_x(x0, x1).unwrap_or((extent.min()[1], extent.max()[1]));
                let ypad = ((y1 - y0).abs() * 0.08).max(1e-12);
                PlotBounds::from_min_max([x0, y0 - ypad], [x1, y1 + ypad])
            }
            CursorAxis::Y => {
                let (Some(a), Some(b)) = (self.y1, self.y2) else {
                    return;
                };
                let (lo, hi) = ordered(a, b);
                if (hi - lo).abs() < f64::EPSILON {
                    return;
                }
                let pad = (hi - lo) * 0.05;
                let (x0, x1) = self
                    .last_x_range
                    .unwrap_or((extent.min()[0], extent.max()[0]));
                PlotBounds::from_min_max([x0, lo - pad], [x1, hi + pad])
            }
        };
        self.pending_bounds = Some(self.clamp_view_bounds(bounds));
        self.fit_request = false;
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
    }

    /// Y zoom using egui_plot convention: `zoom > 1` zooms in, `zoom < 1` zooms out.
    ///
    /// Selected channel → multiply `y_scale` around ground (native 0 / `y_offset`).
    /// No selection → shrink/expand plot Y span (inverse of `zoom`).
    fn apply_y_zoom_factor(&mut self, zoom: f64) {
        if !zoom.is_finite() || (zoom - 1.0).abs() <= 1e-6 {
            return;
        }
        if let Some(i) = self.selected {
            if let Some(w) = self.waves.get_mut(i) {
                w.y_scale = (w.y_scale * zoom).clamp(1e-12, 1e12);
                self.mark_extent_dirty();
                return;
            }
        }
        self.request_zoom_axis(false, 1.0 / zoom);
    }

    fn data_extent(&self) -> Option<PlotBounds> {
        let (xmin, xmax, ymin, ymax) = self.cached_extent?;
        if (xmax - xmin).abs() < VIEW_MIN_SPAN_ABS {
            return None;
        }
        let xpad = ((xmax - xmin).abs() * 0.02).max(VIEW_MIN_SPAN_ABS);
        let ypad = ((ymax - ymin).abs() * 0.08).max(VIEW_MIN_SPAN_ABS);
        Some(PlotBounds::from_min_max(
            [xmin - xpad, ymin - ypad],
            [xmax + xpad, ymax + ypad],
        ))
    }

    fn mark_extent_dirty(&mut self) {
        self.extent_dirty = true;
    }

    fn refresh_cached_extent(&mut self) {
        if self.waves.is_empty() {
            self.cached_extent = None;
            return;
        }
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for w in &self.waves {
            xmin = xmin.min(w.x_min);
            xmax = xmax.max(w.x_max);
            let (dy0, dy1) = wave_display_y_bounds(w);
            ymin = ymin.min(dy0);
            ymax = ymax.max(dy1);
        }
        if !xmin.is_finite() {
            self.cached_extent = None;
            return;
        }
        if (xmax - xmin).abs() < VIEW_MIN_SPAN_ABS {
            xmax = xmin + VIEW_MIN_SPAN_ABS;
        }
        if (ymax - ymin).abs() < VIEW_MIN_SPAN_ABS {
            ymax = ymin + VIEW_MIN_SPAN_ABS;
        }
        self.cached_extent = Some((xmin, xmax, ymin, ymax));
    }

    /// Extent of the selected wave only.
    fn selected_extent(&self) -> Option<PlotBounds> {
        let w = self.selected.and_then(|i| self.waves.get(i))?;
        extent_of_traces(&[w])
    }

    /// Channel used for Y-cursor / hover readout (selected, else first).
    fn y_measure_wave_index(&self) -> Option<usize> {
        self.selected
            .filter(|&i| i < self.waves.len())
            .or_else(|| (!self.waves.is_empty()).then_some(0))
    }

    /// Extent of every channel that belongs to the selected file (multi-channel WFM).
    fn document_extent(&self) -> Option<PlotBounds> {
        let path = self.selected.and_then(|i| self.waves.get(i)).map(|w| w.path.as_path())?;
        let peers: Vec<&LoadedWave> = self
            .waves
            .iter()
            .filter(|w| same_wave_path(&w.path, path))
            .collect();
        if peers.is_empty() {
            return None;
        }
        extent_of_traces(&peers)
    }

    /// Clamp view so it cannot zoom/pan to infinity.
    fn clamp_view_bounds(&self, bounds: PlotBounds) -> PlotBounds {
        let Some(extent) = self.data_extent() else {
            return bounds;
        };
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

        // Keep center inside outer box. When span ≈ outer width, float error can make
        // (outer0 + span/2) > (outer1 - span/2); f64::clamp panics if min > max.
        let clamp_center = |c: f64, outer0: f64, outer1: f64, span: f64| -> (f64, f64) {
            let half = span * 0.5;
            let lo = outer0 + half;
            let hi = outer1 - half;
            if lo <= hi && lo.is_finite() && hi.is_finite() {
                (c.clamp(lo, hi), span)
            } else {
                ((outer0 + outer1) * 0.5, (outer1 - outer0).abs().max(VIEW_MIN_SPAN_ABS))
            }
        };
        (cx, x_span) = clamp_center(cx, outer_x0, outer_x1, x_span);
        (cy, y_span) = clamp_center(cy, outer_y0, outer_y1, y_span);

        PlotBounds::from_min_max(
            [cx - x_span * 0.5, cy - y_span * 0.5],
            [cx + x_span * 0.5, cy + y_span * 0.5],
        )
    }

    fn y_range_in_x(&self, x0: f64, x1: f64) -> Option<(f64, f64)> {
        let w = self.selected.and_then(|i| self.waves.get(i))?;
        let n = w.trace.x.len().min(w.trace.y.len());
        if n == 0 {
            return None;
        }
        let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let xs = &w.trace.x[..n];
        let start = xs.partition_point(|&x| x < lo);
        let end = xs.partition_point(|&x| x <= hi);
        if start >= end {
            return None;
        }
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for &yv in &w.trace.y[start..end] {
            ymin = ymin.min(yv);
            ymax = ymax.max(yv);
        }
        if ymin.is_finite() && ymax.is_finite() {
            Some((ymin, ymax))
        } else {
            None
        }
    }

    fn browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 6))
            .corner_radius(CornerRadius::same(6))
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = inner_w;

                ui.label(
                    RichText::new(tr(lang, "wave.browser_dir"))
                        .size(12.0)
                        .strong()
                        .color(tokens.text_primary),
                );
                let browser_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.browser_dir)
                        .desired_width(ctrl_w)
                        .hint_text(tr(lang, "wave.browser_hint"))
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if browser_edit.lost_focus() {
                    self.refresh_wave_browser();
                    self.persist_browser_dir();
                }

                let gap = 6.0;
                let btn_w = ((ctrl_w - gap) * 0.5).max(48.0);
                ui.horizontal(|ui| {
                    ui.set_max_width(ctrl_w);
                    ui.spacing_mut().item_spacing.x = gap;
                    if ui
                        .add_sized(
                            egui::vec2(btn_w, 26.0),
                            egui::Button::new(tr(lang, "btn.browse_dir")),
                        )
                        .clicked()
                    {
                        self.browse_wave_browser_dir();
                    }
                    if ui
                        .add_sized(
                            egui::vec2(btn_w, 26.0),
                            egui::Button::new(tr(lang, "btn.refresh_browser")),
                        )
                        .clicked()
                    {
                        self.refresh_wave_browser();
                    }
                });

                self.ensure_wave_browser_fresh();

                if self.browser_dir.trim().is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "wave.browser_hint"))
                            .size(11.0)
                            .color(tokens.text_muted),
                    );
                } else if self.browser_folders.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "wave.browser_empty"))
                            .size(11.0)
                            .color(tokens.text_muted),
                    );
                } else {
                    let list_h = ui.available_height().max(72.0);
                    let selected_path = self
                        .selected
                        .and_then(|i| self.waves.get(i))
                        .map(|w| w.path.clone());
                    let mut open_path: Option<PathBuf> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("wave_browser_tree")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ctrl_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for folder in &self.browser_folders {
                                let header =
                                    format!("{}  ({})", folder.name, folder.files.len());
                                egui::CollapsingHeader::new(
                                    RichText::new(header).size(12.0).strong(),
                                )
                                .id_salt(("wave-browser-folder", folder.name.as_str()))
                                .default_open(false)
                                .show(ui, |ui| {
                                    ui.set_max_width(ctrl_w);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    for (name, path) in &folder.files {
                                        let selected = selected_path
                                            .as_ref()
                                            .is_some_and(|p| same_wave_path(p, path));
                                        let resp = ui.selectable_label(
                                            selected,
                                            RichText::new(name)
                                                .size(12.0)
                                                .color(tokens.text_primary),
                                        );
                                        if resp.clicked() {
                                            open_path = Some(path.clone());
                                        }
                                    }
                                });
                            }
                        });
                    if let Some(path) = open_path {
                        self.open_browser_file(&path, lang);
                    }
                }
            });
    }

    fn browse_wave_browser_dir(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        let start = if !self.browser_dir.trim().is_empty() {
            project_path(&self.browser_dir)
        } else if self.open_dir.is_dir() {
            self.open_dir.clone()
        } else {
            PathBuf::new()
        };
        if start.is_dir() {
            dialog = dialog.set_directory(start);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.browser_dir = dir.to_string_lossy().into_owned();
            self.refresh_wave_browser();
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

    /// Scan `browser_dir` for immediate subfolders that contain waveform sources.
    fn refresh_wave_browser(&mut self) {
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
            let mut waves = Vec::new();
            for file in files.flatten() {
                let file_path = file.path();
                if !file_path.is_file() {
                    continue;
                }
                if !is_waveform_source_ext(&file_path) {
                    continue;
                }
                let name = file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file_path.display().to_string());
                waves.push((name, file_path));
            }
            if waves.is_empty() {
                continue;
            }
            waves.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            folders.push(WaveBrowserFolder { name, files: waves });
        }
        folders.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        self.browser_folders = folders;
    }

    fn ensure_wave_browser_fresh(&mut self) {
        if self.browser_dir.trim() != self.browser_scanned_dir.trim() {
            self.refresh_wave_browser();
        }
    }

    fn persist_browser_dir(&self) {
        match load_config() {
            Ok(mut cfg) => {
                cfg.apps.instruments.waveform_browser_dir = self.browser_dir.clone();
                let _ = save_config(&cfg);
            }
            Err(_) => {
                let mut cfg = AppConfig::default();
                cfg.apps.instruments.waveform_browser_dir = self.browser_dir.clone();
                let _ = save_config(&cfg);
            }
        }
    }

    fn open_browser_file(&mut self, path: &Path, lang: Lang) {
        if let Some(parent) = path.parent() {
            self.open_dir = parent.to_path_buf();
        }
        if self.waves.iter().any(|w| same_wave_path(&w.path, path)) {
            // Browser = single-document switch: keep every channel from this file
            // (multi-channel WFM expands to N LoadedWave entries sharing one path).
            self.waves.retain(|w| same_wave_path(&w.path, path));
            self.next_color = self.waves.len();
            self.activate_wave(0);
            self.status = format!(
                "{} {} ({} ch)",
                t(lang, "已打开", "Opened"),
                path.display(),
                self.waves.len()
            );
            return;
        }
        // Replace current document with the newly loaded file.
        self.waves.clear();
        self.selected = None;
        self.next_color = 0;
        self.last_x_range = None;
        self.last_y_range = None;
        self.pending_bounds = None;
        self.view_line_cache = None;
        self.gated_measure_cache = None;
        self.start_load_path(path.to_path_buf(), lang);
    }

    fn start_load_path(&mut self, path: PathBuf, lang: Lang) {
        if self.pending_load.is_some() {
            self.status = t(lang, "正在加载上一文件…", "Previous file still loading…").into();
            return;
        }
        let (tx, rx) = unbounded();
        self.pending_load = Some(PendingWaveLoad {
            path: path.clone(),
            rx,
            wave_range: None,
        });
        self.status = format!(
            "{} {}…",
            t(lang, "正在加载", "Loading"),
            path.display()
        );
        thread::spawn(move || {
            load_waveform_file_worker(&path, tx);
        });
    }

    fn apply_preview_channels(
        &mut self,
        path: &Path,
        channels: Vec<LoadedChannelDraft>,
        max_pts: usize,
        lang: Lang,
    ) -> (usize, usize) {
        let nch = channels.len();
        let first_idx = self.waves.len();
        for draft in channels {
            let color = color_for_channel(&draft.trace.channel).unwrap_or_else(|| {
                let c = TRACE_COLORS[self.next_color % TRACE_COLORS.len()];
                self.next_color += 1;
                c
            });
            self.waves.push(LoadedWave {
                path: path.to_path_buf(),
                label: draft.label,
                trace: draft.trace,
                overview: draft.overview,
                x_min: draft.x_min,
                x_max: draft.x_max,
                y_min: draft.y_min,
                y_max: draft.y_max,
                measures: draft.measures,
                measures_pending: true,
                color,
                y_offset: 0.0,
                y_scale: 1.0,
                x_monotonic: draft.x_monotonic,
            });
        }
        let end = self.waves.len();
        if nch > 1 {
            auto_stagger_channel_offsets(&mut self.waves, first_idx..end);
        }
        self.view_line_cache = None;
        self.pending_viewport = None;
        self.queued_viewport = None;
        self.gated_measure_cache = None;
        self.mark_extent_dirty();
        self.activate_wave(first_idx);
        self.fit_request = true;
        self.status = format!(
            "{} {} ({} ch, {} pts/ch)",
            t(lang, "已加载", "Loaded"),
            path.display(),
            nch,
            max_pts
        );
        self.bus_decode_dirty = true;
        (first_idx, end)
    }

    fn apply_final_measures(&mut self, wave_range: (usize, usize), measures: Vec<WaveformMeasurements>, extents: Vec<(f64, f64, f64, f64)>) {
        let (first, end) = wave_range;
        for (i, (m, (x_min, x_max, y_min, y_max))) in (first..end)
            .zip(measures.into_iter().zip(extents.into_iter()))
        {
            if let Some(w) = self.waves.get_mut(i) {
                w.measures = m;
                w.x_min = x_min;
                w.x_max = x_max;
                w.y_min = y_min;
                w.y_max = y_max;
                w.measures_pending = false;
            }
        }
        self.mark_extent_dirty();
        self.gated_measure_cache = None;
    }

    fn poll_pending_loads(&mut self, lang: Lang) {
        loop {
            let event = {
                let Some(load) = self.pending_load.as_ref() else {
                    return;
                };
                match load.rx.try_recv() {
                    Ok(ev) => ev,
                    Err(_) => return,
                }
            };
            let path = self
                .pending_load
                .as_ref()
                .map(|l| l.path.clone())
                .unwrap_or_default();
            match event {
                WaveLoadEvent::Preview { channels, max_pts } => {
                    let (first, end) =
                        self.apply_preview_channels(&path, channels, max_pts, lang);
                    if let Some(load) = self.pending_load.as_mut() {
                        load.wave_range = Some((first, end));
                    }
                }
                WaveLoadEvent::Final {
                    measures,
                    extents,
                } => {
                    if let Some((first, end)) = self
                        .pending_load
                        .as_ref()
                        .and_then(|l| l.wave_range)
                    {
                        self.apply_final_measures((first, end), measures, extents);
                    }
                    self.pending_load = None;
                }
                WaveLoadEvent::Error(err) => {
                    self.pending_load = None;
                    self.status = format!(
                        "{} {}: {err}",
                        t(lang, "加载失败", "Load failed"),
                        path.display()
                    );
                }
            }
        }
    }

    fn poll_pending_viewport(&mut self) {
        let Some(pending) = self.pending_viewport.as_ref() else {
            return;
        };
        let Ok((key, per_wave)) = pending.rx.try_recv() else {
            return;
        };
        let expected_key = pending.view_key;
        self.pending_viewport = None;
        let queued = self.queued_viewport.take();
        let latest = queued.as_ref().map(|q| q.0).unwrap_or(expected_key);
        if key == expected_key && key == latest {
            self.view_line_cache = Some(ViewLineCache {
                view_key: key,
                per_wave,
            });
        }
        if let Some((qk, x0, x1, plot_width_px)) = queued {
            if qk != key {
                self.schedule_viewport_build(qk, x0, x1, plot_width_px);
            }
        }
    }

    fn schedule_viewport_build(&mut self, key: u64, x0: f64, x1: f64, plot_width_px: f32) {
        if self
            .pending_viewport
            .as_ref()
            .is_some_and(|p| p.view_key == key)
        {
            self.queued_viewport = None;
            return;
        }
        if self.pending_viewport.is_some() {
            self.queued_viewport = Some((key, x0, x1, plot_width_px));
            return;
        }
        self.queued_viewport = None;
        let jobs: Vec<ViewportBuildJob> = self
            .waves
            .iter()
            .enumerate()
            .filter_map(|(i, w)| {
                if view_covers_overview(x0, x1, w.x_min, w.x_max) {
                    return None;
                }
                let lo = x0.min(x1);
                let hi = x0.max(x1);
                let pad = (hi - lo).abs() * 0.02;
                if w.x_max < lo - pad || w.x_min > hi + pad {
                    return None;
                }
                let prev_uniform = self
                    .view_line_cache
                    .as_ref()
                    .and_then(|c| c.per_wave.get(&i))
                    .is_some_and(|s| s.is_uniform());
                Some(ViewportBuildJob {
                    index: i,
                    x: Arc::clone(&w.trace.x),
                    y: Arc::clone(&w.trace.y),
                    x0,
                    x1,
                    plot_width_px,
                    prev_uniform,
                    x_monotonic: w.x_monotonic,
                })
            })
            .collect();
        if jobs.is_empty() {
            self.view_line_cache = Some(ViewLineCache {
                view_key: key,
                per_wave: HashMap::new(),
            });
            return;
        }
        let (tx, rx) = unbounded();
        self.pending_viewport = Some(PendingViewportBuild { view_key: key, rx });
        thread::spawn(move || {
            let per_wave: HashMap<usize, WaveViewportSeries> = jobs
                .into_par_iter()
                .map(|job| {
                    let n = job.x.len().min(job.y.len());
                    (
                        job.index,
                        build_viewport_series_ex(
                            &job.x[..n],
                            &job.y[..n],
                            job.x0,
                            job.x1,
                            job.plot_width_px,
                            job.prev_uniform,
                            job.x_monotonic,
                        ),
                    )
                })
                .collect();
            let _ = tx.send((key, per_wave));
        });
    }

    fn bus_overlay_channel(&self) -> Option<usize> {
        let ch = &self.bus_settings.channels;
        match self.bus_settings.kind {
            BusKind::Off => None,
            BusKind::Uart => ch.uart_signal,
            BusKind::I2c => ch.i2c_sda.or(ch.i2c_scl),
            BusKind::Spi => ch.spi_mosi.or(ch.spi_clk),
            BusKind::I2s => ch.i2s_data.or(ch.i2s_ws).or(ch.i2s_bclk),
        }
    }

    fn bus_channels_ready(&self) -> bool {
        match self.bus_settings.kind {
            BusKind::Off => false,
            BusKind::Uart => self.bus_settings.channels.uart_signal.is_some(),
            BusKind::I2c => {
                self.bus_settings.channels.i2c_scl.is_some()
                    && self.bus_settings.channels.i2c_sda.is_some()
            }
            BusKind::Spi => {
                let clk = self.bus_settings.channels.spi_clk.is_some();
                let io0 = self.bus_settings.channels.spi_mosi.is_some();
                match self.bus_settings.spi.wire {
                    SpiWire::TwoWire | SpiWire::FourWire => clk && io0,
                    SpiWire::Dual => {
                        clk && io0 && self.bus_settings.channels.spi_miso.is_some()
                    }
                    SpiWire::Quad => {
                        clk && io0
                            && self.bus_settings.channels.spi_miso.is_some()
                            && self.bus_settings.channels.spi_io2.is_some()
                            && self.bus_settings.channels.spi_io3.is_some()
                    }
                }
            }
            BusKind::I2s => {
                self.bus_settings.channels.i2s_bclk.is_some()
                    && self.bus_settings.channels.i2s_ws.is_some()
                    && self.bus_settings.channels.i2s_data.is_some()
            }
        }
    }

    fn effective_bus_settings(&self) -> BusDecodeSettings {
        let mut settings = self.bus_settings.clone();
        settings.uart.baud = self
            .uart_baud_text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|&b| b > 0.0)
            .or(settings.uart.baud);
        if let (Some(a), Some(b)) = (self.x1, self.x2) {
            settings.time_gate = Some((a, b));
        } else {
            settings.time_gate = None;
        }
        settings
    }

    fn schedule_bus_decode(&mut self) -> bool {
        if self.bus_settings.kind == BusKind::Off || self.waves.is_empty() {
            self.bus_result = None;
            self.pending_bus_decode = None;
            self.selected_bus_frame = None;
            return true;
        }
        if !self.bus_channels_ready() {
            self.bus_result = None;
            return true;
        }
        if self.pending_bus_decode.is_some() {
            return false;
        }
        let settings = self.effective_bus_settings();
        let (indices, settings) =
            compact_bus_decode_indices(self.waves.len(), &settings);
        let traces: Vec<WaveformTrace> = indices
            .iter()
            .filter_map(|&i| self.waves.get(i).map(|w| w.trace.clone()))
            .collect();
        let generation = self.bus_decode_gen.wrapping_add(1);
        self.bus_decode_gen = generation;
        let (tx, rx) = unbounded();
        self.pending_bus_decode = Some(PendingBusDecode { generation, rx });
        thread::spawn(move || {
            let result = decode_bus(&traces, &settings);
            let _ = tx.send((generation, result));
        });
        true
    }

    fn poll_pending_bus_decode(&mut self) {
        let Some(pending) = self.pending_bus_decode.as_ref() else {
            return;
        };
        let Ok((gen, result)) = pending.rx.try_recv() else {
            return;
        };
        self.pending_bus_decode = None;
        if gen != self.bus_decode_gen {
            return;
        }
        self.bus_result = Some(result);
        self.selected_bus_frame = None;
    }

    fn bus_decode_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let mut changed = false;
        if self.waves.is_empty() {
            ui.label(
                RichText::new(t(lang, "加载波形后可用", "Load a waveform first"))
                    .small()
                    .color(tokens.text_muted),
            );
            return;
        }

        let layout = bus_form_layout(ui);
        ui.spacing_mut().item_spacing.y = 4.0;

        egui::Grid::new("bus_decode_form")
            .num_columns(2)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                let prev = self.bus_settings.kind;
                bus_grid_label(ui, layout, &t(lang, "协议", "Protocol"));
                egui::ComboBox::from_id_salt("bus_kind")
                    .selected_text(self.bus_settings.kind.label())
                    .width(layout.field_w)
                    .show_ui(ui, |ui| {
                        for kind in BusKind::all_selectable() {
                            if ui
                                .selectable_label(self.bus_settings.kind == *kind, kind.label())
                                .clicked()
                            {
                                self.bus_settings.kind = *kind;
                                changed = true;
                            }
                        }
                    });
                if self.bus_settings.kind != prev {
                    changed = true;
                }
                ui.end_row();

                match self.bus_settings.kind {
                    BusKind::Off => {}
                    BusKind::Uart => {
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "uart_sig",
                            &t(lang, "信号", "Signal"),
                            &self.waves,
                            &mut self.bus_settings.channels.uart_signal,
                        );
                        bus_grid_label(ui, layout, &t(lang, "波特率", "Baud"));
                        let baud_hint = t(lang, "自动", "auto");
                        let resp = ui.add_sized(
                            [layout.field_w, layout.row_h],
                            egui::TextEdit::singleline(&mut self.uart_baud_text)
                                .hint_text(baud_hint),
                        );
                        let enter = ui.input(|i| {
                            i.events.iter().any(|e| {
                                matches!(
                                    e,
                                    egui::Event::Key {
                                        key: egui::Key::Enter,
                                        pressed: true,
                                        ..
                                    }
                                )
                            })
                        });
                        if resp.lost_focus() || (resp.has_focus() && enter) {
                            changed = true;
                        }
                        ui.end_row();
                        changed |= bus_grid_buttons(
                            ui,
                            layout,
                            &t(lang, "数据位", "Data"),
                            &mut self.bus_settings.uart.data_bits,
                            &[("5", 5), ("6", 6), ("7", 7), ("8", 8), ("9", 9)],
                            22.0,
                        );
                        changed |= bus_grid_parity(
                            ui,
                            layout,
                            &t(lang, "校验", "Parity"),
                            &mut self.bus_settings.uart.parity,
                        );
                        changed |= bus_grid_buttons(
                            ui,
                            layout,
                            &t(lang, "停止位", "Stop"),
                            &mut self.bus_settings.uart.stop_bits,
                            &[("1", 1), ("2", 2)],
                            28.0,
                        );
                        changed |= bus_grid_bool_pair(
                            ui,
                            layout,
                            &t(lang, "极性", "Polarity"),
                            &mut self.bus_settings.uart.inverted,
                            ("TTL", "Inv"),
                        );
                    }
                    BusKind::I2c => {
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "i2c_scl",
                            "SCL",
                            &self.waves,
                            &mut self.bus_settings.channels.i2c_scl,
                        );
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "i2c_sda",
                            "SDA",
                            &self.waves,
                            &mut self.bus_settings.channels.i2c_sda,
                        );
                        bus_grid_label(ui, layout, &t(lang, "寄存器位宽", "Reg bits"));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let bits = if self.bus_settings.i2c.reg_addr_bits == 16 {
                                16u8
                            } else {
                                8
                            };
                            for (label, value) in [("8", 8u8), ("16", 16)] {
                                if ui
                                    .add_sized(
                                        [28.0, layout.row_h],
                                        egui::Button::new(label).selected(bits == value),
                                    )
                                    .clicked()
                                    && bits != value
                                {
                                    self.bus_settings.i2c.reg_addr_bits = value;
                                    changed = true;
                                }
                            }
                        });
                        ui.end_row();
                    }
                    BusKind::Spi => {
                        changed |= bus_grid_spi_wire(
                            ui,
                            layout,
                            &t(lang, "线制", "Wires"),
                            &mut self.bus_settings.spi.wire,
                        );
                        changed |= bus_grid_spi_mode(
                            ui,
                            layout,
                            &t(lang, "模式", "Mode"),
                            &mut self.bus_settings.spi.mode,
                        );
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "spi_clk",
                            "CLK",
                            &self.waves,
                            &mut self.bus_settings.channels.spi_clk,
                        );
                        let packed = self.bus_settings.spi.wire.is_packed();
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "spi_mosi",
                            if packed { "IO0" } else { "MOSI" },
                            &self.waves,
                            &mut self.bus_settings.channels.spi_mosi,
                        );
                        if self.bus_settings.spi.wire != SpiWire::TwoWire {
                            changed |= bus_grid_channel_combo(
                                ui,
                                layout,
                                "spi_miso",
                                if packed { "IO1" } else { "MISO" },
                                &self.waves,
                                &mut self.bus_settings.channels.spi_miso,
                            );
                        }
                        if self.bus_settings.spi.wire == SpiWire::Quad {
                            changed |= bus_grid_channel_combo(
                                ui,
                                layout,
                                "spi_io2",
                                "IO2",
                                &self.waves,
                                &mut self.bus_settings.channels.spi_io2,
                            );
                            changed |= bus_grid_channel_combo(
                                ui,
                                layout,
                                "spi_io3",
                                "IO3",
                                &self.waves,
                                &mut self.bus_settings.channels.spi_io3,
                            );
                        }
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "spi_cs",
                            "CS",
                            &self.waves,
                            &mut self.bus_settings.channels.spi_cs,
                        );
                        changed |= bus_grid_buttons(
                            ui,
                            layout,
                            &t(lang, "位宽", "Word"),
                            &mut self.bus_settings.spi.word_bits,
                            &[("8", 8), ("16", 16), ("24", 24), ("32", 32)],
                            28.0,
                        );
                        changed |= bus_grid_bool_pair(
                            ui,
                            layout,
                            &t(lang, "位序", "Bit order"),
                            &mut self.bus_settings.spi.msb_first,
                            ("LSB", "MSB"),
                        );
                        changed |= bus_grid_bool_pair(
                            ui,
                            layout,
                            &t(lang, "片选", "Chip sel"),
                            &mut self.bus_settings.spi.cs_active_low,
                            ("High", "Low"),
                        );
                    }
                    BusKind::I2s => {
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "i2s_bclk",
                            "BCLK",
                            &self.waves,
                            &mut self.bus_settings.channels.i2s_bclk,
                        );
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "i2s_ws",
                            "WS",
                            &self.waves,
                            &mut self.bus_settings.channels.i2s_ws,
                        );
                        changed |= bus_grid_channel_combo(
                            ui,
                            layout,
                            "i2s_data",
                            "DATA",
                            &self.waves,
                            &mut self.bus_settings.channels.i2s_data,
                        );
                        bus_grid_label(ui, layout, &t(lang, "位宽", "Bits"));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let bits = self.bus_settings.i2s.bits_per_sample;
                            for (label, value) in [("16", 16u8), ("24", 24), ("32", 32)] {
                                if ui
                                    .add_sized(
                                        [28.0, layout.row_h],
                                        egui::Button::new(label).selected(bits == value),
                                    )
                                    .clicked()
                                    && bits != value
                                {
                                    self.bus_settings.i2s.bits_per_sample = value;
                                    changed = true;
                                }
                            }
                        });
                        ui.end_row();
                        changed |= bus_grid_i2s_format(
                            ui,
                            layout,
                            &t(lang, "格式", "Format"),
                            &mut self.bus_settings.i2s.format,
                        );
                    }
                }
            });

        if self.bus_settings.kind == BusKind::Off {
            ui.label(
                RichText::new(t(lang, "选择协议并映射通道", "Pick a protocol and map channels"))
                    .small()
                    .color(tokens.text_muted),
            );
        }

        ui.add_space(4.0);
        if changed {
            self.bus_decode_dirty = true;
        }

        if self.pending_bus_decode.is_some() {
            ui.label(
                RichText::new(t(lang, "解码中…", "Decoding…"))
                    .small()
                    .color(tokens.text_muted),
            );
        }
        if let Some(result) = &self.bus_result {
            if let Some(err) = &result.error {
                ui.label(RichText::new(err).small().color(tokens.stop_bg));
            }
            if result.truncated {
                ui.label(
                    RichText::new(t(
                        lang,
                        "已达 512 字节上限，后续数据未解析",
                        "Stopped at 512-byte limit; remaining data was not decoded",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            }
            if !result.info.is_empty() {
                ui.label(RichText::new(&result.info).small().color(tokens.text_muted));
            }
            if result.frames.is_empty() && result.error.is_none() {
                ui.label(
                    RichText::new(t(lang, "无解码帧", "No decoded frames"))
                        .small()
                        .color(tokens.text_muted),
                );
            } else if !result.frames.is_empty() {
                ui.label(
                    RichText::new(t(
                        lang,
                        &format!(
                            "已在波形上标注 {} 条（点击标注可定位）",
                            result.frames.len()
                        ),
                        &format!(
                            "{} annotations on the waveform (click a label to zoom)",
                            result.frames.len()
                        ),
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            }
        }
    }

    fn zoom_to_bus_frame(&mut self, frame: &wiparse_core::bus_decode::BusFrame) {
        let pad = (frame.t_end - frame.t_start).abs().max(1e-6) * 6.0;
        let x0 = frame.t_start - pad;
        let x1 = frame.t_end + pad;
        if let Some(ext) = self.document_extent().or_else(|| self.data_extent()) {
            let y0 = ext.min()[1];
            let y1 = ext.max()[1];
            self.pending_bounds = Some(PlotBounds::from_min_max([x0, y0], [x1, y1]));
            self.last_x_range = Some((x0, x1));
            self.last_y_range = Some((y0, y1));
        } else {
            self.pending_bounds = Some(PlotBounds::from_min_max([x0, -1.0], [x1, 1.0]));
        }
    }

    fn load_path_sync(&mut self, path: &Path) -> Result<usize, String> {
        let (tx, rx) = unbounded();
        load_waveform_file_worker(path, tx);
        let WaveLoadEvent::Preview { channels, max_pts } = rx
            .recv()
            .map_err(|e| e.to_string())?
        else {
            return Err("unexpected load event".into());
        };
        let (first, end) = {
            let nch = channels.len();
            let first_idx = self.waves.len();
            for draft in channels {
                let color = color_for_channel(&draft.trace.channel).unwrap_or_else(|| {
                    let c = TRACE_COLORS[self.next_color % TRACE_COLORS.len()];
                    self.next_color += 1;
                    c
                });
                self.waves.push(LoadedWave {
                    path: path.to_path_buf(),
                    label: draft.label,
                    trace: draft.trace,
                    overview: draft.overview,
                    x_min: draft.x_min,
                    x_max: draft.x_max,
                    y_min: draft.y_min,
                    y_max: draft.y_max,
                    measures: draft.measures,
                    measures_pending: true,
                    color,
                    y_offset: 0.0,
                    y_scale: 1.0,
                    x_monotonic: draft.x_monotonic,
                });
            }
            let end = self.waves.len();
            if nch > 1 {
                auto_stagger_channel_offsets(&mut self.waves, first_idx..end);
            }
            (first_idx, end)
        };
        if let Ok(WaveLoadEvent::Final { measures, extents }) = rx.recv() {
            self.apply_final_measures((first, end), measures, extents);
        }
        self.view_line_cache = None;
        self.gated_measure_cache = None;
        self.mark_extent_dirty();
        self.activate_wave(first);
        Ok(max_pts)
    }

    #[allow(dead_code)]
    fn load_path(&mut self, path: &Path) -> Result<usize, String> {
        self.load_path_sync(path)
    }

    /// Select a loaded wave and reset the plot viewport to the whole document.
    fn activate_wave(&mut self, index: usize) {
        if index >= self.waves.len() {
            return;
        }
        self.selected = Some(index);
        self.fit_request = true;
        self.pending_bounds = None;
        self.view_line_cache = None;
        self.gated_measure_cache = None;
        // Pre-seed viewport from all channels of this file. egui_plot's plot_bounds()
        // returns *last frame* until draw finishes, so we must not read it for Fit.
        if let Some(ext) = self.document_extent().or_else(|| self.selected_extent()) {
            self.last_x_range = Some((ext.min()[0], ext.max()[0]));
            self.last_y_range = Some((ext.min()[1], ext.max()[1]));
            self.pending_bounds = Some(ext);
        } else {
            self.last_x_range = None;
            self.last_y_range = None;
        }
        self.x1 = None;
        self.x2 = None;
        self.y1 = None;
        self.y2 = None;
        self.dragging_cursor = None;
    }

    fn measures_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        egui::ScrollArea::vertical()
            .id_salt("waveform-measures")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Some(i) = self.selected else {
                    ui.label(
                        RichText::new(t(lang, "无选中波形", "No waveform selected"))
                            .color(tokens.text_muted)
                            .small(),
                    );
                    return;
                };
                let gated = self.cached_gated_measures(i);
                let Some(w) = self.waves.get(i) else {
                    return;
                };
                ui.label(
                    RichText::new(&w.label)
                        .small()
                        .strong()
                        .color(tokens.text_primary),
                );
                ui.add_space(4.0);
                let yu = w.trace.y_unit.as_str();
                let xu = w.trace.x_unit.as_str();
                let m = gated.as_ref().unwrap_or(&w.measures);
                let scope = if gated.is_some() {
                    t(lang, "X1–X2 区间", "X1–X2 gate")
                } else {
                    t(lang, "全波形", "Full wave")
                };
                ui.label(
                    RichText::new(scope)
                        .small()
                        .color(tokens.text_muted),
                );
                if w.measures_pending && gated.is_none() {
                    ui.label(
                        RichText::new(t(
                            lang,
                            "精确测量计算中…",
                            "Refining measurements…",
                        ))
                        .small()
                        .color(tokens.text_muted),
                    );
                }
                ui.add_space(2.0);
                measure_row(ui, tokens, "N", &m.count.to_string());
                measure_row(ui, tokens, "min", &format_eng(m.min, yu));
                measure_row(ui, tokens, "max", &format_eng(m.max, yu));
                measure_row(ui, tokens, "pp", &format_eng(m.pp, yu));
                measure_row(ui, tokens, "mean", &format_eng(m.mean, yu));
                measure_row(ui, tokens, "rms", &format_eng(m.rms, yu));
                measure_row(ui, tokens, "Δt", &format_eng(m.dt, xu));
                if let Some(f) = m.freq_hz {
                    measure_row(ui, tokens, "freq", &format_eng(f, "Hz"));
                } else {
                    measure_row(ui, tokens, "freq", "—");
                }
                if let Some(p) = m.period {
                    measure_row(ui, tokens, "period", &format_eng(p, xu));
                } else {
                    measure_row(ui, tokens, "period", "—");
                }
                ui.add_space(8.0);
                let title = match self.cursor_axis {
                    CursorAxis::X => t(lang, "光标 X（空格→Y）", "Cursors X (Space→Y)"),
                    CursorAxis::Y => t(lang, "光标 Y（空格→X）", "Cursors Y (Space→X)"),
                };
                ui.label(RichText::new(title).strong().size(12.0));
                ui.add_space(4.0);
                match self.cursor_axis {
                    CursorAxis::X => self.draw_x_cursor_measures(ui, tokens, lang, &w.trace, xu, yu),
                    CursorAxis::Y => self.draw_y_cursor_measures(ui, tokens, lang, Some(i)),
                }
            });
    }

    /// X1–X2 gated measures, cached while cursors / selection are unchanged.
    fn cached_gated_measures(&mut self, wave_index: usize) -> Option<WaveformMeasurements> {
        let (a, b) = (self.x1?, self.x2?);
        let key = f64_pair_key(a, b);
        if let Some((wi, k, m)) = &self.gated_measure_cache {
            if *wi == wave_index && *k == key {
                return Some(m.clone());
            }
        }
        let w = self.waves.get(wave_index)?;
        let m = measure_waveform_range(&w.trace, a, b);
        self.gated_measure_cache = Some((wave_index, key, m.clone()));
        Some(m)
    }

    fn draw_x_cursor_measures(
        &self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        lang: Lang,
        trace: &WaveformTrace,
        xu: &str,
        yu: &str,
    ) {
        match (self.x1, self.x2) {
            (Some(a), Some(b)) => {
                let dx = b - a;
                measure_row(ui, tokens, "X1", &format_eng(a, xu));
                if let Some(ya) = interpolate_y(trace, a) {
                    measure_row(ui, tokens, "Y@X1", &format_eng(ya, yu));
                }
                measure_row(ui, tokens, "X2", &format_eng(b, xu));
                if let Some(yb) = interpolate_y(trace, b) {
                    measure_row(ui, tokens, "Y@X2", &format_eng(yb, yu));
                }
                measure_row(ui, tokens, "ΔX", &format_eng(dx, xu));
                measure_row(ui, tokens, "|ΔX|", &format_eng(dx.abs(), xu));
                if let (Some(ya), Some(yb)) = (interpolate_y(trace, a), interpolate_y(trace, b)) {
                    measure_row(ui, tokens, "ΔY", &format_eng(yb - ya, yu));
                }
                if dx.abs() > f64::EPSILON {
                    measure_row(ui, tokens, "freq", &format_freq(1.0 / dx.abs()));
                    measure_row(ui, tokens, "period", &format_eng(dx.abs(), xu));
                }
            }
            (Some(a), None) => {
                measure_row(ui, tokens, "X1", &format_eng(a, xu));
                if let Some(ya) = interpolate_y(trace, a) {
                    measure_row(ui, tokens, "Y@X1", &format_eng(ya, yu));
                }
                ui.label(
                    RichText::new(t(lang, "再点击设置 X2", "Click again to set X2"))
                        .small()
                        .color(tokens.text_muted),
                );
            }
            _ => {
                ui.label(
                    RichText::new(t(
                        lang,
                        "光标模式点击设置 X1 / X2",
                        "In Cursor mode, click to set X1 / X2",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            }
        }
    }

    fn draw_y_cursor_measures(
        &self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        lang: Lang,
        measure_idx: Option<usize>,
    ) {
        let measure_idx = measure_idx.or_else(|| self.y_measure_wave_index());
        match (self.y1, self.y2) {
            (Some(y1_plot), Some(y2_plot)) => {
                // Always lead with the active channel (GND = 0 for that channel).
                let order: Vec<usize> = {
                    let mut idx: Vec<usize> = (0..self.waves.len()).collect();
                    if let Some(sel) = measure_idx {
                        if let Some(pos) = idx.iter().position(|&i| i == sel) {
                            idx.swap(0, pos);
                        }
                    }
                    idx
                };
                for (k, idx) in order.iter().enumerate() {
                    let w = &self.waves[*idx];
                    let ch = short_channel_label(&w.trace.channel);
                    let yu = w.trace.y_unit.as_str();
                    let y1 = plot_y_to_native(y1_plot, w.y_scale, w.y_offset);
                    let y2 = plot_y_to_native(y2_plot, w.y_scale, w.y_offset);
                    let dy = y2 - y1;
                    ui.label(
                        RichText::new(&ch)
                            .small()
                            .strong()
                            .color(w.color),
                    );
                    measure_row(ui, tokens, "Y1", &format_eng(y1, yu));
                    measure_row(ui, tokens, "Y2", &format_eng(y2, yu));
                    measure_row(ui, tokens, "ΔY", &format_eng(dy, yu));
                    measure_row(ui, tokens, "|ΔY|", &format_eng(dy.abs(), yu));
                    if k + 1 < order.len() {
                        ui.add_space(6.0);
                    }
                }
            }
            (Some(y1_plot), None) => {
                if let Some(w) = measure_idx.and_then(|i| self.waves.get(i)) {
                    let yu = w.trace.y_unit.as_str();
                    let y1 = plot_y_to_native(y1_plot, w.y_scale, w.y_offset);
                    measure_row(ui, tokens, "Y1", &format_eng(y1, yu));
                }
                ui.label(
                    RichText::new(t(lang, "再点击设置 Y2", "Click again to set Y2"))
                        .small()
                        .color(tokens.text_muted),
                );
            }
            _ => {
                ui.label(
                    RichText::new(t(
                        lang,
                        "光标模式点击设置 Y1 / Y2",
                        "In Cursor mode, click to set Y1 / Y2",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            }
        }
    }

    /// Shared Y axis strip: all channel handles on one axis (select nearest to interact).
    fn channel_offset_axes(
        &mut self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        lang: Lang,
        y0: f64,
        y1: f64,
    ) {
        let n = self.waves.len();
        if n == 0 {
            return;
        }
        let (y_lo, y_hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let y_span = (y_hi - y_lo).max(1e-30);
        let strip_h = ui.available_height();
        let strip_w = ui.available_width();
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        let shift = ui.input(|i| i.modifiers.shift);
        let y_unit = self
            .selected
            .and_then(|i| self.waves.get(i))
            .map(|w| w.trace.y_unit.as_str())
            .or_else(|| self.waves.first().map(|w| w.trace.y_unit.as_str()))
            .unwrap_or("V");

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(strip_w, strip_h),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        // Scope-style axis rail: subtle fill + left edge line.
        painter.rect_filled(rect, CornerRadius::same(2), tokens.surface_bg);
        painter.line_segment(
            [rect.left_top(), rect.left_bottom()],
            Stroke::new(1.0_f32, tokens.border),
        );
        let tick_x0 = rect.left() + 1.0;
        let tick_x1 = tick_x0 + AXIS_TICK_LEN;
        let show_tick_labels = resp.hovered() || self.selected.is_some();
        for (frac, with_unit) in [(0.0_f32, true), (0.5, false), (1.0, false)] {
            let y = egui::lerp(rect.top()..=rect.bottom(), frac);
            painter.line_segment(
                [egui::pos2(tick_x0, y), egui::pos2(tick_x1, y)],
                Stroke::new(0.5_f32, tokens.border.gamma_multiply(0.65)),
            );
            if show_tick_labels {
                let y_plot = y_lo + (1.0 - f64::from(frac)) * (y_hi - y_lo);
                let tick_ch = self
                    .selected
                    .filter(|&i| i < n)
                    .or(if n > 0 { Some(0) } else { None });
                let y_val = tick_ch
                    .and_then(|i| self.waves.get(i))
                    .map(|w| plot_y_to_native(y_plot, w.y_scale, w.y_offset))
                    .unwrap_or(y_plot);
                painter.text(
                    egui::pos2(rect.right() - 1.0, y),
                    egui::Align2::RIGHT_CENTER,
                    format_axis_tick(y_val, y_unit, with_unit),
                    egui::FontId::monospace(7.0),
                    tokens.text_muted,
                );
            }
        }

        let axis_cx = rect.center().x;
        // Channel marker tracks native 0 V (GND), not the amplitude midpoint.
        let zeros: Vec<f64> = self.waves.iter().map(wave_display_y_zero).collect();
        let handle_y = |i: usize| -> (f32, bool) {
            channel_gnd_screen_y(zeros[i], y_lo, y_span, rect)
        };
        let pick_at = |pos: egui::Pos2| -> Option<usize> {
            let mut best = None;
            let mut best_d2 = 14.0_f32 * 14.0_f32;
            for i in 0..n {
                let (hy, _) = handle_y(i);
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
            let w = &self.waves[i];
            let color = w.color;
            let (hy, on_screen) = handle_y(i);
            let selected = self.selected == Some(i);
            let active = self.dragging_channel_offset == Some(i)
                || self.dragging_channel_scale == Some(i);
            let highlighted = selected || active || hover_ch == Some(i);
            let radius = if highlighted {
                AXIS_HANDLE_RADIUS + 1.0
            } else {
                AXIS_HANDLE_RADIUS
            };
            paint_channel_gnd_handle(
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
                let channel = short_channel_label(&w.trace.channel);
                painter.text(
                    egui::pos2(axis_cx, hy - radius - 1.0),
                    egui::Align2::CENTER_BOTTOM,
                    channel,
                    egui::FontId::proportional(9.0),
                    color,
                );
                let scale = w.y_scale;
                if (scale - 1.0).abs() > 1e-6 {
                    painter.text(
                        egui::pos2(axis_cx, hy + radius + 1.0),
                        egui::Align2::CENTER_TOP,
                        format_scale(scale),
                        egui::FontId::monospace(8.0),
                        color.gamma_multiply(0.85),
                    );
                }
            }
        }

        if let Some(i) = self.selected {
            if let Some(w) = self.waves.get(i) {
                let div = format_channel_div(w, y_span);
                painter.text(
                    egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    div,
                    egui::FontId::monospace(8.0),
                    tokens.text_muted,
                );
            }
        }

        let active_ch = self
            .dragging_channel_offset
            .or(self.dragging_channel_scale)
            .or(hover_ch)
            .or(self.selected.filter(|&i| i < n));

        if resp.hovered() {
            if let Some(scroll) = {
                let s = ui.input(|inp| {
                    inp.smooth_scroll_delta.y as f64 + inp.raw_scroll_delta.y as f64
                });
                if s.abs() > 0.0 { Some(s) } else { None }
            } {
                let Some(i) = active_ch else {
                    return;
                };
                if shift {
                    self.waves[i].y_offset += -(scroll / 120.0) * y_span * 0.08;
                } else {
                    let factor = (scroll * 0.002).exp();
                    self.waves[i].y_scale =
                        (self.waves[i].y_scale * factor).clamp(1e-12, 1e12);
                }
                self.mark_extent_dirty();
                self.selected = Some(i);
            }
        }

        if resp.drag_started() {
            if let Some(i) = hover_ch {
                self.selected = Some(i);
                if ctrl {
                    self.dragging_channel_scale = Some(i);
                } else {
                    self.dragging_channel_offset = Some(i);
                }
            }
        }
        if let Some(i) = self.dragging_channel_scale {
            if resp.dragged() {
                let dy = resp.drag_delta().y as f64;
                let factor = (-dy * 0.008).exp();
                self.waves[i].y_scale =
                    (self.waves[i].y_scale * factor).clamp(1e-12, 1e12);
                self.mark_extent_dirty();
            }
        } else if let Some(i) = self.dragging_channel_offset {
            if resp.dragged() {
                let dy = resp.drag_delta().y;
                self.waves[i].y_offset += -(dy as f64 / rect.height() as f64) * y_span;
                self.mark_extent_dirty();
            }
        }
        if resp.double_clicked() {
            if let Some(i) = hover_ch.or(self.selected) {
                if ctrl {
                    self.waves[i].y_scale = 1.0;
                } else {
                    self.waves[i].y_offset = 0.0;
                }
                self.mark_extent_dirty();
                self.selected = Some(i);
            }
        }
        if resp.clicked() && !resp.dragged() {
            if let Some(i) = hover_ch {
                self.selected = Some(i);
            }
        }
        if resp.hovered() {
            if let Some(i) = active_ch {
                let y_unit = self.waves[i].trace.y_unit.clone();
                let sc = self.waves[i].y_scale;
                let ch = short_channel_label(&self.waves[i].trace.channel);
                let div = format_channel_div(&self.waves[i], y_span);
                let tip = format!(
                    "{}\n{}\n{}\n{}\n{}: {}\n{}: {}\n{}: {}",
                    ch,
                    t(lang, "拖动：移动零基准（GND）", "Drag: move ground (0)"),
                    t(lang, "滚轮：比例（绕零基准） · Shift+滚轮：位移", "Wheel: scale about ground · Shift+wheel: offset"),
                    t(lang, "Ctrl+拖动：比例缩放", "Ctrl+drag: scale"),
                    t(lang, "零基准", "Ground"),
                    format_eng(0.0, &y_unit),
                    t(lang, "比例", "Scale"),
                    format_scale(sc),
                    t(lang, "量程", "Range"),
                    div,
                );
                resp.on_hover_text(tip);
            }
        }

        if !ui.input(|i| i.pointer.primary_down()) {
            self.dragging_channel_offset = None;
            self.dragging_channel_scale = None;
        }
    }

    fn plot_area(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let height = ui.available_height().max(80.0);
        if self.waves.is_empty() {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(
                        RichText::new(t(
                            lang,
                            "从左侧目录打开示波器源文件（CSV / ISF / WFM）",
                            "Open a scope source from the left browser (CSV / ISF / WFM)",
                        ))
                        .color(tokens.text_muted),
                    );
                },
            );
            return;
        }

        if self.extent_dirty {
            self.refresh_cached_extent();
            self.extent_dirty = false;
        }

        // Space toggles X/Y cursor measure (when not typing in a text field).
        let typing = ui.ctx().wants_keyboard_input();
        if !typing && ui.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_cursor_axis();
        }

        let (ctrl, shift) = ui.input(|i| {
            (
                i.modifiers.ctrl || i.modifiers.command || i.modifiers.mac_cmd,
                i.modifiers.shift,
            )
        });
        // Ctrl+wheel → X zoom via egui_plot.
        // Ctrl+Shift+wheel → Y zoom MUST be manual: egui remaps Shift+wheel to
        // horizontal scroll (delta.y=0), so zoom_delta never fires for Y.
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
        let mode = self.mode;
        let axis = self.cursor_axis;
        let x_view = self.last_x_range;
        let mut next_x_range = None;
        let mut next_y_range = None;
        let mut click_val = None;
        let mut click_bus_frame: Option<usize> = None;
        let mut drag_cursor: Option<(u8, f64)> = None;
        let mut end_drag = false;
        let mut needs_clamp = false;

        let total_w = ui.available_width();
        let nch = self.waves.len();
        let strip_w = if nch > 0 { CHANNEL_AXIS_COL_W } else { 0.0 };
        let plot_w = (total_w - strip_w - 3.0).max(80.0);
        let plot_width_px = plot_w.max(256.0);
        let mut y_for_axes = self.last_y_range;
        let mut plot_response = None;
        let xu = self
            .selected
            .and_then(|i| self.waves.get(i))
            .map(|w| w.trace.x_unit.clone())
            .unwrap_or_else(|| "s".into());
        let y_maps: Vec<(String, f64, f64, String)> = self
            .waves
            .iter()
            .map(|w| {
                (
                    w.label.clone(),
                    w.y_scale,
                    w.y_offset,
                    w.trace.y_unit.clone(),
                )
            })
            .collect();
        let y_prefer = self.y_measure_wave_index();
        let y_axis_map = y_prefer.and_then(|i| y_maps.get(i).cloned()).or_else(|| {
            y_maps.first().cloned()
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            plot_response = Some(
                ui.allocate_ui_with_layout(
                    egui::vec2(plot_w, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(plot_w);
                        ui.set_max_width(plot_w);

        // Rebuild viewport envelope cache when pan/zoom moves by ≥1 display column.
        if let Some((x0, x1)) = x_view {
            let zoom_cols = viewport_column_count(plot_width_px);
            let (data_xmin, data_xmax) = self
                .cached_extent
                .map(|(a, b, _, _)| (a, b))
                .unwrap_or((0.0, 1.0));
            let key = quantize_view_cache_key(x0, x1, data_xmin, data_xmax, zoom_cols);
            let need = self
                .view_line_cache
                .as_ref()
                .map(|c| c.view_key != key)
                .unwrap_or(true);
            if need {
                self.schedule_viewport_build(key, x0, x1, plot_width_px);
            }
        } else {
            self.view_line_cache = None;
        }
        let view_cache = self.view_line_cache.as_ref();

        let x1 = self.x1;
        let x2 = self.x2;
        let y1 = self.y1;
        let y2 = self.y2;
        let dragging = self.dragging_cursor;
        let bus_overlay_y = self.bus_overlay_channel().and_then(|i| {
            self.waves
                .get(i)
                .map(|w| (w.y_offset, w.y_scale, w.y_min, w.y_max))
        });
        let bus_markers: Vec<(usize, f64, f64, bool, String)> = self
            .bus_result
            .as_ref()
            .map(|r| {
                r.frames
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        (
                            i,
                            f.t_start,
                            f.t_end,
                            self.selected_bus_frame == Some(i),
                            f.summary.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        Plot::new("waveform-analysis-plot")
            .height(height)
            .allow_zoom(allow_zoom)
            .allow_drag(mode == InteractMode::Pan)
            .allow_scroll(!ctrl)
            .allow_boxed_zoom(mode == InteractMode::Pan)
            .legend(Legend::default())
            .y_axis_formatter({
                let map = y_axis_map.clone();
                move |mark: GridMark, _range| {
                    let y = match &map {
                        Some((_, sc, off, _)) => plot_y_to_native(mark.value, *sc, *off),
                        None => mark.value,
                    };
                    format_axis_tick(y, "", false)
                }
            })
            .label_formatter({
                let xu = xu.clone();
                let maps = y_maps.clone();
                let prefer = y_prefer;
                move |name, value| {
                    let (y, yu) = native_y_for_series(value.y, name, &maps, prefer);
                    if name.is_empty() {
                        format!(
                            "t={}\nv={}",
                            format_eng(value.x, &xu),
                            format_eng(y, &yu)
                        )
                    } else {
                        format!(
                            "{name}\nt={}\nv={}",
                            format_eng(value.x, &xu),
                            format_eng(y, &yu)
                        )
                    }
                }
            })
            .coordinates_formatter(
                Corner::LeftBottom,
                CoordinatesFormatter::new({
                    let xu = xu.clone();
                    let maps = y_maps.clone();
                    let prefer = y_prefer;
                    move |value, _bounds| {
                        let (y, yu) = native_y_for_series(value.y, "", &maps, prefer);
                        format!(
                            "t={}   v={}",
                            format_eng(value.x, &xu),
                            format_eng(y, &yu)
                        )
                    }
                }),
            )
            .show(ui, |plot_ui| {
                // IMPORTANT: `plot_ui.plot_bounds()` returns *last frame* until the plot
                // is drawn. `set_plot_bounds` only queues a modification. Never seed
                // last_x_range from plot_bounds() on a Fit frame — that re-applies the
                // previous file's window and makes the new waveform "disappear".
                let mut applied: Option<PlotBounds> = None;
                if fit {
                    if let Some(ext) = self
                        .document_extent()
                        .or_else(|| self.selected_extent())
                        .or_else(|| self.data_extent())
                    {
                        let clamped = self.clamp_view_bounds(ext);
                        plot_ui.set_plot_bounds(clamped);
                        applied = Some(clamped);
                    } else {
                        plot_ui.set_auto_bounds(Vec2b::new(true, true));
                    }
                }
                if let Some(b) = pending {
                    let clamped = self.clamp_view_bounds(b);
                    plot_ui.set_plot_bounds(clamped);
                    applied = Some(clamped);
                }

                let bounds = applied.unwrap_or_else(|| plot_ui.plot_bounds());
                next_x_range = Some((bounds.min()[0], bounds.max()[0]));
                next_y_range = Some((bounds.min()[1], bounds.max()[1]));
                let x_span = (bounds.max()[0] - bounds.min()[0]).abs().max(1e-30);
                let y_span = (bounds.max()[1] - bounds.min()[1]).abs().max(1e-30);
                let grab_x = x_span * CURSOR_GRAB_FRAC;
                let grab_y = y_span * CURSOR_GRAB_FRAC;

                let x_view_lo = x_view.map(|(a, b)| a.min(b));
                let x_view_hi = x_view.map(|(a, b)| a.max(b));

                for (i, w) in self.waves.iter().enumerate() {
                    let stroke = if self.selected == Some(i) {
                        2.0_f32
                    } else {
                        1.2_f32
                    };
                    let highlighted = self.selected == Some(i);
                    let use_overview = match (x_view_lo, x_view_hi) {
                        (Some(lo), Some(hi)) => view_covers_overview(lo, hi, w.x_min, w.x_max),
                        _ => true,
                    };
                    if use_overview {
                        if !w.overview.is_empty() {
                            plot_ui.add(ScopeEnvelopePlotItem::new(
                                Arc::clone(&w.overview),
                                w.color,
                                stroke,
                                w.label.clone(),
                                highlighted,
                                w.y_scale,
                                w.y_offset,
                            ));
                        }
                    } else if let Some(series) = view_cache.and_then(|c| c.per_wave.get(&i)) {
                        match series {
                            WaveViewportSeries::Envelope(cols) if !cols.is_empty() => {
                                plot_ui.add(ScopeEnvelopePlotItem::new(
                                    Arc::clone(cols),
                                    w.color,
                                    stroke,
                                    w.label.clone(),
                                    highlighted,
                                    w.y_scale,
                                    w.y_offset,
                                ));
                            }
                            WaveViewportSeries::Uniform(pts) if pts.len() >= 2 => {
                                plot_ui.add(ScopeVectorPlotItem::new(
                                    Arc::clone(pts),
                                    w.color,
                                    stroke,
                                    w.label.clone(),
                                    highlighted,
                                    w.y_scale,
                                    w.y_offset,
                                ));
                            }
                            _ => {
                                if !w.overview.is_empty() {
                                    plot_ui.add(ScopeEnvelopePlotItem::new(
                                        Arc::clone(&w.overview),
                                        w.color,
                                        stroke,
                                        w.label.clone(),
                                        highlighted,
                                        w.y_scale,
                                        w.y_offset,
                                    ));
                                }
                            }
                        }
                    } else if !w.overview.is_empty() {
                        plot_ui.add(ScopeEnvelopePlotItem::new(
                            Arc::clone(&w.overview),
                            w.color,
                            stroke,
                            w.label.clone(),
                            highlighted,
                            w.y_scale,
                            w.y_offset,
                        ));
                    }
                }

                if let Some(v) = x1 {
                    plot_ui.vline(
                        VLine::new(v)
                            .color(Color32::from_rgb(0xF5, 0xA6, 0x23))
                            .width(1.5_f32)
                            .name("X1"),
                    );
                }
                if let Some(v) = x2 {
                    plot_ui.vline(
                        VLine::new(v)
                            .color(Color32::from_rgb(0x2E, 0xC4, 0xB6))
                            .width(1.5_f32)
                            .name("X2"),
                    );
                }
                if let Some(v) = y1 {
                    plot_ui.hline(
                        HLine::new(v)
                            .color(Color32::from_rgb(0xF5, 0xA6, 0x23))
                            .width(1.5_f32)
                            .name("Y1"),
                    );
                }
                if let Some(v) = y2 {
                    plot_ui.hline(
                        HLine::new(v)
                            .color(Color32::from_rgb(0x2E, 0xC4, 0xB6))
                            .width(1.5_f32)
                            .name("Y2"),
                    );
                }

                let label_y = {
                    let pad = y_span * 0.02;
                    if let Some((off, sc, ymin, ymax)) = bus_overlay_y {
                        let top = off + sc * ymax;
                        let bot = off + sc * ymin;
                        let ch_span = (top - bot).abs().max(y_span * 0.02);
                        (top + ch_span * 0.18).clamp(
                            bounds.min()[1] + pad,
                            bounds.max()[1] - pad,
                        )
                    } else {
                        bounds.max()[1] - y_span * 0.06
                    }
                };
                let min_label_dt = x_span * (64.0 / plot_width_px as f64);
                let x_lo = bounds.min()[0];
                let x_hi = bounds.max()[0];
                let is_ctrl_event = |summary: &str| {
                    matches!(summary, "START" | "Sr" | "STOP" | "CS" | "CS#")
                };
                for (_i, t0, t1, selected, summary) in &bus_markers {
                    if *t1 < x_lo || *t0 > x_hi {
                        continue;
                    }
                    let color = if *selected {
                        Color32::from_rgb(0xFF, 0x6B, 0x6B)
                    } else if summary == "START" || summary == "Sr" || summary == "CS" {
                        Color32::from_rgb(0x22, 0xC5, 0x5E)
                    } else if summary == "STOP" || summary == "CS#" || summary == "BREAK" {
                        Color32::from_rgb(0xF9, 0x73, 0x16)
                    } else {
                        Color32::from_rgb(0xA7, 0x8B, 0xFA)
                    };
                    plot_ui.vline(
                        VLine::new(*t0)
                            .color(color)
                            .width(if *selected { 3.25_f32 } else { 2.15_f32 })
                            .name(""),
                    );
                }
                // Data first so BREAK / control events cannot crowd 0xNN labels off the plot.
                let mut shown_t: Vec<f64> = Vec::new();
                let far_enough = |t: f64, shown: &[f64]| {
                    shown.iter().all(|u| (t - *u).abs() >= min_label_dt)
                };
                let paint_label = |plot_ui: &mut egui_plot::PlotUi,
                                       t_label: f64,
                                       summary: &str,
                                       selected: bool,
                                       color: Color32| {
                    plot_ui.add(
                        PlotTextLabel::new(t_label, label_y, summary, color, 15.0)
                            .highlight(selected),
                    );
                };
                let marker_color = |selected: bool, summary: &str| {
                    if selected {
                        Color32::from_rgb(0xFF, 0x6B, 0x6B)
                    } else if summary == "START" || summary == "Sr" || summary == "CS" {
                        Color32::from_rgb(0x22, 0xC5, 0x5E)
                    } else if summary == "STOP" || summary == "CS#" || summary == "BREAK" {
                        Color32::from_rgb(0xF9, 0x73, 0x16)
                    } else {
                        Color32::from_rgb(0xA7, 0x8B, 0xFA)
                    }
                };
                for (_i, t0, t1, selected, summary) in &bus_markers {
                    if *t1 < x_lo || *t0 > x_hi {
                        continue;
                    }
                    if is_ctrl_event(summary) || summary == "BREAK" {
                        continue;
                    }
                    let t_label = (*t0 + *t1) * 0.5;
                    if *selected || far_enough(t_label, &shown_t) {
                        paint_label(plot_ui, t_label, summary, *selected, marker_color(*selected, summary));
                        shown_t.push(t_label);
                    }
                }
                for (_i, t0, t1, selected, summary) in &bus_markers {
                    if *t1 < x_lo || *t0 > x_hi {
                        continue;
                    }
                    if !is_ctrl_event(summary) {
                        continue;
                    }
                    paint_label(plot_ui, *t0, summary, *selected, marker_color(*selected, summary));
                    shown_t.push(*t0);
                }
                for (_i, t0, t1, selected, summary) in &bus_markers {
                    if *t1 < x_lo || *t0 > x_hi || summary != "BREAK" {
                        continue;
                    }
                    if *selected || far_enough(*t0, &shown_t) {
                        paint_label(plot_ui, *t0, summary, *selected, marker_color(*selected, summary));
                        shown_t.push(*t0);
                    }
                }

                let resp = plot_ui.response().clone();
                let primary_down = plot_ui.ctx().input(|i| i.pointer.primary_down());
                if let Some(ptr) = plot_ui.pointer_coordinate() {
                    plot_ui.points(
                        Points::new(PlotPoints::from(vec![[ptr.x, ptr.y]]))
                            .radius(0.01_f32)
                            .color(Color32::TRANSPARENT),
                    );

                    let pick_bus = || {
                        if (ptr.y - label_y).abs() > y_span * 0.14 {
                            return None;
                        }
                        let mut best: Option<(f64, usize)> = None;
                        let hit = grab_x * 4.0;
                        for (i, t0, t1, _, _) in &bus_markers {
                            let lo = t0.min(*t1);
                            let hi = t0.max(*t1);
                            let dist = if ptr.x >= lo && ptr.x <= hi {
                                0.0
                            } else {
                                (ptr.x - *t0).abs().min((ptr.x - *t1).abs())
                            };
                            if dist <= hit && best.map(|(d, _)| dist < d).unwrap_or(true) {
                                best = Some((dist, *i));
                            }
                        }
                        best.map(|(_, i)| i)
                    };

                    if mode == InteractMode::Cursor {
                        if resp.drag_started()
                            || (primary_down && dragging.is_none() && resp.hovered())
                        {
                            match axis {
                                CursorAxis::X => {
                                    let near1 = x1
                                        .map(|a| (ptr.x - a).abs() <= grab_x)
                                        .unwrap_or(false);
                                    let near2 = x2
                                        .map(|b| (ptr.x - b).abs() <= grab_x)
                                        .unwrap_or(false);
                                    if near1 {
                                        drag_cursor = Some((1, ptr.x));
                                    } else if near2 {
                                        drag_cursor = Some((2, ptr.x));
                                    }
                                }
                                CursorAxis::Y => {
                                    let near1 = y1
                                        .map(|a| (ptr.y - a).abs() <= grab_y)
                                        .unwrap_or(false);
                                    let near2 = y2
                                        .map(|b| (ptr.y - b).abs() <= grab_y)
                                        .unwrap_or(false);
                                    if near1 {
                                        drag_cursor = Some((1, ptr.y));
                                    } else if near2 {
                                        drag_cursor = Some((2, ptr.y));
                                    }
                                }
                            }
                        }
                        if primary_down {
                            if let Some(which) = dragging {
                                let v = match axis {
                                    CursorAxis::X => ptr.x,
                                    CursorAxis::Y => ptr.y,
                                };
                                drag_cursor = Some((which, v));
                            }
                        } else if dragging.is_some() {
                            end_drag = true;
                        }
                        if resp.clicked() && dragging.is_none() && drag_cursor.is_none() {
                            if let Some(i) = pick_bus() {
                                click_bus_frame = Some(i);
                            } else {
                                click_val = Some(match axis {
                                    CursorAxis::X => ptr.x,
                                    CursorAxis::Y => ptr.y,
                                });
                            }
                        }
                    } else if resp.clicked() {
                        if let Some(i) = pick_bus() {
                            click_bus_frame = Some(i);
                        }
                    }
                } else if dragging.is_some() && !primary_down {
                    end_drag = true;
                }

                // Clamp after user zoom/pan — skip on Fit frames so we don't overwrite
                // the queued Set(bounds) with a clamp of the *previous* viewport.
                if applied.is_none() {
                    let after = plot_ui.plot_bounds();
                    let clamped = self.clamp_view_bounds(after);
                    if clamped.min() != after.min() || clamped.max() != after.max() {
                        plot_ui.set_plot_bounds(clamped);
                        needs_clamp = true;
                    }
                }
            })
                    })
                .inner,
            );

            if nch > 0 {
                if let Some((y0, y1)) = next_y_range.or(y_for_axes) {
                    y_for_axes = Some((y0, y1));
                    ui.allocate_ui_with_layout(
                        egui::vec2(strip_w, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.channel_offset_axes(ui, tokens, lang, y0, y1),
                    );
                }
            }
        });

        let response = plot_response.expect("waveform plot");

        if fit {
            self.fit_request = false;
        }
        if let Some(r) = next_x_range {
            self.last_x_range = Some(r);
        }
        if let Some(r) = next_y_range {
            self.last_y_range = Some(r);
        }
        let _ = needs_clamp;

        // Apply Ctrl+Shift+wheel Y zoom after ranges are captured for this frame.
        if y_wheel != 0.0 && response.response.hovered() {
            let zoom_speed = ui.ctx().options(|o| o.scroll_zoom_speed);
            // Same sign as egui Ctrl+wheel → X zoom: positive wheel delta zooms in.
            // (Previously this path fed a range-multiplier into apply_y_zoom_factor,
            // so selected-channel scale moved opposite to the Y+/Y− buttons.)
            let zoom = (zoom_speed * y_wheel).exp() as f64;
            self.apply_y_zoom_factor(zoom);
        }

        if let Some((which, v)) = drag_cursor {
            self.dragging_cursor = Some(which);
            match (self.cursor_axis, which) {
                (CursorAxis::X, 1) => self.x1 = Some(v),
                (CursorAxis::X, 2) => self.x2 = Some(v),
                (CursorAxis::Y, 1) => self.y1 = Some(v),
                (CursorAxis::Y, 2) => self.y2 = Some(v),
                _ => {}
            }
            if self.cursor_axis == CursorAxis::X {
                self.bus_decode_dirty = true;
            }
        }
        if end_drag {
            self.dragging_cursor = None;
        }
        if let Some(v) = click_val {
            match self.cursor_axis {
                CursorAxis::X => match (self.x1, self.x2) {
                    (None, _) => self.x1 = Some(v),
                    (Some(_), None) => self.x2 = Some(v),
                    (Some(_), Some(_)) => {
                        self.x1 = Some(v);
                        self.x2 = None;
                    }
                },
                CursorAxis::Y => match (self.y1, self.y2) {
                    (None, _) => self.y1 = Some(v),
                    (Some(_), None) => self.y2 = Some(v),
                    (Some(_), Some(_)) => {
                        self.y1 = Some(v);
                        self.y2 = None;
                    }
                },
            }
            if self.cursor_axis == CursorAxis::X {
                self.bus_decode_dirty = true;
            }
        }
        if let Some(i) = click_bus_frame {
            self.selected_bus_frame = Some(i);
            if let Some(frame) = self
                .bus_result
                .as_ref()
                .and_then(|r| r.frames.get(i))
                .cloned()
            {
                self.zoom_to_bus_frame(&frame);
            }
        }

        // On-plot measurement overlay (X1/X2 or Y1/Y2 + delta).
        let overlay = match self.cursor_axis {
            CursorAxis::X => match (self.x1, self.x2) {
                (Some(a), Some(b)) => {
                    let dx = b - a;
                    let freq = if dx.abs() > f64::EPSILON {
                        format!("   f={}", format_freq(1.0 / dx.abs()))
                    } else {
                        String::new()
                    };
                    Some(format!(
                        "X1={}   X2={}   ΔX={}{freq}",
                        format_eng(a, &xu),
                        format_eng(b, &xu),
                        format_eng(dx, &xu),
                    ))
                }
                (Some(a), None) => Some(format!("X1={}   X2=—", format_eng(a, &xu))),
                _ => None,
            },
            CursorAxis::Y => {
                let sel = self.y_measure_wave_index();
                match (sel, self.y1, self.y2) {
                    (Some(i), Some(y1p), Some(y2p)) if self.waves.get(i).is_some() => {
                        let w = &self.waves[i];
                        let yu = w.trace.y_unit.as_str();
                        let y1 = plot_y_to_native(y1p, w.y_scale, w.y_offset);
                        let y2 = plot_y_to_native(y2p, w.y_scale, w.y_offset);
                        Some(format!(
                            "Y1={}   Y2={}   ΔY={} ({})",
                            format_eng(y1, yu),
                            format_eng(y2, yu),
                            format_eng(y2 - y1, yu),
                            short_channel_label(&w.trace.channel),
                        ))
                    }
                    (Some(i), Some(y1p), None) => {
                        if let Some(w) = self.waves.get(i) {
                            let y1 = plot_y_to_native(y1p, w.y_scale, w.y_offset);
                            Some(format!(
                                "Y1={}   Y2=— ({})",
                                format_eng(y1, w.trace.y_unit.as_str()),
                                short_channel_label(&w.trace.channel),
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
        };
        if let Some(text) = overlay {
            let rect = response.response.rect;
            let pos = egui::pos2(rect.left() + 10.0, rect.top() + 8.0);
            ui.painter().text(
                pos,
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(13.0),
                tokens.text_primary,
            );
        }
        let _ = lang;
    }

    fn open_files(&mut self, lang: Lang) {
        let paths = rfd::FileDialog::new()
            .set_directory(&self.open_dir)
            .add_filter(
                t(lang, "波形源文件", "Waveform sources"),
                &["csv", "isf", "wfm", "txt"],
            )
            .add_filter("CSV (Tek/Rigol/WiParse)", &["csv", "txt"])
            .add_filter("Tektronix ISF", &["isf"])
            .add_filter("Tektronix WFM", &["wfm"])
            .pick_files();
        let Some(paths) = paths else {
            return;
        };
        // Multi-open from dialog may overlay; browser clicks use single-document switch.
        for path in paths {
            if let Some(parent) = path.parent() {
                self.open_dir = parent.to_path_buf();
            }
            if self
                .waves
                .iter()
                .any(|w| same_wave_path(&w.path, &path))
            {
                continue;
            }
            self.start_load_path(path, lang);
        }
    }

    /// Load a path programmatically (e.g. deep-link / automation).
    pub fn open_path(&mut self, path: &Path, lang: Lang) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            self.open_dir = parent.to_path_buf();
        }
        if let Some(i) = self
            .waves
            .iter()
            .position(|w| same_wave_path(&w.path, path))
        {
            self.selected = Some(i);
            self.status = format!("{} {}", t(lang, "已打开", "Opened"), path.display());
            return Ok(());
        }
        match self.load_path_sync(path) {
            Ok(n) => {
                self.status = format!(
                    "{} {} · {} pts/ch",
                    t(lang, "已加载", "Loaded"),
                    path.display(),
                    n
                );
                Ok(())
            }
            Err(err) => {
                self.status = format!(
                    "{} {}: {err}",
                    t(lang, "加载失败", "Load failed"),
                    path.display()
                );
                Err(err)
            }
        }
    }

    pub fn api_snapshot(&self) -> serde_json::Value {
        let files: Vec<_> = self
            .waves
            .iter()
            .map(|w| {
                serde_json::json!({
                    "path": w.path.display().to_string(),
                    "channel": w.trace.channel,
                })
            })
            .collect();
        serde_json::json!({
            "status": self.status,
            "selected": self.selected,
            "files": files,
            "browser_dir": self.browser_dir,
            "bus": self.bus_settings.kind.label(),
            "cursors": {
                "x1": self.x1, "x2": self.x2, "y1": self.y1, "y2": self.y2
            }
        })
    }

    pub fn api_open(&mut self, lang: Lang, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
            return err("ui.wave.open", "missing path");
        };
        match self.open_path(Path::new(path), lang) {
            Ok(()) => ok("ui.wave.open", self.api_snapshot()),
            Err(e) => err("ui.wave.open", &e),
        }
    }

    pub fn api_close(&mut self, lang: Lang) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        self.waves.clear();
        self.selected = None;
        self.next_color = 0;
        self.view_line_cache = None;
        self.gated_measure_cache = None;
        self.bus_result = None;
        self.status = t(lang, "已关闭", "Closed").into();
        ok("ui.wave.close", self.api_snapshot())
    }

    pub fn api_cursors(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        if let Some(v) = params.get("x1").and_then(|x| x.as_f64()) {
            self.x1 = Some(v);
        }
        if let Some(v) = params.get("x2").and_then(|x| x.as_f64()) {
            self.x2 = Some(v);
        }
        if let Some(v) = params.get("y1").and_then(|x| x.as_f64()) {
            self.y1 = Some(v);
        }
        if let Some(v) = params.get("y2").and_then(|x| x.as_f64()) {
            self.y2 = Some(v);
        }
        if params.get("clear").and_then(|x| x.as_bool()) == Some(true) {
            self.x1 = None;
            self.x2 = None;
            self.y1 = None;
            self.y2 = None;
        }
        ok("ui.wave.cursor", self.api_snapshot())
    }

    pub fn api_fit(&mut self) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        self.fit_request = true;
        ok("ui.wave.fit", serde_json::json!({ "fit": true }))
    }

    pub fn api_bus(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        if let Some(kind) = params.get("kind").and_then(|v| v.as_str()) {
            self.bus_settings.kind = match kind.trim().to_ascii_lowercase().as_str() {
                "off" | "none" => BusKind::Off,
                "uart" => BusKind::Uart,
                "i2c" => BusKind::I2c,
                "spi" => BusKind::Spi,
                "i2s" => BusKind::I2s,
                _ => return err("ui.wave.bus", "kind must be off|uart|i2c|spi|i2s"),
            };
        }
        let ch = &mut self.bus_settings.channels;
        fn idx(v: &serde_json::Value, key: &str) -> Option<usize> {
            v.get(key).and_then(|x| x.as_u64()).map(|n| n as usize)
        }
        if let Some(n) = idx(params, "uart") {
            ch.uart_signal = Some(n);
        }
        if let Some(n) = idx(params, "scl") {
            ch.i2c_scl = Some(n);
        }
        if let Some(n) = idx(params, "sda") {
            ch.i2c_sda = Some(n);
        }
        if let Some(n) = idx(params, "clk") {
            ch.spi_clk = Some(n);
        }
        if let Some(n) = idx(params, "mosi") {
            ch.spi_mosi = Some(n);
        }
        if let Some(n) = idx(params, "miso") {
            ch.spi_miso = Some(n);
        }
        if let Some(n) = idx(params, "cs") {
            ch.spi_cs = Some(n);
        }
        if let Some(n) = idx(params, "bclk") {
            ch.i2s_bclk = Some(n);
        }
        if let Some(n) = idx(params, "ws") {
            ch.i2s_ws = Some(n);
        }
        if let Some(n) = idx(params, "data") {
            ch.i2s_data = Some(n);
        }
        if let Some(v) = params.get("threshold").and_then(|x| x.as_f64()) {
            self.bus_settings.threshold = Some(v);
        }
        if let Some(v) = params.get("baud").and_then(|x| x.as_f64()) {
            self.bus_settings.uart.baud = Some(v);
            self.uart_baud_text = format!("{v}");
        }
        self.bus_decode_dirty = true;
        ok("ui.wave.bus", self.api_snapshot())
    }

    pub fn api_select(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(index) = params
            .get("index")
            .or_else(|| params.get("selected"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
        else {
            return err("ui.wave.select", "missing index");
        };
        if index >= self.waves.len() {
            return err("ui.wave.select", "index out of range");
        }
        self.activate_wave(index);
        ok("ui.wave.select", self.api_snapshot())
    }

    pub fn api_set_browser_dir(
        &mut self,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let Some(dir) = params.get("dir").and_then(|v| v.as_str()) else {
            return err("ui.wave.browser", "missing dir");
        };
        self.browser_dir = dir.to_string();
        self.persist_browser_dir();
        self.refresh_wave_browser();
        ok("ui.wave.browser", self.api_snapshot())
    }

    fn export_selected(&mut self, lang: Lang) {
        let Some(i) = self.selected else {
            return;
        };
        let Some(w) = self.waves.get(i) else {
            return;
        };
        let default = self.open_dir.join(format!(
            "waveform_{}.isf",
            w.trace.channel.replace(['/', '\\', ':'], "_")
        ));
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(&self.open_dir)
            .set_file_name(
                default
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            )
            .add_filter("Tektronix ISF", &["isf"])
            .add_filter("Tektronix WFM", &["wfm"])
            .add_filter("CSV", &["csv"])
            .save_file()
        {
            match save_waveform_file(&path, None, None, Some(&w.trace)) {
                Ok(()) => {
                    self.status = format!("{} {}", t(lang, "已导出", "Exported"), path.display());
                }
                Err(err) => self.status = err.to_string(),
            }
        }
    }
}

struct ViewportBuildJob {
    index: usize,
    x: Arc<[f64]>,
    y: Arc<[f64]>,
    x0: f64,
    x1: f64,
    plot_width_px: f32,
    prev_uniform: bool,
    x_monotonic: bool,
}

fn load_waveform_file_worker(path: &Path, tx: Sender<WaveLoadEvent>) {
    let result = (|| -> Result<(), String> {
        let traces = load_waveform_file_all(path).map_err(|e| e.to_string())?;
        if traces.is_empty() {
            return Err("empty waveform".into());
        }
        let max_pts = traces.iter().map(|t| t.x.len()).max().unwrap_or(0);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("wave");

        let channels: Vec<LoadedChannelDraft> = traces
            .par_iter()
            .map(|trace| {
                let (overview, snap) = build_load_snapshot(&trace.x, &trace.y);
                LoadedChannelDraft {
                    label: format!("{file_name} · {}", trace.channel),
                    trace: trace.clone(),
                    overview,
                    x_min: snap.x_min,
                    x_max: snap.x_max,
                    y_min: snap.y_min,
                    y_max: snap.y_max,
                    measures: snap.to_quick_measures(),
                    x_monotonic: snap.x_monotonic,
                }
            })
            .collect();

        tx.send(WaveLoadEvent::Preview { channels, max_pts })
            .map_err(|e| e.to_string())?;

        let (measures, extents): (Vec<_>, Vec<_>) = traces
            .par_iter()
            .map(|t| (measure_waveform(t), trace_extent(t)))
            .unzip();

        tx.send(WaveLoadEvent::Final { measures, extents })
            .map_err(|e| e.to_string())?;
        Ok(())
    })();

    if let Err(err) = result {
        let _ = tx.send(WaveLoadEvent::Error(err));
    }
}

/// Tektronix default channel colors (2/3/4/5/6 Series FAQ):
/// CH1 Yellow, CH2 Cyan, CH3 Red, CH4 Green, CH5 Orange, CH6 Blue, CH7 Magenta, CH8 Mint.
const TEK_CHANNEL_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xF7, 0xD6, 0x18), // CH1 Yellow
    Color32::from_rgb(0x00, 0xD4, 0xFF), // CH2 Cyan
    Color32::from_rgb(0xFF, 0x3B, 0x30), // CH3 Red
    Color32::from_rgb(0x2E, 0xD1, 0x58), // CH4 Green
    Color32::from_rgb(0xFF, 0x9F, 0x0A), // CH5 Orange
    Color32::from_rgb(0x3B, 0x82, 0xF6), // CH6 Blue
    Color32::from_rgb(0xE0, 0x40, 0xFB), // CH7 Magenta
    Color32::from_rgb(0x6E, 0xF7, 0xC8), // CH8 Mint
];

/// Fallback palette when the channel name is not CH1…CH8.
const TRACE_COLORS: [Color32; 6] = [
    TEK_CHANNEL_COLORS[0],
    TEK_CHANNEL_COLORS[1],
    TEK_CHANNEL_COLORS[2],
    TEK_CHANNEL_COLORS[3],
    TEK_CHANNEL_COLORS[4],
    TEK_CHANNEL_COLORS[5],
];

/// Public helper for Tek-style CH1…CH8 colors (shared with instrument plots).
pub fn tek_channel_color(channel: &str) -> Option<Color32> {
    color_for_channel(channel)
}

/// Short label for per-channel offset axis (e.g. CH1).
fn short_channel_label(channel: &str) -> String {
    let s = channel.trim();
    if s.len() <= 4 {
        return s.to_ascii_uppercase();
    }
    let upper = s.to_ascii_uppercase();
    if let Some(rest) = upper.strip_prefix("CHANNEL") {
        return format!("CH{}", rest.trim());
    }
    if upper.starts_with("CH") && upper.len() <= 4 {
        return upper;
    }
    if s.len() > 6 {
        format!("{}…", &s[..5])
    } else {
        s.to_string()
    }
}

/// Map `CH1`…`CH8` (also `C1`, `Channel 1`) to Tek default colors.
fn color_for_channel(channel: &str) -> Option<Color32> {
    let s = channel.trim();
    let upper = s.to_ascii_uppercase();
    let num = if let Some(rest) = upper.strip_prefix("CHANNEL") {
        rest.trim().parse::<usize>().ok()
    } else if let Some(rest) = upper.strip_prefix("CH") {
        rest.trim().parse::<usize>().ok()
    } else if let Some(rest) = upper.strip_prefix('C') {
        // Avoid matching bare letters; require a digit start.
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            rest.parse::<usize>().ok()
        } else {
            None
        }
    } else {
        upper.parse::<usize>().ok()
    }?;
    if (1..=TEK_CHANNEL_COLORS.len()).contains(&num) {
        Some(TEK_CHANNEL_COLORS[num - 1])
    } else {
        None
    }
}

fn is_waveform_source_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "csv" | "isf" | "wfm" | "txt"
    )
}

fn same_wave_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(aa), Ok(bb)) => aa == bb,
        _ => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

/// Read Ctrl+Shift+wheel scroll for Y zoom.
///
/// egui converts Shift+wheel into horizontal scroll (`delta.y = 0`) before building
/// `zoom_delta`, so plot Y-zoom via `allow_zoom(y)` never sees a non-1 factor.
/// We read the original `Event::MouseWheel` (pre-remap) and also accept horizontal
/// deltas that some Windows drivers already emit when Shift is held.
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
            // Prefer vertical notch; fall back to horizontal (Shift remap / OS).
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

fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Plot-Y of channel ground (native 0 V after scale/offset).
///
/// Display mapping is `y_plot = y_raw * y_scale + y_offset`, so ground is `y_offset`.
/// Scale zoom must not move this value — the marker stays locked to 0 V.
fn wave_display_y_zero(w: &LoadedWave) -> f64 {
    w.y_offset
}

/// Screen Y of a channel ground marker, plus whether ground is inside the current Y view.
fn channel_gnd_screen_y(y_zero_plot: f64, y_lo: f64, y_span: f64, rect: egui::Rect) -> (f32, bool) {
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

fn paint_channel_gnd_handle(
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

/// Display-space Y bounds for a loaded wave (scale + offset applied).
fn wave_display_y_bounds(w: &LoadedWave) -> (f64, f64) {
    let y0 = w.y_min * w.y_scale + w.y_offset;
    let y1 = w.y_max * w.y_scale + w.y_offset;
    (y0.min(y1), y0.max(y1))
}

/// Stack multi-channel traces vertically on first load (avoids overlap).
fn auto_stagger_channel_offsets(waves: &mut [LoadedWave], range: std::ops::Range<usize>) {
    if range.end <= range.start + 1 {
        return;
    }
    let avg_pp: f64 = waves[range.clone()]
        .iter()
        .map(|w| (w.y_max - w.y_min).abs())
        .sum::<f64>()
        / (range.end - range.start) as f64;
    let gap = (avg_pp * 0.12).max(1e-12);
    let start = range.start;
    let mut top =
        waves[start].y_max * waves[start].y_scale + waves[start].y_offset;
    for i in range.start + 1..range.end {
        let y_min = waves[i].y_min;
        let scale = waves[i].y_scale;
        waves[i].y_offset = top + gap - y_min * scale;
        top = waves[i].y_max * scale + waves[i].y_offset;
    }
}

/// Effective native units per vertical division for the selected channel.
fn format_channel_div(w: &LoadedWave, plot_y_span: f64) -> String {
    let native_span = plot_y_span / w.y_scale.abs().max(1e-30);
    let per_div = native_span / 10.0;
    format!("{}/div", format_eng(per_div, w.trace.y_unit.as_str()))
}

/// Convert plot Y coordinate to native channel value (0 = that channel's GND).
fn plot_y_to_native(y_plot: f64, y_scale: f64, y_offset: f64) -> f64 {
    if y_scale.abs() < f64::EPSILON {
        y_plot
    } else {
        (y_plot - y_offset) / y_scale
    }
}

/// Native Y for a plot coordinate: match series name, else the preferred channel.
fn native_y_for_series(
    y_plot: f64,
    series_name: &str,
    maps: &[(String, f64, f64, String)],
    prefer: Option<usize>,
) -> (f64, String) {
    if !series_name.is_empty() {
        if let Some((_, sc, off, unit)) = maps.iter().find(|(label, ..)| label == series_name) {
            return (plot_y_to_native(y_plot, *sc, *off), unit.clone());
        }
    }
    let i = prefer.filter(|i| *i < maps.len()).unwrap_or(0);
    if let Some((_, sc, off, unit)) = maps.get(i) {
        (plot_y_to_native(y_plot, *sc, *off), unit.clone())
    } else {
        (y_plot, "V".into())
    }
}

/// Compact Y-axis tick label for the narrow strip (unit on top tick only).
fn format_axis_tick(value: f64, unit: &str, with_unit: bool) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if with_unit {
        return format_eng(value, unit);
    }
    let full = format_eng(value, unit);
    full.split_whitespace().next().unwrap_or("—").to_string()
}

/// Adaptive frequency label (Hz / kHz / MHz / GHz via engineering prefix).
fn format_freq(hz: f64) -> String {
    if !hz.is_finite() || hz <= 0.0 {
        return "—".into();
    }
    format_eng(hz, "Hz")
}

fn format_scale(scale: f64) -> String {
    if !scale.is_finite() {
        return "×—".into();
    }
    if (scale - 1.0).abs() < 1e-6 {
        "×1".into()
    } else if scale >= 10.0 {
        format!("×{scale:.1}")
    } else if scale >= 1.0 {
        format!("×{scale:.2}")
    } else {
        format!("×{scale:.3}")
    }
}

/// Engineering notation for scope values (ns / µs / ms / s, mV / V, …).
fn format_eng(value: f64, unit: &str) -> String {
    if !value.is_finite() {
        return format!("— {unit}");
    }
    let a = value.abs();
    let (scaled, prefix) = if a == 0.0 {
        (0.0, "")
    } else if a >= 1e9 {
        (value / 1e9, "G")
    } else if a >= 1e6 {
        (value / 1e6, "M")
    } else if a >= 1e3 {
        (value / 1e3, "k")
    } else if a >= 1.0 {
        (value, "")
    } else if a >= 1e-3 {
        (value * 1e3, "m")
    } else if a >= 1e-6 {
        (value * 1e6, "µ")
    } else if a >= 1e-9 {
        (value * 1e9, "n")
    } else if a >= 1e-12 {
        (value * 1e12, "p")
    } else {
        (value, "")
    };
    if prefix.is_empty() && a >= 1.0 {
        format!("{scaled:.4} {unit}")
    } else if prefix.is_empty() {
        format!("{value:.6} {unit}")
    } else {
        format!("{scaled:.4} {prefix}{unit}")
    }
}

fn interpolate_y(trace: &WaveformTrace, x: f64) -> Option<f64> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some(trace.y[0]);
    }
    let xs = &trace.x[..n];
    if x <= xs[0] {
        return Some(trace.y[0]);
    }
    if x >= xs[n - 1] {
        return Some(trace.y[n - 1]);
    }
    let i = xs.partition_point(|&t| t < x).max(1).min(n - 1);
    let x0 = xs[i - 1];
    let x1 = xs[i];
    let y0 = trace.y[i - 1];
    let y1 = trace.y[i];
    let t = if (x1 - x0).abs() < f64::EPSILON {
        0.0
    } else {
        (x - x0) / (x1 - x0)
    };
    Some(y0 + t * (y1 - y0))
}

fn measure_row(ui: &mut egui::Ui, tokens: &Tokens, key: &str, value: &str) {
    let row_w = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_width(row_w);
        ui.set_max_width(row_w);
        ui.label(
            RichText::new(key)
                .small()
                .monospace()
                .color(tokens.text_muted),
        );
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).small().monospace());
        });
    });
}

fn extent_of_traces(waves: &[&LoadedWave]) -> Option<PlotBounds> {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    let mut any = false;
    for w in waves {
        if w.trace.x.is_empty() || w.trace.y.is_empty() {
            continue;
        }
        any = true;
        xmin = xmin.min(w.x_min);
        xmax = xmax.max(w.x_max);
        let (dy0, dy1) = wave_display_y_bounds(w);
        ymin = ymin.min(dy0);
        ymax = ymax.max(dy1);
    }
    if !any || !xmin.is_finite() {
        return None;
    }
    if (xmax - xmin).abs() < VIEW_MIN_SPAN_ABS {
        xmax = xmin + VIEW_MIN_SPAN_ABS;
    }
    if (ymax - ymin).abs() < VIEW_MIN_SPAN_ABS {
        ymax = ymin + VIEW_MIN_SPAN_ABS;
    }
    let xpad = ((xmax - xmin).abs() * 0.02).max(VIEW_MIN_SPAN_ABS);
    let ypad = ((ymax - ymin).abs() * 0.08).max(VIEW_MIN_SPAN_ABS);
    Some(PlotBounds::from_min_max(
        [xmin - xpad, ymin - ypad],
        [xmax + xpad, ymax + ypad],
    ))
}

fn trace_extent(trace: &WaveformTrace) -> (f64, f64, f64, f64) {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return (0.0, 1.0, 0.0, 1.0);
    }
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    for i in 0..n {
        let x = trace.x[i];
        if x.is_finite() {
            xmin = xmin.min(x);
            xmax = xmax.max(x);
        }
    }
    if !xmin.is_finite() {
        xmin = 0.0;
        xmax = 1.0;
    }
    let (mut ymin, mut ymax) = robust_y_range(&trace.y[..n]);
    if (xmax - xmin).abs() < VIEW_MIN_SPAN_ABS {
        xmax = xmin + VIEW_MIN_SPAN_ABS;
    }
    if (ymax - ymin).abs() < VIEW_MIN_SPAN_ABS {
        ymax = ymin + VIEW_MIN_SPAN_ABS;
    }
    (xmin, xmax, ymin, ymax)
}

/// Robust Y extent for autoscale — ignores NaN/Inf and extreme outliers (0.1–99.9%).
fn robust_y_range(y: &[f64]) -> (f64, f64) {
    const SAMPLE: usize = 65_536;
    let n = y.len();
    if n == 0 {
        return (0.0, 1.0);
    }
    let mut vals: Vec<f64> = if n <= SAMPLE {
        y.iter().copied().filter(|v| v.is_finite()).collect()
    } else {
        let step = n as f64 / SAMPLE as f64;
        let mut f = 0.0;
        let mut out = Vec::with_capacity(SAMPLE);
        while out.len() < SAMPLE {
            let i = (f as usize).min(n - 1);
            let v = y[i];
            if v.is_finite() {
                out.push(v);
            }
            f += step;
        }
        out
    };
    if vals.is_empty() {
        return (0.0, 1.0);
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo_i = ((vals.len() as f64) * 0.001).floor() as usize;
    let hi_i = ((vals.len() as f64) * 0.999).ceil() as usize;
    let lo_i = lo_i.min(vals.len() - 1);
    let hi_i = hi_i.saturating_sub(1).max(lo_i);
    let mut lo = vals[lo_i];
    let mut hi = vals[hi_i];
    if (hi - lo).abs() < VIEW_MIN_SPAN_ABS {
        lo -= VIEW_MIN_SPAN_ABS * 0.5;
        hi += VIEW_MIN_SPAN_ABS * 0.5;
    } else {
        let pad = (hi - lo).abs() * 0.04;
        lo -= pad;
        hi += pad;
    }
    (lo, hi)
}

#[inline]
fn f64_pair_key(a: f64, b: f64) -> u64 {
    a.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.to_bits()
}

/// True when the plot window spans (almost) the entire trace — use overview envelope.
fn view_covers_overview(x0: f64, x1: f64, data_xmin: f64, data_xmax: f64) -> bool {
    let (lo, hi) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let data_span = (data_xmax - data_xmin).abs().max(VIEW_MIN_SPAN_ABS);
    let view_span = (hi - lo).abs();
    view_span >= data_span * 0.985
        && lo <= data_xmin + data_span * 0.02
        && hi >= data_xmax - data_span * 0.02
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
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.set_max_size(ui.available_size());
                ui.label(RichText::new(title).strong().size(13.0));
                ui.add_space(6.0);
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

fn bus_form_layout(ui: &egui::Ui) -> BusFormLayout {
    let w = ui.available_width().max(140.0);
    let label_w = 78.0_f32.min(w * 0.36).max(54.0);
    BusFormLayout {
        label_w,
        field_w: (w - label_w - 6.0).max(76.0),
        row_h: 24.0,
    }
}

#[derive(Clone, Copy)]
struct BusFormLayout {
    label_w: f32,
    field_w: f32,
    row_h: f32,
}

fn bus_grid_label(ui: &mut egui::Ui, layout: BusFormLayout, text: &str) {
    ui.add_sized(
        [layout.label_w, layout.row_h],
        egui::Label::new(RichText::new(text).small()),
    );
}

fn bus_grid_channel_combo(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    id: &str,
    label: &str,
    waves: &[LoadedWave],
    current: &mut Option<usize>,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    let selected = current
        .and_then(|i| waves.get(i))
        .map(|w| w.label.as_str())
        .unwrap_or("—");
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(layout.field_w)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "—").clicked() {
                *current = None;
                changed = true;
            }
            for (i, w) in waves.iter().enumerate() {
                if ui
                    .selectable_label(*current == Some(i), &w.label)
                    .clicked()
                {
                    *current = Some(i);
                    changed = true;
                }
            }
        });
    ui.end_row();
    changed
}

fn bus_grid_buttons(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut u8,
    options: &[(&str, u8)],
    btn_w: f32,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = *current;
        for (text, value) in options {
            if ui
                .add_sized(
                    [btn_w, layout.row_h],
                    egui::Button::new(*text).selected(cur == *value),
                )
                .clicked()
                && cur != *value
            {
                *current = *value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn bus_grid_parity(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut UartParity,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = *current;
        for (text, value) in [
            ("N", UartParity::None),
            ("E", UartParity::Even),
            ("O", UartParity::Odd),
        ] {
            if ui
                .add_sized(
                    [22.0, layout.row_h],
                    egui::Button::new(text).selected(cur == value),
                )
                .clicked()
                && cur != value
            {
                *current = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn bus_grid_spi_wire(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut SpiWire,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let cur = *current;
        for (text, value) in [
            ("2", SpiWire::TwoWire),
            ("4", SpiWire::FourWire),
            ("Dual", SpiWire::Dual),
            ("Quad", SpiWire::Quad),
        ] {
            let w = if text.len() > 1 { 40.0 } else { 24.0 };
            if ui
                .add_sized(
                    [w, layout.row_h],
                    egui::Button::new(text).selected(cur == value),
                )
                .clicked()
                && cur != value
            {
                *current = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn bus_grid_spi_mode(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut SpiMode,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = *current;
        for (text, value) in [
            ("0", SpiMode::Mode0),
            ("1", SpiMode::Mode1),
            ("2", SpiMode::Mode2),
            ("3", SpiMode::Mode3),
        ] {
            if ui
                .add_sized(
                    [22.0, layout.row_h],
                    egui::Button::new(text).selected(cur == value),
                )
                .clicked()
                && cur != value
            {
                *current = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn bus_grid_bool_pair(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut bool,
    (false_l, true_l): (&str, &str),
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = *current;
        for (text, value) in [(false_l, false), (true_l, true)] {
            let w = (text.len() as f32 * 8.0 + 12.0).clamp(28.0, 52.0);
            if ui
                .add_sized(
                    [w, layout.row_h],
                    egui::Button::new(text).selected(cur == value),
                )
                .clicked()
                && cur != value
            {
                *current = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn bus_grid_i2s_format(
    ui: &mut egui::Ui,
    layout: BusFormLayout,
    label: &str,
    current: &mut I2sFormat,
) -> bool {
    let mut changed = false;
    bus_grid_label(ui, layout, label);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cur = *current;
        for (text, value) in [("Philips", I2sFormat::Philips), ("Left-J", I2sFormat::LeftJustified)]
        {
            if ui
                .add_sized(
                    [52.0, layout.row_h],
                    egui::Button::new(text).selected(cur == value),
                )
                .clicked()
                && cur != value
            {
                *current = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

fn t(lang: Lang, zh: &str, en: &str) -> String {
    match lang {
        Lang::Zh => zh.to_string(),
        Lang::En => en.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_ground_stays_at_offset_when_scale_changes() {
        let y_offset = 1.25;
        for scale in [0.25_f64, 1.0, 4.0, 16.0] {
            let gnd = 0.0 * scale + y_offset;
            assert!((gnd - y_offset).abs() < 1e-15);
        }
        // Amplitude midpoint is not ground and walks when scale changes.
        let y_min = 0.2;
        let y_max = 1.0;
        let mid_1 = 0.5 * (y_min + y_max) * 1.0 + y_offset;
        let mid_4 = 0.5 * (y_min + y_max) * 4.0 + y_offset;
        assert!((mid_1 - y_offset).abs() > 0.1);
        assert!((mid_4 - mid_1).abs() > 0.5);
    }

    #[test]
    fn gnd_screen_y_matches_plot_zero_inside_view() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(28.0, 100.0));
        let y_lo = -2.0;
        let y_span = 4.0;
        let (hy, on) = channel_gnd_screen_y(0.0, y_lo, y_span, rect);
        assert!(on);
        assert!((hy - 50.0).abs() < 0.01);
        let (hy_pos, on_pos) = channel_gnd_screen_y(1.0, y_lo, y_span, rect);
        assert!(on_pos);
        assert!((hy_pos - 25.0).abs() < 0.01);
    }

    #[test]
    fn gnd_screen_y_pins_to_edge_when_off_view() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(28.0, 100.0));
        let (hy, on) = channel_gnd_screen_y(10.0, -1.0, 2.0, rect);
        assert!(!on);
        assert!((hy - 7.0).abs() < 0.01);
        let (hy2, on2) = channel_gnd_screen_y(-10.0, -1.0, 2.0, rect);
        assert!(!on2);
        assert!((hy2 - 93.0).abs() < 0.01);
    }

    #[test]
    fn y_cursor_zero_matches_channel_ground() {
        let y_offset = 3.5;
        let y_scale = 2.0;
        assert!((plot_y_to_native(y_offset, y_scale, y_offset)).abs() < 1e-12);
        assert!((plot_y_to_native(y_offset + 4.0, y_scale, y_offset) - 2.0).abs() < 1e-12);
        assert!((plot_y_to_native(y_offset - 1.0, y_scale, y_offset) + 0.5).abs() < 1e-12);
    }

    #[test]
    fn y_readout_uses_hovered_channel_map_not_plot_axis() {
        let maps = vec![
            ("CH1".into(), 1.0, 0.0, "V".into()),
            ("CH2".into(), 1.0, 5.0, "V".into()),
        ];
        let (y_ch2, unit) = native_y_for_series(5.0, "CH2", &maps, Some(0));
        assert_eq!(unit, "V");
        assert!(
            y_ch2.abs() < 1e-12,
            "CH2 GND at plot Y=5 must read 0, got {y_ch2}"
        );
        let (y_ch1, _) = native_y_for_series(5.0, "CH1", &maps, Some(0));
        assert!((y_ch1 - 5.0).abs() < 1e-12);
        let (y_pref, _) = native_y_for_series(5.0, "", &maps, Some(1));
        assert!(y_pref.abs() < 1e-12, "preferred CH2 GND must read 0");
    }
}
