//! Virtualized log viewport — O(visible rows) memory, disk-backed line fetch.

use std::sync::Arc;

use crate::theme::Tokens;
use egui::{
    text::{CCursor, LayoutJob, TextFormat},
    Align, Align2, Color32, FontId, Layout, Pos2, Rect, Sense, Ui, UiBuilder, Vec2,
};
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::log::LogStore;

use super::log_tab::{
    apply_line_parse, PaneState, SearchSelectionState, TextCaret, LOG_FONT_MAX, LOG_FONT_MIN,
    LOG_FONT_STEP,
};

const TEXT_LEFT_PAD: f32 = 4.0;
const SEARCH_PREFIX_CELLS: f32 = 27.0;
/// Pointer movement below this (points) counts as a click, not a drag-select.
const SELECT_DRAG_THRESHOLD: f32 = 4.0;

/// Hit-test rect for text selection: visible content minus floating scrollbar overlay.
fn text_select_interact_rect(ui: &Ui, body: Rect, clip: Rect) -> Rect {
    let scroll = ui.spacing().scroll;
    let gutter = scroll.bar_width
        + scroll.bar_outer_margin
        + scroll.bar_inner_margin
        + scroll.floating_width
        + 4.0;
    let mut rect = body.intersect(clip);
    if rect.width() > gutter + 16.0 {
        rect.max.x -= gutter;
    }
    if rect.height() > gutter + 16.0 {
        rect.max.y -= gutter;
    }
    rect
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
) -> Option<super::log_tab::TextCaret> {
    let total = store.line_count();
    if total == 0 {
        return None;
    }
    let row = (((pos.y - body.top()) / line_height).floor().max(0.0) as usize)
        .min(total.saturating_sub(1));
    let row_top = body.top() + row as f32 * line_height;
    let row_rect = Rect::from_min_max(
        Pos2::new(body.left(), row_top),
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
    if copy_event || ctrl_c {
        let text = copy_selection(store, pane).unwrap_or_else(|| pane.last_selected_text.clone());
        if !text.is_empty() {
            ui.ctx().copy_text(text);
        }
    }
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
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
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
    let max_chars = store.max_line_chars().max(1);
    // Two cells per character is a safe upper bound for CJK fallback glyphs.
    let content_w = (max_chars as f32 * glyph_w * 2.0 + 24.0).max(ui.available_width());

    if ui.input(|i| i.smooth_scroll_delta.y.abs() > 0.0 || i.raw_scroll_delta.y.abs() > 0.0) {
        pane.scroll_pinned = false;
    }

    // Own an exact viewport rectangle. The old nested horizontal ScrollArea had
    // no vertical maximum and consumed the search pane's remaining height.
    let viewport_size = Vec2::new(ui.available_width().max(1.0), height.max(0.0));
    let (viewport_rect, _) = ui.allocate_exact_size(viewport_size, Sense::hover());
    if viewport_rect.height() <= 0.0 {
        return;
    }
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
            // Jump only on the vertical axis. ui.scroll_to_rect on a very wide
            // row also centered the horizontal axis and hid the line prefix.
            if let Some(row) = scroll_to_row {
                scroll_area = scroll_area
                    .horizontal_scroll_offset(0.0)
                    .vertical_scroll_offset(vertical_offset_for_row(
                        row,
                        line_h,
                        viewport_rect.height(),
                    ));
            }
            scroll_area.show(ui, |ui| {
                let total_h = (total_lines as f32 * line_h).max(line_h);
                // Hover-only on the full body so the floating scrollbar can own
                // clicks on the right/bottom edge; selection uses a shrunk rect.
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

                let pointer = ui.ctx().pointer_interact_pos();
                let primary_down = ui.input(|i| i.pointer.primary_down());
                let primary_released = ui.input(|i| i.pointer.primary_released());

                // Match TextEdit: anchor at press, extend on drag; plain click clears.
                if response.drag_started() {
                    if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                        response.request_focus();
                        pane.select_origin = Some(pos);
                        pane.select_dragged = false;
                        pane.selecting = true;
                        if let Some(caret) =
                            caret_at_pos(ui, store, body, pos, line_h, pane.font_size)
                        {
                            pane.last_selected_text.clear();
                            pane.sel_anchor = Some(caret);
                            pane.sel_focus = Some(caret);
                        }
                    }
                } else if pane.selecting && primary_down {
                    if let (Some(origin), Some(pos)) = (pane.select_origin, pointer) {
                        if origin.distance(pos) >= SELECT_DRAG_THRESHOLD {
                            pane.select_dragged = true;
                        }
                        if pane.select_dragged {
                            if let Some(caret) =
                                caret_at_pos(ui, store, body, pos, line_h, pane.font_size)
                            {
                                pane.sel_focus = Some(caret);
                            }
                        }
                    }
                } else if pane.selecting && primary_released {
                    if pane.select_dragged {
                        pane.selecting = false;
                        pane.select_origin = None;
                        pane.select_dragged = false;
                        if let Some(text) = copy_selection(store, pane) {
                            pane.last_selected_text = text;
                        } else {
                            pane.last_selected_text.clear();
                        }
                    } else {
                        pane.clear_sel();
                        pane.last_selected_text.clear();
                    }
                } else if response.clicked() && !response.dragged() {
                    pane.clear_sel();
                    pane.last_selected_text.clear();
                    pane.select_dragged = false;
                }

                if response.secondary_clicked() {
                    response.context_menu(|ui| {
                        if ui.button("Copy").clicked() {
                            let text = copy_selection(store, pane)
                                .unwrap_or_else(|| pane.last_selected_text.clone());
                            if !text.is_empty() {
                                ui.ctx().copy_text(text);
                            }
                            ui.close_menu();
                        }
                    });
                }

                let copy_active = response.hovered() || response.has_focus() || pane.selecting;
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
                    let galley = layout_log_line(ui, &line, pane.font_size, t.text_primary);
                    let text_pos = log_row_text_pos(row_rect, line_h, &galley);
                    let row_clip = clip.intersect(row_rect);
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
                            let len = line.chars().count();
                            pane.sel_anchor = Some(super::log_tab::TextCaret { row, col: 0 });
                            pane.sel_focus = Some(super::log_tab::TextCaret { row, col: len });
                            pane.last_selected_text = line.to_string();
                        }
                    }
                }

                if pane.auto_parse && response.clicked() {
                    if let Some(pos) = pointer.filter(|p| body.contains(*p)) {
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
}

fn highlight_ranges(line: &str, needles: &[String], case_sensitive: bool) -> Vec<(usize, usize)> {
    let haystack = if case_sensitive {
        line.to_owned()
    } else {
        line.to_lowercase()
    };
    let mut ranges = Vec::new();
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        let prepared = if case_sensitive {
            needle.clone()
        } else {
            needle.to_lowercase()
        };
        for (start, _) in haystack.match_indices(&prepared) {
            let end = start + prepared.len();
            // Unicode lowercase can change byte length. Only retain ranges
            // that are valid boundaries in the original text.
            if line.is_char_boundary(start) && line.is_char_boundary(end) {
                ranges.push((start, end));
            }
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn highlighted_job(
    text: &str,
    needles: &[String],
    case_sensitive: bool,
    font_size: f32,
    text_color: Color32,
) -> LayoutJob {
    let font_id = FontId::monospace(font_size);
    let normal = TextFormat {
        font_id: font_id.clone(),
        color: text_color,
        ..Default::default()
    };
    let matched = TextFormat {
        font_id,
        color: Color32::from_rgb(0xF5, 0x9E, 0x0B),
        background: Color32::from_rgba_unmultiplied(245, 158, 11, 36),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut cursor = 0;
    for (start, end) in highlight_ranges(text, needles, case_sensitive) {
        if cursor < start {
            job.append(&text[cursor..start], 0.0, normal.clone());
        }
        job.append(&text[start..end], 0.0, matched.clone());
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
    let mut out = String::new();
    for row in lo.row..=hi.row.min(search_map.len().saturating_sub(1)) {
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
                    let (body, _) = ui.allocate_exact_size(
                        Vec2::new(content_w, total_h.max(line_h)),
                        Sense::hover(),
                    );
                    let clip = ui.clip_rect();
                    let interact_rect = text_select_interact_rect(ui, body, clip);
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

                    // Same as the main log pane: anchor at press, extend on drag.
                    if response.drag_started() {
                        if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                            response.request_focus();
                            selection.select_origin = Some(pos);
                            selection.select_dragged = false;
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
                        );
                        let rest_job = highlighted_job(
                            parts.rest,
                            needles,
                            case_sensitive,
                            font_size,
                            t.text_primary,
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
                        if let Some(pos) = response.interact_pointer_pos() {
                            let row = ((pos.y - body.top()) / line_h).floor().max(0.0) as usize;
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

                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let row = ((pos.y - body.top()) / line_h).floor().max(0.0) as usize;
                            if row < search_map.len() {
                                clicked = Some(row);
                            }
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
    let mut out = String::new();
    for row in lo.row..=hi.row.min(store.line_count().saturating_sub(1)) {
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
    fn highlight_ranges_merge_overlapping_matches() {
        let needles = vec!["ASK".to_string(), "SK ".to_string()];
        assert_eq!(highlight_ranges("ASK packet", &needles, true), vec![(0, 4)]);
        assert_eq!(
            highlight_ranges("ask packet", &["ASK".into()], false),
            vec![(0, 3)]
        );
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
}
