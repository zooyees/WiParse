//! UI tokens aligned with Python `theme_palette.LIGHT` / `DARK`.

use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Tokens {
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub canvas_bg: Color32,
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub header_bg: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub button_bg: Color32,
    pub button_hover: Color32,
    pub tab_inactive_bg: Color32,
    pub tab_inactive_text: Color32,
    pub stop_bg: Color32,
    pub input_bg: Color32,
}

impl Tokens {
    pub fn light() -> Self {
        Self {
            text_primary: Color32::from_rgb(0x0F, 0x17, 0x2A),
            text_muted: Color32::from_rgb(0x64, 0x74, 0x8B),
            canvas_bg: Color32::from_rgb(0xE2, 0xE8, 0xF0),
            panel_bg: Color32::WHITE,
            surface_bg: Color32::from_rgb(0xF8, 0xFA, 0xFC),
            header_bg: Color32::from_rgb(0xF1, 0xF5, 0xF9),
            border: Color32::from_rgb(0x94, 0xA3, 0xB8),
            accent: Color32::from_rgb(0x02, 0x84, 0xC7),
            accent_text: Color32::WHITE,
            button_bg: Color32::from_rgb(0xCB, 0xD5, 0xE1),
            button_hover: Color32::from_rgb(0x94, 0xA3, 0xB8),
            tab_inactive_bg: Color32::from_rgb(0xE2, 0xE8, 0xF0),
            tab_inactive_text: Color32::from_rgb(0x64, 0x74, 0x8B),
            stop_bg: Color32::from_rgb(0xDC, 0x26, 0x26),
            input_bg: Color32::WHITE,
        }
    }

    pub fn dark() -> Self {
        Self {
            text_primary: Color32::from_rgb(0xF1, 0xF5, 0xF9),
            text_muted: Color32::from_rgb(0xCB, 0xD5, 0xE1),
            canvas_bg: Color32::from_rgb(0x0B, 0x12, 0x20),
            panel_bg: Color32::from_rgb(0x16, 0x20, 0x32),
            surface_bg: Color32::from_rgb(0x1A, 0x23, 0x32),
            header_bg: Color32::from_rgb(0x11, 0x18, 0x27),
            border: Color32::from_rgb(0x8B, 0xA3, 0xBD),
            accent: Color32::from_rgb(0x02, 0x84, 0xC7),
            accent_text: Color32::WHITE,
            button_bg: Color32::from_rgb(0x3D, 0x52, 0x6E),
            button_hover: Color32::from_rgb(0x52, 0x65, 0x7A),
            tab_inactive_bg: Color32::from_rgb(0x15, 0x1D, 0x2E),
            tab_inactive_text: Color32::from_rgb(0x94, 0xA3, 0xB8),
            stop_bg: Color32::from_rgb(0xEF, 0x44, 0x44),
            input_bg: Color32::from_rgb(0x24, 0x30, 0x49),
        }
    }
}

pub fn apply_theme(ctx: &egui::Context, light: bool) -> Tokens {
    let t = if light {
        Tokens::light()
    } else {
        Tokens::dark()
    };
    let mut visuals = if light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    visuals.window_fill = t.panel_bg;
    visuals.panel_fill = t.canvas_bg;
    visuals.extreme_bg_color = t.input_bg;
    visuals.widgets.noninteractive.bg_fill = t.surface_bg;
    visuals.widgets.inactive.bg_fill = t.button_bg;
    visuals.widgets.hovered.bg_fill = t.button_hover;
    visuals.widgets.active.bg_fill = t.accent;
    visuals.selection.bg_fill = t.accent;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, t.text_primary);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.border);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, t.border);
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(4);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style.visuals = visuals;
    ctx.set_style(style);
    t
}

pub fn accent_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    ui.add_sized(
        [w, 30.0],
        egui::Button::new(
            egui::RichText::new(label.into())
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.accent)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(5)),
    )
}

pub fn secondary_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    ui.add_sized(
        [w, 26.0],
        egui::Button::new(egui::RichText::new(label.into()).color(t.text_primary))
            .fill(t.button_bg)
            .stroke(Stroke::new(1.0_f32, t.border))
            .corner_radius(CornerRadius::same(5)),
    )
}

pub fn stop_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    ui.add_sized(
        [w, 30.0],
        egui::Button::new(
            egui::RichText::new(label.into())
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.stop_bg)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(5)),
    )
}
