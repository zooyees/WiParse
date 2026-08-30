//! Oscilloscope-style MMFL envelope rendering.
//!
//! One time-ordered polyline using each column's real first/min/max/last sample times.
//! Peaks stay on the path (no I-beams) and are not shifted to 35%/65% of the bucket.

use egui::{Align2, Color32, Pos2, Rect, Shape, Stroke, TextStyle};
use egui_plot::{
    ClosestElem, Cursor, LabelFormatter, PlotBounds, PlotConfig, PlotGeometry, PlotItem, PlotPoint,
    PlotTransform,
};
use std::sync::Arc;
use wiparse_core::wave_display::{envelope_bounds, ScopeEnvelopeColumn};

/// Skip interior min/max ticks shorter than this on screen (pixels).
const PIXEL_ENVELOPE_MIN: f32 = 0.75;
const DUP_EPS_SQ: f32 = 1e-6;

#[inline]
fn display_y(y_raw: f64, y_scale: f64, y_offset: f64) -> f64 {
    y_raw * y_scale + y_offset
}

#[inline]
fn append_pos(pts: &mut Vec<Pos2>, p: Pos2) {
    if let Some(last) = pts.last() {
        if (*last - p).length_sq() <= DUP_EPS_SQ {
            return;
        }
    }
    pts.push(p);
}

fn take_strip(pts: &mut Vec<Pos2>, out: &mut Vec<Vec<Pos2>>) {
    if !pts.is_empty() {
        out.push(std::mem::take(pts));
    }
}

fn flush_polyline(out: &mut Vec<Shape>, pts: &mut Vec<Pos2>, stroke: Stroke) {
    match pts.len() {
        0 => {}
        1 => {
            let p = pts[0];
            let w = stroke.width.max(1.0);
            out.push(Shape::line_segment([p, Pos2::new(p.x + w, p.y)], stroke));
        }
        _ => out.push(Shape::line(std::mem::take(pts), stroke)),
    }
    pts.clear();
}

fn map_plot(transform: &PlotTransform, x: f64, y: f64) -> Pos2 {
    transform.position_from_point(&PlotPoint::new(x, y))
}

/// Screen-space strokes (single vector path; hull/verticals kept for tests).
#[derive(Default)]
pub struct EnvelopeStrokes {
    pub vectors: Vec<Vec<Pos2>>,
    pub hull_top: Vec<Vec<Pos2>>,
    pub hull_bot: Vec<Vec<Pos2>>,
    pub verticals: Vec<[Pos2; 2]>,
}

/// Build a continuous time-ordered MMFL polyline.
pub fn envelope_screen_polylines(
    columns: &[ScopeEnvelopeColumn],
    mut map: impl FnMut(f64, f64) -> Pos2,
    y_scale: f64,
    y_offset: f64,
) -> EnvelopeStrokes {
    let mut strokes = EnvelopeStrokes::default();
    let mut vpts: Vec<Pos2> = Vec::with_capacity(columns.len().saturating_mul(3).max(4));
    let mut prev_x1: Option<f64> = None;
    let mut buf = [(0.0, 0.0); 4];

    for col in columns {
        let ymin = display_y(col.y_min, y_scale, y_offset);
        let ymax = display_y(col.y_max, y_scale, y_offset);
        let span = (col.x1 - col.x0).abs();
        let xc = 0.5 * (col.x0 + col.x1);
        let envelope_px = (map(xc, ymin).y - map(xc, ymax).y).abs();

        if let Some(px1) = prev_x1 {
            if span > 0.0 && col.x0 > px1 + span * 0.51 {
                take_strip(&mut vpts, &mut strokes.vectors);
            }
        }

        let src_n = if envelope_px >= PIXEL_ENVELOPE_MIN {
            col.mmfl_points_n(&mut buf)
        } else {
            buf[0] = (col.x_first, col.y_first);
            buf[1] = (col.x_last, col.y_last);
            2
        };
        for &(x, y) in buf.iter().take(src_n) {
            append_pos(&mut vpts, map(x, display_y(y, y_scale, y_offset)));
        }
        prev_x1 = Some(col.x1);
    }

    take_strip(&mut vpts, &mut strokes.vectors);
    strokes
}

fn push_envelope_strokes(out: &mut Vec<Shape>, mut strokes: EnvelopeStrokes, stroke: Stroke) {
    for mut pts in strokes.vectors {
        flush_polyline(out, &mut pts, stroke);
    }
    for mut pts in strokes.hull_top {
        flush_polyline(out, &mut pts, stroke);
    }
    for mut pts in strokes.hull_bot {
        flush_polyline(out, &mut pts, stroke);
    }
    out.reserve(strokes.verticals.len());
    for [a, b] in strokes.verticals.drain(..) {
        if (a - b).length_sq() > DUP_EPS_SQ {
            out.push(Shape::line_segment([a, b], stroke));
        } else {
            let w = stroke.width.max(1.0);
            out.push(Shape::line_segment([a, Pos2::new(a.x + w, a.y)], stroke));
        }
    }
}

/// Push scope-style envelope as batched polylines using the plot's current transform.
pub fn push_envelope_column_shapes(
    transform: &PlotTransform,
    columns: &[ScopeEnvelopeColumn],
    stroke: Stroke,
    _frame: Rect,
    out: &mut Vec<Shape>,
    y_scale: f64,
    y_offset: f64,
) {
    let strokes = envelope_screen_polylines(
        columns,
        |x, y| map_plot(transform, x, y),
        y_scale,
        y_offset,
    );
    push_envelope_strokes(out, strokes, stroke);
}

/// Paint envelope columns into a screen-space rectangle (instrument preview).
pub fn paint_envelope_columns(
    painter: &egui::Painter,
    columns: &[ScopeEnvelopeColumn],
    map: impl Fn([f64; 2]) -> Pos2,
    _inner: Rect,
    stroke: Stroke,
) {
    let strokes = envelope_screen_polylines(
        columns,
        |x, y| map([x, y]),
        1.0,
        0.0,
    );
    let mut shapes = Vec::new();
    push_envelope_strokes(&mut shapes, strokes, stroke);
    for shape in shapes {
        painter.add(shape);
    }
}

/// egui_plot item: min/max envelope columns with correct frame transform.
pub struct ScopeEnvelopePlotItem {
    pub columns: Arc<Vec<ScopeEnvelopeColumn>>,
    pub color: Color32,
    pub width: f32,
    pub name: String,
    pub highlighted: bool,
    /// Display-only vertical scale (1.0 = native).
    pub y_scale: f64,
    /// Display-only vertical shift (plot Y units).
    pub y_offset: f64,
    cached_bounds: PlotBounds,
}

impl ScopeEnvelopePlotItem {
    pub fn new(
        columns: Arc<Vec<ScopeEnvelopeColumn>>,
        color: Color32,
        width: f32,
        name: impl Into<String>,
        highlighted: bool,
        y_scale: f64,
        y_offset: f64,
    ) -> Self {
        let (xmin, xmax, ymin, ymax) = envelope_bounds(columns.as_slice());
        let cached_bounds = PlotBounds::from_min_max(
            [xmin, display_y(ymin, y_scale, y_offset)],
            [xmax, display_y(ymax, y_scale, y_offset)],
        );
        Self {
            columns,
            color,
            width,
            name: name.into(),
            highlighted,
            y_scale,
            y_offset,
            cached_bounds,
        }
    }
}

impl PlotItem for ScopeEnvelopePlotItem {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let stroke = Stroke::new(
            if self.highlighted {
                self.width + 0.5
            } else {
                self.width
            },
            self.color,
        );
        push_envelope_column_shapes(
            transform,
            &self.columns,
            stroke,
            *transform.frame(),
            shapes,
            self.y_scale,
            self.y_offset,
        );
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn name(&self) -> &str {
        &self.name
    }

    fn color(&self) -> Color32 {
        self.color
    }

    fn highlight(&mut self) {
        self.highlighted = true;
    }

    fn highlighted(&self) -> bool {
        self.highlighted
    }

    fn allow_hover(&self) -> bool {
        false
    }

    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    fn bounds(&self) -> PlotBounds {
        self.cached_bounds
    }

    fn id(&self) -> Option<egui::Id> {
        None
    }
}

/// Vector (uniform) polyline with display scale/offset applied in `shapes()`, no per-frame copy.
pub struct ScopeVectorPlotItem {
    pub pts: Arc<Vec<[f64; 2]>>,
    pub color: Color32,
    pub width: f32,
    pub name: String,
    pub highlighted: bool,
    pub y_scale: f64,
    pub y_offset: f64,
    cached_bounds: PlotBounds,
}

impl ScopeVectorPlotItem {
    pub fn new(
        pts: Arc<Vec<[f64; 2]>>,
        color: Color32,
        width: f32,
        name: impl Into<String>,
        highlighted: bool,
        y_scale: f64,
        y_offset: f64,
    ) -> Self {
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for p in pts.iter() {
            if p[0].is_finite() {
                xmin = xmin.min(p[0]);
                xmax = xmax.max(p[0]);
            }
            let y = display_y(p[1], y_scale, y_offset);
            if y.is_finite() {
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
        if !xmin.is_finite() {
            xmin = 0.0;
            xmax = 1.0;
        }
        if !ymin.is_finite() {
            ymin = 0.0;
            ymax = 1.0;
        }
        Self {
            pts,
            color,
            width,
            name: name.into(),
            highlighted,
            y_scale,
            y_offset,
            cached_bounds: PlotBounds::from_min_max([xmin, ymin], [xmax, ymax]),
        }
    }

    #[inline]
    fn plot_point(&self, p: [f64; 2]) -> PlotPoint {
        PlotPoint::new(p[0], display_y(p[1], self.y_scale, self.y_offset))
    }
}

impl PlotItem for ScopeVectorPlotItem {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let stroke = Stroke::new(self.width, self.color);
        let mut screen = Vec::with_capacity(self.pts.len());
        for &p in self.pts.iter() {
            if p[0].is_finite() && p[1].is_finite() {
                screen.push(map_plot(
                    transform,
                    p[0],
                    display_y(p[1], self.y_scale, self.y_offset),
                ));
            }
        }
        match screen.len() {
            0 => {}
            1 => {
                let p = screen[0];
                let w = stroke.width.max(1.0);
                shapes.push(Shape::line_segment([p, Pos2::new(p.x + w, p.y)], stroke));
            }
            _ => shapes.push(Shape::line(screen, stroke)),
        }
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn name(&self) -> &str {
        &self.name
    }

    fn color(&self) -> Color32 {
        self.color
    }

    fn highlight(&mut self) {
        self.highlighted = true;
    }

    fn highlighted(&self) -> bool {
        self.highlighted
    }

    fn allow_hover(&self) -> bool {
        true
    }

    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    fn bounds(&self) -> PlotBounds {
        self.cached_bounds
    }

    fn id(&self) -> Option<egui::Id> {
        None
    }

    fn find_closest(&self, point: Pos2, transform: &PlotTransform) -> Option<ClosestElem> {
        self.pts
            .iter()
            .enumerate()
            .filter(|(_, p)| p[0].is_finite() && p[1].is_finite())
            .map(|(index, &p)| {
                let pos = transform.position_from_point(&self.plot_point(p));
                ClosestElem {
                    index,
                    dist_sq: point.distance_sq(pos),
                }
            })
            .min_by(|a, b| a.dist_sq.total_cmp(&b.dist_sq))
    }

    fn on_hover(
        &self,
        elem: ClosestElem,
        shapes: &mut Vec<Shape>,
        cursors: &mut Vec<Cursor>,
        plot: &PlotConfig<'_>,
        label_formatter: &LabelFormatter<'_>,
    ) {
        let Some(&p) = self.pts.get(elem.index) else {
            return;
        };
        let value = self.plot_point(p);
        let pointer = plot.transform.position_from_point(&value);
        let line_color = if plot.ui.visuals().dark_mode {
            Color32::from_gray(100).additive()
        } else {
            Color32::from_black_alpha(180)
        };
        shapes.push(Shape::circle_filled(pointer, 3.0, line_color));
        if plot.show_x {
            cursors.push(Cursor::Vertical { x: value.x });
        }
        if plot.show_y {
            cursors.push(Cursor::Horizontal { y: value.y });
        }
        let text = if let Some(custom) = label_formatter {
            custom(&self.name, &value)
        } else if self.name.is_empty() {
            format!("x = {}\ny = {}", value.x, value.y)
        } else {
            format!("{}\nx = {}\ny = {}", self.name, value.x, value.y)
        };
        let font_id = TextStyle::Body.resolve(plot.ui.style());
        plot.ui.fonts(|f| {
            shapes.push(Shape::text(
                f,
                pointer + egui::vec2(3.0, -2.0),
                Align2::LEFT_BOTTOM,
                text,
                font_id,
                plot.ui.visuals().text_color(),
            ));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiparse_core::wave_display::decimate_envelope_columns;

    #[test]
    fn dc_ramp_polyline_is_connected_not_dots() {
        let x: Vec<f64> = (0..2000).map(|i| i as f64).collect();
        let y = vec![3.0; 2000];
        let cols = decimate_envelope_columns(&x, &y, 40);
        let strokes = envelope_screen_polylines(
            &cols,
            |x, y| Pos2::new(x as f32, y as f32),
            1.0,
            0.0,
        );
        assert_eq!(strokes.vectors.len(), 1);
        assert!(strokes.verticals.is_empty());
        let pts = &strokes.vectors[0];
        assert!(pts.len() >= 2, "expected a continuous polyline, got {} points", pts.len());
        let xs: Vec<f32> = pts.iter().map(|p| p.x).collect();
        for w in xs.windows(2) {
            assert!(w[1] >= w[0] - 1e-3);
        }
        assert!(pts.iter().all(|p| (p.y - 3.0).abs() < 1e-3));
    }

    #[test]
    fn shallow_ramp_has_no_interior_ticks() {
        let n = 8000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| v * 0.001).collect();
        let cols = decimate_envelope_columns(&x, &y, 80);
        let strokes = envelope_screen_polylines(
            &cols,
            |x, y| Pos2::new(x as f32 * 0.05, y as f32 * 40.0),
            1.0,
            0.0,
        );
        assert_eq!(strokes.vectors.len(), 1);
        assert!(strokes.verticals.is_empty());
        assert!(
            strokes.vectors[0].len() <= 80 * 2 + 4,
            "ramp should not insert peak-detect ticks, got {} verts",
            strokes.vectors[0].len()
        );
    }

    fn assert_one_time_ordered_polyline(strokes: &EnvelopeStrokes) {
        assert!(strokes.verticals.is_empty(), "I-beam verticals");
        assert!(strokes.hull_top.is_empty(), "I-beam hull top");
        assert!(strokes.hull_bot.is_empty(), "I-beam hull bot");
        assert_eq!(strokes.vectors.len(), 1);
        let xs: Vec<f32> = strokes.vectors[0].iter().map(|p| p.x).collect();
        for w in xs.windows(2) {
            assert!(w[1] >= w[0] - 1e-3, "x must be time-ordered, got {:?}", w);
        }
    }

    #[test]
    fn reversing_square_stays_one_polyline_without_ibeams() {
        let n = 4000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let cols = decimate_envelope_columns(&x, &y, 40);
        let strokes = envelope_screen_polylines(
            &cols,
            |x, y| Pos2::new(x as f32, y as f32 * 20.0),
            1.0,
            0.0,
        );
        assert_one_time_ordered_polyline(&strokes);
        assert!(strokes.vectors[0].len() >= 2);
    }

    #[test]
    fn analog_sine_stays_one_polyline_without_ibeams() {
        let n = 8000;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| (i as f64 * std::f64::consts::TAU / 200.0).sin())
            .collect();
        let cols = decimate_envelope_columns(&x, &y, 80);
        assert!(
            cols.iter().any(|c| c.has_interior_extrema()),
            "fixture must include sine peaks/troughs inside a column"
        );
        let strokes = envelope_screen_polylines(
            &cols,
            |x, y| Pos2::new(x as f32, y as f32 * 40.0),
            1.0,
            0.0,
        );
        assert_one_time_ordered_polyline(&strokes);
        let ymax = strokes.vectors[0]
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let ymin = strokes.vectors[0]
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min);
        assert!(ymax > 30.0, "peaks should remain, ymax={ymax}");
        assert!(ymin < -30.0, "troughs should remain, ymin={ymin}");
    }

    #[test]
    fn analog_peak_is_drawn_at_sample_time_not_bucket_fraction() {
        let n = 100;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut y = vec![0.0; n];
        y[25] = 10.0;
        let cols = decimate_envelope_columns(&x, &y, 10);
        let spike = cols.iter().find(|c| c.y_max > 5.0).expect("spike column");
        let strokes = envelope_screen_polylines(
            &cols,
            |x, y| Pos2::new(x as f32, y as f32),
            1.0,
            0.0,
        );
        assert_one_time_ordered_polyline(&strokes);
        let pts = &strokes.vectors[0];
        assert!(
            pts.iter()
                .any(|p| (p.x - 25.0).abs() < 0.05 && (p.y - 10.0).abs() < 0.05),
            "peak vertex must sit on sample t=25, got {:?}",
            pts.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()
        );
        let span = (spike.x1 - spike.x0) as f32;
        let fake_a = spike.x0 as f32 + span * 0.35;
        let fake_b = spike.x0 as f32 + span * 0.65;
        assert!(
            !pts.iter()
                .any(|p| (p.y - 10.0).abs() < 0.05 && ((p.x - fake_a).abs() < 0.05 || (p.x - fake_b).abs() < 0.05)),
            "must not place the peak at 35%/65% of the bucket"
        );
    }
}
