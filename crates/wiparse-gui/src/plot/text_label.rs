//! Plot text overlay with a configurable font size (egui_plot `Text` is hardcoded to Small).
//!
//! Bus decode labels sit in four screen-pixel lanes *above* the decoded waveform's top
//! edge, so they follow channel Y-offset drags and stay off the analog trace.

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Shape, Stroke};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotTransform};

/// Vertical spacing between decode lanes (packet / byte / bit / chip).
pub const BUS_LANE_STEP_PX: f32 = 18.0;
/// Gap between the chip lane and the waveform top edge.
pub const BUS_LANE_PAD_PX: f32 = 6.0;

pub const LANE_PACKET: u8 = 0;
pub const LANE_BYTE: u8 = 1;
pub const LANE_BIT: u8 = 2;
pub const LANE_CHIP: u8 = 3;

/// Screen Y of lane 0 (packet), stacked above `wave_top_screen`, clamped into `frame`.
pub fn bus_stack_packet_screen_y(wave_top_screen: f32, frame: Rect) -> f32 {
    let chip_y = wave_top_screen - BUS_LANE_PAD_PX;
    let packet_y = chip_y - f32::from(LANE_CHIP) * BUS_LANE_STEP_PX;
    let min_y = frame.top() + 4.0;
    let max_y = frame.bottom() - 4.0 - f32::from(LANE_CHIP) * BUS_LANE_STEP_PX;
    if max_y < min_y {
        min_y
    } else {
        packet_y.clamp(min_y, max_y)
    }
}

pub fn bus_lane_screen_y(packet_screen_y: f32, lane: u8) -> f32 {
    packet_screen_y + f32::from(lane) * BUS_LANE_STEP_PX
}

/// Which decode lane a screen Y falls into, if any.
pub fn bus_lane_from_screen_y(packet_screen_y: f32, screen_y: f32) -> Option<u8> {
    let rel = screen_y - packet_screen_y + BUS_LANE_STEP_PX * 0.35;
    if rel < 0.0 {
        return None;
    }
    let lane = (rel / BUS_LANE_STEP_PX).floor() as i32;
    if (0..=3).contains(&lane) {
        Some(lane as u8)
    } else {
        None
    }
}

pub struct PlotTextLabel {
    x: f64,
    lane: u8,
    /// Plot-Y of the decoded waveform's upper edge (`y_scale * y_max + y_offset`).
    wave_top_plot: f64,
    text: String,
    color: Color32,
    font_size: f32,
    highlighted: bool,
}

impl PlotTextLabel {
    /// Label at plot time `x`, on a lane stacked above `wave_top_plot`.
    pub fn lane(
        x: f64,
        lane: u8,
        text: impl Into<String>,
        color: Color32,
        font_size: f32,
        wave_top_plot: f64,
    ) -> Self {
        Self {
            x,
            lane,
            wave_top_plot,
            text: text.into(),
            color,
            font_size,
            highlighted: false,
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
        let x = transform.position_from_point_x(self.x);
        let wave_top_sy = transform.position_from_point_y(self.wave_top_plot);
        let packet_sy = bus_stack_packet_screen_y(wave_top_sy, *transform.frame());
        let y = bus_lane_screen_y(packet_sy, self.lane);
        let pos = Pos2::new(x, y);
        let rect = Align2::CENTER_TOP.anchor_size(pos, galley.size());
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
        PlotBounds::NOTHING
    }

    fn id(&self) -> Option<Id> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn chip_lane_sits_above_waveform_top() {
        let frame = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 400.0));
        let wave_top = 200.0;
        let pkt = bus_stack_packet_screen_y(wave_top, frame);
        let chip = bus_lane_screen_y(pkt, LANE_CHIP);
        assert!(chip > pkt, "packet should be higher on screen than chip");
        assert!(
            chip < wave_top,
            "chip lane should stay above the waveform top"
        );
        assert_eq!(bus_lane_from_screen_y(pkt, pkt + 2.0), Some(LANE_PACKET));
        assert_eq!(
            bus_lane_from_screen_y(pkt, pkt + BUS_LANE_STEP_PX * 3.0 + 2.0),
            Some(LANE_CHIP)
        );
    }
}
