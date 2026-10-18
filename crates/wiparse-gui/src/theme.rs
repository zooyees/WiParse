//! Industrial × Apple-minimal design tokens and shared controls.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, Vec2, Visuals,
};

/// Control / card radii (Apple-like consistency).
pub const RADIUS_CTRL: u8 = 6;
pub const RADIUS_CARD: u8 = 8;
pub const RADIUS_DIALOG: u8 = 10;

/// Spacing grid: 4 / 8 / 12 / 16.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;

/// Unified control height.
pub const CTRL_H: f32 = 28.0;
pub const SIDEBAR_W: f32 = 272.0;

/// Width bounds always fit both panes, including unusually small windows.
fn sidebar_width_bounds(total: f32, gap: f32, min_side: f32, min_main: f32) -> (f32, f32) {
    let budget = (total - gap).max(0.0);
    let main = min_main.max(budget * 0.30).min(budget * 0.55);
    let max = (budget - main).max(0.0);
    (min_side.min(max).max(0.0), max)
}

/// Shared draggable gutter for manually allocated tool layouts. Context memory
/// keeps an independent preferred width per tool without changing config files.
pub fn resizable_sidebar_width(
    ui: &mut egui::Ui,
    key: &str,
    body: Rect,
    default: f32,
    min_side: f32,
    min_main: f32,
    gap: f32,
    tokens: &Tokens,
) -> f32 {
    let id = ui.make_persistent_id(("tool_sidebar_width", key));
    let preferred = ui.ctx().data(|data| data.get_temp::<f32>(id)).unwrap_or(default);
    let (min, max) = if matches!(key, "serial_tool" | "waveform_analysis") {
        // These data viewers may be narrowed further without changing other tools.
        let budget = (body.width() - gap).max(0.0);
        let main = min_main.max(budget * 0.20).min(budget * 0.55);
        let max = (budget - main).max(0.0);
        (min_side.min(max).max(0.0), max)
    } else {
        sidebar_width_bounds(body.width(), gap, min_side, min_main)
    };
    let mut width = preferred.clamp(min, max);
    let gutter = Rect::from_min_max(
        egui::pos2(body.left() + width, body.top()),
        egui::pos2((body.left() + width + gap).min(body.right()), body.bottom()),
    );
    let response = ui.interact(gutter, id.with("drag"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("拖动调整侧栏宽度 / Drag to resize\n双击恢复默认 / Double-click to reset");
    if response.double_clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(id, default));
        width = default.clamp(min, max);
    } else if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        width = (width + response.drag_delta().x).clamp(min, max);
        ui.ctx().data_mut(|data| data.insert_temp(id, width));
    }
    let x = body.left() + width + gap * 0.5;
    ui.painter().line_segment(
        [egui::pos2(x, body.top()), egui::pos2(x, body.bottom())],
        Stroke::new(if response.hovered() || response.dragged() { 2.0 } else { 1.0 },
            if response.hovered() || response.dragged() { tokens.accent } else { tokens.divider }),
    );
    width
}

/// Type scale.
pub const FONT_CAPTION: f32 = 12.0;
pub const FONT_BODY: f32 = 13.0;
pub const FONT_TITLE: f32 = 14.0;
pub const FONT_SECTION: f32 = 16.0;

#[derive(Clone, Copy)]
pub struct Tokens {
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub canvas_bg: Color32,
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub header_bg: Color32,
    pub border: Color32,
    pub divider: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub accent_soft: Color32,
    pub button_bg: Color32,
    pub button_hover: Color32,
    pub tab_inactive_bg: Color32,
    pub tab_inactive_text: Color32,
    pub stop_bg: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub input_bg: Color32,
    pub focus_ring: Color32,
    /// Plot / media inset background (may stay slightly contrasting).
    pub plot_bg: Color32,
    pub plot_border: Color32,
    /// Tick / placeholder text drawn on `plot_bg`.
    pub plot_fg: Color32,
    /// True when this token set is the near-black scope workbench.
    pub scope_dark: bool,
}

impl Tokens {
    /// Light theme — clean Apple-like office chrome; works for all tools.
    pub fn light() -> Self {
        Self {
            text_primary: Color32::from_rgb(0x1D, 0x1D, 0x1F),
            text_muted: Color32::from_rgb(0x6E, 0x6E, 0x73),
            canvas_bg: Color32::from_rgb(0xF5, 0xF5, 0xF7),
            panel_bg: Color32::WHITE,
            surface_bg: Color32::WHITE,
            header_bg: Color32::from_rgb(0xFA, 0xFA, 0xFC),
            border: Color32::from_rgb(0xD2, 0xD2, 0xD7),
            divider: Color32::from_rgb(0xE8, 0xE8, 0xED),
            // Slightly quieter than pure systemBlue — still clear as CTA.
            accent: Color32::from_rgb(0x00, 0x66, 0xD0),
            accent_text: Color32::WHITE,
            accent_soft: Color32::from_rgba_unmultiplied(0x00, 0x66, 0xD0, 28),
            button_bg: Color32::from_rgb(0xF2, 0xF2, 0xF7),
            button_hover: Color32::from_rgb(0xE5, 0xE5, 0xEA),
            tab_inactive_bg: Color32::TRANSPARENT,
            tab_inactive_text: Color32::from_rgb(0x6E, 0x6E, 0x73),
            stop_bg: Color32::from_rgb(0xFF, 0x3B, 0x30),
            success: Color32::from_rgb(0x34, 0xC7, 0x59),
            warning: Color32::from_rgb(0xFF, 0x9F, 0x0A),
            input_bg: Color32::from_rgb(0xF2, 0xF2, 0xF7),
            focus_ring: Color32::from_rgba_unmultiplied(0x00, 0x66, 0xD0, 90),
            // Soft charcoal well so Tek channel colors stay readable in light chrome.
            plot_bg: Color32::from_rgb(0x2C, 0x2C, 0x2E),
            plot_border: Color32::from_rgb(0xC7, 0xC7, 0xCC),
            plot_fg: Color32::from_rgb(0xE5, 0xE5, 0xEA),
            scope_dark: false,
        }
    }

    /// Global dark (settings preference).
    pub fn dark() -> Self {
        Self::scope()
    }

    /// Scope / instrument workbench (near-black).
    pub fn scope() -> Self {
        Self {
            text_primary: Color32::from_rgb(0xF5, 0xF5, 0xF7),
            // Brighter muted text so secondary / disabled labels stay readable.
            text_muted: Color32::from_rgb(0xAE, 0xAE, 0xB2),
            canvas_bg: Color32::from_rgb(0x0C, 0x0C, 0x0E),
            panel_bg: Color32::from_rgb(0x16, 0x16, 0x18),
            surface_bg: Color32::from_rgb(0x1C, 0x1C, 0x1E),
            header_bg: Color32::from_rgb(0x12, 0x12, 0x14),
            border: Color32::from_rgb(0x3A, 0x3A, 0x3C),
            divider: Color32::from_rgb(0x2C, 0x2C, 0x2E),
            // Desaturated mid-blue — readable CTA without neon glare on near-black.
            accent: Color32::from_rgb(0x3D, 0x7E, 0xC8),
            accent_text: Color32::WHITE,
            accent_soft: Color32::from_rgba_unmultiplied(0x3D, 0x7E, 0xC8, 40),
            button_bg: Color32::from_rgb(0x2C, 0x2C, 0x2E),
            button_hover: Color32::from_rgb(0x3A, 0x3A, 0x3C),
            tab_inactive_bg: Color32::TRANSPARENT,
            tab_inactive_text: Color32::from_rgb(0xC7, 0xC7, 0xCC),
            stop_bg: Color32::from_rgb(0xFF, 0x45, 0x3A),
            success: Color32::from_rgb(0x30, 0xD1, 0x58),
            warning: Color32::from_rgb(0xFF, 0xD6, 0x0A),
            input_bg: Color32::from_rgb(0x1C, 0x1C, 0x1E),
            focus_ring: Color32::from_rgba_unmultiplied(0x3D, 0x7E, 0xC8, 110),
            plot_bg: Color32::from_rgb(0x0C, 0x0C, 0x0E),
            plot_border: Color32::from_rgb(0x3A, 0x3A, 0x3C),
            plot_fg: Color32::from_rgb(0xAE, 0xAE, 0xB2),
            scope_dark: true,
        }
    }
}

/// Style the current `Ui` for a scope plot well (dark inset + readable ticks), then restore.
pub fn with_plot_well_visuals<R>(
    ui: &mut egui::Ui,
    t: &Tokens,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let prev = ui.visuals().clone();
    {
        let v = ui.visuals_mut();
        v.extreme_bg_color = t.plot_bg;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.plot_border);
        if !t.scope_dark {
            v.override_text_color = Some(t.plot_fg);
        }
    }
    let out = add(ui);
    *ui.visuals_mut() = prev;
    out
}

/// Apply egui visuals from tokens. `light_chrome` selects egui light/dark widget defaults.
pub fn apply_tokens(ctx: &egui::Context, t: Tokens) -> Tokens {
    let light_chrome = !t.scope_dark;
    let mut visuals = if light_chrome {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    visuals.override_text_color = None;
    visuals.window_fill = t.panel_bg;
    visuals.panel_fill = t.canvas_bg;
    visuals.extreme_bg_color = t.input_bg;
    visuals.faint_bg_color = t.surface_bg;
    // Widget fills
    visuals.widgets.noninteractive.bg_fill = t.surface_bg;
    visuals.widgets.noninteractive.weak_bg_fill = t.surface_bg;
    visuals.widgets.inactive.bg_fill = t.button_bg;
    visuals.widgets.inactive.weak_bg_fill = t.button_bg;
    visuals.widgets.hovered.bg_fill = t.button_hover;
    visuals.widgets.hovered.weak_bg_fill = t.button_hover;
    // Pressed fill stays elevated gray — not neon accent (CTA buttons set fill explicitly).
    visuals.widgets.active.bg_fill = t.button_hover;
    visuals.widgets.active.weak_bg_fill = t.button_hover;
    visuals.widgets.open.bg_fill = t.button_hover;
    visuals.widgets.open.weak_bg_fill = t.button_hover;
    visuals.selection.bg_fill = t.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0_f32, t.accent);
    // IMPORTANT: `RichText::strong()` and CollapsingHeader titles resolve via
    // `Visuals::strong_text_color()` → `widgets.active.fg_stroke`. That must stay
    // `text_primary`, never white CTA text — otherwise light mode tree labels vanish.
    let fg = Stroke::new(1.0_f32, t.text_primary);
    visuals.widgets.noninteractive.fg_stroke = fg;
    visuals.widgets.inactive.fg_stroke = fg;
    visuals.widgets.hovered.fg_stroke = fg;
    visuals.widgets.active.fg_stroke = fg;
    visuals.widgets.open.fg_stroke = fg;
    let stroke = Stroke::new(1.0_f32, t.border);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, t.divider);
    visuals.widgets.inactive.bg_stroke = stroke;
    visuals.widgets.hovered.bg_stroke = stroke;
    visuals.widgets.active.bg_stroke = stroke;
    visuals.widgets.open.bg_stroke = stroke;
    visuals.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_CTRL);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_CTRL);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_CTRL);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_CTRL);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_XS + 1.0);
    // Vertical pad must not push a CTRL_H button past 28.
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.window_margin = egui::Margin::same(SPACE_SM as i8);
    style.interaction.show_tooltips_only_when_still = true;
    style.visuals = visuals;
    ctx.set_style(style);
    t
}

/// Backward-compatible entry: light office vs scope dark.
pub fn apply_theme(ctx: &egui::Context, light: bool) -> Tokens {
    let t = if light {
        Tokens::light()
    } else {
        Tokens::scope()
    };
    apply_tokens(ctx, t)
}

/// Respect user light/dark preference for every tool (no forced scope override).
pub fn tokens_for_preference(light_pref: bool) -> Tokens {
    if light_pref {
        Tokens::light()
    } else {
        Tokens::scope()
    }
}

/// Backward-compatible alias.
pub fn tokens_for_panel(light_pref: bool, _scope_workbench: bool) -> Tokens {
    tokens_for_preference(light_pref)
}

fn btn_radius() -> CornerRadius {
    CornerRadius::same(RADIUS_CTRL)
}

/// Primary CTA — one per screen region.
pub fn primary_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    let resp = ui.add_sized(
        [w, CTRL_H],
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.accent)
        .stroke(Stroke::NONE)
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

/// Alias kept for existing call sites.
pub fn accent_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    primary_button(ui, t, label)
}

pub fn secondary_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    let resp = ui.add_sized(
        [w, CTRL_H],
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.text_primary),
        )
        .fill(t.button_bg)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

pub fn ghost_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    let resp = ui.add_sized(
        [w, CTRL_H],
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.text_primary),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, t.divider))
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

pub fn stop_button(ui: &mut egui::Ui, t: &Tokens, label: impl Into<String>) -> egui::Response {
    let w = ui.available_width();
    let resp = ui.add_sized(
        [w, CTRL_H],
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.stop_bg)
        .stroke(Stroke::NONE)
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

/// Fixed-size primary (toolbars).
pub fn primary_btn_sized(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
) -> egui::Response {
    let resp = ui.add_sized(
        size,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.accent)
        .stroke(Stroke::NONE)
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

pub fn primary_btn_sized_enabled(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
    enabled: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| primary_btn_sized(ui, t, label, size))
        .inner
}

pub fn stop_btn_sized(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
) -> egui::Response {
    let resp = ui.add_sized(
        size,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.accent_text)
                .strong(),
        )
        .fill(t.stop_bg)
        .stroke(Stroke::NONE)
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

pub fn secondary_btn_sized(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
) -> egui::Response {
    let resp = ui.add_sized(
        size,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(t.text_primary),
        )
        .fill(t.button_bg)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

pub fn ghost_btn_sized(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
    selected: bool,
) -> egui::Response {
    // Selected = quiet elevated chip (not a second neon CTA).
    let (fill, stroke, fg) = if selected {
        (
            t.button_hover,
            Stroke::new(1.0_f32, t.accent),
            t.text_primary,
        )
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::new(1.0_f32, t.divider),
            t.text_primary,
        )
    };
    let resp = ui.add_sized(
        size,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(fg)
                .strong(),
        )
        .fill(fill)
        .stroke(stroke)
        .corner_radius(btn_radius()),
    );
    paint_focus_ring(ui, t, &resp);
    resp
}

/// Secondary control that stays readable when disabled (avoids over-dark gray-out).
pub fn secondary_btn_sized_enabled(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
    enabled: bool,
) -> egui::Response {
    let fg = if enabled { t.text_primary } else { t.text_muted };
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(fg),
        )
        .fill(t.button_bg)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(btn_radius())
        .min_size(size),
    )
}

pub fn ghost_btn_sized_enabled(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: impl Into<String>,
    size: Vec2,
    enabled: bool,
) -> egui::Response {
    let fg = if enabled { t.text_primary } else { t.text_muted };
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label.into())
                .size(FONT_TITLE)
                .color(fg),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, t.divider))
        .corner_radius(btn_radius())
        .min_size(size),
    )
}

fn paint_focus_ring(ui: &egui::Ui, t: &Tokens, resp: &egui::Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            resp.rect.expand(1.5),
            CornerRadius::same(RADIUS_CTRL + 1),
            Stroke::new(1.5_f32, t.focus_ring),
            egui::StrokeKind::Outside,
        );
    }
}

/// Two-option segmented control. Returns `Some(want_left)` if selection changed.
///
/// Segments are laid out into exact rects with a gap so the two rounded
/// chips cannot wrap or paint on top of each other.
pub fn segmented_two(
    ui: &mut egui::Ui,
    t: &Tokens,
    left: &str,
    right: &str,
    left_selected: bool,
    size: Vec2,
) -> Option<bool> {
    let gap = SPACE_XS;
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let half_w = ((rect.width() - gap) * 0.5).max(1.0);
    let left_rect = Rect::from_min_size(rect.min, Vec2::new(half_w, rect.height()));
    let right_x = left_rect.max.x + gap;
    let right_rect = Rect::from_min_max(
        egui::pos2(right_x, rect.min.y),
        egui::pos2(rect.max.x, rect.max.y),
    );
    let mut out = None;
    if ghost_btn_in_rect(ui, t, left, left_rect, left_selected).clicked() && !left_selected {
        out = Some(true);
    }
    if ghost_btn_in_rect(ui, t, right, right_rect, !left_selected).clicked() && left_selected {
        out = Some(false);
    }
    out
}

fn ghost_btn_in_rect(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    rect: Rect,
    selected: bool,
) -> egui::Response {
    in_allocated_rect(
        ui,
        rect,
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ghost_btn_sized(ui, t, label, rect.size(), selected)
        },
    )
}

/// Paint into a rectangle already reserved by the caller. Unlike scope_builder,
/// this does not allocate again or move the parent's cursor backwards when a
/// title, inset, or first column is shorter than the reserved row.
/// Draw an absolute child without advancing its parent's already reserved area.
/// Use normal `scope_builder` for children that should allocate flow space.
pub fn scope_in_allocated_rect<R>(
    ui: &mut egui::Ui,
    builder: egui::UiBuilder,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut child = ui.new_child(builder);
    child.set_clip_rect(child.max_rect().intersect(ui.clip_rect()));
    let inner = add(&mut child);
    egui::InnerResponse::new(inner, child.response())
}

pub fn in_allocated_rect<R>(
    ui: &mut egui::Ui,
    rect: Rect,
    layout: egui::Layout,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.set_min_size(rect.size());
    child.set_max_size(rect.size());
    add(&mut child)
}

/// Status dot + caption for the bottom bar.
pub fn status_line(ui: &mut egui::Ui, t: &Tokens, tone: StatusTone, text: &str) {
    let color = match tone {
        StatusTone::Neutral => t.text_muted,
        StatusTone::Ok => t.success,
        StatusTone::Warn => t.warning,
        StatusTone::Error => t.stop_bg,
        StatusTone::Busy => t.accent,
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, CTRL_H.min(18.0)), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.5, color);
    ui.add_space(SPACE_XS);
    ui.label(
        RichText::new(text)
            .size(FONT_BODY)
            .color(t.text_muted),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Ok,
    Warn,
    Error,
    Busy,
}

/// Infer a tone from common status strings (zh/en keywords).
pub fn tone_from_status(s: &str) -> StatusTone {
    let l = s.to_ascii_lowercase();
    if l.contains("fail")
        || l.contains("error")
        || l.contains("失败")
        || l.contains("错误")
        || l.contains("超时")
    {
        StatusTone::Error
    } else if l.contains("warn") || l.contains("警告") {
        StatusTone::Warn
    } else if l.contains("load")
        || l.contains("…")
        || l.contains("ing")
        || l.contains("连接中")
        || l.contains("加载")
        || l.contains("计算中")
        || l.contains("decoding")
        || l.contains("扫描")
    {
        StatusTone::Busy
    } else if l.contains("ok")
        || l.contains("ready")
        || l.contains("connected")
        || l.contains("loaded")
        || l.contains("就绪")
        || l.contains("已连接")
        || l.contains("已加载")
        || l.contains("stopped")
        || l.contains("已停止")
    {
        StatusTone::Ok
    } else {
        StatusTone::Neutral
    }
}

/// Quiet section frame (weak border) for side panels.
pub fn section_frame(t: &Tokens) -> Frame {
    Frame::NONE
        .fill(t.surface_bg)
        .stroke(Stroke::new(1.0_f32, t.divider))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::symmetric(SPACE_SM as i8, SPACE_SM as i8 - 2))
}

pub fn section_title(ui: &mut egui::Ui, t: &Tokens, title: &str) {
    ui.label(
        RichText::new(title)
            .size(FONT_TITLE)
            .color(t.text_muted)
            .strong(),
    );
    ui.add_space(SPACE_XS);
}

/// Form label + field row helper.
pub fn field_label(ui: &mut egui::Ui, t: &Tokens, label: &str, label_w: f32, row_h: f32) {
    ui.add_sized(
        [label_w, row_h],
        egui::Label::new(
            RichText::new(label)
                .size(FONT_BODY)
                .color(t.text_muted),
        ),
    );
}

pub fn caption(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.label(
        RichText::new(text)
            .size(FONT_CAPTION)
            .color(t.text_muted),
    );
}

pub fn mono_body(text: impl Into<String>, t: &Tokens) -> RichText {
    RichText::new(text.into())
        .font(FontId::monospace(FONT_BODY))
        .color(t.text_primary)
}

/// Equal column widths for one row. `sum(widths) + SPACE_SM * (n - 1) == row_width`
/// when `row_width` is large enough; otherwise each column is at least 1.
pub fn equal_widths(row_width: f32, n: usize) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    let gaps = SPACE_SM * (n.saturating_sub(1) as f32);
    let each = ((row_width - gaps) / n as f32).max(1.0);
    vec![each; n]
}

/// Text field whose outer frame is `outer_w` by `CTRL_H`.
/// `TextEdit::desired_width` is the text area; margin sits outside it.
pub fn singleline_sized<'a>(
    text: &'a mut String,
    outer_w: f32,
) -> egui::TextEdit<'a> {
    const MARGIN_X: f32 = 6.0;
    egui::TextEdit::singleline(text)
        .desired_width((outer_w - MARGIN_X * 2.0).max(1.0))
        .margin(egui::vec2(MARGIN_X, 4.0))
}

pub fn add_singleline(
    ui: &mut egui::Ui,
    text: &mut String,
    outer_w: f32,
) -> egui::Response {
    ui.add_sized([outer_w, CTRL_H], singleline_sized(text, outer_w))
}

#[cfg(test)]
mod tests {
    use super::{equal_widths, SPACE_SM};

    #[test]
    fn sidebar_bounds_fit_small_windows_and_reserve_main_content() {
        for total in [0.0, 4.0, 8.0, 40.0, 160.0, 640.0, 1920.0] {
            let (min, max) = super::sidebar_width_bounds(total, 8.0, 220.0, 280.0);
            assert!(min >= 0.0 && max >= min);
            assert!(max <= (total - 8.0_f32).max(0.0));
            if total > 8.0 { assert!(max < total - 8.0); }
        }
    }

    #[test]
    fn sidebar_drag_remembers_independent_widths_and_double_click_resets() {
        use egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};
        let ctx = egui::Context::default();
        let tokens = super::Tokens::dark();
        let mut time = 0.0;
        let mut frame = |key: &str, screen_w: f32, events: Vec<Event>| {
            time += 0.05;
            let mut width = 0.0;
            let output = ctx.run(RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(screen_w, 600.0))),
                time: Some(time), events, ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (body, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                    width = super::resizable_sidebar_width(ui, key, body, 272.0, 220.0, 200.0, 8.0, &tokens);
                });
            });
            (width, output.platform_output.cursor_icon)
        };
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() };
        let start = Pos2::new(8.0 + 272.0 + 4.0, 100.0);
        assert_eq!(frame("wave", 900.0, vec![]).0, 272.0);
        assert_eq!(frame("wave", 900.0, vec![Event::PointerMoved(start)]).1, egui::CursorIcon::ResizeHorizontal);
        frame("wave", 900.0, vec![button(start, true)]);
        let end = start + Vec2::new(60.0, 0.0);
        assert_eq!(frame("wave", 900.0, vec![Event::PointerMoved(end)]).0, 332.0);
        frame("wave", 900.0, vec![button(end, false)]);
        assert_eq!(frame("wave", 900.0, vec![]).0, 332.0);
        assert_eq!(frame("serial", 900.0, vec![]).0, 272.0);
        assert!(frame("wave", 160.0, vec![]).0 < 160.0);
        assert_eq!(frame("wave", 900.0, vec![]).0, 332.0);
        frame("wave", 900.0, vec![Event::PointerMoved(end), button(end, true)]);
        frame("wave", 900.0, vec![button(end, false)]);
        frame("wave", 900.0, vec![button(end, true)]);
        assert_eq!(frame("wave", 900.0, vec![button(end, false)]).0, 272.0);
    }

    #[test]
    fn inset_children_do_not_rewind_reserved_rows() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (row, _) = ui.allocate_exact_size(egui::vec2(360.0, 60.0), egui::Sense::hover());
                let after = ui.cursor();
                for inset in [0.0, 4.0, 10.0] {
                    super::scope_in_allocated_rect(ui,
                        egui::UiBuilder::new().max_rect(row.shrink(inset)), |ui| {
                            ui.label("short child");
                        });
                    assert_eq!(ui.cursor(), after);
                }
                super::in_allocated_rect(ui, row.shrink(8.0),
                    egui::Layout::left_to_right(egui::Align::Center), |ui| { ui.button("button"); });
                assert_eq!(ui.cursor(), after);
                let next = ui.label("next row");
                assert!(next.rect.top() >= row.bottom());
            });
        });
    }

    #[test]
    fn equal_widths_fill_the_row() {
        for n in [2usize, 4] {
            let row = 272.0;
            let parts = equal_widths(row, n);
            assert_eq!(parts.len(), n);
            let sum = parts.iter().sum::<f32>() + SPACE_SM * (n - 1) as f32;
            assert!((sum - row).abs() < 0.02, "n={n} sum={sum}");
        }
    }
}

