//! Plot text overlay with a configurable font size (egui_plot `Text` is hardcoded to Small).

use egui::{Align2, Color32, FontId, Id, Shape, Stroke};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotPoint, PlotTransform};

pub struct PlotTextLabel {
    position: PlotPoint,
    text: String,
    color: Color32,
    font_size: f32,
    highlighted: bool,
    anchor: Align2,
}

impl PlotTextLabel {
    pub fn new(x: f64, y: f64, text: impl Into<String>, color: Color32, font_size: f32) -> Self {
        Self {
            position: PlotPoint::new(x, y),
            text: text.into(),
            color,
            font_size,
            highlighted: false,
            anchor: Align2::CENTER_BOTTOM,
        }
    }

    pub fn highlight(mut self, on: bool) -> Self {
        self.highlighted = on;
        self
    }
}

impl PlotItem for PlotTextLabel {
    fn shapes(&self, ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let font_id = FontId::monospace(self.font_size);
        let galley = ui.fonts(|f| f.layout_no_wrap(self.text.clone(), font_id, self.color));
        let pos = transform.position_from_point(&self.position);
        let rect = self.anchor.anchor_size(pos, galley.size());
        shapes.push(egui::epaint::TextShape::new(rect.min, galley, self.color).into());
        if self.highlighted {
            shapes.push(Shape::rect_stroke(
                rect.expand(2.0),
                2.0,
                Stroke::new(1.2_f32, self.color),
                egui::StrokeKind::Outside,
            ));
        }
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn name(&self) -> &str {
        ""
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
        let mut bounds = PlotBounds::NOTHING;
        bounds.extend_with(&self.position);
        bounds
    }

    fn id(&self) -> Option<Id> {
        None
    }
}
