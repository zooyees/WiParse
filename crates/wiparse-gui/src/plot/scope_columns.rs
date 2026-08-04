//! Oscilloscope-style envelope column rendering (vertical edges + horizontal plateaus).

use egui::{Color32, Pos2, Rect, Shape, Stroke};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotPoint, PlotTransform};
use std::sync::Arc;
use wiparse_core::wave_display::{envelope_bounds, ScopeEnvelopeColumn};

fn clip_segment(a: Pos2, b: Pos2, rect: Rect) -> Option<(Pos2, Pos2)> {
    let a = Pos2::new(
        a.x.clamp(rect.left(), rect.right()),
        a.y.clamp(rect.top(), rect.bottom()),
    );
    let b = Pos2::new(
        b.x.clamp(rect.left(), rect.right()),
        b.y.clamp(rect.top(), rect.bottom()),
    );
    if (a - b).length_sq() > 0.0 {
        Some((a, b))
    } else {
        None
    }
}

/// Push scope-style column shapes into `out` using the plot's current transform.
pub fn push_envelope_column_shapes(
    transform: &PlotTransform,
    columns: &[ScopeEnvelopeColumn],
    stroke: Stroke,
    frame: Rect,
    out: &mut Vec<Shape>,
) {
    out.reserve(columns.len());
    for col in columns {
        if col.is_flat() {
            let a = transform.position_from_point(&PlotPoint::new(col.x0, col.y_min));
            let b = transform.position_from_point(&PlotPoint::new(col.x1, col.y_min));
            if let Some((a, b)) = clip_segment(a, b, frame) {
                out.push(Shape::line_segment([a, b], stroke));
            }
        } else {
            let xc = 0.5 * (col.x0 + col.x1);
            let a = transform.position_from_point(&PlotPoint::new(xc, col.y_min));
            let b = transform.position_from_point(&PlotPoint::new(xc, col.y_max));
            if let Some((a, b)) = clip_segment(a, b, frame) {
                out.push(Shape::line_segment([a, b], stroke));
            }
        }
    }
}

/// Paint envelope columns into a screen-space rectangle (instrument preview).
pub fn paint_envelope_columns(
    painter: &egui::Painter,
    columns: &[ScopeEnvelopeColumn],
    map: impl Fn([f64; 2]) -> Pos2,
    inner: Rect,
    stroke: Stroke,
) {
    for col in columns {
        if col.is_flat() {
            let a = map([col.x0, col.y_min]);
            let b = map([col.x1, col.y_min]);
            let a = Pos2::new(
                a.x.clamp(inner.left(), inner.right()),
                a.y.clamp(inner.top(), inner.bottom()),
            );
            let b = Pos2::new(
                b.x.clamp(inner.left(), inner.right()),
                b.y.clamp(inner.top(), inner.bottom()),
            );
            if (a - b).length_sq() > 0.0 {
                painter.line_segment([a, b], stroke);
            }
        } else {
            let xc = 0.5 * (col.x0 + col.x1);
            let a = map([xc, col.y_min]);
            let b = map([xc, col.y_max]);
            let a = Pos2::new(
                a.x.clamp(inner.left(), inner.right()),
                a.y.clamp(inner.top(), inner.bottom()),
            );
            let b = Pos2::new(
                b.x.clamp(inner.left(), inner.right()),
                b.y.clamp(inner.top(), inner.bottom()),
            );
            if (a - b).length_sq() > 0.0 {
                painter.line_segment([a, b], stroke);
            }
        }
    }
}

/// egui_plot item: min/max envelope columns with correct frame transform.
pub struct ScopeEnvelopePlotItem {
    pub columns: Arc<Vec<ScopeEnvelopeColumn>>,
    pub color: Color32,
    pub width: f32,
    pub name: String,
    pub highlighted: bool,
    cached_bounds: PlotBounds,
}

impl ScopeEnvelopePlotItem {
    pub fn new(
        columns: Arc<Vec<ScopeEnvelopeColumn>>,
        color: Color32,
        width: f32,
        name: impl Into<String>,
        highlighted: bool,
    ) -> Self {
        let (xmin, xmax, ymin, ymax) = envelope_bounds(columns.as_slice());
        let cached_bounds = PlotBounds::from_min_max([xmin, ymin], [xmax, ymax]);
        Self {
            columns,
            color,
            width,
            name: name.into(),
            highlighted,
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
