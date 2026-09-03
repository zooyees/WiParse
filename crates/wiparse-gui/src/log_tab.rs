//! Log tab — disk-backed file store + live ring buffer, virtualized viewport.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, CornerRadius, Frame, Margin, Pos2, RichText, Stroke, Vec2};
use wiparse_core::i18n::{tr, tr_fmt, Lang};
use wiparse_core::log::{
    build_file_store_worker, collect_match_hits, match_ranges_in_line, parse_filter_patterns,
    prepared_needles, unique_match_rows, FileBuildEvent, FileLogStore, LiveLogStore, LogStore,
    TextHit, MAX_FILTER_MATCHES, MAX_LIVE_LINES,
};
use wiparse_core::protocol::{decode_qi_message, format_qi_tooltip, QiTipLine, QiTipRole};

use crate::log_view::{
    show_virtual_line_editor, show_virtual_log_pane, show_virtual_search_pane, LineEditorSession,
};
use crate::theme::Tokens;

pub(crate) const LOG_FONT_DEFAULT: f32 = 13.0;
pub(crate) const LOG_FONT_MIN: f32 = 8.0;
pub(crate) const LOG_FONT_MAX: f32 = 24.0;
pub(crate) const LOG_FONT_STEP: f32 = 0.25;
const PROTO_PANEL_W_DEFAULT: f32 = 200.0;
const PROTO_PANEL_W_MIN: f32 = 140.0;
const PROTO_PANEL_W_MAX: f32 = 420.0;
const PROTO_FONT_DEFAULT: f32 = 11.0;
const PROTO_RESIZE_W: f32 = 5.0;
static NEXT_LOG_VIEW_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextCaret {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindJump {
    Next,
    Prev,
    FirstAt { row: usize, start: usize },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SearchSelectionState {
    pub(crate) anchor: Option<TextCaret>,
    pub(crate) focus: Option<TextCaret>,
    pub(crate) selecting: bool,
    pub(crate) select_origin: Option<Pos2>,
    pub(crate) select_dragged: bool,
    pub(crate) last_selected_text: String,
}

impl SearchSelectionState {
    pub(crate) fn clear(&mut self) {
        self.anchor = None;
        self.focus = None;
        self.selecting = false;
        self.select_origin = None;
        self.select_dragged = false;
        self.last_selected_text.clear();
    }
}

#[derive(Clone)]
pub(crate) struct PaneState {
    pub(crate) filter_draft: String,
    pub(crate) filter_applied: String,
    pub(crate) auto_parse: bool,
    pub(crate) font_size: f32,
    pub(crate) proto_font_size: f32,
    pub(crate) proto_width: f32,
    pub(crate) sel_anchor: Option<TextCaret>,
    pub(crate) sel_focus: Option<TextCaret>,
    pub(crate) selecting: bool,
    pub(crate) select_origin: Option<Pos2>,
    /// True once the pointer moved past the click threshold during this drag.
    pub(crate) select_dragged: bool,
    pub(crate) search_selection: SearchSelectionState,
    pub(crate) parse_tip: Option<Rc<Vec<QiTipLine>>>,
    pub(crate) parse_hint: String,
    pub(crate) parse_row: Option<usize>,
    pub(crate) search_map: Vec<usize>,
    pub(crate) show_search: bool,
    pub(crate) search_ratio: f32,
    pub(crate) scroll_to_master: Option<usize>,
    pub(crate) scroll_generation: u64,
    pub(crate) scroll_hold_frames: u8,
    pub(crate) highlight_master: Option<usize>,
    pub(crate) last_selected_text: String,
    pub(crate) focus_filter_requested: bool,
    /// When false, live tail follow (`stick_to_bottom`) is paused until user scrolls to end.
    pub(crate) scroll_pinned: bool,
    pub(crate) reset_horizontal_scroll: bool,
    pub(crate) cached_row_height: Option<(f32, f32)>, // font_size, height
    /// Last first-visible master row — used to prefetch on tab activate.
    pub(crate) last_view_row: Option<usize>,
    pub(crate) find_hits: Vec<TextHit>,
    pub(crate) find_current: Option<usize>,
    pub(crate) find_truncated: bool,
    pub(crate) find_jump: Option<FindJump>,
    pub(crate) filter_apply_at: Option<f64>,
    pub(crate) request_find: bool,
    pub(crate) request_goto: bool,
    filter_rx: Option<Receiver<FilterScanResult>>,
    filter_busy: bool,
    filter_generation: u64,
}

impl Default for PaneState {
    fn default() -> Self {
        Self {
            filter_draft: String::new(),
            filter_applied: String::new(),
            auto_parse: false,
            font_size: LOG_FONT_DEFAULT,
            proto_font_size: PROTO_FONT_DEFAULT,
            proto_width: PROTO_PANEL_W_DEFAULT,
            sel_anchor: None,
            sel_focus: None,
            selecting: false,
            select_origin: None,
            select_dragged: false,
            search_selection: SearchSelectionState::default(),
            parse_tip: None,
            parse_hint: String::new(),
            parse_row: None,
            search_map: Vec::new(),
            show_search: false,
            search_ratio: 0.38,
            scroll_to_master: None,
            scroll_generation: 0,
            scroll_hold_frames: 0,
            highlight_master: None,
            last_selected_text: String::new(),
            focus_filter_requested: false,
            scroll_pinned: true,
            reset_horizontal_scroll: true,
            cached_row_height: None,
            last_view_row: None,
            find_hits: Vec::new(),
            find_current: None,
            find_truncated: false,
            find_jump: None,
            filter_apply_at: None,
            request_find: false,
            request_goto: false,
            filter_rx: None,
            filter_busy: false,
            filter_generation: 0,
        }
    }
}

struct FilterScanResult {
    generation: u64,
    hits: Vec<TextHit>,
    truncated: bool,
}

#[derive(Debug, Clone, Copy)]
struct PaneSplitHeights {
    top: f32,
    bottom: f32,
}

fn pane_split_heights(available: f32, ratio: f32, fixed: f32) -> PaneSplitHeights {
    let usable = (available - fixed).max(0.0);
    let min_each = (usable * 0.5).min(60.0);
    let bottom =
        (usable * ratio.clamp(0.15, 0.70)).clamp(min_each, (usable - min_each).max(min_each));
    PaneSplitHeights {
        top: usable - bottom,
        bottom,
    }
}

impl PaneState {
    pub(crate) fn has_text_selection(&self) -> bool {
        matches!(
            (self.sel_anchor, self.sel_focus),
            (Some(anchor), Some(focus)) if anchor != focus
        )
    }

    pub(crate) fn clear_sel(&mut self) {
        self.sel_anchor = None;
        self.sel_focus = None;
        self.selecting = false;
        self.select_origin = None;
        self.select_dragged = false;
        self.last_selected_text.clear();
    }

    fn clear_search_sel(&mut self) {
        self.search_selection.clear();
    }

    fn reset_navigation(&mut self) {
        self.clear_sel();
        self.clear_search_sel();
        self.parse_tip = None;
        self.parse_hint.clear();
        self.parse_row = None;
        self.highlight_master = None;
        self.scroll_to_master = None;
        self.find_current = None;
        self.find_jump = None;
        self.last_selected_text.clear();
        self.last_view_row = None;
    }

    fn apply_find_index(&mut self, idx: usize) {
        let Some(hit) = self.find_hits.get(idx).copied() else {
            return;
        };
        self.find_current = Some(idx);
        self.scroll_to_master = Some(hit.row);
        self.scroll_hold_frames = 16;
        self.scroll_pinned = false;
        self.highlight_master = Some(hit.row);
        self.parse_row = None;
    }

    fn consume_find_jump(&mut self) {
        let Some(jump) = self.find_jump.take() else {
            return;
        };
        if self.find_hits.is_empty() {
            self.find_current = None;
            return;
        }
        let last = self.find_hits.len() - 1;
        let idx = match jump {
            FindJump::Next => self
                .find_current
                .map(|i| if i >= last { 0 } else { i + 1 })
                .unwrap_or(0),
            FindJump::Prev => self
                .find_current
                .map(|i| if i == 0 { last } else { i - 1 })
                .unwrap_or(last),
            FindJump::FirstAt { row, start } => self
                .find_hits
                .iter()
                .position(|hit| (hit.row, hit.start) >= (row, start))
                .unwrap_or(0),
        };
        self.apply_find_index(idx);
    }

    fn current_find_hit(&self) -> Option<TextHit> {
        self.find_current
            .and_then(|idx| self.find_hits.get(idx).copied())
    }

    fn shift_after_front_eviction(&mut self, evicted: usize) {
        if evicted == 0 {
            return;
        }
        let shift_caret = |caret: TextCaret| {
            caret.row.checked_sub(evicted).map(|row| TextCaret {
                row,
                col: caret.col,
            })
        };
        self.sel_anchor = self.sel_anchor.and_then(shift_caret);
        self.sel_focus = self.sel_focus.and_then(shift_caret);
        if self.sel_anchor.is_none() || self.sel_focus.is_none() {
            self.clear_sel();
        }
        self.clear_search_sel();
        self.parse_row = self.parse_row.and_then(|row| row.checked_sub(evicted));
        self.scroll_to_master = self
            .scroll_to_master
            .and_then(|row| row.checked_sub(evicted));
        self.highlight_master = self
            .highlight_master
            .and_then(|row| row.checked_sub(evicted));
        self.last_view_row = self.last_view_row.and_then(|row| row.checked_sub(evicted));
        let dropped = self
            .find_hits
            .iter()
            .take_while(|hit| hit.row < evicted)
            .count();
        self.find_hits = self
            .find_hits
            .iter()
            .filter_map(|hit| {
                hit.row.checked_sub(evicted).map(|row| TextHit {
                    row,
                    start: hit.start,
                    end: hit.end,
                })
            })
            .collect();
        self.find_current = self.find_current.and_then(|idx| {
            idx.checked_sub(dropped)
                .filter(|next| *next < self.find_hits.len())
        });
        if let Some(FindJump::FirstAt { row, start }) = self.find_jump {
            self.find_jump = row.checked_sub(evicted).map(|row| FindJump::FirstAt {
                row,
                start,
            });
        }
        self.search_map = self
            .search_map
            .iter()
            .filter_map(|&i| i.checked_sub(evicted))
            .collect();
    }
}

enum TabBackend {
    Live(LiveLogStore),
    File(Arc<FileLogStore>),
    Pending,
}

impl TabBackend {
    fn line_count(&self) -> usize {
        match self {
            Self::Live(s) => s.line_count(),
            Self::File(s) => s.line_count(),
            Self::Pending => 0,
        }
    }

    fn line_at(&self, index: usize) -> Option<std::sync::Arc<str>> {
        match self {
            Self::Live(s) => s.line_at(index),
            Self::File(s) => s.line_at(index),
            Self::Pending => None,
        }
    }
}

pub struct LogTabPage {
    view_id: u64,
    pub live: bool,
    pub filepath: Option<String>,
    pub title: String,
    pub loading: bool,
    /// Lines indexed so far (file tab index build progress).
    pub index_progress: usize,
    backend: TabBackend,
    split_enabled: bool,
    pane_count: usize,
    same_page: bool,
    case_sensitive: bool,
    panes: Vec<PaneState>,
    selected_pane: usize,
    auto_scroll: bool,
    edit_mode: bool,
    edit: LineEditorSession,
    edit_dirty: bool,
    edit_status: String,
    edit_find: String,
    edit_replace: String,
    edit_show_find: bool,
    edit_match_status: String,
    edit_cursor_status: String,
    edit_exit_confirm_open: bool,
    goto_open: bool,
    goto_focus_requested: bool,
    goto_draft: String,
    file_reload_rx: Option<Receiver<FileBuildEvent>>,
    /// Live filter maps are stale (appended while serial tab was hidden).
    filter_dirty: bool,
}

impl LogTabPage {
    pub fn live_tab(lang: Lang) -> Self {
        let mut page = Self::new(true, None, tr(lang, "log.live_tab"));
        page.ensure_panes(1);
        page
    }

    pub fn file_tab_empty(path: String) -> Self {
        let title = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let mut page = Self::new(false, Some(path), title);
        page.backend = TabBackend::Pending;
        page.loading = true;
        page.ensure_panes(1);
        page
    }

    fn new(live: bool, filepath: Option<String>, title: String) -> Self {
        Self {
            view_id: NEXT_LOG_VIEW_ID.fetch_add(1, Ordering::Relaxed),
            live,
            filepath,
            title,
            loading: false,
            index_progress: 0,
            backend: TabBackend::Live(LiveLogStore::new()),
            split_enabled: false,
            pane_count: 2,
            same_page: false,
            case_sensitive: false,
            panes: Vec::new(),
            selected_pane: 0,
            auto_scroll: true,
            edit_mode: false,
            edit: LineEditorSession::default(),
            edit_dirty: false,
            edit_status: String::new(),
            edit_find: String::new(),
            edit_replace: String::new(),
            edit_show_find: false,
            edit_match_status: String::new(),
            edit_cursor_status: String::new(),
            edit_exit_confirm_open: false,
            goto_open: false,
            goto_focus_requested: false,
            goto_draft: String::new(),
            file_reload_rx: None,
            filter_dirty: false,
        }
    }

    pub fn display_title(&self) -> String {
        if self.edit_dirty {
            format!("{} *", self.title)
        } else {
            self.title.clone()
        }
    }

    fn enter_edit_mode(&mut self) -> bool {
        if self.live || self.loading {
            return false;
        }
        if self.has_background_filter() {
            self.edit_status = "Wait for the active filter task to finish.".into();
            return false;
        }
        let Some(path) = self.filepath.as_ref() else {
            return false;
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // FileLogStore holds a Windows read-only mmap. Drop every mapping
                // (backend + in-flight reload) before editing so Save can truncate.
                self.release_mapped_file();
                for pane in &mut self.panes {
                    pane.filter_rx = None;
                    pane.filter_busy = false;
                    pane.clear_sel();
                }
                self.edit = LineEditorSession::from_text(&text);
                self.edit_dirty = false;
                self.edit_status.clear();
                self.edit_match_status.clear();
                self.edit_cursor_status.clear();
                self.edit_mode = true;
                true
            }
            Err(e) => {
                self.edit_status = e.to_string();
                false
            }
        }
    }

    fn clear_edit_ui_state(&mut self) {
        self.edit.clear();
        self.edit_status.clear();
        self.edit_match_status.clear();
        self.edit_cursor_status.clear();
    }

    fn exit_edit_mode(&mut self) {
        self.edit_mode = false;
        self.edit_dirty = false;
        self.edit_exit_confirm_open = false;
        self.clear_edit_ui_state();
        if let Some(path) = self.filepath.as_ref().map(PathBuf::from) {
            self.start_file_reload(path);
        }
    }

    fn request_exit_edit_mode(&mut self) {
        if self.edit_dirty {
            self.edit_exit_confirm_open = true;
        } else {
            self.exit_edit_mode();
        }
    }

    fn save_and_exit_edit_mode(&mut self) -> bool {
        if !self.save_edit() {
            return false;
        }
        self.edit_mode = false;
        self.edit_exit_confirm_open = false;
        self.clear_edit_ui_state();
        if let Some(path) = self.filepath.as_ref().map(PathBuf::from) {
            self.start_file_reload(path);
        }
        true
    }

    fn discard_edit(&mut self) {
        if let Some(path) = self.filepath.as_ref() {
            if let Ok(text) = std::fs::read_to_string(path) {
                self.edit = LineEditorSession::from_text(&text);
            }
        }
        self.edit_dirty = false;
        self.edit_status.clear();
        self.edit_match_status.clear();
    }

    fn save_edit(&mut self) -> bool {
        let Some(path) = self.filepath.as_ref() else {
            return false;
        };
        let path = PathBuf::from(path);
        self.release_mapped_file();
        match write_log_file(&path, self.edit.to_text().as_bytes()) {
            Ok(()) => {
                self.edit_dirty = false;
                self.edit_status.clear();
                true
            }
            Err(e) => {
                self.edit_status = e.to_string();
                false
            }
        }
    }

    fn save_edit_as(&mut self, path: PathBuf) -> bool {
        match write_log_file(&path, self.edit.to_text().as_bytes()) {
            Ok(()) => {
                self.filepath = Some(path.to_string_lossy().into_owned());
                self.title = path
                    .file_stem()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                self.edit_dirty = false;
                self.edit_status.clear();
                true
            }
            Err(error) => {
                self.edit_status = error.to_string();
                false
            }
        }
    }

    /// Drop mmap / in-flight file workers so Windows can truncate the log.
    fn release_mapped_file(&mut self) {
        self.backend = TabBackend::Pending;
        if let Some(rx) = self.file_reload_rx.take() {
            while let Ok(ev) = rx.try_recv() {
                drop(ev);
            }
        }
    }

    pub fn is_editing(&self) -> bool {
        self.edit_mode
    }

    fn start_file_reload(&mut self, path: PathBuf) {
        if !self.edit_mode {
            self.backend = TabBackend::Pending;
            self.loading = true;
            self.index_progress = 0;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        self.file_reload_rx = Some(rx);
        thread::spawn(move || build_file_store_worker(path, tx));
    }

    fn poll_file_reload(&mut self) {
        let Some(rx) = self.file_reload_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(FileBuildEvent::Progress { lines }) => {
                if !self.edit_mode {
                    self.set_index_progress(lines);
                }
                self.file_reload_rx = Some(rx);
            }
            Ok(FileBuildEvent::Done(store)) => {
                if self.edit_mode {
                    // Do not remap while the user is still editing — Windows
                    // denies truncate/write on a mapped file (os error 5).
                    drop(store);
                } else {
                    self.set_file_store(store);
                }
            }
            Ok(FileBuildEvent::Err(e)) => {
                self.edit_status = e;
                if !self.edit_mode {
                    self.finish_loading();
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                self.file_reload_rx = Some(rx);
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                if !self.edit_mode {
                    self.finish_loading();
                }
            }
        }
    }

    fn ensure_panes(&mut self, n: usize) {
        let n = n.clamp(1, 16);
        while self.panes.len() < n {
            self.panes.push(PaneState::default());
        }
        self.panes.truncate(n);
    }

    pub fn clear_display(&mut self) {
        if let TabBackend::Live(s) = &mut self.backend {
            s.clear();
        }
        self.filter_dirty = false;
        for pane in &mut self.panes {
            pane.reset_navigation();
        }
        self.refresh_all_views();
    }

    pub fn append_lines(&mut self, lines: impl IntoIterator<Item = String>, update_filters: bool) {
        if let TabBackend::Live(s) = &mut self.backend {
            let lines: Vec<String> = lines.into_iter().collect();
            if lines.is_empty() {
                return;
            }
            let added = lines.len();
            let evicted = s.append_lines(lines);
            for pane in &mut self.panes {
                pane.shift_after_front_eviction(evicted);
            }
            if update_filters {
                self.append_live_filter_matches(added);
                self.filter_dirty = false;
            } else {
                self.filter_dirty = true;
            }
        }
    }

    pub fn on_activated(&mut self) {
        if self.filter_dirty {
            self.refresh_all_views();
            self.filter_dirty = false;
        }
        // Drop stale layout metrics; never cache Galley across tabs — glyph UVs
        // in the font atlas can change when another file introduces new CJK.
        for pane in &mut self.panes {
            pane.cached_row_height = None;
        }
        if let TabBackend::File(store) = &self.backend {
            let anchor = self
                .panes
                .first()
                .and_then(|p| {
                    p.scroll_to_master
                        .or(p.highlight_master)
                        .or(p.last_view_row)
                })
                .unwrap_or(0);
            let start = anchor.saturating_sub(256);
            let end = anchor
                .saturating_add(1024)
                .min(store.line_count().saturating_sub(1).saturating_add(1));
            store.prefetch_range(start, end.max(start.saturating_add(1)));
        }
    }

    fn append_live_filter_matches(&mut self, added: usize) {
        if added == 0 {
            return;
        }
        let case_sensitive = self.case_sensitive;
        let split = self.split_enabled;
        let TabBackend::Live(live) = &self.backend else {
            return;
        };
        let line_count = live.line_count();
        let start = line_count.saturating_sub(added);

        let pane_updates: Vec<Option<(Vec<TextHit>, bool)>> = self
            .panes
            .iter()
            .enumerate()
            .map(|(i, pane)| {
                let filter = if !split && i > 0 {
                    ""
                } else {
                    pane.filter_applied.as_str()
                };
                if filter.is_empty() {
                    return None;
                }
                let patterns = parse_filter_patterns(filter);
                let needles = prepared_needles(patterns.as_deref(), case_sensitive);
                if needles.is_empty() {
                    return None;
                }
                if pane.find_hits.len() >= MAX_FILTER_MATCHES {
                    return Some((Vec::new(), true));
                }
                let mut new_hits = Vec::new();
                let mut truncated = pane.find_truncated;
                for idx in start..line_count {
                    if let Some(line) = live.line_at(idx) {
                        for (byte_start, byte_end) in
                            match_ranges_in_line(&line, &needles, case_sensitive)
                        {
                            if pane.find_hits.len() + new_hits.len() >= MAX_FILTER_MATCHES {
                                truncated = true;
                                break;
                            }
                            new_hits.push(TextHit {
                                row: idx,
                                start: byte_start,
                                end: byte_end,
                            });
                        }
                    }
                    if truncated {
                        break;
                    }
                }
                Some((new_hits, truncated))
            })
            .collect();

        for (pane, update) in self.panes.iter_mut().zip(pane_updates) {
            match update {
                None => {
                    pane.search_map.clear();
                    pane.find_hits.clear();
                    pane.find_truncated = false;
                    pane.find_current = None;
                }
                Some((new_hits, truncated)) => {
                    pane.find_truncated = truncated;
                    pane.find_hits.extend(new_hits);
                    pane.search_map = unique_match_rows(&pane.find_hits);
                }
            }
        }
    }

    pub fn set_file_store(&mut self, store: Arc<FileLogStore>) {
        self.backend = TabBackend::File(store);
        self.loading = false;
        self.index_progress = self.line_count();
        for pane in &mut self.panes {
            pane.reset_navigation();
        }
        self.refresh_all_views();
    }

    pub fn set_index_progress(&mut self, lines: usize) {
        self.index_progress = lines;
        self.loading = true;
    }

    pub fn finish_loading(&mut self) {
        self.loading = false;
        self.index_progress = self.line_count();
    }

    pub fn line_count(&self) -> usize {
        self.backend.line_count()
    }

    pub fn lines_slice(&self, from: usize, limit: usize) -> Vec<String> {
        let total = self.line_count();
        let start = from.min(total);
        let end = (start + limit).min(total);
        (start..end)
            .filter_map(|i| self.backend.line_at(i).map(|s| s.to_string()))
            .collect()
    }

    pub fn recent_lines(&self, limit: usize) -> Vec<String> {
        let total = self.line_count();
        let start = total.saturating_sub(limit);
        self.lines_slice(start, limit)
    }

    pub fn release_display_buffers(&mut self) {
        for pane in &mut self.panes {
            pane.clear_sel();
        }
    }

    fn store_file(&self) -> Option<&Arc<FileLogStore>> {
        match &self.backend {
            TabBackend::File(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn set_pane_filter(&mut self, pane: usize, query: &str) -> Result<(), String> {
        if self.panes.get(pane).is_none() {
            return Err("pane not found".into());
        }
        if let Some(p) = self.panes.get_mut(pane) {
            p.filter_draft = query.to_string();
        }
        self.apply_filter(pane);
        Ok(())
    }

    fn apply_filter(&mut self, idx: usize) {
        let Some(pane) = self.panes.get_mut(idx) else {
            return;
        };
        pane.filter_apply_at = None;
        let changed = pane.filter_applied != pane.filter_draft;
        if !changed {
            if !pane.filter_busy {
                pane.consume_find_jump();
            }
            return;
        }
        pane.filter_applied = pane.filter_draft.clone();
        if !matches!(pane.find_jump, Some(FindJump::FirstAt { .. })) {
            pane.find_current = None;
        }
        self.rebuild_pane_matches(idx);
    }

    fn list_all_matches(&mut self, idx: usize) {
        self.apply_filter(idx);
        if let Some(pane) = self.panes.get_mut(idx) {
            pane.show_search = !pane.filter_applied.is_empty();
        }
    }

    fn caret_byte(&self, caret: TextCaret) -> usize {
        self.backend
            .line_at(caret.row)
            .map(|line| line.chars().take(caret.col).map(|ch| ch.len_utf8()).sum())
            .unwrap_or(0)
    }

    fn queue_find_jump(&mut self, idx: usize, jump: FindJump) {
        if let Some(pane) = self.panes.get_mut(idx) {
            pane.find_jump = Some(jump);
            if pane.filter_busy {
                return;
            }
            pane.consume_find_jump();
        }
    }

    pub fn poll_background_tasks(&mut self) {
        self.poll_file_reload();
        for pane in &mut self.panes {
            let Some(rx) = pane.filter_rx.take() else {
                continue;
            };
            match rx.try_recv() {
                Ok(result) => {
                    if result.generation == pane.filter_generation {
                        let was_visible = pane.show_search;
                        pane.find_hits = result.hits;
                        pane.find_truncated = result.truncated;
                        pane.search_map = unique_match_rows(&pane.find_hits);
                        pane.show_search = was_visible
                            && (!pane.search_map.is_empty() || !pane.filter_applied.is_empty());
                        pane.filter_busy = false;
                        if pane
                            .find_current
                            .is_some_and(|i| i >= pane.find_hits.len())
                        {
                            pane.find_current = None;
                        }
                        pane.consume_find_jump();
                    } else {
                        pane.filter_busy = false;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    pane.filter_rx = Some(rx);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    pane.filter_busy = false;
                }
            }
        }
    }

    pub fn has_background_filter(&self) -> bool {
        self.panes.iter().any(|pane| pane.filter_busy)
    }

    fn start_file_filter_scan(&mut self, pane_idx: usize) {
        let file = match &self.backend {
            TabBackend::File(f) => Arc::clone(f),
            _ => return,
        };
        let (needles, case_sensitive) = {
            let Some(pane) = self.panes.get(pane_idx) else {
                return;
            };
            let patterns = parse_filter_patterns(&pane.filter_applied);
            let needles = prepared_needles(patterns.as_deref(), self.case_sensitive);
            (needles, self.case_sensitive)
        };
        if needles.is_empty() {
            if let Some(pane) = self.panes.get_mut(pane_idx) {
                pane.search_map.clear();
                pane.find_hits.clear();
                pane.find_truncated = false;
                pane.find_current = None;
            }
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let generation = if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.filter_rx = Some(rx);
            pane.filter_busy = true;
            pane.filter_generation
        } else {
            return;
        };
        thread::spawn(move || {
            let (hits, truncated) = collect_match_hits(file.as_ref(), &needles, case_sensitive);
            let _ = tx.send(FilterScanResult {
                generation,
                hits,
                truncated,
            });
        });
    }

    fn rebuild_pane_matches(&mut self, pane_idx: usize) {
        let case_sensitive = self.case_sensitive;
        let split = self.split_enabled;
        let filter = self
            .panes
            .get(pane_idx)
            .map(|pane| {
                if !split && pane_idx > 0 {
                    String::new()
                } else {
                    pane.filter_applied.clone()
                }
            })
            .unwrap_or_default();
        let patterns = parse_filter_patterns(&filter);
        let needles = prepared_needles(patterns.as_deref(), case_sensitive);

        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.filter_generation = pane.filter_generation.wrapping_add(1);
            pane.filter_rx = None;
            pane.filter_busy = false;
            if needles.is_empty() {
                pane.search_map.clear();
                pane.find_hits.clear();
                pane.find_truncated = false;
                pane.find_current = None;
                pane.find_jump = None;
                pane.show_search = false;
                return;
            }
        }

        match &self.backend {
            TabBackend::Live(live) => {
                let (hits, truncated) = collect_match_hits(live, &needles, case_sensitive);
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.find_hits = hits;
                    pane.find_truncated = truncated;
                    pane.search_map = unique_match_rows(&pane.find_hits);
                    if pane
                        .find_current
                        .is_some_and(|i| i >= pane.find_hits.len())
                    {
                        pane.find_current = None;
                    }
                    pane.consume_find_jump();
                }
            }
            TabBackend::File(_) => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.search_map.clear();
                    pane.find_hits.clear();
                    pane.find_truncated = false;
                    if pane.find_jump.is_none() {
                        pane.find_current = None;
                    }
                }
                self.start_file_filter_scan(pane_idx);
            }
            TabBackend::Pending => {
                if let Some(pane) = self.panes.get_mut(pane_idx) {
                    pane.search_map.clear();
                    pane.find_hits.clear();
                    pane.find_truncated = false;
                    if pane.find_jump.is_none() {
                        pane.find_current = None;
                    }
                }
            }
        }
    }

    fn refresh_all_views(&mut self) {
        let n = self.panes.len();
        for i in 0..n {
            if let Some(pane) = self.panes.get_mut(i) {
                if pane.find_jump.is_none() {
                    pane.find_current = None;
                }
            }
            self.rebuild_pane_matches(i);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        self.poll_background_tasks();
        let now = ui.input(|input| input.time);
        self.poll_filter_debounce(now);
        self.handle_view_shortcuts(ui);

        const TOOLBAR_ROW_H: f32 = 28.0;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), TOOLBAR_ROW_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("log_toolbar")
                    .max_height(TOOLBAR_ROW_H)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(TOOLBAR_ROW_H);
                            ui.set_max_height(TOOLBAR_ROW_H);
                            self.toolbar_row_main(ui, lang, t);
                        });
                    });
            },
        );

        ui.add_space(2.0);
        Frame::NONE
            .fill(t.panel_bg)
            .inner_margin(Margin::symmetric(4, 4))
            .show(ui, |ui| {
                self.content_body(ui, lang, t);
            });
        if self.has_background_filter()
            || self
                .panes
                .iter()
                .any(|pane| pane.filter_apply_at.is_some())
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        }
        self.paint_edit_exit_confirm(ui.ctx(), lang, t);
        self.paint_goto_dialog(ui.ctx(), lang, t);
        self.handle_pane_requests();
    }

    fn poll_filter_debounce(&mut self, now: f64) {
        let due: Vec<usize> = self
            .panes
            .iter()
            .enumerate()
            .filter_map(|(idx, pane)| {
                pane.filter_apply_at.filter(|at| now >= *at).map(|_| idx)
            })
            .collect();
        for idx in due {
            self.apply_filter(idx);
        }
    }

    fn handle_view_shortcuts(&mut self, ui: &egui::Ui) {
        let ctrl = ui.input(|input| input.modifiers.ctrl || input.modifiers.command);
        let shift = ui.input(|input| input.modifiers.shift);
        let key_f = ui.input(|input| input.key_pressed(egui::Key::F));
        let key_f3 = ui.input(|input| input.key_pressed(egui::Key::F3));
        let key_g = ui.input(|input| input.key_pressed(egui::Key::G));

        if self.edit_mode {
            if ctrl && key_f {
                self.edit_show_find = true;
            }
            return;
        }
        if self.goto_open || self.edit_exit_confirm_open {
            return;
        }

        let pane_idx = self.selected_pane.min(self.panes.len().saturating_sub(1));
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            if let Some(pane) = self.panes.get_mut(pane_idx) {
                pane.clear_sel();
                pane.clear_search_sel();
            }
        }
        if ctrl && key_g {
            self.open_goto_dialog();
            return;
        }
        if ctrl && key_f {
            let has_selection = self
                .panes
                .get(pane_idx)
                .is_some_and(|pane| pane.has_text_selection() && !pane.last_selected_text.is_empty());
            self.focus_find(pane_idx, has_selection);
            return;
        }
        if key_f3 && ctrl {
            self.find_selection_or_next(pane_idx);
            return;
        }
        if key_f3 {
            self.ensure_filter_applied(pane_idx);
            if shift {
                self.queue_find_jump(pane_idx, FindJump::Prev);
            } else {
                self.queue_find_jump(pane_idx, FindJump::Next);
            }
        }
    }

    fn open_goto_dialog(&mut self) {
        let pane_idx = self.selected_pane.min(self.panes.len().saturating_sub(1));
        let line = self
            .panes
            .get(pane_idx)
            .and_then(|pane| {
                pane.sel_focus
                    .map(|caret| caret.row)
                    .or(pane.highlight_master)
                    .or(pane.last_view_row)
            })
            .unwrap_or(0)
            + 1;
        self.goto_draft = line.to_string();
        self.goto_open = true;
        self.goto_focus_requested = true;
    }

    fn goto_line(&mut self, line_1based: usize) {
        let total = self.line_count();
        if total == 0 {
            return;
        }
        let row = line_1based.saturating_sub(1).min(total - 1);
        let pane_idx = self.selected_pane.min(self.panes.len().saturating_sub(1));
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.scroll_to_master = Some(row);
            pane.scroll_hold_frames = 16;
            pane.scroll_pinned = false;
            pane.highlight_master = Some(row);
        }
    }

    fn focus_find(&mut self, pane_idx: usize, jump: bool) {
        let (selected_text, caret) = self.panes.get(pane_idx).map_or((String::new(), None), |pane| {
            let text = if pane.has_text_selection() {
                pane.last_selected_text.clone()
            } else {
                String::new()
            };
            (text, pane.sel_anchor)
        });
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.focus_filter_requested = true;
            if !selected_text.is_empty() {
                pane.filter_draft = selected_text;
            }
        }
        if jump {
            let start = caret
                .map(|c| (c.row, self.caret_byte(c)))
                .unwrap_or((0, 0));
            if let Some(pane) = self.panes.get_mut(pane_idx) {
                pane.find_jump = Some(FindJump::FirstAt {
                    row: start.0,
                    start: start.1,
                });
            }
            self.apply_filter(pane_idx);
        }
    }

    fn find_selection_or_next(&mut self, pane_idx: usize) {
        let selected = self
            .panes
            .get(pane_idx)
            .and_then(|pane| {
                pane.has_text_selection()
                    .then(|| pane.last_selected_text.clone())
                    .filter(|text| !text.is_empty())
            })
            .unwrap_or_default();
        if selected.is_empty() {
            self.ensure_filter_applied(pane_idx);
            self.queue_find_jump(pane_idx, FindJump::Next);
            return;
        }
        let caret = self.panes.get(pane_idx).and_then(|pane| pane.sel_focus);
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.filter_draft = selected;
        }
        let start = caret
            .map(|c| (c.row, self.caret_byte(c)))
            .unwrap_or((0, 0));
        if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.find_jump = Some(FindJump::FirstAt {
                row: start.0,
                start: start.1.saturating_add(1),
            });
        }
        self.apply_filter(pane_idx);
    }

    fn ensure_filter_applied(&mut self, pane_idx: usize) {
        let dirty = self
            .panes
            .get(pane_idx)
            .is_some_and(|pane| pane.filter_draft != pane.filter_applied);
        if dirty {
            self.apply_filter(pane_idx);
        }
    }

    fn handle_pane_requests(&mut self) {
        let mut find_idx = None;
        let mut goto = false;
        for (idx, pane) in self.panes.iter_mut().enumerate() {
            if pane.request_find {
                pane.request_find = false;
                find_idx = Some(idx);
            }
            if pane.request_goto {
                pane.request_goto = false;
                goto = true;
            }
        }
        if let Some(idx) = find_idx {
            self.focus_find(idx, true);
        }
        if goto {
            self.open_goto_dialog();
        }
    }

    fn paint_goto_dialog(&mut self, ctx: &egui::Context, lang: Lang, t: &Tokens) {
        if !self.goto_open {
            return;
        }
        let mut open = true;
        let mut dismiss = false;
        let mut go = false;
        let request_focus = self.goto_focus_requested;
        self.goto_focus_requested = false;
        egui::Window::new(tr(lang, "log.goto_title"))
            .id(egui::Id::new(("log-goto", self.view_id)))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .frame(
                Frame::window(&ctx.style())
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(240.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.goto_draft)
                        .hint_text(tr(lang, "log.goto_placeholder"))
                        .desired_width(200.0),
                );
                if request_focus {
                    edit.request_focus();
                }
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr(lang, "log.goto")).clicked() || (edit.has_focus() && enter)
                    {
                        go = true;
                    }
                    if ui.button(tr(lang, "log.cancel")).clicked() {
                        dismiss = true;
                    }
                });
            });
        if go {
            if let Ok(line) = self.goto_draft.trim().parse::<usize>() {
                if line > 0 {
                    self.goto_line(line);
                }
            }
            dismiss = true;
        }
        if !open || dismiss {
            self.goto_open = false;
            self.goto_focus_requested = false;
        }
    }

    fn toolbar_row_main(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        if !self.live && !self.loading {
            self.toolbar_edit_controls(ui, lang, t);
            ui.separator();
        }

        if !self.edit_mode {
            self.toolbar_row_find(ui, lang, t);
            ui.separator();
        }

        let was_split = self.split_enabled;
        ui.checkbox(
            &mut self.split_enabled,
            RichText::new(tr(lang, "log.split_enable")).size(12.5),
        );
        if self.split_enabled != was_split {
            if self.split_enabled {
                self.ensure_panes(self.pane_count.max(2));
            } else {
                self.same_page = false;
                self.ensure_panes(1);
            }
        }

        ui.add_enabled_ui(self.split_enabled, |ui| {
            ui.label(
                RichText::new(format!("{}:", tr(lang, "log.split_count")))
                    .size(12.5)
                    .color(t.text_muted),
            );
            let mut count = self.pane_count as i32;
            if ui
                .add_sized([34.0, 24.0], egui::DragValue::new(&mut count).range(1..=16))
                .changed()
            {
                self.pane_count = count as usize;
                self.ensure_panes(self.pane_count);
            }
            ui.checkbox(
                &mut self.same_page,
                RichText::new(tr(lang, "log.same_page")).size(12.5),
            );
        });

        ui.separator();
        if ui
            .checkbox(
                &mut self.auto_scroll,
                RichText::new(tr(lang, "log.follow_tail")).size(12.5),
            )
            .changed()
            && self.auto_scroll
        {
            for pane in &mut self.panes {
                pane.scroll_pinned = true;
            }
        }

        if ui
            .add_sized(
                [56.0, 24.0],
                egui::Button::new(tr(lang, "log.goto")),
            )
            .clicked()
        {
            self.open_goto_dialog();
        }

        if self.loading {
            let n = if self.index_progress > 0 {
                self.index_progress
            } else {
                self.line_count()
            };
            ui.label(
                RichText::new(tr_fmt(lang, "log.loading_lines", n))
                    .size(12.5)
                    .color(t.text_muted),
            );
        }

        if self.live && self.line_count() >= MAX_LIVE_LINES {
            ui.label(
                RichText::new(tr(lang, "log.live_cap"))
                    .size(11.0)
                    .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
            );
        }

        let pane_idx = self.selected_pane.min(self.panes.len().saturating_sub(1));
        if let Some(pane) = self.panes.get(pane_idx) {
            let line = pane
                .sel_focus
                .map(|caret| caret.row)
                .or(pane.highlight_master)
                .or(pane.last_view_row)
                .unwrap_or(0)
                + 1;
            ui.label(
                RichText::new(format!("{} {line}", tr(lang, "log.line")))
                    .size(11.0)
                    .color(t.text_muted),
            );
        }
    }

    fn toolbar_row_find(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let n_filters = if self.split_enabled {
            self.pane_count
        } else {
            1
        };
        self.ensure_panes(n_filters);

        let mut apply_idx = None;
        let mut next_idx = None;
        let mut prev_idx = None;
        let mut list_idx = None;
        let mut refresh_case = false;
        let mut focused_filter = None;
        let now = ui.input(|input| input.time);
        let focus_target = self.selected_pane.min(n_filters.saturating_sub(1));

        for i in 0..n_filters {
            if self.split_enabled {
                ui.label(
                    RichText::new(tr_fmt(lang, "log.pane_filter", i + 1))
                        .size(12.5)
                        .color(t.text_muted),
                );
            } else {
                ui.label(
                    RichText::new(tr(lang, "log.find"))
                        .size(12.5)
                        .color(t.text_muted),
                );
            }
            {
                let pane = &mut self.panes[i];
                let filter_width = if self.split_enabled { 140.0 } else { 200.0 };
                let edit = ui.add_sized(
                    [filter_width, 24.0],
                    egui::TextEdit::singleline(&mut pane.filter_draft)
                        .hint_text(tr(lang, "log.filter_placeholder"))
                        .margin(egui::vec2(6.0, 3.0)),
                );
                if edit.changed() {
                    if pane.filter_draft.is_empty() {
                        pane.filter_apply_at = Some(now);
                    } else {
                        pane.filter_apply_at = Some(now + 0.15);
                    }
                }
                let enter_pressed = ui.input(|input| {
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: egui::Key::Enter,
                                pressed: true,
                                modifiers,
                                ..
                            } if !modifiers.shift
                        )
                    })
                });
                if pane.focus_filter_requested && i == focus_target {
                    edit.request_focus();
                    pane.focus_filter_requested = false;
                }
                if edit.has_focus() {
                    focused_filter = Some(i);
                }
                if (edit.has_focus() || edit.lost_focus()) && enter_pressed {
                    apply_idx = Some(i);
                    next_idx = Some(i);
                }
            }
            if i == 0
                && ui
                    .checkbox(
                        &mut self.case_sensitive,
                        RichText::new(tr(lang, "log.case_sensitive")).size(12.5),
                    )
                    .changed()
            {
                refresh_case = true;
            }
            ui.checkbox(
                &mut self.panes[i].auto_parse,
                RichText::new(tr(lang, "log.auto_parse")).size(12.5),
            );
        }

        if let Some(i) = focused_filter {
            self.selected_pane = i;
        }
        let selected = self.selected_pane.min(n_filters.saturating_sub(1));

        if ui
            .add_sized(
                [56.0, 24.0],
                egui::Button::new(tr(lang, "log.find_prev")),
            )
            .clicked()
        {
            prev_idx = Some(selected);
        }
        if ui
            .add_sized(
                [56.0, 24.0],
                egui::Button::new(tr(lang, "log.find_next")),
            )
            .clicked()
        {
            next_idx = Some(selected);
        }
        if ui
            .add_sized(
                [72.0, 24.0],
                egui::Button::new(tr(lang, "log.list_all")),
            )
            .clicked()
        {
            list_idx = Some(selected);
        }

        let pane = &self.panes[selected];
        let count_text = if pane.filter_busy {
            tr(lang, "log.search_scanning")
        } else if pane.filter_applied.is_empty() {
            String::new()
        } else if pane.find_hits.is_empty() {
            tr(lang, "log.no_matches")
        } else {
            let current = pane.find_current.map(|idx| idx + 1).unwrap_or(0);
            let mut text = format!("{current}/{}", pane.find_hits.len());
            if pane.find_truncated {
                text.push(' ');
                text.push_str(&tr(lang, "log.truncated"));
            }
            text
        };
        if !count_text.is_empty() {
            ui.label(
                RichText::new(count_text)
                    .size(12.0)
                    .color(t.text_muted),
            );
        }
        ui.label(
            RichText::new(tr(lang, "log.find_hint"))
                .size(11.0)
                .color(t.text_muted),
        );

        if let Some(i) = list_idx {
            self.list_all_matches(i);
        } else if let Some(i) = apply_idx {
            self.apply_filter(i);
        } else if refresh_case {
            self.refresh_all_views();
        }
        if let Some(i) = next_idx {
            self.ensure_filter_applied(i);
            self.queue_find_jump(i, FindJump::Next);
        }
        if let Some(i) = prev_idx {
            self.ensure_filter_applied(i);
            self.queue_find_jump(i, FindJump::Prev);
        }
    }

    fn paint_edit_exit_confirm(&mut self, ctx: &egui::Context, lang: Lang, t: &Tokens) {
        if !self.edit_exit_confirm_open {
            return;
        }
        let mut open = true;
        let mut dismiss = false;
        egui::Window::new(tr(lang, "log.unsaved_title"))
            .id(egui::Id::new(("log-edit-exit-confirm", self.view_id)))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .frame(
                Frame::window(&ctx.style())
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                ui.label(tr(lang, "log.unsaved_message"));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(tr(lang, "log.save")).clicked() {
                        if self.save_and_exit_edit_mode() {
                            dismiss = true;
                        }
                    }
                    if ui.button(tr(lang, "log.discard_exit")).clicked() {
                        self.exit_edit_mode();
                        dismiss = true;
                    }
                    if ui.button(tr(lang, "log.cancel")).clicked() {
                        dismiss = true;
                    }
                });
            });
        if !open || dismiss {
            self.edit_exit_confirm_open = false;
        }
    }

    fn toolbar_edit_controls(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let before = self.edit_mode;
        if ui
            .checkbox(
                &mut self.edit_mode,
                RichText::new(tr(lang, "log.edit_mode")).size(12.5),
            )
            .changed()
        {
            if self.edit_mode && !before {
                if !self.enter_edit_mode() {
                    self.edit_mode = false;
                }
            } else if !self.edit_mode && before {
                self.edit_mode = true;
                self.request_exit_edit_mode();
            }
        }

        if self.edit_mode
            && ui
                .button(tr(lang, "log.exit_edit"))
                .on_hover_text(tr(lang, "log.exit_edit_hint"))
                .clicked()
        {
            self.request_exit_edit_mode();
        }

        if !self.edit_mode {
            return;
        }

        let save_shortcut = ui.input(|input| {
            (input.modifiers.ctrl || input.modifiers.command)
                && input.key_pressed(egui::Key::S)
                && !input.modifiers.shift
        });
        let save_as_shortcut = ui.input(|input| {
            (input.modifiers.ctrl || input.modifiers.command)
                && input.modifiers.shift
                && input.key_pressed(egui::Key::S)
        });
        if save_shortcut
            || ui
                .add_enabled(
                    self.edit_dirty,
                    egui::Button::new(tr(lang, "log.save")).min_size(egui::vec2(46.0, 24.0)),
                )
                .clicked()
        {
            self.save_edit();
        }
        if save_as_shortcut
            || ui
                .add(egui::Button::new(tr(lang, "log.save_as")).min_size(egui::vec2(64.0, 24.0)))
                .clicked()
        {
            let mut dialog = rfd::FileDialog::new().add_filter("Log/Text", &["log", "txt"]);
            if let Some(path) = self.filepath.as_ref().map(PathBuf::from) {
                if let Some(parent) = path.parent() {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(name) = path.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            }
            if let Some(path) = dialog.save_file() {
                self.save_edit_as(path);
            }
        }
        if self.edit_dirty
            && ui
                .add(egui::Button::new(tr(lang, "log.discard")).min_size(egui::vec2(64.0, 24.0)))
                .clicked()
        {
            self.discard_edit();
        }
        if self.edit_dirty {
            ui.label(
                RichText::new(tr(lang, "log.unsaved"))
                    .size(11.0)
                    .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
            );
        }
        ui.label(
            RichText::new(tr(lang, "log.edit_hint"))
                .size(11.0)
                .color(t.text_muted),
        );
        if !self.edit_status.is_empty() {
            ui.label(
                RichText::new(&self.edit_status)
                    .size(11.0)
                    .color(Color32::from_rgb(0xEF, 0x44, 0x44)),
            );
        }
    }

    fn edit_body(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let font_size = self
            .panes
            .first()
            .map(|p| p.font_size)
            .unwrap_or(LOG_FONT_DEFAULT);
        let save_shortcut = ui.input(|input| {
            (input.modifiers.ctrl || input.modifiers.command)
                && input.key_pressed(egui::Key::S)
                && !input.modifiers.shift
        });
        if save_shortcut {
            self.save_edit();
        }

        #[derive(Clone, Copy)]
        enum FindAction {
            Next,
            Prev,
            Replace,
            ReplaceAll,
        }
        let mut find_action = None;
        let f3 = ui.input(|input| input.key_pressed(egui::Key::F3));
        let shift = ui.input(|input| input.modifiers.shift);
        if f3 {
            self.edit_show_find = true;
            find_action = Some(if shift {
                FindAction::Prev
            } else {
                FindAction::Next
            });
        }
        if self.edit_show_find {
            Frame::NONE
                .fill(t.surface_bg)
                .stroke(Stroke::new(1.0_f32, t.border))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "log.find"));
                        let find = ui.add(
                            egui::TextEdit::singleline(&mut self.edit_find)
                                .desired_width(180.0)
                                .hint_text(tr(lang, "log.find_placeholder")),
                        );
                        if find.has_focus() {
                            self.edit.has_focus = false;
                        }
                        if find.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            find_action = Some(FindAction::Next);
                        }
                        ui.label(tr(lang, "log.replace"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_replace).desired_width(180.0),
                        );
                        if ui.button(tr(lang, "log.find_next")).clicked() {
                            find_action = Some(FindAction::Next);
                        }
                        if ui.button(tr(lang, "log.find_prev")).clicked() {
                            find_action = Some(FindAction::Prev);
                        }
                        if ui.button(tr(lang, "log.replace_one")).clicked() {
                            find_action = Some(FindAction::Replace);
                        }
                        if ui.button(tr(lang, "log.replace_all")).clicked() {
                            find_action = Some(FindAction::ReplaceAll);
                        }
                        ui.checkbox(&mut self.case_sensitive, tr(lang, "log.case_sensitive"));
                        if ui.small_button("×").clicked() {
                            self.edit_show_find = false;
                        }
                        if !self.edit_match_status.is_empty() {
                            ui.label(
                                RichText::new(&self.edit_match_status)
                                    .small()
                                    .color(t.text_muted),
                            );
                        }
                    });
                });
            ui.add_space(4.0);
        }

        let editor_id_salt = format!("log-document-editor-{}", self.view_id);
        let available_height = ui.available_height().max(120.0);
        let status_h = 22.0;
        let editor_h = (available_height - status_h).max(80.0);
        if show_virtual_line_editor(
            ui,
            &mut self.edit,
            font_size,
            &editor_id_salt,
            editor_h,
            t,
        ) {
            self.edit_dirty = true;
        }

        if let Some(action) = find_action {
            let row = self.edit.row;
            let col = self.edit.col;
            match action {
                FindAction::Next => {
                    if let Some((hit_row, start, end, ordinal, total)) = find_next_in_lines(
                        &self.edit.lines,
                        &self.edit_find,
                        row,
                        col,
                        self.case_sensitive,
                    ) {
                        apply_edit_find_hit(&mut self.edit, hit_row, start, end);
                        self.edit_match_status = format!("{ordinal}/{total}");
                    } else {
                        self.edit_match_status = tr(lang, "log.no_matches");
                    }
                }
                FindAction::Prev => {
                    if let Some((hit_row, start, end, ordinal, total)) = find_prev_in_lines(
                        &self.edit.lines,
                        &self.edit_find,
                        row,
                        col,
                        self.case_sensitive,
                    ) {
                        apply_edit_find_hit(&mut self.edit, hit_row, start, end);
                        self.edit_match_status = format!("{ordinal}/{total}");
                    } else {
                        self.edit_match_status = tr(lang, "log.no_matches");
                    }
                }
                FindAction::Replace => {
                    let row = self.edit.row.min(self.edit.lines.len().saturating_sub(1));
                    let col = self.edit.col;
                    if let Some((s, e)) = current_line_match(
                        &self.edit.lines[row],
                        &self.edit_find,
                        col,
                        self.case_sensitive,
                    ) {
                        replace_char_range(
                            &mut self.edit.lines[row],
                            s,
                            e,
                            &self.edit_replace,
                        );
                        self.edit_dirty = true;
                        self.edit.rescan_max_chars();
                        let new_col = s + self.edit_replace.chars().count();
                        self.edit.col = new_col;
                        self.edit.pending_cursor = Some(CCursorRange::two(
                            CCursor::new(s),
                            CCursor::new(new_col),
                        ));
                        self.edit.want_focus = true;
                    } else {
                        self.edit_match_status = tr(lang, "log.select_match");
                    }
                }
                FindAction::ReplaceAll => {
                    let count = replace_all_in_lines(
                        &mut self.edit.lines,
                        &self.edit_find,
                        &self.edit_replace,
                        self.case_sensitive,
                    );
                    if count > 0 {
                        self.edit_dirty = true;
                        self.edit.rescan_max_chars();
                    }
                    self.edit_match_status = format!("{}: {count}", tr(lang, "log.replaced"));
                }
            }
        }

        self.edit_cursor_status = format!(
            "{} {}, {} {}  |  {} {}",
            tr(lang, "log.line"),
            self.edit.row + 1,
            tr(lang, "log.column"),
            self.edit.col + 1,
            tr(lang, "log.line_count"),
            self.edit.lines.len()
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&self.edit_cursor_status)
                    .small()
                    .color(t.text_muted),
            );
            if self.edit_dirty {
                ui.label(
                    RichText::new(tr(lang, "log.unsaved"))
                        .small()
                        .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                );
            }
        });
    }

    fn content_body(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let content_h = ui.available_height().max(120.0);
        let frame = Frame::NONE
            .fill(t.input_bg)
            .stroke(Stroke::new(1.0_f32, t.border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(4));

        frame.show(ui, |ui| {
            ui.set_min_height(content_h);
            if self.edit_mode && !self.live {
                self.edit_body(ui, lang, t);
                return;
            }
            if self.split_enabled && self.same_page {
                let n = self.pane_count;
                ui.columns(n, |cols| {
                    for (i, col) in cols.iter_mut().enumerate() {
                        col.label(
                            RichText::new(tr_fmt(lang, "log.pane_tab", i + 1))
                                .size(12.0)
                                .color(t.text_muted),
                        );
                        self.show_pane(col, i, lang, t);
                    }
                });
            } else if self.split_enabled {
                ui.horizontal(|ui| {
                    for i in 0..self.pane_count {
                        if ui
                            .selectable_label(
                                self.selected_pane == i,
                                tr_fmt(lang, "log.pane_tab", i + 1),
                            )
                            .clicked()
                        {
                            self.selected_pane = i;
                        }
                    }
                });
                let idx = self.selected_pane.min(self.pane_count.saturating_sub(1));
                self.show_pane(ui, idx, lang, t);
            } else {
                self.show_pane(ui, 0, lang, t);
            }
        });
    }

    fn show_pane(&mut self, ui: &mut egui::Ui, idx: usize, lang: Lang, t: &Tokens) {
        if self.panes.get(idx).is_none() {
            return;
        }
        if ui.ui_contains_pointer() {
            self.selected_pane = idx;
        }
        let auto_parse = self.panes[idx].auto_parse;
        if !auto_parse {
            self.panes[idx].parse_tip = None;
            self.panes[idx].parse_hint.clear();
        }

        let full_w = ui.available_width();
        let full_h = ui.available_height().max(100.0);
        let pane_salt = format!("log_{}_pane_{idx}", self.view_id);
        let search_salt = format!("log_{}_search_{idx}", self.view_id);
        let proto_w = if auto_parse {
            self.panes[idx].proto_width
        } else {
            0.0
        };
        let resize_w = if auto_parse { PROTO_RESIZE_W } else { 0.0 };
        let log_w = (full_w - proto_w - resize_w - if auto_parse { 4.0 } else { 0.0 }).max(120.0);

        ui.horizontal(|ui| {
            if auto_parse {
                show_protocol_panel(ui, &mut self.panes[idx], idx, proto_w, full_h, t, lang);
                let (handle_rect, handle_resp) =
                    ui.allocate_exact_size(egui::vec2(PROTO_RESIZE_W, full_h), egui::Sense::drag());
                ui.painter().rect_filled(handle_rect, 0.0, t.border);
                if handle_resp.dragged() {
                    self.panes[idx].proto_width = (self.panes[idx].proto_width
                        + handle_resp.drag_delta().x)
                        .clamp(PROTO_PANEL_W_MIN, PROTO_PANEL_W_MAX);
                }
            }

            ui.allocate_ui_with_layout(
                egui::vec2(log_w, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let show_search = self.panes[idx].show_search
                        && (!self.panes[idx].search_map.is_empty()
                            || self.panes[idx].filter_busy
                            || !self.panes[idx].filter_applied.is_empty());
                    let avail = ui.available_height().max(0.0);
                    let search_needles = {
                        let patterns = parse_filter_patterns(&self.panes[idx].filter_applied);
                        prepared_needles(patterns.as_deref(), self.case_sensitive)
                    };
                    let current_hit = self.panes[idx]
                        .current_find_hit()
                        .map(|hit| (hit.row, hit.start, hit.end));

                    if show_search {
                        let split_h = 6.0_f32.min(avail);
                        let header_h = 22.0_f32.min((avail - split_h).max(0.0));
                        let ratio = self.panes[idx].search_ratio.clamp(0.15, 0.70);
                        let heights = pane_split_heights(avail, ratio, header_h + split_h);
                        let bottom_h = heights.bottom;
                        let top_h = heights.top;

                        let scroll_once = self.panes[idx].scroll_hold_frames > 0;
                        if self.panes[idx].scroll_hold_frames > 0 {
                            self.panes[idx].scroll_hold_frames =
                                self.panes[idx].scroll_hold_frames.saturating_sub(1);
                        }
                        let scroll_to = if scroll_once {
                            self.panes[idx].scroll_to_master
                        } else {
                            self.panes[idx].scroll_to_master.take()
                        };
                        let highlight = self.panes[idx].highlight_master;

                        match &self.backend {
                            TabBackend::Live(s) => show_virtual_log_pane(
                                ui,
                                s,
                                &mut self.panes[idx],
                                self.auto_scroll && scroll_to.is_none(),
                                &pane_salt,
                                top_h,
                                scroll_to,
                                highlight,
                                t,
                                lang,
                                &search_needles,
                                self.case_sensitive,
                                current_hit,
                            ),
                            TabBackend::File(s) => show_virtual_log_pane(
                                ui,
                                s.as_ref(),
                                &mut self.panes[idx],
                                false,
                                &pane_salt,
                                top_h,
                                scroll_to,
                                highlight,
                                t,
                                lang,
                                &search_needles,
                                self.case_sensitive,
                                current_hit,
                            ),
                            TabBackend::Pending => {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), top_h),
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| {
                                        ui.label(
                                            RichText::new(tr_fmt(
                                                lang,
                                                "log.loading_lines",
                                                self.index_progress,
                                            ))
                                            .color(t.text_muted),
                                        );
                                    },
                                );
                            }
                        }

                        let (split_rect, split_resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), split_h),
                            egui::Sense::drag(),
                        );
                        ui.painter().rect_filled(split_rect, 0.0, t.border);
                        if split_resp.dragged() {
                            let dy = split_resp.drag_delta().y;
                            let usable = top_h + bottom_h;
                            if usable > 1.0 {
                                self.panes[idx].search_ratio =
                                    (self.panes[idx].search_ratio - dy / usable).clamp(0.15, 0.70);
                            }
                        }

                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), header_h),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_height(header_h);
                                ui.label(
                                    RichText::new(format!(
                                        "{} ({}){}",
                                        tr(lang, "log.search_results"),
                                        self.panes[idx].search_map.len(),
                                        if self.panes[idx].find_truncated {
                                            format!(" · {}", tr(lang, "log.truncated"))
                                        } else {
                                            String::new()
                                        }
                                    ))
                                    .size(12.0)
                                    .color(t.text_muted),
                                );
                                if self.panes[idx].filter_busy {
                                    ui.label(RichText::new("…").size(12.0).color(t.text_muted));
                                }
                                if ui
                                    .small_button(tr(lang, "log.search_results_close"))
                                    .clicked()
                                {
                                    self.panes[idx].show_search = false;
                                }
                            },
                        );

                        let pane = &mut self.panes[idx];
                        let active_row = pane
                            .highlight_master
                            .and_then(|master_idx| pane.search_map.binary_search(&master_idx).ok());
                        let jump = match &self.backend {
                            TabBackend::Live(s) => show_virtual_search_pane(
                                ui,
                                s,
                                &pane.search_map,
                                &search_salt,
                                bottom_h,
                                t,
                                lang,
                                active_row,
                                pane.font_size,
                                &search_needles,
                                self.case_sensitive,
                                pane.filter_busy,
                                &mut pane.search_selection,
                            ),
                            TabBackend::File(s) => show_virtual_search_pane(
                                ui,
                                s.as_ref(),
                                &pane.search_map,
                                &search_salt,
                                bottom_h,
                                t,
                                lang,
                                active_row,
                                pane.font_size,
                                &search_needles,
                                self.case_sensitive,
                                pane.filter_busy,
                                &mut pane.search_selection,
                            ),
                            TabBackend::Pending => {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), bottom_h),
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| {
                                        ui.label(
                                            RichText::new(tr(lang, "log.search_scanning"))
                                                .color(t.text_muted),
                                        );
                                    },
                                );
                                None
                            }
                        };
                        if let Some(master_idx) =
                            jump.and_then(|search_row| self.panes[idx].search_map.get(search_row))
                        {
                            let master_idx = *master_idx;
                            self.panes[idx].scroll_to_master = Some(master_idx);
                            // Hold long enough for ScrollArea layout to settle; unpin
                            // live-tail follow so stick_to_bottom cannot yank away.
                            self.panes[idx].scroll_hold_frames = 16;
                            self.panes[idx].scroll_pinned = false;
                            self.panes[idx].highlight_master = Some(master_idx);
                            self.panes[idx].parse_row = None;
                            self.panes[idx].find_current = self.panes[idx]
                                .find_hits
                                .iter()
                                .position(|hit| hit.row == master_idx);
                        }
                    } else {
                        let scroll_once = self.panes[idx].scroll_hold_frames > 0;
                        if self.panes[idx].scroll_hold_frames > 0 {
                            self.panes[idx].scroll_hold_frames =
                                self.panes[idx].scroll_hold_frames.saturating_sub(1);
                        }
                        let scroll_to = if scroll_once {
                            self.panes[idx].scroll_to_master
                        } else {
                            self.panes[idx].scroll_to_master.take()
                        };
                        let highlight = self.panes[idx].highlight_master;
                        let pin = highlight.is_some()
                            || self.panes[idx].parse_row.is_some()
                            || scroll_to.is_some()
                            || scroll_once;

                        match &self.backend {
                            TabBackend::Live(s) => show_virtual_log_pane(
                                ui,
                                s,
                                &mut self.panes[idx],
                                self.auto_scroll && !pin,
                                &pane_salt,
                                avail,
                                scroll_to,
                                highlight,
                                t,
                                lang,
                                &search_needles,
                                self.case_sensitive,
                                current_hit,
                            ),
                            TabBackend::File(s) => show_virtual_log_pane(
                                ui,
                                s.as_ref(),
                                &mut self.panes[idx],
                                false,
                                &pane_salt,
                                avail,
                                scroll_to,
                                highlight,
                                t,
                                lang,
                                &search_needles,
                                self.case_sensitive,
                                current_hit,
                            ),
                            TabBackend::Pending => {
                                ui.centered_and_justified(|ui| {
                                    ui.label(
                                        RichText::new(tr_fmt(
                                            lang,
                                            "log.loading_lines",
                                            self.index_progress,
                                        ))
                                        .color(t.text_muted),
                                    );
                                });
                            }
                        }
                    }
                },
            );
        });
    }
}

fn show_protocol_panel(
    ui: &mut egui::Ui,
    pane: &mut PaneState,
    pane_idx: usize,
    width: f32,
    height: f32,
    t: &Tokens,
    lang: Lang,
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    let zoom_factor = ui.input(|i| i.zoom_delta());

    let out = ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::NONE
                .fill(t.surface_bg)
                .stroke(Stroke::new(1.0_f32, t.border))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr(lang, "log.parse_panel"))
                            .size(12.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt(format!("qi_proto_{pane_idx}"))
                        .max_height((height - 48.0).max(60.0))
                        .enable_scrolling(!ctrl)
                        .show(ui, |ui| {
                            let light = t.canvas_bg.r() > 128;
                            if let Some(tip) = pane.parse_tip.as_ref() {
                                show_qi_tip_ui(ui, tip, light, pane.proto_font_size);
                            } else if !pane.parse_hint.is_empty() {
                                ui.label(
                                    RichText::new(&pane.parse_hint)
                                        .size(pane.proto_font_size)
                                        .color(t.text_muted),
                                );
                            } else {
                                ui.label(
                                    RichText::new(tr(lang, "log.parse_empty"))
                                        .size(pane.proto_font_size)
                                        .color(t.text_muted),
                                );
                            }
                        });
                });
        },
    );

    if out.response.hovered() && ctrl && (zoom_factor - 1.0).abs() > 0.0005 {
        let dir = if zoom_factor > 1.0 { 1.0 } else { -1.0 };
        pane.proto_font_size =
            (pane.proto_font_size + dir * LOG_FONT_STEP).clamp(LOG_FONT_MIN, LOG_FONT_MAX);
    }
}

pub(crate) fn apply_line_parse(pane: &mut PaneState, line: &str, lang: Lang) {
    if line.contains("ASK ") || line.contains("FSK ") {
        match cached_qi_tip_lines(line) {
            Some(tip) => {
                pane.parse_tip = Some(tip);
                pane.parse_hint.clear();
            }
            None => {
                pane.parse_tip = None;
                pane.parse_hint = tr(lang, "log.parse_failed").to_string();
            }
        }
    } else {
        pane.parse_tip = None;
        pane.parse_hint = tr(lang, "log.parse_skip").to_string();
    }
}

fn cached_qi_tip_lines(line: &str) -> Option<Rc<Vec<QiTipLine>>> {
    thread_local! {
        static CACHE: RefCell<(String, Option<Rc<Vec<QiTipLine>>>)> =
            RefCell::new((String::new(), None));
    }
    CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.0 == line {
            return slot.1.clone();
        }
        let tip = decode_qi_message(line).map(|d| Rc::new(format_qi_tooltip(&d)));
        *slot = (line.to_string(), tip.clone());
        tip
    })
}

fn show_qi_tip_ui(ui: &mut egui::Ui, lines: &[QiTipLine], light: bool, font_size: f32) {
    let (ask, fsk, code, hex, ok, err, warn, field, muted, border) = if light {
        (
            Color32::from_rgb(0x03, 0x69, 0xA1),
            Color32::from_rgb(0xB4, 0x53, 0x09),
            Color32::from_rgb(0xF5, 0x9E, 0x0B),
            Color32::from_rgb(0x64, 0x74, 0x8B),
            Color32::from_rgb(0x16, 0xA3, 0x4A),
            Color32::from_rgb(0xDC, 0x26, 0x26),
            Color32::from_rgb(0xD9, 0x77, 0x06),
            Color32::from_rgb(0x0F, 0x17, 0x2A),
            Color32::from_rgb(0x64, 0x74, 0x8B),
            Color32::from_rgb(0x94, 0xA3, 0xB8),
        )
    } else {
        (
            Color32::from_rgb(0x38, 0xBD, 0xF8),
            Color32::from_rgb(0xFB, 0x92, 0x3C),
            Color32::from_rgb(0xF5, 0x9E, 0x0B),
            Color32::from_rgb(0xCB, 0xD5, 0xE1),
            Color32::from_rgb(0x4A, 0xDE, 0x80),
            Color32::from_rgb(0xF8, 0x71, 0x71),
            Color32::from_rgb(0xFB, 0xBF, 0x24),
            Color32::from_rgb(0xF1, 0xF5, 0xF9),
            Color32::from_rgb(0x94, 0xA3, 0xB8),
            Color32::from_rgb(0x47, 0x55, 0x69),
        )
    };
    let fs = font_size.clamp(LOG_FONT_MIN, LOG_FONT_MAX);
    for line in lines {
        let color = match line.role {
            QiTipRole::Separator => {
                ui.add_space(2.0);
                continue;
            }
            QiTipRole::TitleAsk => ask,
            QiTipRole::TitleFsk => fsk,
            QiTipRole::Meta | QiTipRole::Field => field,
            QiTipRole::Code => code,
            QiTipRole::Hex => hex,
            QiTipRole::Ok => ok,
            QiTipRole::Err => err,
            QiTipRole::Warn => warn,
            QiTipRole::Muted => muted,
        };
        ui.label(RichText::new(&line.text).size(fs).color(color));
    }
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn byte_to_char(text: &str, byte_index: usize) -> usize {
    text[..byte_index.min(text.len())].chars().count()
}

fn char_slice(text: &str, start: usize, end: usize) -> &str {
    &text[char_to_byte(text, start)..char_to_byte(text, end)]
}

fn replace_char_range(text: &mut String, start: usize, end: usize, replacement: &str) {
    let byte_start = char_to_byte(text, start);
    let byte_end = char_to_byte(text, end);
    text.replace_range(byte_start..byte_end, replacement);
}

fn find_match_ranges(text: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let (haystack, needle) = if case_sensitive {
        (text.to_owned(), needle.to_owned())
    } else {
        (text.to_ascii_lowercase(), needle.to_ascii_lowercase())
    };
    haystack
        .match_indices(&needle)
        .map(|(byte_start, value)| {
            let byte_end = byte_start + value.len();
            (
                byte_to_char(&haystack, byte_start),
                byte_to_char(&haystack, byte_end),
            )
        })
        .collect()
}

fn find_next_range(
    text: &str,
    needle: &str,
    after: usize,
    case_sensitive: bool,
) -> Option<(usize, usize, usize, usize)> {
    let ranges = find_match_ranges(text, needle, case_sensitive);
    let total = ranges.len();
    let index = ranges
        .iter()
        .position(|(start, _)| *start >= after)
        .unwrap_or(0);
    ranges
        .get(index)
        .map(|(start, end)| (*start, *end, index + 1, total))
}

fn find_prev_range(
    text: &str,
    needle: &str,
    before: usize,
    case_sensitive: bool,
) -> Option<(usize, usize, usize, usize)> {
    let ranges = find_match_ranges(text, needle, case_sensitive);
    let total = ranges.len();
    let index = ranges
        .iter()
        .rposition(|(start, _)| *start < before)
        .unwrap_or(total.saturating_sub(1));
    ranges
        .get(index)
        .map(|(start, end)| (*start, *end, index + 1, total))
}

fn collect_line_hits(
    lines: &[String],
    needle: &str,
    case_sensitive: bool,
) -> Vec<(usize, usize, usize)> {
    let mut hits = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        for (start, end) in find_match_ranges(line, needle, case_sensitive) {
            hits.push((row, start, end));
        }
    }
    hits
}

fn find_next_in_lines(
    lines: &[String],
    needle: &str,
    after_row: usize,
    after_col: usize,
    case_sensitive: bool,
) -> Option<(usize, usize, usize, usize, usize)> {
    let hits = collect_line_hits(lines, needle, case_sensitive);
    let total = hits.len();
    if total == 0 {
        return None;
    }
    let index = hits
        .iter()
        .position(|(row, start, _)| *row > after_row || (*row == after_row && *start >= after_col))
        .unwrap_or(0);
    let (row, start, end) = hits[index];
    Some((row, start, end, index + 1, total))
}

fn find_prev_in_lines(
    lines: &[String],
    needle: &str,
    before_row: usize,
    before_col: usize,
    case_sensitive: bool,
) -> Option<(usize, usize, usize, usize, usize)> {
    let hits = collect_line_hits(lines, needle, case_sensitive);
    let total = hits.len();
    if total == 0 {
        return None;
    }
    let index = hits
        .iter()
        .rposition(|(row, start, _)| {
            *row < before_row || (*row == before_row && *start < before_col)
        })
        .unwrap_or(total.saturating_sub(1));
    let (row, start, end) = hits[index];
    Some((row, start, end, index + 1, total))
}

fn current_line_match(
    line: &str,
    needle: &str,
    col: usize,
    case_sensitive: bool,
) -> Option<(usize, usize)> {
    let ranges = find_match_ranges(line, needle, case_sensitive);
    ranges
        .iter()
        .copied()
        .find(|(start, end)| *start <= col && col <= *end)
        .or_else(|| ranges.first().copied())
}

fn apply_edit_find_hit(edit: &mut LineEditorSession, row: usize, start: usize, end: usize) {
    edit.row = row;
    edit.col = end;
    edit.pending_cursor = Some(CCursorRange::two(CCursor::new(start), CCursor::new(end)));
    edit.scroll_to = Some(row);
    edit.want_focus = true;
}

fn replace_all_in_lines(
    lines: &mut [String],
    needle: &str,
    replacement: &str,
    case_sensitive: bool,
) -> usize {
    let mut count = 0;
    for line in lines.iter_mut() {
        count += replace_all_matches(line, needle, replacement, case_sensitive);
    }
    count
}

fn replace_all_matches(
    text: &mut String,
    needle: &str,
    replacement: &str,
    case_sensitive: bool,
) -> usize {
    let ranges = find_match_ranges(text, needle, case_sensitive);
    for (start, end) in ranges.iter().rev().copied() {
        replace_char_range(text, start, end, replacement);
    }
    ranges.len()
}

/// Write a log file, retrying Windows sharing/access errors after mmap teardown.
fn write_log_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut last = None;
    for _ in 0..12 {
        match std::fs::write(path, bytes) {
            Ok(()) => return Ok(()),
            Err(e) if is_file_lock_error(&e) => {
                last = Some(e);
                thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::from_raw_os_error(5)))
}

fn is_file_lock_error(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(32))
        || e.kind() == io::ErrorKind::PermissionDenied
}

fn line_column_at(text: &str, char_index: usize) -> (usize, usize) {
    let prefix = char_slice(text, 0, char_index.min(text.chars().count()));
    let line = prefix.chars().filter(|value| *value == '\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count() + 1)
        .unwrap_or_else(|| prefix.chars().count() + 1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_heights_never_exceed_available_space() {
        for available in [0.0, 40.0, 100.0, 240.0, 800.0] {
            let fixed = 28.0_f32.min(available);
            let heights = pane_split_heights(available, 0.38, fixed);
            assert!(heights.top >= 0.0);
            assert!(heights.bottom >= 0.0);
            assert!((heights.top + heights.bottom + fixed - available).abs() < 0.01);
        }
    }

    #[test]
    fn tabs_have_independent_scroll_state_ids() {
        let first = LogTabPage::live_tab(Lang::En);
        let second = LogTabPage::live_tab(Lang::En);
        assert_ne!(first.view_id, second.view_id);
        assert!(first.panes[0].reset_horizontal_scroll);
        assert!(second.panes[0].reset_horizontal_scroll);
    }

    #[test]
    fn file_tab_opens_in_view_mode_by_default() {
        let page = LogTabPage::file_tab_empty("sample.log".into());
        assert!(!page.edit_mode);
    }

    #[test]
    fn pending_file_preserves_committed_search_intent() {
        let mut page = LogTabPage::file_tab_empty("pending.log".into());
        page.panes[0].filter_draft = "ASK".into();
        page.apply_filter(0);

        assert_eq!(page.panes[0].filter_applied, "ASK");
        assert!(!page.panes[0].show_search);
        assert!(!page.panes[0].filter_busy);
    }

    #[test]
    fn live_front_eviction_shifts_selection_and_targets() {
        let mut pane = PaneState::default();
        pane.sel_anchor = Some(TextCaret { row: 10, col: 2 });
        pane.sel_focus = Some(TextCaret { row: 12, col: 4 });
        pane.parse_row = Some(11);
        pane.scroll_to_master = Some(20);
        pane.highlight_master = Some(20);

        pane.shift_after_front_eviction(5);

        assert_eq!(pane.sel_anchor, Some(TextCaret { row: 5, col: 2 }));
        assert_eq!(pane.sel_focus, Some(TextCaret { row: 7, col: 4 }));
        assert_eq!(pane.parse_row, Some(6));
        assert_eq!(pane.scroll_to_master, Some(15));
        assert_eq!(pane.highlight_master, Some(15));
    }

    #[test]
    fn stale_scan_result_is_discarded_and_closed_panel_stays_closed() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.panes[0].filter_applied = "ASK".into();
        page.panes[0].filter_generation = 2;
        page.panes[0].filter_busy = true;
        page.panes[0].show_search = false;
        let (tx, rx) = crossbeam_channel::unbounded();
        page.panes[0].filter_rx = Some(rx);
        tx.send(FilterScanResult {
            generation: 1,
            hits: vec![
                TextHit {
                    row: 3,
                    start: 0,
                    end: 3,
                },
                TextHit {
                    row: 7,
                    start: 0,
                    end: 3,
                },
            ],
            truncated: false,
        })
        .unwrap();

        page.poll_background_tasks();

        assert!(page.panes[0].search_map.is_empty());
        assert!(!page.panes[0].show_search);
        assert!(!page.panes[0].filter_busy);

        let (tx, rx) = crossbeam_channel::unbounded();
        page.panes[0].filter_rx = Some(rx);
        page.panes[0].filter_busy = true;
        tx.send(FilterScanResult {
            generation: 2,
            hits: vec![
                TextHit {
                    row: 3,
                    start: 0,
                    end: 3,
                },
                TextHit {
                    row: 7,
                    start: 0,
                    end: 3,
                },
            ],
            truncated: false,
        })
        .unwrap();
        page.poll_background_tasks();
        assert_eq!(page.panes[0].search_map, vec![3, 7]);
        assert!(!page.panes[0].show_search);
    }

    #[test]
    fn live_append_does_not_reopen_closed_search_panel() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK first".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.apply_filter(0);
        assert!(!page.panes[0].show_search);
        page.list_all_matches(0);
        assert!(page.panes[0].show_search);

        page.panes[0].show_search = false;
        page.append_lines(vec!["ASK second".into()], true);

        assert_eq!(page.panes[0].search_map, vec![0, 1]);
        assert!(!page.panes[0].show_search);
    }

    #[test]
    fn search_prefill_requires_a_nonempty_current_selection() {
        let mut pane = PaneState::default();
        pane.last_selected_text = "ASK".into();
        assert!(!pane.has_text_selection());

        pane.sel_anchor = Some(TextCaret { row: 2, col: 4 });
        pane.sel_focus = Some(TextCaret { row: 2, col: 4 });
        assert!(!pane.has_text_selection());

        pane.sel_focus = Some(TextCaret { row: 2, col: 7 });
        assert!(pane.has_text_selection());
    }

    #[test]
    fn document_find_wraps_and_tracks_unicode_characters() {
        let text = "第一行 ASK\n第二行 ASK";
        let first = find_next_range(text, "ASK", 0, true).unwrap();
        let second = find_next_range(text, "ASK", first.1, true).unwrap();
        let wrapped = find_next_range(text, "ASK", second.1, true).unwrap();
        assert_eq!(first.2, 1);
        assert_eq!(second.2, 2);
        assert_eq!((wrapped.0, wrapped.1), (first.0, first.1));
        let prev = find_prev_range(text, "ASK", second.0, true).unwrap();
        assert_eq!((prev.0, prev.1), (first.0, first.1));
    }

    #[test]
    fn apply_filter_does_not_open_results_panel_until_list_all() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK first".into(), "FSK second".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.apply_filter(0);
        assert!(!page.panes[0].show_search);
        assert_eq!(page.panes[0].find_hits.len(), 1);
        page.list_all_matches(0);
        assert!(page.panes[0].show_search);
    }

    #[test]
    fn find_jump_wraps_across_hits() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK a".into(), "x".into(), "ASK b".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.panes[0].find_jump = Some(FindJump::FirstAt { row: 0, start: 0 });
        page.apply_filter(0);
        assert_eq!(page.panes[0].find_current, Some(0));
        page.queue_find_jump(0, FindJump::Next);
        assert_eq!(page.panes[0].find_current, Some(1));
        page.queue_find_jump(0, FindJump::Next);
        assert_eq!(page.panes[0].find_current, Some(0));
        page.queue_find_jump(0, FindJump::Prev);
        assert_eq!(page.panes[0].find_current, Some(1));
    }

    #[test]
    fn document_replace_all_supports_case_insensitive_matches() {
        let mut text = "Ask ASK ask".to_owned();
        let count = replace_all_matches(&mut text, "ask", "FSK", false);
        assert_eq!(count, 3);
        assert_eq!(text, "FSK FSK FSK");
    }

    #[test]
    fn document_find_in_lines_wraps_without_joining_the_file() {
        let lines = vec!["ASK a".into(), "x".into(), "ASK b".into()];
        let first = find_next_in_lines(&lines, "ASK", 0, 0, true).unwrap();
        assert_eq!((first.0, first.1, first.3), (0, 0, 1));
        let second = find_next_in_lines(&lines, "ASK", first.0, first.2, true).unwrap();
        assert_eq!((second.0, second.3), (2, 2));
        let wrapped = find_next_in_lines(&lines, "ASK", second.0, second.2, true).unwrap();
        assert_eq!((wrapped.0, wrapped.1), (first.0, first.1));
    }

    #[test]
    fn document_cursor_reports_line_and_column() {
        assert_eq!(line_column_at("abc\n中文", 6), (2, 3));
    }

    #[test]
    fn debounce_clears_timer_when_draft_matches_applied() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.panes[0].filter_draft = "ASK".into();
        page.panes[0].filter_applied = "ASK".into();
        page.panes[0].filter_apply_at = Some(1.0);
        page.poll_filter_debounce(1.0);
        assert!(page.panes[0].filter_apply_at.is_none());
    }

    #[test]
    fn apply_filter_keeps_current_hit_when_query_unchanged() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK a".into(), "ASK b".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.panes[0].find_jump = Some(FindJump::FirstAt { row: 0, start: 0 });
        page.apply_filter(0);
        page.queue_find_jump(0, FindJump::Next);
        assert_eq!(page.panes[0].find_current, Some(1));
        page.apply_filter(0);
        assert_eq!(page.panes[0].find_current, Some(1));
        assert_eq!(page.panes[0].find_hits.len(), 2);
        page.panes[0].find_jump = Some(FindJump::FirstAt { row: 0, start: 0 });
        page.apply_filter(0);
        assert_eq!(page.panes[0].find_current, Some(0));
    }

    #[test]
    fn apply_filter_resets_current_hit_when_query_changes() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK a".into(), "FSK b".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.panes[0].find_jump = Some(FindJump::FirstAt { row: 0, start: 0 });
        page.apply_filter(0);
        assert_eq!(page.panes[0].find_current, Some(0));
        page.panes[0].filter_draft = "FSK".into();
        page.apply_filter(0);
        assert_eq!(page.panes[0].find_current, None);
        assert_eq!(page.panes[0].find_hits.len(), 1);
        assert_eq!(page.panes[0].find_hits[0].row, 1);
    }

    #[test]
    fn clear_display_drops_selection_and_find_cursor() {
        let mut page = LogTabPage::live_tab(Lang::En);
        page.append_lines(vec!["ASK a".into()], true);
        page.panes[0].filter_draft = "ASK".into();
        page.panes[0].find_jump = Some(FindJump::FirstAt { row: 0, start: 0 });
        page.apply_filter(0);
        page.panes[0].sel_anchor = Some(TextCaret { row: 0, col: 0 });
        page.panes[0].sel_focus = Some(TextCaret { row: 0, col: 3 });
        page.panes[0].highlight_master = Some(0);
        page.clear_display();
        assert!(page.panes[0].sel_anchor.is_none());
        assert!(page.panes[0].find_current.is_none());
        assert!(page.panes[0].highlight_master.is_none());
        assert!(page.panes[0].find_hits.is_empty());
    }
}
