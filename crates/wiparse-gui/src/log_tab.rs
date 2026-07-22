//! Log tab — disk-backed file store + live ring buffer, virtualized viewport.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::Receiver;
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, RichText, Stroke, Vec2};
use wiparse_core::i18n::{tr, tr_fmt, Lang};
use wiparse_core::log::{
    build_file_store_worker, collect_match_indices, line_matches_prepared, parse_filter_patterns,
    prepared_needles, FileBuildEvent, FileLogStore, LiveLogStore, LogStore,
};
use wiparse_core::protocol::{decode_qi_message, format_qi_tooltip, QiTipLine, QiTipRole};

use crate::log_view::{show_virtual_log_pane, show_virtual_search_pane};
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
            filter_rx: None,
            filter_busy: false,
            filter_generation: 0,
        }
    }
}

struct FilterScanResult {
    generation: u64,
    map: Vec<usize>,
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
    fn has_text_selection(&self) -> bool {
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
    }

    fn clear_search_sel(&mut self) {
        self.search_selection.clear();
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
        self.last_view_row = self
            .last_view_row
            .and_then(|row| row.checked_sub(evicted));
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
    edit_buffer: String,
    edit_dirty: bool,
    edit_status: String,
    edit_find: String,
    edit_replace: String,
    edit_show_find: bool,
    edit_match_status: String,
    edit_cursor_status: String,
    edit_exit_confirm_open: bool,
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
            edit_buffer: String::new(),
            edit_dirty: false,
            edit_status: String::new(),
            edit_find: String::new(),
            edit_replace: String::new(),
            edit_show_find: false,
            edit_match_status: String::new(),
            edit_cursor_status: String::new(),
            edit_exit_confirm_open: false,
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
                // FileLogStore owns a read-only mmap. Drop it before editing so
                // Windows allows the document to be truncated and rewritten.
                self.backend = TabBackend::Pending;
                for pane in &mut self.panes {
                    pane.filter_rx = None;
                    pane.filter_busy = false;
                    pane.clear_sel();
                }
                self.edit_buffer = text;
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
        self.edit_buffer.clear();
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
        true
    }

    fn discard_edit(&mut self) {
        if let Some(path) = self.filepath.as_ref() {
            if let Ok(text) = std::fs::read_to_string(path) {
                self.edit_buffer = text;
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
        match std::fs::write(&path, &self.edit_buffer) {
            Ok(()) => {
                self.edit_dirty = false;
                self.edit_status.clear();
                self.start_file_reload(path);
                true
            }
            Err(e) => {
                self.edit_status = e.to_string();
                false
            }
        }
    }

    fn save_edit_as(&mut self, path: PathBuf) -> bool {
        match std::fs::write(&path, &self.edit_buffer) {
            Ok(()) => {
                self.filepath = Some(path.to_string_lossy().into_owned());
                self.title = path
                    .file_stem()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                self.edit_dirty = false;
                self.edit_status.clear();
                self.start_file_reload(path);
                true
            }
            Err(error) => {
                self.edit_status = error.to_string();
                false
            }
        }
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
                    self.backend = TabBackend::File(store);
                    self.loading = false;
                    self.index_progress = self.line_count();
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

        let pane_updates: Vec<Option<Vec<usize>>> = self
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
                    return None; // clear/skip
                }
                let patterns = parse_filter_patterns(filter);
                let needles = prepared_needles(patterns.as_deref(), case_sensitive);
                if needles.is_empty() {
                    return None;
                }
                let mut new_matches = Vec::new();
                for idx in start..line_count {
                    if let Some(line) = live.line_at(idx) {
                        if line_matches_prepared(&line, &needles, case_sensitive) {
                            new_matches.push(idx);
                        }
                    }
                }
                Some(new_matches)
            })
            .collect();

        for (pane, update) in self.panes.iter_mut().zip(pane_updates) {
            match update {
                None => {
                    pane.search_map.clear();
                }
                Some(new_matches) => {
                    pane.search_map.extend(new_matches);
                }
            }
        }
    }

    pub fn set_file_store(&mut self, store: Arc<FileLogStore>) {
        self.backend = TabBackend::File(store);
        self.loading = false;
        self.index_progress = self.line_count();
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

    fn refresh_live_views_incremental(&mut self) {
        let case_sensitive = self.case_sensitive;
        let split = self.split_enabled;
        let TabBackend::Live(live) = &self.backend else {
            return;
        };
        let maps: Vec<Vec<usize>> = self
            .panes
            .iter()
            .enumerate()
            .map(|(i, pane)| {
                let filter = if !split && i > 0 {
                    ""
                } else {
                    pane.filter_applied.as_str()
                };
                let patterns = parse_filter_patterns(filter);
                let needles = prepared_needles(patterns.as_deref(), case_sensitive);
                if needles.is_empty() {
                    Vec::new()
                } else {
                    collect_match_indices(live, &needles, case_sensitive)
                }
            })
            .collect();
        for (pane, map) in self.panes.iter_mut().zip(maps) {
            pane.search_map = map;
            // Receiving another live row must not undo an explicit close.
            // Applying/changing the filter opens the panel in refresh_all_views;
            // incremental updates only refresh its contents.
        }
    }

    fn apply_filter(&mut self, idx: usize) {
        if let Some(pane) = self.panes.get_mut(idx) {
            pane.filter_applied = pane.filter_draft.clone();
        }
        self.refresh_all_views();
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
                        pane.search_map = result.map;
                        pane.show_search = was_visible
                            && (!pane.search_map.is_empty() || !pane.filter_applied.is_empty());
                    }
                    pane.filter_busy = false;
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
                pane.show_search = false;
            }
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let generation = if let Some(pane) = self.panes.get_mut(pane_idx) {
            pane.filter_generation = pane.filter_generation.wrapping_add(1);
            pane.filter_rx = Some(rx);
            pane.filter_busy = true;
            pane.show_search = true;
            pane.filter_generation
        } else {
            return;
        };
        thread::spawn(move || {
            let map = collect_match_indices(file.as_ref(), &needles, case_sensitive);
            let _ = tx.send(FilterScanResult { generation, map });
        });
    }

    fn refresh_all_views(&mut self) {
        let case_sensitive = self.case_sensitive;
        let split = self.split_enabled;
        let is_file = matches!(self.backend, TabBackend::File(_));

        let live_maps = if let TabBackend::Live(live) = &self.backend {
            Some(
                self.panes
                    .iter()
                    .enumerate()
                    .map(|(i, pane)| {
                        let filter = if !split && i > 0 {
                            ""
                        } else {
                            pane.filter_applied.as_str()
                        };
                        let patterns = parse_filter_patterns(filter);
                        let needles = prepared_needles(patterns.as_deref(), case_sensitive);
                        if needles.is_empty() {
                            Vec::new()
                        } else {
                            collect_match_indices(live, &needles, case_sensitive)
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        for (i, pane) in self.panes.iter_mut().enumerate() {
            pane.filter_generation = pane.filter_generation.wrapping_add(1);
            pane.filter_rx = None;
            pane.filter_busy = false;
            pane.clear_sel();
            pane.clear_search_sel();
            pane.scroll_to_master = None;
            pane.scroll_generation = 0;
            pane.scroll_hold_frames = 0;
            pane.highlight_master = None;
            pane.parse_row = None;

            let filter = if !split && i > 0 {
                ""
            } else {
                pane.filter_applied.as_str()
            };
            let patterns = parse_filter_patterns(filter);
            let needles = prepared_needles(patterns.as_deref(), case_sensitive);

            if needles.is_empty() {
                pane.search_map.clear();
                pane.show_search = false;
            } else if is_file {
                pane.search_map.clear();
                pane.show_search = true;
            } else if let Some(maps) = &live_maps {
                pane.search_map = maps[i].clone();
                pane.show_search = !pane.search_map.is_empty() || !pane.filter_applied.is_empty();
            } else {
                // File index is still loading. Preserve the committed search
                // intent so set_file_store() can start the scan afterwards.
                pane.search_map.clear();
                pane.show_search = !pane.filter_applied.is_empty();
            }
        }

        if is_file {
            for i in 0..self.panes.len() {
                if !self.panes[i].filter_applied.is_empty() {
                    self.start_file_filter_scan(i);
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        self.poll_background_tasks();

        const TOOLBAR_ROW_H: f32 = 28.0;
        egui::ScrollArea::horizontal()
            .id_salt("log_toolbar")
            .max_height(TOOLBAR_ROW_H)
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(TOOLBAR_ROW_H);
                    ui.set_max_height(TOOLBAR_ROW_H);
                    self.toolbar_body(ui, lang, t);
                });
            });

        ui.add_space(2.0);
        Frame::NONE
            .fill(t.panel_bg)
            .inner_margin(Margin::symmetric(4, 4))
            .show(ui, |ui| {
                self.content_body(ui, lang, t);
            });
        if self.has_background_filter() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        }
        self.paint_edit_exit_confirm(ui.ctx(), lang, t);
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

    fn toolbar_body(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        let focus_edit_find = self.edit_mode
            && !self.live
            && ui.input(|input| {
                (input.modifiers.ctrl || input.modifiers.command) && input.key_pressed(egui::Key::F)
            });
        if focus_edit_find {
            self.edit_show_find = true;
        }

        let focus_filter = !self.edit_mode
            && ui.input(|input| {
                (input.modifiers.ctrl || input.modifiers.command) && input.key_pressed(egui::Key::F)
            });

        if !self.live && !self.loading {
            self.toolbar_edit_controls(ui, lang, t);
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
        let n_filters = if self.split_enabled {
            self.pane_count
        } else {
            1
        };
        self.ensure_panes(n_filters);

        let mut apply_idx = None;
        let mut refresh_case = false;
        for i in 0..n_filters {
            if self.split_enabled {
                ui.label(
                    RichText::new(tr_fmt(lang, "log.pane_filter", i + 1))
                        .size(12.5)
                        .color(t.text_muted),
                );
            } else {
                ui.label(
                    RichText::new(tr(lang, "log.filter"))
                        .size(12.5)
                        .color(t.text_muted),
                );
            }
            {
                let pane = &mut self.panes[i];
                if focus_filter
                    && i == self.selected_pane.min(n_filters - 1)
                    && pane.has_text_selection()
                    && !pane.last_selected_text.is_empty()
                {
                    pane.filter_draft = pane.last_selected_text.clone();
                }
                let edit = ui.add_sized(
                    [if self.split_enabled { 110.0 } else { 180.0 }, 24.0],
                    egui::TextEdit::singleline(&mut pane.filter_draft)
                        .hint_text(tr(lang, "log.filter_placeholder"))
                        .margin(egui::vec2(6.0, 3.0)),
                );
                let enter_pressed = ui.input(|i| {
                    i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Key {
                                key: egui::Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
                    })
                });
                if (focus_filter || pane.focus_filter_requested)
                    && i == self.selected_pane.min(n_filters - 1)
                {
                    edit.request_focus();
                    pane.focus_filter_requested = false;
                }
                // A single-line TextEdit can relinquish focus while handling
                // Enter. Accept both response states so Enter and Apply always
                // commit through the same path.
                if (edit.has_focus() || edit.lost_focus()) && enter_pressed {
                    apply_idx = Some(i);
                }
                if ui
                    .add_sized(
                        [46.0, 24.0],
                        egui::Button::new(tr(lang, "log.filter_apply")),
                    )
                    .clicked()
                {
                    apply_idx = Some(i);
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
            if self.panes[i].filter_draft != self.panes[i].filter_applied {
                ui.label(
                    RichText::new(tr(lang, "log.filter_pending"))
                        .size(11.0)
                        .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                );
            }
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
        } else if self.panes.iter().any(|p| p.filter_busy) {
            ui.label(
                RichText::new(tr(lang, "log.filter_pending"))
                    .size(12.5)
                    .color(t.text_muted),
            );
        }

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

        if let Some(i) = apply_idx {
            self.apply_filter(i);
        } else if refresh_case {
            self.refresh_all_views();
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
            Replace,
            ReplaceAll,
        }
        let mut find_action = None;
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

        let editor_id = ui.make_persistent_id(("log-document-editor", self.view_id));
        let available_height = ui.available_height().max(120.0);
        let desired_rows = ((available_height - 28.0) / (font_size + 4.0)).max(4.0) as usize;
        let mut output = egui::ScrollArea::both()
            .id_salt(("log-document-scroll", self.view_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_size(egui::vec2(
                    ui.available_width().max(320.0),
                    available_height - 28.0,
                ));
                egui::TextEdit::multiline(&mut self.edit_buffer)
                    .id(editor_id)
                    .font(FontId::monospace(font_size))
                    .desired_width(f32::INFINITY)
                    .desired_rows(desired_rows)
                    .code_editor()
                    .show(ui)
            })
            .inner;

        if output.response.changed() {
            self.edit_dirty = true;
        }
        if output.response.clicked() {
            if let Some(pointer) = output.response.interact_pointer_pos() {
                let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
                output
                    .state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(cursor.ccursor)));
                output.state.clone().store(ui.ctx(), output.response.id);
                output.response.request_focus();
            }
        }

        let current_range = output
            .cursor_range
            .map(|range| range.as_ccursor_range())
            .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
        if let Some(action) = find_action {
            match action {
                FindAction::Next => {
                    if let Some((start, end, ordinal, total)) = find_next_range(
                        &self.edit_buffer,
                        &self.edit_find,
                        current_range.primary.index,
                        self.case_sensitive,
                    ) {
                        output.state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(start),
                            CCursor::new(end),
                        )));
                        output.state.store(ui.ctx(), editor_id);
                        output.response.request_focus();
                        self.edit_match_status = format!("{ordinal}/{total}");
                    } else {
                        self.edit_match_status = tr(lang, "log.no_matches");
                    }
                }
                FindAction::Replace => {
                    let (start, end) = sorted_char_range(current_range);
                    let selected = char_slice(&self.edit_buffer, start, end);
                    if !self.edit_find.is_empty()
                        && text_equals(selected, &self.edit_find, self.case_sensitive)
                    {
                        replace_char_range(&mut self.edit_buffer, start, end, &self.edit_replace);
                        self.edit_dirty = true;
                        let replacement_end = start + self.edit_replace.chars().count();
                        output.state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(start),
                            CCursor::new(replacement_end),
                        )));
                        output.state.store(ui.ctx(), editor_id);
                    } else {
                        self.edit_match_status = tr(lang, "log.select_match");
                    }
                }
                FindAction::ReplaceAll => {
                    let count = replace_all_matches(
                        &mut self.edit_buffer,
                        &self.edit_find,
                        &self.edit_replace,
                        self.case_sensitive,
                    );
                    if count > 0 {
                        self.edit_dirty = true;
                    }
                    self.edit_match_status = format!("{}: {count}", tr(lang, "log.replaced"));
                }
            }
        }

        if let Some(range) = output.cursor_range {
            let index = range.primary.ccursor.index;
            let (line, column) = line_column_at(&self.edit_buffer, index);
            self.edit_cursor_status = format!(
                "{} {line}, {} {column}  |  {} {}",
                tr(lang, "log.line"),
                tr(lang, "log.column"),
                tr(lang, "log.characters"),
                self.edit_buffer.chars().count()
            );
        }
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

                    if show_search {
                        let split_h = 6.0_f32.min(avail);
                        let header_h = 22.0_f32.min((avail - split_h).max(0.0));
                        let ratio = self.panes[idx].search_ratio.clamp(0.15, 0.70);
                        let heights = pane_split_heights(avail, ratio, header_h + split_h);
                        let bottom_h = heights.bottom;
                        let top_h = heights.top;
                        let patterns = parse_filter_patterns(&self.panes[idx].filter_applied);
                        let search_needles =
                            prepared_needles(patterns.as_deref(), self.case_sensitive);

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
                                        "{} ({})",
                                        tr(lang, "log.search_results"),
                                        self.panes[idx].search_map.len()
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
                            self.panes[idx].scroll_hold_frames = 12;
                            self.panes[idx].highlight_master = Some(master_idx);
                            self.panes[idx].parse_row = None;
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

fn sorted_char_range(range: CCursorRange) -> (usize, usize) {
    let a = range.primary.index;
    let b = range.secondary.index;
    (a.min(b), a.max(b))
}

fn char_slice(text: &str, start: usize, end: usize) -> &str {
    &text[char_to_byte(text, start)..char_to_byte(text, end)]
}

fn replace_char_range(text: &mut String, start: usize, end: usize, replacement: &str) {
    let byte_start = char_to_byte(text, start);
    let byte_end = char_to_byte(text, end);
    text.replace_range(byte_start..byte_end, replacement);
}

fn text_equals(left: &str, right: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
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
        assert!(page.panes[0].show_search);
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
            map: vec![3, 7],
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
            map: vec![3, 7],
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
    }

    #[test]
    fn document_replace_all_supports_case_insensitive_matches() {
        let mut text = "Ask ASK ask".to_owned();
        let count = replace_all_matches(&mut text, "ask", "FSK", false);
        assert_eq!(count, 3);
        assert_eq!(text, "FSK FSK FSK");
    }

    #[test]
    fn document_cursor_reports_line_and_column() {
        assert_eq!(line_column_at("abc\n中文", 6), (2, 3));
    }
}
