//! Offline waveform analysis — folder browser, open scope sources, zoom/pan, measure.

use crate::theme::Tokens;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Vec2b};
use egui_plot::{HLine, Legend, Line, Plot, PlotBounds, PlotPoints, Points, VLine};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wiparse_core::config::{load_config, save_config, AppConfig};
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::instrument::WaveformTrace;
use wiparse_core::paths::project_path;
use wiparse_core::waveform_file::{
    load_waveform_file_all, measure_waveform, measure_waveform_range, save_waveform_file,
    WaveformMeasurements,
};

const PLOT_DISPLAY_POINTS: usize = 8_192;
const TOOLBAR_H: f32 = 44.0;
const PANEL_GAP: f32 = 8.0;
const SIDE_W: f32 = 280.0;
const BTN: egui::Vec2 = egui::vec2(108.0, 28.0);
const BTN_SM: egui::Vec2 = egui::vec2(72.0, 28.0);
const CARD_MARGIN_X: i8 = 8;
/// Cursor grab distance as a fraction of visible axis span.
const CURSOR_GRAB_FRAC: f64 = 0.012;
/// Max zoom-out: view may extend this factor beyond full data extent.
const VIEW_MAX_PAD: f64 = 1.25;
/// Min zoom-in: visible span ≥ full data span / this factor (and ≥ absolute floor).
const VIEW_MAX_ZOOM: f64 = 50_000.0;
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
    plot_points: Arc<Vec<[f64; 2]>>,
    measures: WaveformMeasurements,
    color: Color32,
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
            browser_folders: Vec::new(),
            mode: InteractMode::Pan,
            dragging_cursor: None,
            fit_request: false,
            pending_bounds: None,
            last_x_range: None,
            last_y_range: None,
        };
        panel.refresh_wave_browser();
        panel
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
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

        // Browser (top ~58%) + measurements (bottom), matching serial sidebar card width.
        let browser_h = ((body_h - PANEL_GAP) * 0.58).max(180.0).min(body_h * 0.72);
        let meas_h = (body_h - browser_h - PANEL_GAP).max(100.0);
        let browser_rect =
            egui::Rect::from_min_size(side_rect.min, egui::vec2(side_w, browser_h));
        let meas_rect = egui::Rect::from_min_size(
            egui::pos2(side_rect.min.x, side_rect.min.y + browser_h + PANEL_GAP),
            egui::vec2(side_w, meas_h),
        );

        panel_in_rect(ui, browser_rect, |ui| {
            self.browser_panel(ui, lang, tokens);
        });
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
                            } else {
                                self.activate_wave(0);
                            }
                            self.status = t(lang, "已关闭波形", "Waveform closed").into();
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
                        .on_hover_text(t(lang, "纵向放大", "Zoom in Y"))
                        .clicked()
                    {
                        self.request_zoom_axis(false, 0.5);
                    }
                    if ui
                        .add(egui::Button::new("Y−").min_size(egui::vec2(36.0, 28.0)))
                        .on_hover_text(t(lang, "纵向缩小", "Zoom out Y"))
                        .clicked()
                    {
                        self.request_zoom_axis(false, 2.0);
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
                                "Ctrl+滚轮=X · Ctrl+Shift+滚轮=Y · 空格=X/Y光标",
                                "Ctrl+wheel=X · Ctrl+Shift+wheel=Y · Space=X/Y cursors",
                            ))
                            .small()
                            .color(tokens.text_muted),
                        );
                    });
                });
            });
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

    fn data_extent(&self) -> Option<PlotBounds> {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        let mut any = false;
        for w in &self.waves {
            let n = w.trace.x.len().min(w.trace.y.len());
            if n == 0 {
                continue;
            }
            any = true;
            for i in 0..n {
                xmin = xmin.min(w.trace.x[i]);
                xmax = xmax.max(w.trace.x[i]);
                ymin = ymin.min(w.trace.y[i]);
                ymax = ymax.max(w.trace.y[i]);
            }
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
        Some(PlotBounds::from_min_max([xmin, ymin], [xmax, ymax]))
    }

    /// Extent of the selected wave only.
    fn selected_extent(&self) -> Option<PlotBounds> {
        let w = self.selected.and_then(|i| self.waves.get(i))?;
        extent_of_traces(&[w])
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
        let outer_y0 = ey0 - y_span_data * (VIEW_MAX_PAD - 1.0);
        let outer_y1 = ey1 + y_span_data * (VIEW_MAX_PAD - 1.0);

        let min_x_span = (x_span_data / VIEW_MAX_ZOOM).max(VIEW_MIN_SPAN_ABS);
        let min_y_span = (y_span_data / VIEW_MAX_ZOOM).max(VIEW_MIN_SPAN_ABS);
        let max_x_span = (outer_x1 - outer_x0).abs();
        let max_y_span = (outer_y1 - outer_y0).abs();

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
        let mut y_span = (y1 - y0).max(min_y_span).min(max_y_span);
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
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for i in 0..n {
            let x = w.trace.x[i];
            if x >= lo && x <= hi {
                ymin = ymin.min(w.trace.y[i]);
                ymax = ymax.max(w.trace.y[i]);
            }
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
        match self.load_path(path) {
            Ok(()) => {
                self.status = format!(
                    "{} {} ({} ch)",
                    t(lang, "已加载", "Loaded"),
                    path.display(),
                    self.waves.len()
                );
            }
            Err(err) => {
                self.status = format!(
                    "{} {}: {err}",
                    t(lang, "加载失败", "Load failed"),
                    path.display()
                );
            }
        }
    }

    /// Select a loaded wave and reset the plot viewport to the whole document.
    fn activate_wave(&mut self, index: usize) {
        if index >= self.waves.len() {
            return;
        }
        self.selected = Some(index);
        self.fit_request = true;
        self.pending_bounds = None;
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
                let yu = &w.trace.y_unit;
                let xu = &w.trace.x_unit;

                // Prefer gated measurements when X1/X2 are both set.
                let gated = match (self.x1, self.x2) {
                    (Some(a), Some(b)) => Some(measure_waveform_range(&w.trace, a, b)),
                    _ => None,
                };
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
                    CursorAxis::Y => self.draw_y_cursor_measures(ui, tokens, lang, yu),
                }
            });
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
                    measure_row(ui, tokens, "1/|ΔX|", &format_eng(1.0 / dx.abs(), "Hz"));
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
        yu: &str,
    ) {
        match (self.y1, self.y2) {
            (Some(a), Some(b)) => {
                let dy = b - a;
                measure_row(ui, tokens, "Y1", &format_eng(a, yu));
                measure_row(ui, tokens, "Y2", &format_eng(b, yu));
                measure_row(ui, tokens, "ΔY", &format_eng(dy, yu));
                measure_row(ui, tokens, "|ΔY|", &format_eng(dy.abs(), yu));
            }
            (Some(a), None) => {
                measure_row(ui, tokens, "Y1", &format_eng(a, yu));
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
        let mut drag_cursor: Option<(u8, f64)> = None;
        let mut end_drag = false;
        let mut needs_clamp = false;

        let view_points: Option<(usize, Vec<[f64; 2]>)> =
            if let (Some(i), Some((x0, x1))) = (self.selected, x_view) {
                self.waves.get(i).map(|w| {
                    (
                        i,
                        windowed_points(&w.trace, x0, x1, PLOT_DISPLAY_POINTS),
                    )
                })
            } else {
                None
            };

        let x1 = self.x1;
        let x2 = self.x2;
        let y1 = self.y1;
        let y2 = self.y2;
        let dragging = self.dragging_cursor;
        let xu = self
            .selected
            .and_then(|i| self.waves.get(i))
            .map(|w| w.trace.x_unit.clone())
            .unwrap_or_else(|| "s".into());
        let yu = self
            .selected
            .and_then(|i| self.waves.get(i))
            .map(|w| w.trace.y_unit.clone())
            .unwrap_or_else(|| "V".into());

        let response = Plot::new("waveform-analysis-plot")
            .height(height)
            .allow_zoom(allow_zoom)
            .allow_drag(mode == InteractMode::Pan)
            .allow_scroll(!ctrl)
            .allow_boxed_zoom(mode == InteractMode::Pan)
            .legend(Legend::default())
            .label_formatter(|name, value| {
                if name.is_empty() {
                    format!("t={}\nv={}", format_eng(value.x, "s"), format_eng(value.y, "V"))
                } else {
                    format!(
                        "{name}\nt={}\nv={}",
                        format_eng(value.x, "s"),
                        format_eng(value.y, "V")
                    )
                }
            })
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

                for (i, w) in self.waves.iter().enumerate() {
                    let pts = if view_points.as_ref().is_some_and(|(si, _)| *si == i) {
                        PlotPoints::from(view_points.as_ref().unwrap().1.clone())
                    } else {
                        PlotPoints::from(w.plot_points.as_slice().to_vec())
                    };
                    let stroke = if self.selected == Some(i) {
                        2.0_f32
                    } else {
                        1.2_f32
                    };
                    plot_ui.line(
                        Line::new(pts)
                            .name(&w.label)
                            .color(w.color)
                            .width(stroke),
                    );
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

                let resp = plot_ui.response().clone();
                let primary_down = plot_ui.ctx().input(|i| i.pointer.primary_down());
                if let Some(ptr) = plot_ui.pointer_coordinate() {
                    plot_ui.points(
                        Points::new(PlotPoints::from(vec![[ptr.x, ptr.y]]))
                            .radius(0.01_f32)
                            .color(Color32::TRANSPARENT),
                    );

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
                            click_val = Some(match axis {
                                CursorAxis::X => ptr.x,
                                CursorAxis::Y => ptr.y,
                            });
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
            });

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
            let factor = (zoom_speed * y_wheel).exp() as f64;
            if factor.is_finite() && (factor - 1.0).abs() > 1e-6 {
                self.request_zoom_axis(false, factor);
            }
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
        }

        // On-plot measurement overlay (X1/X2 or Y1/Y2 + delta).
        let overlay = match self.cursor_axis {
            CursorAxis::X => match (self.x1, self.x2) {
                (Some(a), Some(b)) => Some(format!(
                    "X1={}   X2={}   ΔX={}",
                    format_eng(a, &xu),
                    format_eng(b, &xu),
                    format_eng(b - a, &xu)
                )),
                (Some(a), None) => Some(format!("X1={}   X2=—", format_eng(a, &xu))),
                _ => None,
            },
            CursorAxis::Y => match (self.y1, self.y2) {
                (Some(a), Some(b)) => Some(format!(
                    "Y1={}   Y2={}   ΔY={}",
                    format_eng(a, &yu),
                    format_eng(b, &yu),
                    format_eng(b - a, &yu)
                )),
                (Some(a), None) => Some(format!("Y1={}   Y2=—", format_eng(a, &yu))),
                _ => None,
            },
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
            match self.load_path(&path) {
                Ok(()) => {
                    self.status = format!("{} {}", t(lang, "已加载", "Loaded"), path.display());
                }
                Err(err) => {
                    self.status = format!(
                        "{} {}: {err}",
                        t(lang, "加载失败", "Load failed"),
                        path.display()
                    );
                }
            }
        }
    }

    fn load_path(&mut self, path: &Path) -> Result<(), String> {
        let traces = load_waveform_file_all(path).map_err(|e| e.to_string())?;
        if traces.is_empty() {
            return Err("empty waveform".into());
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("wave");
        let first_idx = self.waves.len();
        for trace in traces {
            let measures = measure_waveform(&trace);
            let plot_points = Arc::new(downsample_points(&trace, PLOT_DISPLAY_POINTS));
            let label = format!("{file_name} · {}", trace.channel);
            let color = color_for_channel(&trace.channel).unwrap_or_else(|| {
                let c = TRACE_COLORS[self.next_color % TRACE_COLORS.len()];
                self.next_color += 1;
                c
            });
            self.waves.push(LoadedWave {
                path: path.to_path_buf(),
                label,
                trace,
                plot_points,
                measures,
                color,
            });
        }
        self.activate_wave(first_idx);
        Ok(())
    }

    /// Load a path programmatically (e.g. deep-link / automation).
    #[allow(dead_code)]
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
        match self.load_path(path) {
            Ok(()) => {
                self.status = format!("{} {}", t(lang, "已加载", "Loaded"), path.display());
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

/// Min/max downsample within an X window (full-resolution source).
fn windowed_points(
    trace: &WaveformTrace,
    x_min: f64,
    x_max: f64,
    max_points: usize,
) -> Vec<[f64; 2]> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return Vec::new();
    }
    let (lo, hi) = if x_min <= x_max {
        (x_min, x_max)
    } else {
        (x_max, x_min)
    };
    // Expand slightly so edges stay visible while panning.
    let pad = (hi - lo).abs() * 0.02;
    let lo = lo - pad;
    let hi = hi + pad;

    let mut start = 0usize;
    while start < n && trace.x[start] < lo {
        start += 1;
    }
    if start > 0 {
        start -= 1;
    }
    let mut end = start;
    while end < n && trace.x[end] <= hi {
        end += 1;
    }
    end = end.min(n).max(start + 1);
    let slice_n = end - start;
    if slice_n <= max_points.max(2) {
        return trace.x[start..end]
            .iter()
            .zip(&trace.y[start..end])
            .map(|(&x, &y)| [x, y])
            .collect();
    }
    // Local min/max bucket on the windowed slice.
    let buckets = max_points.max(2) / 2;
    let mut points = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let s = start + b * slice_n / buckets;
        let e = (start + (b + 1) * slice_n / buckets)
            .max(s + 1)
            .min(end);
        let mut min_i = s;
        let mut max_i = s;
        for i in s..e {
            if trace.y[i] < trace.y[min_i] {
                min_i = i;
            }
            if trace.y[i] > trace.y[max_i] {
                max_i = i;
            }
        }
        if min_i <= max_i {
            points.push([trace.x[min_i], trace.y[min_i]]);
            if max_i != min_i {
                points.push([trace.x[max_i], trace.y[max_i]]);
            }
        } else {
            points.push([trace.x[max_i], trace.y[max_i]]);
            points.push([trace.x[min_i], trace.y[min_i]]);
        }
    }
    points
}

fn downsample_points(trace: &WaveformTrace, max_points: usize) -> Vec<[f64; 2]> {
    let n = trace.x.len().min(trace.y.len());
    let max_points = max_points.max(2);
    if n == 0 {
        return Vec::new();
    }
    if n <= max_points {
        return trace.x[..n]
            .iter()
            .zip(&trace.y[..n])
            .map(|(&x, &y)| [x, y])
            .collect();
    }
    let buckets = max_points / 2;
    let mut points = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let start = b * n / buckets;
        let end = ((b + 1) * n / buckets).max(start + 1).min(n);
        let mut min_i = start;
        let mut max_i = start;
        for i in start..end {
            if trace.y[i] < trace.y[min_i] {
                min_i = i;
            }
            if trace.y[i] > trace.y[max_i] {
                max_i = i;
            }
        }
        if min_i <= max_i {
            points.push([trace.x[min_i], trace.y[min_i]]);
            if max_i != min_i {
                points.push([trace.x[max_i], trace.y[max_i]]);
            }
        } else {
            points.push([trace.x[max_i], trace.y[max_i]]);
            points.push([trace.x[min_i], trace.y[min_i]]);
        }
    }
    points
}

fn interpolate_y(trace: &WaveformTrace, x: f64) -> Option<f64> {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some(trace.y[0]);
    }
    if x <= trace.x[0] {
        return Some(trace.y[0]);
    }
    if x >= trace.x[n - 1] {
        return Some(trace.y[n - 1]);
    }
    let mut i = 1;
    while i < n && trace.x[i] < x {
        i += 1;
    }
    let x0 = trace.x[i - 1];
    let x1 = trace.x[i];
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
        let n = w.trace.x.len().min(w.trace.y.len());
        if n == 0 {
            continue;
        }
        any = true;
        for i in 0..n {
            xmin = xmin.min(w.trace.x[i]);
            xmax = xmax.max(w.trace.x[i]);
            ymin = ymin.min(w.trace.y[i]);
            ymax = ymax.max(w.trace.y[i]);
        }
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

fn t(lang: Lang, zh: &str, en: &str) -> String {
    match lang {
        Lang::Zh => zh.to_string(),
        Lang::En => en.to_string(),
    }
}
