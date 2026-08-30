//! Virtualized log viewport — O(visible rows) memory, disk-backed line fetch.

use std::sync::Arc;

use crate::theme::Tokens;
use egui::{
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    Align, Align2, Color32, Event, FontId, ImeEvent, Key, Layout, Margin, Pos2, Rect, Sense, Stroke,
    Ui, UiBuilder, Vec2,
};
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::log::{match_ranges_in_line, LogStore};

use super::log_tab::{
    apply_line_parse, PaneState, SearchSelectionState, TextCaret, LOG_FONT_MAX, LOG_FONT_MIN,
    LOG_FONT_STEP,
};

const TEXT_LEFT_PAD: f32 = 4.0;
const SEARCH_PREFIX_CELLS: f32 = 27.0;
/// Pointer movement below this (points) counts as a click, not a drag-select.
const SELECT_DRAG_THRESHOLD: f32 = 4.0;
/// Cap clipboard copies so Ctrl+C on a huge selection cannot freeze the UI.
const MAX_COPY_ROWS: usize = 20_000;

/// Hit-test rect for text selection: the visible viewport minus scrollbar overlay.
///
/// Use the clip (not only the text body) so a click in empty padding below the
/// last line can still dismiss the selection.
fn text_select_interact_rect(ui: &Ui, _body: Rect, clip: Rect) -> Rect {
    let gutter = scroll_bar_gutter(ui);
    let mut rect = clip;
    if rect.width() > gutter + 16.0 {
        rect.max.x -= gutter;
    }
    if rect.height() > gutter + 16.0 {
        rect.max.y -= gutter;
    }
    rect
}

fn pos_in_line_block(body: Rect, pos: Pos2, line_h: f32, total: usize) -> bool {
    if total == 0 || line_h <= 0.0 {
        return false;
    }
    let bottom = body.top() + total as f32 * line_h;
    pos.y >= body.top() && pos.y < bottom
}

/// Search-hit list: only reserve the *vertical* scrollbar strip.
///
/// Insetting the bottom (horizontal bar) makes the last visible row almost
/// entirely unclickable when scrolled to the end — the whole line sits in the
/// dead zone. Jump clicks matter more than grabbing the H-bar on that row.
fn search_interact_rect(ui: &Ui, body: Rect, clip: Rect) -> Rect {
    let gutter = scroll_bar_gutter(ui);
    let mut rect = body.intersect(clip);
    if rect.width() > gutter + 16.0 {
        rect.max.x -= gutter;
    }
    rect
}

fn scroll_bar_gutter(ui: &Ui) -> f32 {
    let scroll = ui.spacing().scroll;
    scroll.bar_width
        + scroll.bar_outer_margin
        + scroll.bar_inner_margin
        + scroll.floating_width
        + 4.0
}

fn search_row_at_pos(body: Rect, line_h: f32, row_count: usize, pos: Pos2) -> Option<usize> {
    if row_count == 0 || line_h <= 0.0 {
        return None;
    }
    let rows_bottom = body.top() + row_count as f32 * line_h;
    if pos.y < body.top() || pos.y >= rows_bottom {
        return None;
    }
    let row = ((pos.y - body.top()) / line_h).floor().max(0.0) as usize;
    (row < row_count).then_some(row)
}

#[derive(Clone, Copy)]
struct LineParts<'a> {
    prefix: &'a str,
    rest: &'a str,
}

struct FrozenRow {
    top: f32,
    bottom: f32,
    galley: Arc<egui::Galley>,
    selection_x: Option<(f32, f32)>,
}

fn frozen_column_clip(viewport: Rect, prefix_width: f32) -> Rect {
    Rect::from_min_max(
        viewport.min,
        Pos2::new(
            (viewport.left() + prefix_width).min(viewport.right()),
            viewport.bottom(),
        ),
    )
}

fn paint_frozen_rows(
    ui: &Ui,
    viewport: Rect,
    prefix_width: f32,
    line_height: f32,
    rows: Vec<FrozenRow>,
    text_color: Color32,
) {
    let clip = frozen_column_clip(viewport, prefix_width);
    let painter = ui.painter().with_clip_rect(clip);
    for row in rows {
        if let Some((start_x, end_x)) = row.selection_x {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(viewport.left() + TEXT_LEFT_PAD + start_x, row.top),
                    Pos2::new(viewport.left() + TEXT_LEFT_PAD + end_x, row.bottom),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(59, 130, 246, 72),
            );
        }
        let y = row.top + (line_height - row.galley.size().y).max(0.0) * 0.5;
        painter.galley(
            Pos2::new(viewport.left() + TEXT_LEFT_PAD, y),
            row.galley,
            text_color,
        );
    }
}

fn split_timestamp_prefix(line: &str) -> LineParts<'_> {
    // Some text sources retain a BOM or leading horizontal whitespace. Treat
    // those bytes as part of the fixed column instead of silently disabling
    // timestamp freezing for the whole line.
    let mut timestamp_start = usize::from(line.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let mut leading_padding = 0;
    while leading_padding < 2 && matches!(line.as_bytes().get(timestamp_start), Some(b' ' | b'\t'))
    {
        timestamp_start += 1;
        leading_padding += 1;
    }
    let all_bytes = line.as_bytes();
    // Imported logs may use a source marker before the timestamp, e.g.
    // `[R][20:30:30.616]`. It belongs to the same fixed metadata column.
    if all_bytes.get(timestamp_start) == Some(&b'[') {
        if let Some(tag_end) = all_bytes[timestamp_start..]
            .iter()
            .position(|&byte| byte == b']')
            .filter(|&end| end <= 8)
        {
            let next = timestamp_start + tag_end + 1;
            if all_bytes.get(next) == Some(&b'[') {
                timestamp_start = next;
            }
        }
    }
    let bytes = &all_bytes[timestamp_start..];
    // `[HH:MM:SS.mmm]` at the start of a line. Keep one following space in
    // the frozen prefix so payload text has a stable visual separator.
    let timestamp = bytes.len() >= 14
        && bytes[0] == b'['
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3] == b':'
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_digit()
        && bytes[6] == b':'
        && bytes[7].is_ascii_digit()
        && bytes[8].is_ascii_digit()
        // Accept legacy Rust captures that used ':' before milliseconds,
        // while new captures use the canonical Python-compatible '.'.
        && matches!(bytes[9], b'.' | b':')
        && bytes[10].is_ascii_digit()
        && bytes[11].is_ascii_digit()
        && bytes[12].is_ascii_digit()
        && bytes[13] == b']';
    if timestamp {
        let timestamp_end = if bytes.get(14).is_some_and(u8::is_ascii_whitespace) {
            15
        } else {
            14
        };
        let end = timestamp_start + timestamp_end;
        LineParts {
            prefix: &line[..end],
            rest: &line[end..],
        }
    } else {
        LineParts {
            prefix: "",
            rest: line,
        }
    }
}

fn line_gutter_width(total: usize, glyph_w: f32) -> f32 {
    let digits = if total == 0 {
        1
    } else {
        ((total as f32).log10().floor() as usize) + 1
    };
    digits as f32 * glyph_w + 10.0
}

fn word_char_range(line: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let idx = col.min(chars.len().saturating_sub(1));
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '-';
    if chars[idx].is_whitespace() {
        return (col.min(chars.len()), col.min(chars.len()));
    }
    let class = is_word(chars[idx]);
    let mut start = idx;
    while start > 0 {
        let prev = chars[start - 1];
        if prev.is_whitespace() || is_word(prev) != class {
            break;
        }
        start -= 1;
    }
    let mut end = idx + 1;
    while end < chars.len() {
        let next = chars[end];
        if next.is_whitespace() || is_word(next) != class {
            break;
        }
        end += 1;
    }
    (start, end)
}

fn row_height(ui: &Ui, font_size: f32) -> f32 {
    let font = FontId::monospace(font_size);
    ui.fonts(|fonts| {
        let family_height = fonts.row_height(&font);
        let fallback_height = fonts
            .layout_no_wrap("Ag中�".to_owned(), font, Color32::WHITE)
            .size()
            .y;
        family_height.max(fallback_height) + 4.0
    })
}

fn visible_row_range(
    body_top: f32,
    clip_top: f32,
    clip_height: f32,
    line_height: f32,
    total: usize,
) -> std::ops::Range<usize> {
    if total == 0 || line_height <= 0.0 {
        return 0..0;
    }
    let first = ((clip_top - body_top).max(0.0) / line_height).floor() as usize;
    let visible = (clip_height.max(0.0) / line_height).ceil() as usize + 2;
    first.min(total)..(first.saturating_add(visible)).min(total)
}

fn vertical_offset_for_row(row: usize, line_height: f32, viewport_height: f32) -> f32 {
    (row as f32 * line_height - (viewport_height - line_height).max(0.0) * 0.5).max(0.0)
}

fn byte_to_char_index(line: &str, byte: usize) -> usize {
    line.char_indices().take_while(|(i, _)| *i < byte).count()
}

fn horizontal_offset_for_hit(
    start_byte: usize,
    line: &str,
    glyph_w: f32,
    gutter_w: f32,
    viewport_w: f32,
) -> f32 {
    let col = byte_to_char_index(line, start_byte) as f32;
    let x = gutter_w + TEXT_LEFT_PAD + col * glyph_w;
    (x - viewport_w * 0.25).max(0.0)
}

fn layout_log_line(ui: &Ui, line: &str, font_size: f32, color: Color32) -> Arc<egui::Galley> {
    ui.fonts(|fonts| {
        fonts.layout_no_wrap(line.to_owned(), FontId::monospace(font_size), color)
    })
}

fn log_row_text_pos(row_rect: Rect, line_height: f32, galley: &egui::Galley) -> Pos2 {
    Pos2::new(
        row_rect.left() + TEXT_LEFT_PAD,
        row_rect.top() + (line_height - galley.size().y).max(0.0) * 0.5,
    )
}

/// Same coordinate transform as `TextEdit`: `galley.cursor_from_pos(pointer - galley_pos)`.
fn caret_col_from_pointer(
    galley: &egui::Galley,
    line_char_len: usize,
    text_pos: Pos2,
    pointer: Pos2,
) -> usize {
    galley
        .cursor_from_pos(pointer - text_pos)
        .ccursor
        .index
        .min(line_char_len)
}

fn selection_x_range(galley: &egui::Galley, start_col: usize, end_col: usize) -> Option<(f32, f32)> {
    if start_col >= end_col {
        return None;
    }
    let start_x = galley.pos_from_ccursor(CCursor::new(start_col)).left();
    let end_x = galley.pos_from_ccursor(CCursor::new(end_col)).left();
    (end_x > start_x).then_some((start_x, end_x))
}

fn caret_at_pos<S: LogStore + ?Sized>(
    ui: &Ui,
    store: &S,
    body: Rect,
    pos: Pos2,
    line_height: f32,
    font_size: f32,
    gutter: f32,
) -> Option<super::log_tab::TextCaret> {
    let total = store.line_count();
    if total == 0 {
        return None;
    }
    let row = (((pos.y - body.top()) / line_height).floor().max(0.0) as usize)
        .min(total.saturating_sub(1));
    let row_top = body.top() + row as f32 * line_height;
    let text_left = body.left() + gutter;
    let row_rect = Rect::from_min_max(
        Pos2::new(text_left, row_top),
        Pos2::new(body.right(), row_top + line_height),
    );
    let line = store.line_at(row)?;
    let galley = layout_log_line(ui, &line, font_size, Color32::WHITE);
    let text_pos = log_row_text_pos(row_rect, line_height, &galley);
    let col = caret_col_from_pointer(&galley, line.chars().count(), text_pos, pos);
    Some(super::log_tab::TextCaret { row, col })
}

fn handle_copy_shortcut<S: LogStore + ?Sized>(
    ui: &Ui,
    store: &S,
    pane: &mut PaneState,
    active: bool,
) {
    if !active {
        return;
    }
    let typing = ui.ctx().wants_keyboard_input();
    let copy_event = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
    let ctrl_c = ui.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::C,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl || modifiers.command
            )
        })
    });
    let ctrl_a = ui.input(|i| {
        (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::A)
    });
    if ctrl_a {
        if typing {
            return;
        }
        let last = store.line_count().saturating_sub(1);
        let last_col = store
            .line_at(last)
            .map(|line| line.chars().count())
            .unwrap_or(0);
        pane.sel_anchor = Some(super::log_tab::TextCaret { row: 0, col: 0 });
        pane.sel_focus = Some(super::log_tab::TextCaret {
            row: last,
            col: last_col,
        });
        return;
    }
    if copy_event || ctrl_c {
        let text = clipboard_text(store, pane);
        if !text.is_empty() {
            ui.ctx().copy_text(text);
        }
    }
}

fn clipboard_text<S: LogStore + ?Sized>(store: &S, pane: &PaneState) -> String {
    if let Some(text) = copy_selection(store, pane) {
        return text;
    }
    if !pane.last_selected_text.is_empty() {
        return pane.last_selected_text.clone();
    }
    copy_current_line(store, pane).unwrap_or_default()
}

fn copy_current_line<S: LogStore + ?Sized>(store: &S, pane: &PaneState) -> Option<String> {
    let row = pane
        .sel_focus
        .map(|caret| caret.row)
        .or(pane.highlight_master)
        .or(pane.last_view_row)?;
    store.line_at(row).map(|line| line.to_string())
}

/// Render a log store with vertical + horizontal scroll; only visible rows are loaded.
pub fn show_virtual_log_pane<S: LogStore + ?Sized>(
    ui: &mut Ui,
    store: &S,
    pane: &mut PaneState,
    auto_scroll: bool,
    salt: &str,
    height: f32,
    scroll_to_row: Option<usize>,
    highlight_row: Option<usize>,
    t: &Tokens,
    lang: Lang,
    needles: &[String],
    case_sensitive: bool,
    current_hit: Option<(usize, usize, usize)>,
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    let shift = ui.input(|i| i.modifiers.shift);
    let zoom_factor = ui.input(|i| i.zoom_delta());

    let total_lines = store.line_count();
    let line_h = if let Some((fs, h)) = pane.cached_row_height {
        if (fs - pane.font_size).abs() < f32::EPSILON {
            h
        } else {
            let h = row_height(ui, pane.font_size);
            pane.cached_row_height = Some((pane.font_size, h));
            h
        }
    } else {
        let h = row_height(ui, pane.font_size);
        pane.cached_row_height = Some((pane.font_size, h));
        h
    };
    let glyph_w = ui
        .fonts(|f| f.glyph_width(&FontId::monospace(pane.font_size), '0'))
        .max(4.0);
    let gutter_w = line_gutter_width(total_lines, glyph_w);
    let max_chars = store.max_line_chars().max(1);
    // Two cells per character is a safe upper bound for CJK fallback glyphs.
    let content_w =
        (gutter_w + max_chars as f32 * glyph_w * 2.0 + 24.0).max(ui.available_width());

    if ui.input(|i| i.smooth_scroll_delta.y.abs() > 0.0 || i.raw_scroll_delta.y.abs() > 0.0) {
        pane.scroll_pinned = false;
    }

    let viewport_size = Vec2::new(ui.available_width().max(1.0), height.max(0.0));
    let (viewport_rect, _) = ui.allocate_exact_size(viewport_size, Sense::hover());
    if viewport_rect.height() <= 0.0 {
        return;
    }
    let mut gutter_rows: Vec<(f32, f32, usize)> = Vec::new();
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(viewport_rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(viewport_rect);
            let stick = auto_scroll
                && pane.scroll_pinned
                && scroll_to_row.is_none()
                && highlight_row.is_none();

            let mut scroll_area = egui::ScrollArea::both()
                .id_salt(salt)
                .auto_shrink([false, false])
                .stick_to_bottom(stick)
                .drag_to_scroll(false)
                .max_width(viewport_rect.width())
                .max_height(height)
                .animated(false)
                .enable_scrolling(!ctrl);
            if pane.reset_horizontal_scroll {
                scroll_area = scroll_area.horizontal_scroll_offset(0.0);
                pane.reset_horizontal_scroll = false;
            }
            if let Some(row) = scroll_to_row {
                let h_off = current_hit
                    .filter(|(hit_row, _, _)| *hit_row == row)
                    .and_then(|(_, start, _)| {
                        store.line_at(row).map(|line| {
                            horizontal_offset_for_hit(
                                start,
                                &line,
                                glyph_w,
                                gutter_w,
                                viewport_rect.width(),
                            )
                        })
                    })
                    .unwrap_or(0.0);
                scroll_area = scroll_area
                    .horizontal_scroll_offset(h_off)
                    .vertical_scroll_offset(vertical_offset_for_row(
                        row,
                        line_h,
                        viewport_rect.height(),
                    ));
            }
            scroll_area.show(ui, |ui| {
                let total_h = (total_lines as f32 * line_h).max(line_h);
                let (body, _) =
                    ui.allocate_exact_size(Vec2::new(content_w, total_h), Sense::hover());
                let clip = ui.clip_rect();
                let interact_rect = text_select_interact_rect(ui, body, clip);
                let response = ui.interact(
                    interact_rect,
                    ui.id().with(("log_text_sel", salt)),
                    Sense::click_and_drag(),
                );

                let visible_rows =
                    visible_row_range(body.top(), clip.top(), clip.height(), line_h, total_lines);
                pane.last_view_row = Some(visible_rows.start);

                if auto_scroll && body.bottom() - clip.bottom() < line_h * 1.5 {
                    pane.scroll_pinned = true;
                }

                let pointer = ui
                    .input(|i| i.pointer.latest_pos())
                    .or_else(|| ui.ctx().pointer_interact_pos());
                let primary_down = ui.input(|i| i.pointer.primary_down());
                let primary_released = ui.input(|i| i.pointer.primary_released());
                let in_gutter = |pos: Pos2| pos.x < viewport_rect.left() + gutter_w;

                if response.drag_started() {
                    if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                        response.request_focus();
                        pane.select_origin = Some(pos);
                        pane.select_dragged = false;
                        if !pos_in_line_block(body, pos, line_h, total_lines) {
                            pane.clear_sel();
                        } else {
                            pane.selecting = true;
                            if in_gutter(pos) {
                                let row = (((pos.y - body.top()) / line_h).floor().max(0.0) as usize)
                                    .min(total_lines.saturating_sub(1));
                                let len = store
                                    .line_at(row)
                                    .map(|line| line.chars().count())
                                    .unwrap_or(0);
                                pane.last_selected_text = store
                                    .line_at(row)
                                    .map(|line| line.to_string())
                                    .unwrap_or_default();
                                pane.sel_anchor = Some(super::log_tab::TextCaret { row, col: 0 });
                                pane.sel_focus = Some(super::log_tab::TextCaret { row, col: len });
                                pane.select_dragged = true;
                            } else if let Some(caret) =
                                caret_at_pos(ui, store, body, pos, line_h, pane.font_size, gutter_w)
                            {
                                if shift && pane.sel_anchor.is_some() {
                                    pane.sel_focus = Some(caret);
                                    pane.select_dragged = true;
                                } else {
                                    pane.last_selected_text.clear();
                                    pane.sel_anchor = Some(caret);
                                    pane.sel_focus = Some(caret);
                                }
                            } else {
                                pane.clear_sel();
                            }
                        }
                    }
                } else if pane.selecting && primary_down {
                    if let (Some(origin), Some(pos)) = (pane.select_origin, pointer) {
                        if origin.distance(pos) >= SELECT_DRAG_THRESHOLD {
                            pane.select_dragged = true;
                        }
                        if pane.select_dragged {
                            if let Some(caret) = caret_at_pos(
                                ui,
                                store,
                                body,
                                pos,
                                line_h,
                                pane.font_size,
                                gutter_w,
                            ) {
                                pane.sel_focus = Some(caret);
                            }
                        }
                    }
                } else if pane.selecting && primary_released {
                    let origin = pane.select_origin;
                    pane.selecting = false;
                    pane.select_origin = None;
                    if pane.select_dragged {
                        pane.select_dragged = false;
                        if let Some(text) = copy_selection(store, pane) {
                            pane.last_selected_text = text;
                        } else {
                            pane.last_selected_text.clear();
                        }
                        response.request_focus();
                    } else {
                        pane.select_dragged = false;
                        if origin.is_some_and(|pos| !pos_in_line_block(body, pos, line_h, total_lines))
                        {
                            pane.clear_sel();
                        } else {
                            pane.last_selected_text.clear();
                        }
                    }
                } else if response.clicked() && !response.dragged() && !shift {
                    if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                        if !pos_in_line_block(body, pos, line_h, total_lines) {
                            pane.clear_sel();
                        } else if let Some(caret) =
                            caret_at_pos(ui, store, body, pos, line_h, pane.font_size, gutter_w)
                        {
                            pane.last_selected_text.clear();
                            pane.sel_anchor = Some(caret);
                            pane.sel_focus = Some(caret);
                        } else {
                            pane.clear_sel();
                        }
                    } else {
                        pane.clear_sel();
                    }
                }

                if response.secondary_clicked() {
                    response.context_menu(|ui| {
                        if ui.button(tr(lang, "log.copy")).clicked() {
                            let text = clipboard_text(store, pane);
                            if !text.is_empty() {
                                ui.ctx().copy_text(text);
                            }
                            ui.close_menu();
                        }
                        if ui.button(tr(lang, "log.copy_line")).clicked() {
                            if let Some(row) = pane
                                .sel_focus
                                .map(|caret| caret.row)
                                .or(pane.highlight_master)
                            {
                                if let Some(line) = store.line_at(row) {
                                    ui.ctx().copy_text(line.to_string());
                                }
                            }
                            ui.close_menu();
                        }
                        if ui.button(tr(lang, "log.find")).clicked() {
                            pane.request_find = true;
                            ui.close_menu();
                        }
                        if ui.button(tr(lang, "log.goto")).clicked() {
                            pane.request_goto = true;
                            ui.close_menu();
                        }
                    });
                }

                let copy_active = response.hovered()
                    || response.has_focus()
                    || pane.selecting
                    || (pane.has_text_selection() && !ui.ctx().wants_keyboard_input());
                handle_copy_shortcut(ui, store, pane, copy_active);

                for row in visible_rows.start..visible_rows.end {
                    let row_top = body.top() + row as f32 * line_h;
                    let row_rect = Rect::from_min_max(
                        Pos2::new(body.left(), row_top),
                        Pos2::new(body.right(), row_top + line_h),
                    );
                    if row_rect.bottom() < clip.top() || row_rect.top() > clip.bottom() {
                        continue;
                    }
                    gutter_rows.push((row_rect.top(), row_rect.bottom(), row));

                    if highlight_row == Some(row) {
                        ui.painter().rect_filled(
                            row_rect,
                            0.0,
                            Color32::from_rgba_unmultiplied(2, 132, 199, 72),
                        );
                    } else if pane.parse_row == Some(row) {
                        ui.painter().rect_filled(
                            row_rect,
                            0.0,
                            Color32::from_rgba_unmultiplied(245, 158, 11, 90),
                        );
                    }

                    let line = store.line_at(row).unwrap_or_else(|| Arc::from(""));
                    let text_rect = Rect::from_min_max(
                        Pos2::new(body.left() + gutter_w, row_rect.top()),
                        row_rect.max,
                    );
                    let current = current_hit.and_then(|(hit_row, start, end)| {
                        (hit_row == row).then_some((start, end))
                    });
                    let galley = if needles.is_empty() {
                        layout_log_line(ui, &line, pane.font_size, t.text_primary)
                    } else {
                        let job = highlighted_job(
                            &line,
                            needles,
                            case_sensitive,
                            pane.font_size,
                            t.text_primary,
                            current,
                        );
                        ui.fonts(|fonts| fonts.layout_job(job))
                    };
                    let text_pos = log_row_text_pos(text_rect, line_h, &galley);
                    let row_clip = clip.intersect(text_rect);
                    if let Some((start_col, end_col)) =
                        selection_columns_for_row(pane, row, line.chars().count())
                    {
                        if let Some((start_x, end_x)) =
                            selection_x_range(&galley, start_col, end_col)
                        {
                            ui.painter().with_clip_rect(row_clip).rect_filled(
                                Rect::from_min_max(
                                    Pos2::new(text_pos.x + start_x, row_rect.top()),
                                    Pos2::new(text_pos.x + end_x, row_rect.bottom()),
                                ),
                                0.0,
                                Color32::from_rgba_unmultiplied(59, 130, 246, 72),
                            );
                        }
                    }
                    ui.painter()
                        .with_clip_rect(row_clip)
                        .galley(text_pos, galley, t.text_primary);
                }

                if response.double_clicked() {
                    if let Some(pos) = pointer.filter(|p| body.contains(*p)) {
                        let row = ((pos.y - body.top()) / line_h).floor().max(0.0) as usize;
                        if let Some(line) = store.line_at(row) {
                            if in_gutter(pos) {
                                let len = line.chars().count();
                                pane.sel_anchor = Some(super::log_tab::TextCaret { row, col: 0 });
                                pane.sel_focus = Some(super::log_tab::TextCaret { row, col: len });
                                pane.last_selected_text = line.to_string();
                            } else if let Some(caret) = caret_at_pos(
                                ui,
                                store,
                                body,
                                pos,
                                line_h,
                                pane.font_size,
                                gutter_w,
                            ) {
                                let (start, end) = word_char_range(&line, caret.col);
                                pane.sel_anchor = Some(super::log_tab::TextCaret { row, col: start });
                                pane.sel_focus = Some(super::log_tab::TextCaret { row, col: end });
                                pane.last_selected_text =
                                    line.chars().skip(start).take(end.saturating_sub(start)).collect();
                            }
                        }
                    }
                }

                if pane.auto_parse && response.clicked() {
                    if let Some(pos) = pointer.filter(|p| body.contains(*p) && !in_gutter(*p)) {
                        let row = ((pos.y - body.top()) / line_h).floor().max(0.0) as usize;
                        if let Some(line) = store.line_at(row) {
                            pane.parse_row = Some(row);
                            pane.highlight_master = None;
                            apply_line_parse(pane, &line, lang);
                            ui.ctx().request_repaint();
                        }
                    }
                }

                if response.hovered() && ctrl && (zoom_factor - 1.0).abs() > 0.0005 {
                    let old = pane.font_size;
                    let notches = ((zoom_factor.ln().abs() / 0.04).round() as i32).clamp(1, 2);
                    let dir = if zoom_factor > 1.0 { 1.0 } else { -1.0 };
                    pane.font_size = (pane.font_size + dir * LOG_FONT_STEP * notches as f32)
                        .clamp(LOG_FONT_MIN, LOG_FONT_MAX);
                    pane.font_size = (pane.font_size * 4.0).round() / 4.0;
                    if (pane.font_size - old).abs() > 0.01 {
                        pane.cached_row_height = None;
                        ui.ctx().request_repaint();
                    }
                }
            });
        },
    );

    let gutter_clip = Rect::from_min_max(
        viewport_rect.min,
        Pos2::new(
            (viewport_rect.left() + gutter_w).min(viewport_rect.right()),
            viewport_rect.bottom(),
        ),
    );
    ui.painter().rect_filled(gutter_clip, 0.0, t.surface_bg);
    ui.painter().vline(
        gutter_clip.right(),
        gutter_clip.top()..=gutter_clip.bottom(),
        Stroke::new(1.0_f32, t.border),
    );
    for (top, bottom, row) in gutter_rows {
        let y = (top + bottom) * 0.5;
        ui.painter().with_clip_rect(gutter_clip).text(
            Pos2::new(gutter_clip.right() - 4.0, y),
            Align2::RIGHT_CENTER,
            format!("{}", row + 1),
            FontId::monospace(pane.font_size),
            t.text_muted,
        );
    }
}

fn highlight_ranges(line: &str, needles: &[String], case_sensitive: bool) -> Vec<(usize, usize)> {
    match_ranges_in_line(line, needles, case_sensitive)
}

fn highlighted_job(
    text: &str,
    needles: &[String],
    case_sensitive: bool,
    font_size: f32,
    text_color: Color32,
    current: Option<(usize, usize)>,
) -> LayoutJob {
    let font_id = FontId::monospace(font_size);
    let normal = TextFormat {
        font_id: font_id.clone(),
        color: text_color,
        ..Default::default()
    };
    let matched = TextFormat {
        font_id: font_id.clone(),
        color: Color32::from_rgb(0xF5, 0x9E, 0x0B),
        background: Color32::from_rgba_unmultiplied(245, 158, 11, 36),
        ..Default::default()
    };
    let current_fmt = TextFormat {
        font_id,
        color: Color32::from_rgb(0x0F, 0x17, 0x2A),
        background: Color32::from_rgba_unmultiplied(245, 158, 11, 160),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut cursor = 0;
    for (start, end) in highlight_ranges(text, needles, case_sensitive) {
        if cursor < start {
            job.append(&text[cursor..start], 0.0, normal.clone());
        }
        let fmt = if current.is_some_and(|(cs, ce)| cs < end && ce > start) {
            current_fmt.clone()
        } else {
            matched.clone()
        };
        job.append(&text[start..end], 0.0, fmt);
        cursor = end;
    }
    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, normal);
    }
    job
}

fn search_display_line(master_idx: usize, line: &str) -> String {
    format!("{:>6} | {line}", master_idx + 1)
}

fn search_sel_range(selection: &SearchSelectionState) -> Option<(TextCaret, TextCaret)> {
    let anchor = selection.anchor?;
    let focus = selection.focus.unwrap_or(anchor);
    Some(if (anchor.row, anchor.col) <= (focus.row, focus.col) {
        (anchor, focus)
    } else {
        (focus, anchor)
    })
}

fn search_selection_columns(
    selection: &SearchSelectionState,
    row: usize,
    line_chars: usize,
) -> Option<(usize, usize)> {
    let (lo, hi) = search_sel_range(selection)?;
    if row < lo.row || row > hi.row {
        return None;
    }
    let start = if row == lo.row {
        lo.col.min(line_chars)
    } else {
        0
    };
    let end = if row == hi.row {
        hi.col.min(line_chars)
    } else {
        line_chars
    };
    (start < end).then_some((start, end))
}

fn copy_search_selection<S: LogStore + ?Sized>(
    store: &S,
    search_map: &[usize],
    selection: &SearchSelectionState,
) -> Option<String> {
    let (lo, hi) = search_sel_range(selection)?;
    if lo == hi || search_map.is_empty() {
        return None;
    }
    let last_row = hi.row.min(search_map.len().saturating_sub(1));
    let last_row = last_row.min(lo.row.saturating_add(MAX_COPY_ROWS.saturating_sub(1)));
    let mut out = String::new();
    for row in lo.row..=last_row {
        if row > lo.row {
            out.push('\n');
        }
        let master_idx = *search_map.get(row)?;
        let line = store.line_at(master_idx)?;
        let display = search_display_line(master_idx, &line);
        let chars: Vec<char> = display.chars().collect();
        let start = if row == lo.row { lo.col } else { 0 }.min(chars.len());
        let end = if row == hi.row { hi.col } else { chars.len() }.min(chars.len());
        if start < end {
            out.extend(chars[start..end].iter());
        }
    }
    (!out.is_empty()).then_some(out)
}

fn search_caret_at_pos<S: LogStore + ?Sized>(
    ui: &Ui,
    store: &S,
    search_map: &[usize],
    body: Rect,
    pos: Pos2,
    line_height: f32,
    font_size: f32,
    viewport_left: f32,
    prefix_width: f32,
) -> Option<TextCaret> {
    let row = (((pos.y - body.top()) / line_height).floor().max(0.0) as usize)
        .min(search_map.len().saturating_sub(1));
    let row_top = body.top() + row as f32 * line_height;
    let row_rect = Rect::from_min_max(
        Pos2::new(body.left(), row_top),
        Pos2::new(body.right(), row_top + line_height),
    );
    let master_idx = *search_map.get(row)?;
    let line = store.line_at(master_idx)?;
    let parts = split_timestamp_prefix(&line);
    let prefix_text = format!("{:>6} | {}", master_idx + 1, parts.prefix);
    let prefix_galley = layout_log_line(ui, &prefix_text, font_size, Color32::WHITE);
    let rest_galley = layout_log_line(ui, parts.rest, font_size, Color32::WHITE);
    let prefix_chars = prefix_text.chars().count();
    let prefix_text_pos = Pos2::new(
        viewport_left + TEXT_LEFT_PAD,
        row_top + (line_height - prefix_galley.size().y).max(0.0) * 0.5,
    );
    let rest_text_pos = Pos2::new(
        row_rect.left() + prefix_width,
        row_top + (line_height - rest_galley.size().y).max(0.0) * 0.5,
    );
    let col = if pos.x < viewport_left + prefix_width {
        caret_col_from_pointer(&prefix_galley, prefix_chars, prefix_text_pos, pos)
    } else {
        prefix_chars
            + caret_col_from_pointer(
                &rest_galley,
                parts.rest.chars().count(),
                rest_text_pos,
                pos,
            )
    };
    Some(TextCaret { row, col })
}

/// Search-hit list — virtualized so broad filters stay responsive.
pub fn show_virtual_search_pane<S: LogStore + ?Sized>(
    ui: &mut Ui,
    store: &S,
    search_map: &[usize],
    salt: &str,
    height: f32,
    t: &Tokens,
    lang: Lang,
    active_search_row: Option<usize>,
    font_size: f32,
    needles: &[String],
    case_sensitive: bool,
    scanning: bool,
    selection: &mut SearchSelectionState,
) -> Option<usize> {
    let mut clicked = None;
    let line_h = row_height(ui, font_size);
    let glyph_w = ui
        .fonts(|f| f.glyph_width(&FontId::monospace(font_size), '0'))
        .max(4.0);
    let prefix_width = SEARCH_PREFIX_CELLS * glyph_w + TEXT_LEFT_PAD * 2.0;
    let content_w =
        ((store.max_line_chars() + 9) as f32 * glyph_w * 2.0 + 24.0).max(ui.available_width());
    let viewport_size = Vec2::new(ui.available_width().max(1.0), height.max(0.0));
    let (viewport_rect, _) = ui.allocate_exact_size(viewport_size, Sense::hover());
    if viewport_rect.height() <= 0.0 {
        return None;
    }
    let mut frozen_rows = Vec::<FrozenRow>::new();

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(viewport_rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(viewport_rect);
            if search_map.is_empty() {
                let message = if scanning {
                    tr(lang, "log.search_scanning")
                } else {
                    tr(lang, "log.search_empty")
                };
                ui.painter().text(
                    Pos2::new(viewport_rect.left() + 8.0, viewport_rect.top() + 8.0),
                    Align2::LEFT_TOP,
                    message,
                    FontId::proportional(12.0),
                    t.text_muted,
                );
                return;
            }

            egui::ScrollArea::both()
                .id_salt(salt)
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .max_width(viewport_rect.width())
                .max_height(viewport_rect.height())
                .animated(false)
                .show(ui, |ui| {
                    let total_h = search_map.len() as f32 * line_h;
                    // Keep the last result row above the horizontal scrollbar so
                    // its hit target is not clipped when scrolled to the end.
                    let bottom_pad = scroll_bar_gutter(ui);
                    let (body, _) = ui.allocate_exact_size(
                        Vec2::new(content_w, (total_h + bottom_pad).max(line_h)),
                        Sense::hover(),
                    );
                    let clip = ui.clip_rect();
                    let interact_rect = search_interact_rect(ui, body, clip);
                    let response = ui.interact(
                        interact_rect,
                        ui.id().with(("log_search_sel", salt)),
                        Sense::click_and_drag(),
                    );
                    let visible_rows = visible_row_range(
                        body.top(),
                        clip.top(),
                        clip.height(),
                        line_h,
                        search_map.len(),
                    );
                    let pointer = ui.ctx().pointer_interact_pos();
                    let primary_down = ui.input(|input| input.pointer.primary_down());
                    let primary_released = ui.input(|input| input.pointer.primary_released());
                    let row_count = search_map.len();

                    // Same as the main log pane: anchor at press, extend on drag.
                    // Jump on plain click is decided on primary_released (not only
                    // response.clicked): with Sense::click_and_drag, egui sometimes
                    // classifies a micro-move click as a drag and skips clicked().
                    if response.drag_started() {
                        if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                            response.request_focus();
                            selection.select_origin = Some(pos);
                            selection.select_dragged = false;
                            if search_row_at_pos(body, line_h, row_count, pos).is_none() {
                                selection.clear();
                            } else {
                                selection.selecting = true;
                                if let Some(caret) = search_caret_at_pos(
                                    ui,
                                    store,
                                    search_map,
                                    body,
                                    pos,
                                    line_h,
                                    font_size,
                                    clip.left(),
                                    prefix_width,
                                ) {
                                    selection.last_selected_text.clear();
                                    selection.anchor = Some(caret);
                                    selection.focus = Some(caret);
                                }
                            }
                        }
                    } else if selection.selecting && primary_down {
                        if let (Some(origin), Some(pos)) = (selection.select_origin, pointer) {
                            if origin.distance(pos) >= SELECT_DRAG_THRESHOLD {
                                selection.select_dragged = true;
                            }
                            if selection.select_dragged {
                                if let Some(caret) = search_caret_at_pos(
                                    ui,
                                    store,
                                    search_map,
                                    body,
                                    pos,
                                    line_h,
                                    font_size,
                                    clip.left(),
                                    prefix_width,
                                ) {
                                    selection.focus = Some(caret);
                                }
                            }
                        }
                    } else if selection.selecting && primary_released {
                        if selection.select_dragged {
                            selection.selecting = false;
                            selection.select_origin = None;
                            selection.select_dragged = false;
                            selection.last_selected_text =
                                copy_search_selection(store, search_map, selection)
                                    .unwrap_or_default();
                        } else {
                            // Plain click → jump even when egui.clicked() is false.
                            let pos = selection
                                .select_origin
                                .or(response.interact_pointer_pos())
                                .or(pointer);
                            if let Some(pos) = pos {
                                clicked = search_row_at_pos(body, line_h, row_count, pos);
                            }
                            selection.clear();
                        }
                    } else if response.clicked() && !response.dragged() {
                        selection.clear();
                    }

                    let copy_requested = ui.input(|input| {
                        input.events.iter().any(|event| {
                            matches!(event, egui::Event::Copy)
                                || matches!(
                                    event,
                                    egui::Event::Key {
                                        key: egui::Key::C,
                                        pressed: true,
                                        modifiers,
                                        ..
                                    } if modifiers.ctrl || modifiers.command
                                )
                        })
                    });
                    if copy_requested
                        && (response.hovered() || response.has_focus() || selection.selecting)
                    {
                        let text = copy_search_selection(store, search_map, selection)
                            .unwrap_or_else(|| selection.last_selected_text.clone());
                        if !text.is_empty() {
                            ui.ctx().copy_text(text);
                        }
                    }

                    for i in visible_rows {
                        let master_idx = search_map[i];
                        let row_rect = Rect::from_min_size(
                            Pos2::new(body.left(), body.top() + i as f32 * line_h),
                            Vec2::new(content_w, line_h),
                        );
                        if active_search_row == Some(i) {
                            ui.painter().rect_filled(
                                row_rect,
                                0.0,
                                Color32::from_rgba_unmultiplied(2, 132, 199, 72),
                            );
                        }
                        let line = store.line_at(master_idx).unwrap_or_else(|| Arc::from(""));
                        let parts = split_timestamp_prefix(&line);
                        let prefix_text = format!("{:>6} | {}", master_idx + 1, parts.prefix);
                        let prefix_job = highlighted_job(
                            &prefix_text,
                            needles,
                            case_sensitive,
                            font_size,
                            t.text_primary,
                            None,
                        );
                        let rest_job = highlighted_job(
                            parts.rest,
                            needles,
                            case_sensitive,
                            font_size,
                            t.text_primary,
                            None,
                        );
                        let prefix_galley = ui.fonts(|fonts| fonts.layout_job(prefix_job));
                        let rest_galley = ui.fonts(|fonts| fonts.layout_job(rest_job));
                        let prefix_chars = prefix_text.chars().count();
                        let display_chars = prefix_chars + parts.rest.chars().count();
                        let rest_y =
                            row_rect.top() + (line_h - rest_galley.size().y).max(0.0) * 0.5;
                        let mut prefix_selection_x = None;
                        if let Some((start_col, end_col)) =
                            search_selection_columns(selection, i, display_chars)
                        {
                            if start_col < prefix_chars {
                                let local_end = end_col.min(prefix_chars);
                                let start_x = prefix_galley
                                    .pos_from_ccursor(CCursor::new(start_col))
                                    .left();
                                let end_x = prefix_galley
                                    .pos_from_ccursor(CCursor::new(local_end))
                                    .left();
                                prefix_selection_x = Some((start_x, end_x));
                            }
                            if end_col > prefix_chars {
                                let local_start = start_col.saturating_sub(prefix_chars);
                                let local_end = end_col.saturating_sub(prefix_chars);
                                let start_x = rest_galley
                                    .pos_from_ccursor(CCursor::new(local_start))
                                    .left();
                                let end_x = rest_galley
                                    .pos_from_ccursor(CCursor::new(local_end))
                                    .left();
                                ui.painter()
                                    .with_clip_rect(Rect::from_min_max(
                                        Pos2::new(clip.left() + prefix_width, row_rect.top()),
                                        Pos2::new(clip.right(), row_rect.bottom()),
                                    ))
                                    .rect_filled(
                                        Rect::from_min_max(
                                            Pos2::new(
                                                row_rect.left() + prefix_width + start_x,
                                                row_rect.top(),
                                            ),
                                            Pos2::new(
                                                row_rect.left() + prefix_width + end_x,
                                                row_rect.bottom(),
                                            ),
                                        ),
                                        0.0,
                                        Color32::from_rgba_unmultiplied(59, 130, 246, 72),
                                    );
                            }
                        }
                        frozen_rows.push(FrozenRow {
                            top: row_rect.top(),
                            bottom: row_rect.bottom(),
                            galley: prefix_galley,
                            selection_x: prefix_selection_x,
                        });
                        ui.painter()
                            .with_clip_rect(Rect::from_min_max(
                                Pos2::new(clip.left() + prefix_width, row_rect.top()),
                                Pos2::new(clip.right(), row_rect.bottom()),
                            ))
                            .galley(
                                Pos2::new(row_rect.left() + prefix_width, rest_y),
                                rest_galley,
                                t.text_primary,
                            );
                    }

                    if response.secondary_clicked() {
                        response.context_menu(|ui| {
                            if ui.button("Copy").clicked() {
                                let text = copy_search_selection(store, search_map, selection)
                                    .unwrap_or_else(|| selection.last_selected_text.clone());
                                if !text.is_empty() {
                                    ui.ctx().copy_text(text);
                                }
                                ui.close_menu();
                            }
                        });
                    }

                    if response.double_clicked() {
                        if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                            if let Some(row) = search_row_at_pos(body, line_h, row_count, pos) {
                                if let Some(&master_idx) = search_map.get(row) {
                                    if let Some(line) = store.line_at(master_idx) {
                                        let len =
                                            search_display_line(master_idx, &line).chars().count();
                                        selection.anchor = Some(TextCaret { row, col: 0 });
                                        selection.focus = Some(TextCaret { row, col: len });
                                        selection.last_selected_text =
                                            search_display_line(master_idx, &line);
                                    }
                                }
                            }
                        }
                    }

                    // Backup: egui reported a click without going through drag_started
                    // (e.g. press began outside then released on the row).
                    if clicked.is_none() && response.clicked() {
                        let pos = response.interact_pointer_pos().or(pointer);
                        if let Some(pos) = pos {
                            clicked = search_row_at_pos(body, line_h, row_count, pos);
                        }
                    }

                    // Last-row / gutter fallback: press landed outside Sense hit
                    // testing (e.g. historic bottom-bar overlap) so drag never started.
                    if clicked.is_none()
                        && primary_released
                        && !selection.selecting
                        && !selection.select_dragged
                    {
                        if let Some(pos) =
                            pointer.filter(|p| viewport_rect.contains(*p) && p.x < interact_rect.right())
                        {
                            clicked = search_row_at_pos(body, line_h, row_count, pos);
                        }
                    }
                });
        },
    );
    paint_frozen_rows(
        ui,
        viewport_rect,
        prefix_width,
        line_h,
        frozen_rows,
        t.text_primary,
    );

    if clicked.is_some() {
        ui.ctx().request_repaint();
    }
    clicked
}

fn selection_columns_for_row(
    pane: &PaneState,
    row: usize,
    line_chars: usize,
) -> Option<(usize, usize)> {
    let (lo, hi) = pane_sel_range(pane)?;
    if row < lo.row || row > hi.row {
        return None;
    }
    let start = if row == lo.row {
        lo.col.min(line_chars)
    } else {
        0
    };
    let end = if row == hi.row {
        hi.col.min(line_chars)
    } else {
        line_chars
    };
    (start < end).then_some((start, end))
}

fn pane_sel_range(
    pane: &PaneState,
) -> Option<(super::log_tab::TextCaret, super::log_tab::TextCaret)> {
    let a = pane.sel_anchor?;
    let b = pane.sel_focus.unwrap_or(a);
    Some(if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    })
}

fn copy_selection<S: LogStore + ?Sized>(store: &S, pane: &PaneState) -> Option<String> {
    let (lo, hi) = pane_sel_range(pane)?;
    if lo.row == hi.row && lo.col == hi.col {
        return None;
    }
    let last_row = hi.row.min(store.line_count().saturating_sub(1));
    let last_row = last_row.min(lo.row.saturating_add(MAX_COPY_ROWS.saturating_sub(1)));
    let mut out = String::new();
    for row in lo.row..=last_row {
        if row > lo.row {
            out.push('\n');
        }
        let line = store.line_at(row)?;
        let chars: Vec<char> = line.chars().collect();
        let start_col = if row == lo.row { lo.col } else { 0 };
        let end_col = if row == hi.row {
            hi.col.min(chars.len())
        } else {
            chars.len()
        };
        if start_col < end_col {
            out.extend(chars[start_col..end_col].iter());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Line-oriented document used by the serial-tool TXT editor.
///
/// View mode already virtualizes; edit mode previously fed the whole file to
/// `TextEdit::multiline`, which laid out every character every frame.
pub struct LineEditorSession {
    pub lines: Vec<String>,
    pub newline: String,
    pub row: usize,
    pub col: usize,
    pub max_chars: usize,
    pub scroll_to: Option<usize>,
    pub want_focus: bool,
    pub has_focus: bool,
    pub pending_cursor: Option<CCursorRange>,
}

impl Default for LineEditorSession {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            newline: "\n".into(),
            row: 0,
            col: 0,
            max_chars: 1,
            scroll_to: None,
            want_focus: false,
            has_focus: false,
            pending_cursor: None,
        }
    }
}

impl LineEditorSession {
    pub fn from_text(text: &str) -> Self {
        let newline = if text.contains("\r\n") {
            "\r\n".to_string()
        } else {
            "\n".to_string()
        };
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let max_chars = lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(1)
            .max(1);
        Self {
            lines,
            newline,
            row: 0,
            col: 0,
            max_chars,
            scroll_to: Some(0),
            want_focus: true,
            has_focus: false,
            pending_cursor: None,
        }
    }

    pub fn to_text(&self) -> String {
        self.lines.join(&self.newline)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn clamp_caret(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.row.min(self.lines.len() - 1);
        let len = self.lines[self.row].chars().count();
        self.col = self.col.min(len);
    }

    fn note_line_chars(&mut self, n: usize) {
        self.max_chars = self.max_chars.max(n.max(1));
    }

    pub fn rescan_max_chars(&mut self) {
        self.max_chars = self
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(1)
            .max(1);
    }
}

fn editor_char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

fn editor_insert_text(session: &mut LineEditorSession, text: &str) {
    session.clamp_caret();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let parts: Vec<&str> = normalized.split('\n').collect();
    if parts.is_empty() {
        return;
    }
    let row = session.row;
    let byte = editor_char_to_byte(&session.lines[row], session.col);
    let suffix = session.lines[row][byte..].to_string();
    session.lines[row].truncate(byte);
    session.lines[row].push_str(parts[0]);
    if parts.len() == 1 {
        session.col = session.lines[row].chars().count();
        session.lines[row].push_str(&suffix);
        session.note_line_chars(session.lines[row].chars().count());
        session.pending_cursor = Some(CCursorRange::one(CCursor::new(session.col)));
    } else {
        session.note_line_chars(session.lines[row].chars().count());
        let last = parts.len() - 1;
        for (i, part) in parts.iter().enumerate().skip(1) {
            let mut line = (*part).to_string();
            if i == last {
                line.push_str(&suffix);
            }
            session.note_line_chars(line.chars().count());
            session.lines.insert(row + i, line);
        }
        session.row = row + last;
        session.col = parts[last].chars().count();
        session.pending_cursor = Some(CCursorRange::one(CCursor::new(session.col)));
        session.scroll_to = Some(session.row);
    }
    session.want_focus = true;
}

fn editor_split_line(session: &mut LineEditorSession) {
    session.clamp_caret();
    let row = session.row;
    let old_len = session.lines[row].chars().count();
    let byte = editor_char_to_byte(&session.lines[row], session.col);
    let rest = session.lines[row].split_off(byte);
    session.note_line_chars(session.lines[row].chars().count());
    session.note_line_chars(rest.chars().count());
    session.lines.insert(row + 1, rest);
    if old_len >= session.max_chars {
        session.rescan_max_chars();
    }
    session.row = row + 1;
    session.col = 0;
    session.pending_cursor = Some(CCursorRange::one(CCursor::new(0)));
    session.scroll_to = Some(session.row);
    session.want_focus = true;
}

fn editor_join_prev(session: &mut LineEditorSession) -> bool {
    session.clamp_caret();
    if session.row == 0 {
        return false;
    }
    let row = session.row;
    let cur = session.lines.remove(row);
    let removed_len = cur.chars().count();
    let prev_len = session.lines[row - 1].chars().count();
    session.lines[row - 1].push_str(&cur);
    session.note_line_chars(session.lines[row - 1].chars().count());
    if removed_len >= session.max_chars {
        session.rescan_max_chars();
    }
    session.row = row - 1;
    session.col = prev_len;
    session.pending_cursor = Some(CCursorRange::one(CCursor::new(prev_len)));
    session.scroll_to = Some(session.row);
    session.want_focus = true;
    true
}

fn editor_join_next(session: &mut LineEditorSession) -> bool {
    session.clamp_caret();
    let row = session.row;
    if row + 1 >= session.lines.len() {
        return false;
    }
    let next = session.lines.remove(row + 1);
    let removed_len = next.chars().count();
    let col = session.lines[row].chars().count();
    session.lines[row].push_str(&next);
    session.note_line_chars(session.lines[row].chars().count());
    if removed_len >= session.max_chars {
        session.rescan_max_chars();
    }
    session.col = col;
    session.pending_cursor = Some(CCursorRange::one(CCursor::new(col)));
    session.want_focus = true;
    true
}

fn editor_move_row(session: &mut LineEditorSession, delta: isize) {
    session.clamp_caret();
    let n = session.lines.len();
    let target = if delta < 0 {
        session.row.saturating_sub((-delta) as usize)
    } else {
        (session.row + delta as usize).min(n.saturating_sub(1))
    };
    if target == session.row {
        return;
    }
    let col = session.col;
    session.row = target;
    session.col = col.min(session.lines[target].chars().count());
    session.pending_cursor = Some(CCursorRange::one(CCursor::new(session.col)));
    session.scroll_to = Some(session.row);
    session.want_focus = true;
}

/// Virtualized line editor: only visible rows are laid out; typing mutates one line.
pub fn show_virtual_line_editor(
    ui: &mut Ui,
    session: &mut LineEditorSession,
    font_size: f32,
    salt: &str,
    height: f32,
    t: &Tokens,
) -> bool {
    let mut changed = false;
    session.clamp_caret();

    let line_h = row_height(ui, font_size);
    let page_rows = ((height / line_h).floor() as isize - 1).max(1);

    if session.has_focus {
        let mut ate_enter = false;
        let mut ate_backspace = false;
        let mut ate_delete = false;
        let mut paste: Option<String> = None;
        let ime_busy = ui.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Ime(ImeEvent::Preedit(_) | ImeEvent::Commit(_))
                )
            })
        });
        ui.input(|i| {
            for event in &i.events {
                match event {
                    Event::Paste(text) if text.contains('\n') || text.contains('\r') => {
                        paste = Some(text.clone());
                    }
                    Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if !(modifiers.ctrl || modifiers.command || modifiers.alt) => {
                        match key {
                            Key::Enter if !ime_busy => ate_enter = true,
                            Key::Backspace if session.col == 0 => ate_backspace = true,
                            Key::Delete
                                if session.col
                                    == session.lines[session.row].chars().count() =>
                            {
                                ate_delete = true;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        });
        if paste.is_some() || ate_enter || ate_backspace || ate_delete {
            ui.input_mut(|i| {
                i.events.retain(|event| match event {
                    Event::Paste(text) if text.contains('\n') || text.contains('\r') => false,
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        repeat: false,
                        ..
                    } if ate_enter => false,
                    Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        repeat: false,
                        ..
                    } if ate_backspace => false,
                    Event::Key {
                        key: Key::Delete,
                        pressed: true,
                        repeat: false,
                        ..
                    } if ate_delete => false,
                    _ => true,
                });
            });
        }
        if let Some(text) = paste {
            editor_insert_text(session, &text);
            changed = true;
        }
        if ate_enter {
            editor_split_line(session);
            changed = true;
        }
        if ate_backspace && editor_join_prev(session) {
            changed = true;
        }
        if ate_delete && editor_join_next(session) {
            changed = true;
        }

        let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
            editor_move_row(session, -1);
        }
        if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
            editor_move_row(session, 1);
        }
        if ui.input(|i| i.key_pressed(Key::PageUp)) {
            editor_move_row(session, -page_rows);
        }
        if ui.input(|i| i.key_pressed(Key::PageDown)) {
            editor_move_row(session, page_rows);
        }
        if ctrl && ui.input(|i| i.key_pressed(Key::Home)) {
            session.row = 0;
            session.col = 0;
            session.pending_cursor = Some(CCursorRange::one(CCursor::new(0)));
            session.scroll_to = Some(0);
            session.want_focus = true;
        }
        if ctrl && ui.input(|i| i.key_pressed(Key::End)) {
            session.row = session.lines.len().saturating_sub(1);
            session.col = session.lines[session.row].chars().count();
            session.pending_cursor = Some(CCursorRange::one(CCursor::new(session.col)));
            session.scroll_to = Some(session.row);
            session.want_focus = true;
        }
    }

    let total_lines = session.lines.len().max(1);
    let glyph_w = ui
        .fonts(|f| f.glyph_width(&FontId::monospace(font_size), '0'))
        .max(4.0);
    let gutter_w = line_gutter_width(total_lines, glyph_w);
    let content_w = (gutter_w
        + session.max_chars as f32 * glyph_w * 2.0
        + 24.0)
        .max(ui.available_width());

    let viewport_size = Vec2::new(ui.available_width().max(1.0), height.max(0.0));
    let (viewport_rect, _) = ui.allocate_exact_size(viewport_size, Sense::hover());
    if viewport_rect.height() <= 0.0 {
        return changed;
    }

    let mut gutter_rows: Vec<(f32, f32, usize)> = Vec::new();
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(viewport_rect)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(viewport_rect);
            let mut scroll_area = egui::ScrollArea::both()
                .id_salt(salt)
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .max_width(viewport_rect.width())
                .max_height(height)
                .animated(false);
            if let Some(row) = session.scroll_to.take() {
                scroll_area = scroll_area.vertical_scroll_offset(vertical_offset_for_row(
                    row,
                    line_h,
                    viewport_rect.height(),
                ));
            }
            scroll_area.show(ui, |ui| {
                let total_h = (total_lines as f32 * line_h).max(line_h);
                let (body, _) =
                    ui.allocate_exact_size(Vec2::new(content_w, total_h), Sense::hover());
                let clip = ui.clip_rect();
                let interact_rect = text_select_interact_rect(ui, body, clip);
                let response = ui.interact(
                    interact_rect,
                    ui.id().with(("line_edit_hit", salt)),
                    Sense::hover(),
                );

                if response.contains_pointer() && ui.input(|i| i.pointer.primary_clicked()) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        let row = ((pos.y - body.top()) / line_h)
                            .floor()
                            .max(0.0) as usize;
                        let row = row.min(total_lines.saturating_sub(1));
                        if row != session.row {
                            let col = ((pos.x - body.left() - gutter_w - TEXT_LEFT_PAD)
                                / glyph_w)
                                .floor()
                                .max(0.0) as usize;
                            session.row = row;
                            session.col = col.min(session.lines[row].chars().count());
                            session.pending_cursor =
                                Some(CCursorRange::one(CCursor::new(session.col)));
                            session.want_focus = true;
                        }
                    }
                }

                let visible =
                    visible_row_range(body.top(), clip.top(), clip.height(), line_h, total_lines);
                let focus_row = session.row;

                for row in visible.start..visible.end {
                    let row_top = body.top() + row as f32 * line_h;
                    let row_rect = Rect::from_min_max(
                        Pos2::new(body.left(), row_top),
                        Pos2::new(body.right(), row_top + line_h),
                    );
                    gutter_rows.push((row_rect.top(), row_rect.bottom(), row));
                    if row == focus_row {
                        ui.painter().rect_filled(
                            row_rect,
                            0.0,
                            Color32::from_rgba_unmultiplied(2, 132, 199, 40),
                        );
                    }
                    let text_rect = Rect::from_min_max(
                        Pos2::new(body.left() + gutter_w, row_rect.top()),
                        row_rect.max,
                    );
                    if row == focus_row {
                        let prev_len = session.lines[row].chars().count();
                        let mut line = std::mem::take(&mut session.lines[row]);
                        let editor_id = ui.id().with(("line-edit-row", salt));
                        let inner = ui.allocate_new_ui(
                            UiBuilder::new()
                                .max_rect(text_rect)
                                .layout(Layout::left_to_right(Align::Center)),
                            |ui| {
                                ui.set_min_width(text_rect.width());
                                let mut output = egui::TextEdit::singleline(&mut line)
                                    .id(editor_id)
                                    .font(FontId::monospace(font_size))
                                    .desired_width(text_rect.width().max(80.0))
                                    .frame(false)
                                    .margin(Margin::symmetric(TEXT_LEFT_PAD as i8, 0))
                                    .clip_text(false)
                                    .lock_focus(true)
                                    .show(ui);
                                if let Some(range) = session.pending_cursor.take() {
                                    output.state.cursor.set_char_range(Some(range));
                                    output.state.store(ui.ctx(), output.response.id);
                                }
                                if session.want_focus {
                                    output.response.request_focus();
                                    session.want_focus = false;
                                }
                                if output.response.changed() {
                                    changed = true;
                                }
                                session.col = output
                                    .cursor_range
                                    .map(|range| range.primary.ccursor.index)
                                    .unwrap_or(session.col);
                                session.has_focus = output.response.has_focus();
                            },
                        );
                        let _ = inner;
                        let new_len = line.chars().count();
                        session.lines[row] = line;
                        if new_len < prev_len && prev_len >= session.max_chars {
                            session.rescan_max_chars();
                        } else {
                            session.note_line_chars(new_len);
                        }
                    } else {
                        let galley = layout_log_line(
                            ui,
                            &session.lines[row],
                            font_size,
                            t.text_primary,
                        );
                        ui.painter().with_clip_rect(clip).galley(
                            Pos2::new(text_rect.left() + TEXT_LEFT_PAD, text_rect.top()),
                            galley,
                            t.text_primary,
                        );
                    }
                }
            });
        },
    );

    let gutter_clip = Rect::from_min_max(
        viewport_rect.min,
        Pos2::new(
            (viewport_rect.left() + gutter_w).min(viewport_rect.right()),
            viewport_rect.bottom(),
        ),
    );
    ui.painter().rect_filled(gutter_clip, 0.0, t.surface_bg);
    ui.painter().vline(
        gutter_clip.right() - 1.0,
        gutter_clip.y_range(),
        Stroke::new(1.0_f32, t.border),
    );
    for (top, bottom, row) in gutter_rows {
        let y = (top + bottom) * 0.5;
        ui.painter().with_clip_rect(gutter_clip).text(
            Pos2::new(gutter_clip.right() - 4.0, y),
            Align2::RIGHT_CENTER,
            format!("{}", row + 1),
            FontId::monospace(font_size),
            t.text_muted,
        );
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_rows_are_bounded_and_include_overscan() {
        assert_eq!(visible_row_range(0.0, 50.0, 40.0, 10.0, 100), 5..11);
        assert_eq!(visible_row_range(0.0, 950.0, 100.0, 10.0, 100), 95..100);
        assert_eq!(visible_row_range(0.0, 0.0, 100.0, 10.0, 0), 0..0);
    }

    #[test]
    fn jump_offset_changes_only_vertical_position() {
        assert_eq!(vertical_offset_for_row(0, 20.0, 200.0), 0.0);
        assert_eq!(vertical_offset_for_row(20, 20.0, 200.0), 310.0);
    }

    #[test]
    fn jump_horizontal_offset_keeps_match_in_view() {
        let line = "................ASK";
        let start = line.find("ASK").unwrap();
        let off = horizontal_offset_for_hit(start, line, 8.0, 30.0, 120.0);
        assert!(off > 0.0);
        assert_eq!(byte_to_char_index("中ASK", 3), 1);
    }

    #[test]
    fn click_below_last_line_is_outside_line_block() {
        let body = Rect::from_min_size(Pos2::new(0.0, 10.0), Vec2::new(200.0, 60.0));
        assert!(pos_in_line_block(body, Pos2::new(10.0, 10.0), 20.0, 3));
        assert!(pos_in_line_block(body, Pos2::new(10.0, 69.9), 20.0, 3));
        assert!(!pos_in_line_block(body, Pos2::new(10.0, 70.0), 20.0, 3));
        assert!(!pos_in_line_block(body, Pos2::new(10.0, 10.0), 20.0, 0));
    }

    #[test]
    fn search_row_at_pos_includes_last_row_edge() {
        let body = Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(200.0, 60.0));
        let line_h = 20.0;
        assert_eq!(
            search_row_at_pos(body, line_h, 3, Pos2::new(10.0, 100.0)),
            Some(0)
        );
        assert_eq!(
            search_row_at_pos(body, line_h, 3, Pos2::new(10.0, 139.9)),
            Some(1)
        );
        // Bottom edge of the last row must still resolve (exclusive upper bound).
        assert_eq!(
            search_row_at_pos(body, line_h, 3, Pos2::new(10.0, 159.9)),
            Some(2)
        );
        assert_eq!(
            search_row_at_pos(body, line_h, 3, Pos2::new(10.0, 160.0)),
            None
        );
    }

    #[test]
    fn frozen_column_is_anchored_to_viewport_coordinates() {
        let viewport = Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(300.0, 100.0));
        let clip = frozen_column_clip(viewport, 120.0);
        assert_eq!(clip.left(), 40.0);
        assert_eq!(clip.right(), 160.0);
        assert_eq!(clip.top(), 20.0);
        assert_eq!(clip.bottom(), 120.0);
    }

    #[test]
    fn selection_columns_are_partial_at_both_ends() {
        let mut pane = PaneState::default();
        pane.sel_anchor = Some(crate::log_tab::TextCaret { row: 2, col: 3 });
        pane.sel_focus = Some(crate::log_tab::TextCaret { row: 4, col: 5 });

        assert_eq!(selection_columns_for_row(&pane, 2, 10), Some((3, 10)));
        assert_eq!(selection_columns_for_row(&pane, 3, 8), Some((0, 8)));
        assert_eq!(selection_columns_for_row(&pane, 4, 10), Some((0, 5)));
        assert_eq!(selection_columns_for_row(&pane, 5, 10), None);
    }

    #[test]
    fn search_result_selection_copies_exact_visible_characters() {
        let mut store = wiparse_core::log::LiveLogStore::new();
        store.append_lines(["ASK packet", "FSK reply"]);
        let search_map = vec![0, 1];
        let selection = SearchSelectionState {
            anchor: Some(TextCaret { row: 0, col: 9 }),
            focus: Some(TextCaret { row: 0, col: 12 }),
            ..Default::default()
        };

        assert_eq!(
            copy_search_selection(&store, &search_map, &selection).as_deref(),
            Some("ASK")
        );
        assert_eq!(search_selection_columns(&selection, 0, 19), Some((9, 12)));
        assert_eq!(search_selection_columns(&selection, 1, 18), None);
    }

    #[test]
    fn copy_selection_and_current_line_use_caret_ranges() {
        let mut store = wiparse_core::log::LiveLogStore::new();
        store.append_lines(["ASK packet", "FSK reply"]);
        let mut pane = PaneState::default();
        pane.sel_anchor = Some(TextCaret { row: 0, col: 0 });
        pane.sel_focus = Some(TextCaret { row: 0, col: 3 });
        assert_eq!(copy_selection(&store, &pane).as_deref(), Some("ASK"));
        assert_eq!(clipboard_text(&store, &pane), "ASK");

        pane.sel_anchor = Some(TextCaret { row: 1, col: 0 });
        pane.sel_focus = Some(TextCaret { row: 1, col: 0 });
        pane.last_selected_text.clear();
        assert_eq!(copy_current_line(&store, &pane).as_deref(), Some("FSK reply"));
        assert_eq!(clipboard_text(&store, &pane), "FSK reply");
    }

    #[test]
    fn highlight_ranges_merge_overlapping_matches() {
        let needles = vec!["ASK".to_string(), "SK ".to_string()];
        assert_eq!(highlight_ranges("ASK packet", &needles, true), vec![(0, 4)]);
        assert_eq!(
            highlight_ranges("ask packet", &["ASK".into()], false),
            vec![(0, 3)]
        );
    }

    #[test]
    fn double_click_word_range_covers_tokens() {
        assert_eq!(word_char_range("ASK packet", 1), (0, 3));
        assert_eq!(word_char_range("TX1_Idx", 3), (0, 7));
        assert_eq!(word_char_range("hello world", 7), (6, 11));
    }

    #[test]
    fn timestamp_prefix_is_split_at_a_character_boundary() {
        let parts = split_timestamp_prefix("[16:56:33.098] ASK 28");
        assert_eq!(parts.prefix, "[16:56:33.098] ");
        assert_eq!(parts.rest, "ASK 28");

        let plain = split_timestamp_prefix("ASK 28");
        assert!(plain.prefix.is_empty());
        assert_eq!(plain.rest, "ASK 28");

        let padded = split_timestamp_prefix("\u{feff}  [16:56:33.098] FSK 11");
        assert_eq!(padded.prefix, "\u{feff}  [16:56:33.098] ");
        assert_eq!(padded.rest, "FSK 11");

        let imported = split_timestamp_prefix("[R][20:30:30.616] TX1 Idx[f2]");
        assert_eq!(imported.prefix, "[R][20:30:30.616] ");
        assert_eq!(imported.rest, "TX1 Idx[f2]");

        let legacy = split_timestamp_prefix("[20:30:30:616] ASK 28");
        assert_eq!(legacy.prefix, "[20:30:30:616] ");
        assert_eq!(legacy.rest, "ASK 28");
    }

    #[test]
    fn line_editor_preserves_windows_newlines() {
        let session = LineEditorSession::from_text("a\r\nb\r\n");
        assert_eq!(session.newline, "\r\n");
        assert_eq!(session.lines, vec!["a".to_string(), "b".to_string(), String::new()]);
        assert_eq!(session.to_text(), "a\r\nb\r\n");
    }

    #[test]
    fn line_editor_split_and_join_keep_caret() {
        let mut session = LineEditorSession::from_text("hello world");
        session.col = 5;
        editor_split_line(&mut session);
        assert_eq!(session.lines, vec!["hello".to_string(), " world".to_string()]);
        assert_eq!(session.row, 1);
        assert_eq!(session.col, 0);
        assert!(editor_join_prev(&mut session));
        assert_eq!(session.to_text(), "hello world");
        assert_eq!(session.row, 0);
        assert_eq!(session.col, 5);
    }

    #[test]
    fn line_editor_split_rescans_max_chars() {
        let mut session = LineEditorSession::from_text("abcdefghij");
        session.rescan_max_chars();
        assert_eq!(session.max_chars, 10);
        session.col = 1;
        editor_split_line(&mut session);
        assert_eq!(session.lines, vec!["a".to_string(), "bcdefghij".to_string()]);
        assert_eq!(session.max_chars, 9);
    }
}
