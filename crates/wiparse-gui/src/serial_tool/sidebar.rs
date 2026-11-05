//! Responsive actions local to the serial connection and directory cards.
use super::*;

pub(super) fn action_pair(
    ui: &mut egui::Ui,
    t: &Tokens,
    width: f32,
    first: &str,
    second: &str,
    second_enabled: bool,
) -> (egui::Response, egui::Response) {
    let gap = ui_theme::SPACE_SM;
    let column = ((width - gap) / 2.0).max(1.0);
    let needed = |text: &str| {
        ui.painter()
            .layout_no_wrap(
                text.into(),
                egui::FontId::proportional(ui_theme::FONT_TITLE),
                t.text_primary,
            )
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
            + 2.0
    };
    let stacked = needed(first).max(needed(second)) > column;
    ui.scope(|ui| {
        // add_sized alone is a minimum; truncation keeps extreme widths bounded.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let button = |ui: &mut egui::Ui, label: &str, enabled: bool, w: f32| {
            ui.add_enabled_ui(enabled, |ui| {
                ui_theme::secondary_btn_sized(ui, t, label, egui::vec2(w, ui_theme::CTRL_H))
            })
            .inner
            .on_hover_text(label)
        };
        if stacked {
            (
                button(ui, first, true, width),
                button(ui, second, second_enabled, width),
            )
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                (
                    button(ui, first, true, column),
                    button(ui, second, second_enabled, column),
                )
            })
            .inner
        }
    })
    .inner
}

#[cfg(test)]
mod tests;
