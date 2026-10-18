//! Test Report tool — browse Markdown reports by folder and preview on the right.
//!
//! HTML/PDF are intentionally unsupported: proper in-app rendering needs a
//! browser/PDF engine, which is out of scope for this egui shell.

use crate::theme::{self as ui_theme, Tokens};
use crate::waveform_analysis::WaveformAnalysisPanel;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::project::{
    add_test, add_test_entry, ensure_project, index_project, list_projects, path_in_project_jail,
    remove_test, sanitize_id, test_case_label, test_ref_from_report, tree_snapshot, ArtifactView,
    IndexedFile, IndexedProject, IndexedResult, PluginMeta, ProjectDoc,
};

const SIDE_W: f32 = ui_theme::SIDEBAR_W;
const PANEL_GAP: f32 = 8.0;
const CARD_MARGIN_X: i8 = 10;
const TREE_ROW_H: f32 = 24.0;
const TREE_PAD: f32 = 6.0;
const TREE_INDENT: f32 = 16.0;
const TREE_CHEVRON_W: f32 = 14.0;
const MAX_PREVIEW_CHARS: usize = 400_000;
const MAX_MD_IMAGE_EDGE: u32 = 1600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportAction {
    None,
    Open {
        path: PathBuf,
        view: ArtifactView,
        overlay: bool,
    },
    GoHub,
    ProjectChanged(String),
}

#[derive(Debug, Clone)]
enum PreviewKind {
    /// Parsed once on open — paint is O(blocks), not re-parse each frame.
    Markdown(std::sync::Arc<MdDoc>),
    Text(std::sync::Arc<String>),
    Image,
    Wave,
    EmptyHint(String),
}

/// Lightweight block IR for report Markdown (CommonMark subset + GFM tables).
#[derive(Debug, Clone)]
struct MdDoc {
    blocks: Vec<MdBlock>,
}

#[derive(Debug, Clone)]
enum MdBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    Bullet {
        depth: u8,
        text: String,
    },
    Ordered {
        depth: u8,
        n: u32,
        text: String,
    },
    Task {
        depth: u8,
        checked: bool,
        text: String,
    },
    Quote {
        text: String,
    },
    Code {
        lang: String,
        body: String,
    },
    Image {
        alt: String,
        src: String,
    },
    Table {
        rows: Vec<Vec<String>>,
    },
    Rule,
}

/// Cached Markdown for untrusted AI text. It never resolves local files or fetches images.
pub(crate) struct TextMarkdown(MdDoc);
impl TextMarkdown {
    pub(crate) fn new(source: &str) -> Self {
        Self(parse_md(source))
    }
    pub(crate) fn ui(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        for (index, block) in self.0.blocks.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.add_space(5.0);
                match block {
                    MdBlock::Heading { level, text } => {
                        ui.add_space(7.0);
                        text_markdown_inline(
                            ui,
                            text,
                            match level {
                                1 => 22.0,
                                2 => 19.0,
                                _ => 16.0,
                            },
                            true,
                            tokens,
                        );
                    }
                    MdBlock::Paragraph { text } => {
                        text_markdown_inline(ui, text, 15.0, false, tokens)
                    }
                    MdBlock::Bullet { depth, text }
                    | MdBlock::Ordered { depth, text, .. }
                    | MdBlock::Task { depth, text, .. } => {
                        let marker = match block {
                            MdBlock::Ordered { n, .. } => format!("{n}."),
                            MdBlock::Task { checked, .. } => {
                                if *checked { "☑" } else { "☐" }.into()
                            }
                            _ => "•".into(),
                        };
                        ui.horizontal_top(|ui| {
                            ui.add_space(f32::from(*depth) * 12.0);
                            ui.label(RichText::new(marker).size(15.0));
                            text_markdown_inline(ui, text, 15.0, false, tokens);
                        });
                    }
                    MdBlock::Quote { text } => {
                        Frame::NONE
                            .fill(tokens.input_bg)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                text_markdown_inline(ui, text, 15.0, false, tokens);
                            });
                    }
                    MdBlock::Code { lang, body } => {
                        Frame::NONE
                            .fill(tokens.input_bg)
                            .corner_radius(6)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.weak(if lang.is_empty() { "code" } else { lang });
                                    if ui.small_button("复制 / Copy").clicked() {
                                        ui.ctx().copy_text(body.clone());
                                    }
                                });
                                egui::ScrollArea::horizontal()
                                    .id_salt("code")
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(body).monospace().size(13.0),
                                            )
                                            .wrap_mode(egui::TextWrapMode::Extend)
                                            .selectable(true),
                                        );
                                    });
                            });
                    }
                    MdBlock::Table { rows } => {
                        let width = ui.available_width();
                        egui::ScrollArea::horizontal()
                            .id_salt("table")
                            .show(ui, |ui| {
                                let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                                let cell = (width / cols as f32 - 12.0).clamp(90.0, 280.0);
                                egui::Grid::new("md-table")
                                    .striped(true)
                                    .min_col_width(cell)
                                    .max_col_width(cell)
                                    .spacing([12.0, 8.0])
                                    .show(ui, |ui| {
                                        for (r, row) in rows.iter().enumerate() {
                                            for c in 0..cols {
                                                ui.vertical(|ui| {
                                                    ui.set_width(cell);
                                                    text_markdown_inline(
                                                        ui,
                                                        row.get(c)
                                                            .map(String::as_str)
                                                            .unwrap_or(""),
                                                        14.0,
                                                        r == 0,
                                                        tokens,
                                                    );
                                                });
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                    MdBlock::Image { alt, src } => {
                        ui.label(format!("[图片 / Image] {alt}")).on_hover_text(src);
                    }
                    MdBlock::Rule => {
                        ui.separator();
                    }
                }
            });
        }
    }
}
fn text_markdown_inline(ui: &mut egui::Ui, text: &str, size: f32, strong: bool, tokens: &Tokens) {
    let (mut job, links) = md_inline_layout(text, size, strong, tokens);
    job.wrap.max_width = ui.available_width().max(1.0);
    let galley = ui.fonts(|f| f.layout_job(job));
    let response = ui.add(egui::Label::new(galley.clone()).selectable(true));
    for (link, (section, target)) in links.iter().enumerate() {
        if !(target.starts_with("https://") || target.starts_with("http://"))
            || target.chars().any(char::is_control)
        {
            continue;
        }
        for (row_index, row) in galley.rows.iter().enumerate() {
            let rect = row
                .glyphs
                .iter()
                .filter(|g| g.section_index == *section)
                .fold(egui::Rect::NOTHING, |r, g| r.union(g.logical_rect()));
            if !rect.is_positive() {
                continue;
            }
            let hit = ui
                .interact(
                    rect.translate(response.rect.min.to_vec2()),
                    response.id.with(("ai-link", link, row_index)),
                    egui::Sense::click(),
                )
                .on_hover_text(target)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if hit.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(target));
            }
        }
    }
}

pub struct TestReportPanel {
    project_id: String,
    new_project_id: String,
    projects: Vec<ProjectDoc>,
    index: IndexedProject,
    selected: Option<PathBuf>,
    preview_title: String,
    return_report: Option<PathBuf>,
    preview: Option<PreviewKind>,
    status: String,
    /// Textures for `![alt](path)` relative to the open report file.
    image_textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    add_test_open: bool,
    new_project_open: bool,
    index_dirty: bool,
    last_index_at: Option<Instant>,
    seen_hub_epoch: u64,
}

impl TestReportPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let project_id = sanitize_id(&cfg.apps.project.active_id, "default");
        Self {
            project_id: project_id.clone(),
            new_project_id: String::new(),
            projects: Vec::new(),
            index: IndexedProject {
                id: project_id,
                name: String::new(),
                tests: Vec::new(),
            },
            selected: None,
            preview_title: String::new(),
            return_report: None,
            preview: None,
            status: String::new(),
            image_textures: HashMap::new(),
            add_test_open: false,
            new_project_open: false,
            index_dirty: true,
            last_index_at: None,
            seen_hub_epoch: 0,
        }
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    fn file_count(&self) -> usize {
        self.index.tests.iter().map(|t| t.file_count()).sum()
    }

    pub fn api_snapshot(&self) -> serde_json::Value {
        let mut tree = tree_snapshot(&self.index);
        if let Some(obj) = tree.as_object_mut() {
            obj.insert(
                "selected".into(),
                serde_json::json!(self.selected.as_ref().map(|p| p.display().to_string())),
            );
            obj.insert("status".into(), serde_json::json!(self.status));
            obj.insert("file_count".into(), serde_json::json!(self.file_count()));
        }
        tree
    }

    pub fn api_set_browser_dir(
        &mut self,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        let _ = params;
        ok(
            "ui.report.browser",
            serde_json::json!({
                "deprecated": true,
                "hint": "use ui.project.select / ui.report.open",
                "snapshot": self.api_snapshot(),
            }),
        )
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        data_root: &Path,
        plugins: &[PluginMeta],
        running_id: Option<&str>,
        hub_project: &str,
        hub_epoch: u64,
        waveform: Option<&mut WaveformAnalysisPanel>,
    ) -> ReportAction {
        if self.status.is_empty() {
            self.status = tr(lang, "report.status_ready");
        }
        if hub_project != self.project_id && wiparse_core::project::is_valid_id(hub_project) {
            self.project_id = hub_project.to_owned();
            self.index_dirty = true;
        }
        self.maybe_rebuild(data_root, plugins, running_id.is_some(), hub_epoch);

        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        if !full.is_positive() {
            return ReportAction::None;
        }

        let side_w = ui_theme::resizable_sidebar_width(
            ui,
            "test_report",
            full,
            SIDE_W,
            220.0,
            220.0,
            PANEL_GAP,
            tokens,
        );
        let view_w = (full.width() - side_w - PANEL_GAP).max(0.0);
        let side_rect = egui::Rect::from_min_size(full.min, egui::vec2(side_w, full.height()));
        let view_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, full.min.y),
            egui::vec2(view_w, full.height()),
        );

        let mut action = ReportAction::None;
        panel_in_rect(ui, side_rect, |ui| {
            action = self.browser_panel(ui, lang, tokens, data_root, plugins, running_id);
        });
        let mut view_action = ReportAction::None;
        panel_in_rect(ui, view_rect, |ui| {
            view_action = self.viewer_panel(ui, lang, tokens, waveform);
        });
        if !matches!(view_action, ReportAction::None) {
            view_action
        } else {
            action
        }
    }

    fn rebuild_index(&mut self, data_root: &Path, plugins: &[PluginMeta]) {
        let doc = ensure_project(data_root, &self.project_id, None);
        self.project_id = doc.id.clone();
        self.refresh_project_list(data_root);
        self.index = index_project(data_root, &doc, plugins);
        self.index_dirty = false;
        self.last_index_at = Some(Instant::now());
    }

    fn refresh_project_list(&mut self, data_root: &Path) {
        self.projects = list_projects(data_root);
        if !self.projects.iter().any(|p| p.id == self.project_id) {
            self.projects.push(wiparse_core::project::ProjectDoc::new(
                &self.project_id,
                None,
            ));
            self.projects
                .sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
        }
    }

    fn maybe_rebuild(
        &mut self,
        data_root: &Path,
        plugins: &[PluginMeta],
        running: bool,
        hub_epoch: u64,
    ) {
        if hub_epoch != self.seen_hub_epoch {
            self.seen_hub_epoch = hub_epoch;
            self.index_dirty = true;
        }
        let interval = if running {
            Duration::from_millis(800)
        } else {
            Duration::from_millis(2500)
        };
        let stale = self
            .last_index_at
            .map(|t| t.elapsed() >= interval)
            .unwrap_or(true);
        if self.index_dirty || stale {
            self.rebuild_index(data_root, plugins);
        }
    }

    fn persist_project(&self) {
        if let Ok(mut cfg) = wiparse_core::config::load_config() {
            cfg.apps.project.active_id = self.project_id.clone();
            if let Err(error) = wiparse_core::config::save_config(&cfg) {
                tracing::warn!("failed to save report project config: {error}");
            }
        }
    }

    pub fn select_project(
        &mut self,
        data_root: &Path,
        plugins: &[PluginMeta],
        id: &str,
    ) -> Result<String, String> {
        let id = sanitize_id(id, "default");
        ensure_project(data_root, &id, None);
        self.project_id = id.clone();
        self.persist_project();
        self.rebuild_index(data_root, plugins);
        Ok(id)
    }

    pub fn create_project(
        &mut self,
        data_root: &Path,
        plugins: &[PluginMeta],
        id: &str,
        name: Option<&str>,
    ) -> Result<String, String> {
        let raw = id.trim();
        if !wiparse_core::project::is_valid_id(raw) {
            return Err("invalid project id".into());
        }
        let doc = ensure_project(data_root, raw, name);
        self.project_id = doc.id.clone();
        self.persist_project();
        self.rebuild_index(data_root, plugins);
        Ok(doc.id)
    }

    pub fn add_hub_test(
        &mut self,
        data_root: &Path,
        plugins: &[PluginMeta],
        test_id: &str,
    ) -> Result<(), String> {
        add_test(data_root, &self.project_id, test_id).map_err(|e| e.to_string())?;
        self.index_dirty = true;
        self.rebuild_index(data_root, plugins);
        Ok(())
    }

    fn apply_open(
        &mut self,
        path: PathBuf,
        view: ArtifactView,
        overlay: bool,
        lang: Lang,
        action: &mut ReportAction,
    ) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let view = open_view_for_file(view, view, name);
        self.selected = Some(path.clone());
        self.preview_title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if is_markdown(&path) {
            self.open_report(path, lang);
            return;
        }
        if view == ArtifactView::Wave || is_wave_file(&path) {
            self.preview = Some(PreviewKind::Wave);
            self.status = format!(
                "{} · {}",
                tr(lang, "report.status_wave"),
                self.preview_title
            );
            *action = ReportAction::Open {
                path,
                view: ArtifactView::Wave,
                overlay,
            };
            return;
        }
        if view == ArtifactView::Log || is_text_file(&path) {
            self.open_text(path, lang);
            return;
        }
        if is_image_file(&path) {
            self.image_textures.clear();
            self.preview = Some(PreviewKind::Image);
            self.status = format!(
                "{} · {}",
                tr(lang, "report.status_loaded"),
                self.preview_title
            );
            return;
        }
        if view == ArtifactView::External {
            if open_path_external(&path) {
                self.preview = Some(PreviewKind::EmptyHint(tr(lang, "report.open_external")));
                self.status = format!(
                    "{} · {}",
                    tr(lang, "report.open_external"),
                    self.preview_title
                );
            } else {
                self.status = tr(lang, "report.status_open_err").to_string();
            }
            return;
        }
        *action = ReportAction::Open {
            path,
            view,
            overlay,
        };
    }

    pub fn remove_hub_test(
        &mut self,
        data_root: &Path,
        plugins: &[PluginMeta],
        test_id: &str,
    ) -> Result<(), String> {
        remove_test(data_root, &self.project_id, test_id).map_err(|e| e.to_string())?;
        self.index_dirty = true;
        self.rebuild_index(data_root, plugins);
        Ok(())
    }

    pub fn prepare_open(
        &self,
        data_root: &Path,
        path: &Path,
        view: Option<&str>,
    ) -> Result<(PathBuf, ArtifactView), String> {
        if !path_in_project_jail(data_root, &self.project_id, path) {
            return Err("path outside project".into());
        }
        if !path.is_file() {
            return Err("not a file".into());
        }
        let view = view
            .map(ArtifactView::parse)
            .unwrap_or_else(|| ArtifactView::from_filename(&path.display().to_string()));
        Ok((path.to_path_buf(), view))
    }

    pub fn open_markdown(&mut self, path: PathBuf, lang: Lang) {
        self.return_report = None;
        self.open_report(path, lang);
    }

    pub fn open_in_pane(&mut self, path: PathBuf, view: ArtifactView, overlay: bool, lang: Lang) {
        let mut action = ReportAction::None;
        self.apply_open(path, view, overlay, lang, &mut action);
        let _ = action;
    }

    fn browser_panel(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        data_root: &Path,
        plugins: &[PluginMeta],
        running_id: Option<&str>,
    ) -> ReportAction {
        self.refresh_project_list(data_root);
        let mut action = ReportAction::None;
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 10))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = inner_w;

                let cap = tr(lang, "report.project");
                let cap_w = ui.fonts(|f| {
                    f.layout_no_wrap(
                        cap.clone(),
                        FontId::proportional(ui_theme::FONT_CAPTION),
                        tokens.text_muted,
                    )
                    .size()
                    .x
                }) + 8.0;
                let (prow, _) = ui.allocate_exact_size(
                    egui::vec2(ctrl_w, ui_theme::CTRL_H),
                    egui::Sense::hover(),
                );
                ui.painter_at(prow).text(
                    egui::pos2(prow.min.x, prow.center().y),
                    egui::Align2::LEFT_CENTER,
                    &cap,
                    FontId::proportional(ui_theme::FONT_CAPTION),
                    tokens.text_muted,
                );
                let refresh_w = ui_theme::CTRL_H;
                let combo_rect = egui::Rect::from_min_max(
                    egui::pos2(prow.min.x + cap_w, prow.min.y),
                    egui::pos2(
                        (prow.max.x - refresh_w - ui_theme::SPACE_SM).max(prow.min.x + cap_w),
                        prow.max.y,
                    ),
                );

                let current_label = self
                    .projects
                    .iter()
                    .find(|p| p.id == self.project_id)
                    .map(|p| {
                        if p.name != p.id {
                            format!("{}  ({})", p.name, p.id)
                        } else {
                            p.name.clone()
                        }
                    })
                    .unwrap_or_else(|| self.project_id.clone());
                let mut picked = self.project_id.clone();
                let mut open_new_project = false;
                let projects = self.projects.clone();
                crate::theme::scope_in_allocated_rect(ui,
                    egui::UiBuilder::new()
                        .max_rect(combo_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().interact_size.y = ui_theme::CTRL_H;
                        egui::ComboBox::from_id_salt("report_project")
                            .selected_text(&current_label)
                            .width(combo_rect.width().max(1.0))
                            .height(280.0)
                            .show_ui(ui, |ui| {
                                for p in &projects {
                                    let label = if p.name != p.id {
                                        format!("{}  ({})", p.name, p.id)
                                    } else {
                                        p.name.clone()
                                    };
                                    ui.selectable_value(&mut picked, p.id.clone(), label);
                                }
                                ui.separator();
                                if ui
                                    .button(tr(lang, "report.project_new_item"))
                                    .clicked()
                                {
                                    open_new_project = true;
                                }
                            });
                    },
                );
                let refresh_rect = egui::Rect::from_min_max(
                    egui::pos2(prow.max.x - refresh_w, prow.min.y),
                    prow.max,
                );
                let refresh_clicked = crate::theme::scope_in_allocated_rect(ui,
                        egui::UiBuilder::new()
                            .max_rect(refresh_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            ui_theme::secondary_btn_sized(
                                ui,
                                tokens,
                                "⟳",
                                egui::vec2(refresh_w, ui_theme::CTRL_H),
                            )
                            .on_hover_text(tr(lang, "btn.refresh_browser"))
                            .clicked()
                        },
                    )
                    .inner;
                if refresh_clicked {
                    self.index_dirty = true;
                    self.rebuild_index(data_root, plugins);
                }
                if open_new_project {
                    self.new_project_open = true;
                }
                if picked != self.project_id {
                    self.project_id = picked;
                    self.persist_project();
                    self.rebuild_index(data_root, plugins);
                    action = ReportAction::ProjectChanged(self.project_id.clone());
                }

                if self.new_project_open {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let edit_w = (ctrl_w - 70.0).max(80.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_project_id)
                                .desired_width(edit_w)
                                .hint_text(tr(lang, "report.project_new_hint"))
                                .margin(egui::vec2(6.0, 4.0)),
                        );
                        if ui_theme::secondary_btn_sized(
                            ui,
                            tokens,
                            tr(lang, "report.project_new"),
                            egui::vec2(64.0, ui_theme::CTRL_H),
                        )
                        .clicked()
                        {
                            let raw = self.new_project_id.trim().to_owned();
                            if wiparse_core::project::is_valid_id(&raw) {
                                let doc = ensure_project(data_root, &raw, Some(&raw));
                                self.project_id = doc.id;
                                self.new_project_id.clear();
                                self.new_project_open = false;
                                self.persist_project();
                                self.rebuild_index(data_root, plugins);
                                action = ReportAction::ProjectChanged(self.project_id.clone());
                            } else {
                                self.status = tr(lang, "report.project_id_invalid").to_string();
                            }
                        }
                    });
                }

                if ui_theme::secondary_btn_sized(
                    ui,
                    tokens,
                    tr(lang, "report.add_test"),
                    egui::vec2((ctrl_w * 0.46).max(96.0), ui_theme::CTRL_H),
                )
                .clicked()
                {
                    self.add_test_open = !self.add_test_open;
                }

                if self.add_test_open {
                    let mut added: Option<wiparse_core::project::ProjectTestRef> = None;
                    for p in plugins {
                        let entry = test_ref_from_report(&p.id, p.report_slug());
                        if self.index.tests.iter().any(|t| t.id == entry.id) {
                            continue;
                        }
                        let title = p.report_label(matches!(lang, Lang::Zh));
                        let title = if title.is_empty() {
                            entry.display_name()
                        } else {
                            title
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("+ {title}"))
                                        .size(ui_theme::FONT_BODY)
                                        .color(tokens.text_primary),
                                )
                                .fill(Color32::TRANSPARENT)
                                .min_size(egui::vec2(ctrl_w, 22.0)),
                            )
                            .clicked()
                        {
                            added = Some(entry);
                        }
                    }
                    if plugins.iter().all(|p| {
                        let entry = test_ref_from_report(&p.id, p.report_slug());
                        self.index.tests.iter().any(|t| t.id == entry.id)
                    }) {
                        ui.label(
                            RichText::new(tr(lang, "report.add_test_empty"))
                                .size(ui_theme::FONT_CAPTION)
                                .color(tokens.text_muted),
                        );
                    }
                    if let Some(entry) = added {
                        let shown = entry.display_name().to_owned();
                        if add_test_entry(data_root, &self.project_id, entry).is_ok() {
                            self.add_test_open = false;
                            self.rebuild_index(data_root, plugins);
                            self.status = format!("{} · {shown}", tr(lang, "report.add_test"));
                        }
                    }
                }

                ui.label(RichText::new(if lang == Lang::Zh {
                    "测试项目 → 运行批次 → MD 报告"
                } else { "Test → Run → Markdown report" })
                    .size(ui_theme::FONT_CAPTION).color(tokens.text_muted));

                let results_n: usize = self.index.tests.iter().map(|t| t.results.len()).sum();
                ui.label(
                    RichText::new(format!(
                        "{} {} · {} {}",
                        self.index.tests.len(),
                        tr(lang, "report.test_count"),
                        results_n,
                        tr(lang, "report.result_count"),
                    ))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
                );

                if self.index.tests.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "report.empty_tests"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "report.go_hub"),
                        egui::vec2(ctrl_w, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        action = ReportAction::GoHub;
                    }
                } else {
                    ui.add_space(4.0);
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        ui.cursor().top(),
                        Stroke::new(1.0_f32, tokens.divider),
                    );
                    ui.add_space(6.0);
                    let list_h = ui.available_height().max(72.0);
                    let mut open_file: Option<(PathBuf, ArtifactView, bool)> = None;
                    let mut remove_id: Option<String> = None;
                    let selected = self.selected.as_ref().filter(|p| is_markdown(p)).cloned()
                        .or_else(|| self.return_report.clone());
                    let tests = self.index.tests.clone();
                    ui.spacing_mut().scroll.floating = false;
                    egui::ScrollArea::vertical()
                        .id_salt("report_project_tests")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (ti, test) in tests.iter().enumerate() {
                                if ti > 0 {
                                    ui.add_space(6.0);
                                }
                                let title = test_display_name(test, plugins, lang);
                                let running = running_id == Some(test.id.as_str())
                                    || running_id == Some(test.plugin_id.as_str());
                                let trail = if running {
                                    Some(TreeTrail::Status {
                                        text: tr(lang, "report.running"),
                                        color: tokens.accent,
                                    })
                                } else if !test.installed {
                                    Some(TreeTrail::Status {
                                        text: tr(lang, "report.missing_plugin"),
                                        color: tokens.text_muted,
                                    })
                                } else {
                                    None
                                };
                                let color = if test.installed {
                                    tokens.text_primary
                                } else {
                                    tokens.text_muted
                                };
                                let (open, header_resp) = tree_branch_row(
                                    ui,
                                    tokens,
                                    ("report-test", test.id.as_str()),
                                    0,
                                    &title,
                                    color,
                                    true,
                                    true,
                                    ti == 0 || running,
                                    false,
                                    trail.as_ref(),
                                );
                                header_resp.context_menu(|ui| {
                                    if ui.button(tr(lang, "report.remove_test")).clicked() {
                                        remove_id = Some(test.id.clone());
                                        ui.close_menu();
                                    }
                                });
                                if header_resp.double_clicked() {
                                    if let Some(f) = test.latest_report() {
                                        open_file = Some((f.path.clone(), ArtifactView::Report, false));
                                    }
                                }
                                if !open {
                                    continue;
                                }
                                if !test.installed {
                                    tree_hint_row(
                                        ui,
                                        tokens,
                                        1,
                                        &tr(lang, "report.test_missing"),
                                    );
                                }
                                if test.results.is_empty() {
                                    tree_hint_row(
                                        ui,
                                        tokens,
                                        1,
                                        &tr(lang, "report.test_no_runs"),
                                    );
                                    continue;
                                }
                                let stamps: Vec<&str> =
                                    test.results.iter().map(|r| r.stamp.as_str()).collect();
                                let titles = run_row_titles(&stamps);
                                for (ri, result) in test.results.iter().enumerate() {
                                    let rtitle = titles.get(ri).map(String::as_str).unwrap_or("");
                                    let trail = result.ok.map(|ok| {
                                        TreeTrail::Dot(if ok {
                                            tokens.success
                                        } else {
                                            tokens.stop_bg
                                        })
                                    });
                                    let reports = report_files(result);
                                    let run_selected = selected.as_ref().is_some_and(|p| {
                                        reports.iter().any(|f| same_path(p, &f.path))
                                    });
                                    let (ropen, rresp) = tree_branch_row(
                                        ui,
                                        tokens,
                                        ("report-result", test.id.as_str(), result.stamp.as_str()),
                                        1,
                                        rtitle,
                                        tokens.text_primary,
                                        false,
                                        true,
                                        ri == 0,
                                        run_selected,
                                        trail.as_ref(),
                                    );
                                    if rresp.clicked() && !tree_chevron_clicked(&rresp, 1) {
                                        if let Some(f) = result.latest_report() {
                                            open_file = Some((f.path.clone(), ArtifactView::Report, false));
                                        } else {
                                            self.selected = None;
                                            self.return_report = None;
                                            self.preview_title = wiparse_core::project::format_run_stamp(&result.stamp);
                                            let hint = if lang == Lang::Zh {
                                                "此批次尚未生成 MD 报告。波形、图片和日志在报告生成后通过正文链接查看。"
                                            } else {
                                                "No Markdown report for this run yet. Evidence is opened through links in the generated report."
                                            };
                                            self.preview = Some(PreviewKind::EmptyHint(hint.into()));
                                            self.status = hint.into();
                                        }
                                    }
                                    if let Some(target) = result.latest_report() {
                                        let path = target.path.clone();
                                        rresp.context_menu(|ui| {
                                            if ui.button(tr(lang, "report.open_external")).clicked()
                                            {
                                                let _ = open_path_external(&path);
                                                ui.close_menu();
                                            }
                                            if ui.button(tr(lang, "report.reveal")).clicked() {
                                                reveal_in_folder(&path);
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                    if ropen {
                                        paint_report_files(
                                            ui,
                                            lang,
                                            tokens,
                                            result,
                                            &selected,
                                            &mut open_file,
                                            &test.id,
                                            &result.stamp,
                                            2,
                                        );
                                    }
                                }
                            }
                            ui.add_space(8.0);
                        });
                    if let Some(id) = remove_id {
                        if self.remove_hub_test(data_root, plugins, &id).is_ok() {
                            self.status = format!("{} · {id}", tr(lang, "report.remove_test"));
                        }
                    }
                    if let Some((path, view, overlay)) = open_file {
                        self.return_report = None;
                        self.apply_open(path, view, overlay, lang, &mut action);
                    }
                }
                let rest = ui.available_height();
                if rest > 1.0 {
                    ui.allocate_exact_size(egui::vec2(inner_w, rest), egui::Sense::hover());
                }
            });
        action
    }

    fn viewer_panel(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        mut waveform: Option<&mut WaveformAnalysisPanel>,
    ) -> ReportAction {
        let mut action = ReportAction::None;
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let title = if self.preview_title.is_empty() {
                    tr(lang, "report.viewer_title")
                } else {
                    self.preview_title.clone()
                };
                let row_w = ui.available_width().max(1.0);
                let button_w = if self.return_report.is_some() {
                    224.0
                } else if self.selected.is_some() {
                    120.0
                } else {
                    0.0
                };
                let gap = if button_w > 0.0 { 8.0 } else { 0.0 };
                let (row, _) = ui
                    .allocate_exact_size(egui::vec2(row_w, ui_theme::CTRL_H), egui::Sense::hover());
                let button_rect = egui::Rect::from_min_max(
                    egui::pos2((row.right() - button_w).max(row.left()), row.top()),
                    row.max,
                );
                let title_rect = egui::Rect::from_min_max(
                    row.min,
                    egui::pos2((button_rect.left() - gap).max(row.left()), row.bottom()),
                );
                crate::theme::scope_in_allocated_rect(
                    ui,
                    egui::UiBuilder::new()
                        .max_rect(title_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_clip_rect(title_rect.intersect(ui.clip_rect()));
                        ui.add(
                            egui::Label::new(
                                RichText::new(title)
                                    .size(ui_theme::FONT_TITLE)
                                    .strong()
                                    .color(tokens.text_primary),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    },
                );
                let mut back = false;
                if button_w > 0.0 {
                    let mut open_external = false;
                    crate::theme::scope_in_allocated_rect(
                        ui,
                        egui::UiBuilder::new()
                            .max_rect(button_rect)
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                        |ui| {
                            open_external = ui_theme::secondary_btn_sized(
                                ui,
                                tokens,
                                tr(lang, "report.open_external"),
                                egui::vec2(120.0, ui_theme::CTRL_H),
                            )
                            .clicked();
                            if self.return_report.is_some() {
                                back = ui_theme::secondary_btn_sized(
                                    ui,
                                    tokens,
                                    if lang == Lang::Zh {
                                        "返回报告"
                                    } else {
                                        "Back"
                                    },
                                    egui::vec2(96.0, ui_theme::CTRL_H),
                                )
                                .clicked();
                            }
                        },
                    );
                    if open_external {
                        self.open_external(lang);
                    }
                }
                ui.add_space(6.0);
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, tokens.divider),
                );
                ui.add_space(8.0);

                let content_h = ui.available_height().max(40.0);
                let mut open_ext = false;
                let mut md_open: Option<(PathBuf, ArtifactView, bool)> = None;
                let preview = self.preview.clone();
                let base_dir = self
                    .selected
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                let selected_path = self.selected.clone();
                let ctrl_hint = tr(lang, "report.ctrl_open");
                if matches!(preview, Some(PreviewKind::Wave)) {
                    ui.set_min_height(content_h);
                    if let Some(wave) = waveform.as_mut() {
                        wave.ui_embedded(ui, lang, tokens);
                    } else {
                        ui.label(
                            RichText::new(tr(lang, "report.status_wave"))
                                .size(ui_theme::FONT_BODY)
                                .color(tokens.text_muted),
                        );
                    }
                } else {
                    Frame::NONE
                        .fill(tokens.canvas_bg)
                        .stroke(Stroke::new(1.0_f32, tokens.divider))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(14, 12))
                        .show(ui, |ui| {
                            ui.set_min_height(ui.available_height().max(1.0));
                            ui.set_min_width(ui.available_width());
                            match &preview {
                                None => {
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            RichText::new(tr(lang, "report.viewer_empty"))
                                                .size(ui_theme::FONT_BODY)
                                                .color(tokens.text_muted),
                                        );
                                    });
                                }
                                Some(PreviewKind::Markdown(doc)) => {
                                    egui::ScrollArea::vertical()
                                        .id_salt("report_md_view")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_min_width(
                                                (ui.available_width() - 4.0).max(80.0),
                                            );
                                            paint_md_doc(
                                                ui,
                                                doc,
                                                tokens,
                                                base_dir.as_deref(),
                                                &mut self.image_textures,
                                                &mut md_open,
                                                &ctrl_hint,
                                            );
                                        });
                                }
                                Some(PreviewKind::Text(body)) => {
                                    egui::ScrollArea::both()
                                        .id_salt("report_text_view")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_min_width(
                                                (ui.available_width() - 4.0).max(80.0),
                                            );
                                            ui.label(
                                                RichText::new(body.as_str())
                                                    .font(FontId::monospace(ui_theme::FONT_CAPTION))
                                                    .color(tokens.text_primary),
                                            );
                                        });
                                }
                                Some(PreviewKind::Image) => {
                                    if let Some(path) = selected_path.as_ref() {
                                        paint_preview_image(
                                            ui,
                                            path,
                                            tokens,
                                            &mut self.image_textures,
                                        );
                                    }
                                }
                                Some(PreviewKind::Wave) => {}
                                Some(PreviewKind::EmptyHint(note)) => {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(24.0);
                                        ui.label(
                                            RichText::new(note.as_str())
                                                .size(ui_theme::FONT_BODY)
                                                .color(tokens.text_muted),
                                        );
                                        ui.add_space(10.0);
                                        if ui_theme::primary_btn_sized(
                                            ui,
                                            tokens,
                                            tr(lang, "report.open_external"),
                                            egui::vec2(140.0, ui_theme::CTRL_H),
                                        )
                                        .clicked()
                                        {
                                            open_ext = true;
                                        }
                                    });
                                }
                            }
                        });
                }
                if open_ext {
                    self.open_external(lang);
                }
                if back {
                    if let Some(path) = self.return_report.take() {
                        self.open_report(path, lang);
                    }
                } else if let Some((path, view, overlay)) = md_open {
                    if self.selected.as_ref().is_some_and(|p| is_markdown(p)) {
                        self.return_report = self.selected.clone();
                    }
                    self.apply_open(path, view, overlay, lang, &mut action);
                }
            });
        action
    }

    fn open_text(&mut self, path: PathBuf, lang: Lang) {
        match read_text_capped(&path) {
            Ok(text) => {
                self.preview = Some(PreviewKind::Text(std::sync::Arc::new(text)));
                self.status = format!(
                    "{} · {}",
                    tr(lang, "report.status_loaded"),
                    self.preview_title
                );
            }
            Err(e) => {
                self.preview = Some(PreviewKind::EmptyHint(e.clone()));
                self.status = format!("{}: {e}", tr(lang, "report.status_read_err"));
            }
        }
    }

    fn open_report(&mut self, path: PathBuf, lang: Lang) {
        self.selected = Some(path.clone());
        self.preview_title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.image_textures.clear();

        if !is_markdown(&path) {
            self.preview = Some(PreviewKind::EmptyHint(tr(lang, "report.status_not_md")));
            self.status = tr(lang, "report.status_not_md");
            return;
        }

        match read_text_capped(&path) {
            Ok(text) => {
                let doc = std::sync::Arc::new(parse_md(&text));
                self.preview = Some(PreviewKind::Markdown(doc));
                self.status = format!(
                    "{} · {}",
                    tr(lang, "report.status_loaded"),
                    self.preview_title
                );
            }
            Err(e) => {
                self.preview = Some(PreviewKind::EmptyHint(e.clone()));
                self.status = format!("{}: {e}", tr(lang, "report.status_read_err"));
            }
        }
    }

    fn open_external(&mut self, lang: Lang) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        if open_path_external(&path) {
            self.status = format!("{} · {}", tr(lang, "report.open_external"), path.display());
        } else {
            self.status = tr(lang, "report.status_open_err");
        }
    }
}

fn panel_in_rect(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::in_allocated_rect(ui, rect, egui::Layout::top_down(egui::Align::Min), add);
}

/// Only Markdown reports are navigable here. Waves/images/logs remain linked
/// evidence inside the report, never sibling rows in the report browser.
fn report_files(result: &IndexedResult) -> Vec<&IndexedFile> {
    result.markdown_reports()
}

fn paint_report_files(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    result: &IndexedResult,
    selected: &Option<PathBuf>,
    open_file: &mut Option<(PathBuf, ArtifactView, bool)>,
    name_prefix: &str,
    stamp: &str,
    depth: u8,
) {
    let reports = report_files(result);
    if reports.is_empty() {
        tree_hint_row(
            ui,
            tokens,
            depth,
            if lang == Lang::Zh {
                "尚无 MD 报告"
            } else {
                "No Markdown report"
            },
        );
        return;
    }
    let labels: Vec<_> = reports
        .iter()
        .map(|f| {
            let label = short_artifact_label(&f.name, name_prefix, stamp);
            if let Some(suffix) = label.strip_prefix("summary") {
                let suffix = suffix.strip_suffix(".md").unwrap_or(suffix);
                format!(
                    "{}{suffix}",
                    if lang == Lang::Zh {
                        "总报告"
                    } else {
                        "Summary"
                    }
                )
            } else {
                label
            }
        })
        .collect();
    let titles = numbered_labels(&labels);
    for (file, title) in reports.into_iter().zip(titles) {
        paint_file_row(
            ui,
            lang,
            tokens,
            depth,
            &title,
            file,
            ArtifactView::Report,
            selected,
            open_file,
        );
    }
}

fn paint_file_row(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    depth: u8,
    title: &str,
    file: &IndexedFile,
    group_view: ArtifactView,
    selected: &Option<PathBuf>,
    open_file: &mut Option<(PathBuf, ArtifactView, bool)>,
) {
    let is_sel = selected.as_ref().is_some_and(|p| same_path(p, &file.path));
    let row = tree_file_row(ui, tokens, depth, title, is_sel);
    let view = open_view_for_file(file.view, group_view, &file.name);
    if row.clicked() {
        let overlay = row.ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
        *open_file = Some((file.path.clone(), view, overlay));
    }
    row.context_menu(|ui| {
        if ui.button(tr(lang, "report.open_external")).clicked() {
            let _ = open_path_external(&file.path);
            ui.close_menu();
        }
        if ui.button(tr(lang, "report.reveal")).clicked() {
            reveal_in_folder(&file.path);
            ui.close_menu();
        }
    });
    row.on_hover_text(format!(
        "{}\n{}  {}",
        file.path.display(),
        format_size(file.size),
        format_mtime(file.mtime)
    ));
}

/// Drop `{prefix}_` and `{YYYYMMDD}_HHMMSS` so `lot_timeout_20260915_143738.isf`
/// becomes `timeout  14:37:38`.
fn short_artifact_label(name: &str, prefix: &str, stamp: &str) -> String {
    let ext = file_ext(name);
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let mut body = stem.to_owned();
    if !prefix.is_empty() {
        if let Some(rest) = body.strip_prefix(prefix) {
            body = rest.trim_start_matches('_').to_owned();
        }
    }
    if stamp.len() == 15 && stamp.as_bytes().get(8) == Some(&b'_') {
        let date = &stamp[..8];
        let token = format!("_{date}_");
        if let Some(idx) = body.find(&token) {
            let head = body[..idx].trim_end_matches('_').to_owned();
            let after = &body[idx + token.len()..];
            let hhmmss = if after.len() >= 6 && after.as_bytes()[..6].iter().all(u8::is_ascii_digit)
            {
                &after[..6]
            } else {
                &stamp[9..]
            };
            let nice = format!("{}:{}:{}", &hhmmss[0..2], &hhmmss[2..4], &hhmmss[4..6]);
            return if head.is_empty() {
                nice
            } else {
                format!("{head} {nice}")
            };
        }
    }
    if body.is_empty() {
        name.to_owned()
    } else if ext.is_empty() {
        body
    } else {
        format!("{body}.{ext}")
    }
}

fn format_result_stamp(stamp: &str) -> String {
    let full = wiparse_core::project::format_run_stamp(stamp);
    if full.len() >= 16 && full.as_bytes().get(4) == Some(&b'-') {
        // "YYYY-MM-DD HH:MM:SS" → "MM-DD HH:MM"
        let md = &full[5..10];
        let hm = if full.len() >= 16 { &full[11..16] } else { "" };
        if hm.is_empty() {
            md.to_owned()
        } else {
            format!("{md}  {hm}")
        }
    } else {
        full
    }
}

fn format_result_stamp_secs(stamp: &str) -> String {
    let full = wiparse_core::project::format_run_stamp(stamp);
    if full.len() >= 19 && full.as_bytes().get(4) == Some(&b'-') {
        format!("{}  {}", &full[5..10], &full[11..19])
    } else {
        format_result_stamp(stamp)
    }
}

/// Same minute shows seconds; identical seconds then get a 1-based index.
fn run_row_titles(stamps: &[&str]) -> Vec<String> {
    let base: Vec<String> = stamps.iter().copied().map(format_result_stamp).collect();
    let base_n = tally(&base);
    let with_sec: Vec<String> = stamps
        .iter()
        .zip(&base)
        .map(|(stamp, b)| {
            if base_n.get(b).copied().unwrap_or(0) > 1 {
                format_result_stamp_secs(stamp)
            } else {
                b.clone()
            }
        })
        .collect();
    let sec_n = tally(&with_sec);
    let mut seen: HashMap<String, usize> = HashMap::new();
    with_sec
        .into_iter()
        .map(|title| {
            if sec_n.get(&title).copied().unwrap_or(0) > 1 {
                let n = seen.entry(title.clone()).or_insert(0);
                *n += 1;
                format!("{title} · {n}")
            } else {
                title
            }
        })
        .collect()
}

fn tally(items: &[String]) -> HashMap<String, usize> {
    let mut n = HashMap::new();
    for item in items {
        *n.entry(item.clone()).or_insert(0) += 1;
    }
    n
}

/// First copy keeps the label; later copies of the same label get ` 2`, ` 3`.
fn numbered_labels(labels: &[String]) -> Vec<String> {
    let total = tally(labels);
    let mut seen: HashMap<String, usize> = HashMap::new();
    labels
        .iter()
        .map(|label| {
            let n = seen.entry(label.clone()).or_insert(0);
            *n += 1;
            if total.get(label).copied().unwrap_or(0) <= 1 || *n == 1 {
                label.clone()
            } else {
                format!("{label} {n}")
            }
        })
        .collect()
}

fn test_display_name(
    test: &wiparse_core::project::IndexedTest,
    plugins: &[PluginMeta],
    lang: Lang,
) -> String {
    let plugin = plugins
        .iter()
        .find(|p| p.id == test.plugin_id || p.id == test.id);
    if plugin.is_some() {
        return test_case_label(
            &test.name,
            &test.id,
            &test.plugin_id,
            plugin,
            matches!(lang, Lang::Zh),
        );
    }
    if matches!(lang, Lang::Zh) && !test.name_zh.trim().is_empty() {
        return test.name_zh.clone();
    }
    if !test.name.trim().is_empty() && !wiparse_core::project::is_run_stamp(&test.name) {
        return test.name.clone();
    }
    test.id.clone()
}

enum TreeTrail {
    Status { text: String, color: Color32 },
    Dot(Color32),
}

fn tree_x(depth: u8) -> f32 {
    TREE_PAD + f32::from(depth) * TREE_INDENT
}

fn tree_branch_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    id: impl std::hash::Hash,
    depth: u8,
    title: &str,
    color: Color32,
    strong: bool,
    expandable: bool,
    default_open: bool,
    selected: bool,
    trail: Option<&TreeTrail>,
) -> (bool, egui::Response) {
    let eid = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        eid,
        default_open,
    );
    let (rect, resp) = allocate_tree_row(ui);
    paint_tree_bg(ui, tokens, rect, resp.hovered(), selected);
    if expandable && resp.clicked() {
        if strong || tree_chevron_clicked(&resp, depth) {
            state.toggle(ui);
        } else {
            state.set_open(true);
        }
    }
    let open = expandable && state.is_open();
    let x = rect.min.x + tree_x(depth);
    let y = rect.center().y;
    let title_x = if expandable {
        // Draw the chevron: a Unicode glyph is missing in some Windows fonts.
        let points = if open {
            vec![
                egui::pos2(x + 1.0, y - 2.0),
                egui::pos2(x + 4.0, y + 1.0),
                egui::pos2(x + 7.0, y - 2.0),
            ]
        } else {
            vec![
                egui::pos2(x + 2.0, y - 4.0),
                egui::pos2(x + 5.0, y),
                egui::pos2(x + 2.0, y + 4.0),
            ]
        };
        ui.painter().add(egui::Shape::line(
            points,
            Stroke::new(1.4_f32, tokens.text_muted),
        ));
        x + TREE_CHEVRON_W
    } else {
        x
    };
    let trail_w = paint_tree_trail(ui, tokens, rect, trail);
    let max_w = (rect.max.x - trail_w - title_x - 6.0).max(12.0);
    let font = FontId::proportional(if strong {
        ui_theme::FONT_TITLE
    } else {
        ui_theme::FONT_BODY
    });
    paint_elided(ui, egui::pos2(title_x, y), title, font, color, max_w);
    state.store(ui.ctx());
    (open, resp)
}

fn tree_chevron_clicked(resp: &egui::Response, depth: u8) -> bool {
    resp.interact_pointer_pos()
        .is_some_and(|p| p.x < resp.rect.min.x + tree_x(depth) + TREE_CHEVRON_W)
}

fn tree_file_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    depth: u8,
    name: &str,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = allocate_tree_row(ui);
    paint_tree_bg(ui, tokens, rect, resp.hovered(), selected);
    let icon_x = rect.min.x + tree_x(depth);
    let icon = egui::Rect::from_min_size(
        egui::pos2(icon_x, rect.center().y - 5.5),
        egui::vec2(8.0, 11.0),
    );
    let icon_color = if selected {
        tokens.accent
    } else {
        tokens.text_muted
    };
    ui.painter().rect_stroke(
        icon,
        CornerRadius::same(1),
        Stroke::new(1.0_f32, icon_color),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            icon.left_center() + egui::vec2(2.0, 1.0),
            icon.right_center() + egui::vec2(-2.0, 1.0),
        ],
        Stroke::new(1.0_f32, icon_color),
    );
    let title_x = icon_x + 15.0;
    let max_w = (rect.max.x - title_x - 8.0).max(12.0);
    paint_elided(
        ui,
        egui::pos2(title_x, rect.center().y),
        name,
        FontId::proportional(ui_theme::FONT_BODY),
        if selected {
            tokens.accent
        } else {
            tokens.text_primary
        },
        max_w,
    );
    resp
}

fn tree_hint_row(ui: &mut egui::Ui, tokens: &Tokens, depth: u8, text: &str) {
    let (rect, _) = allocate_tree_row(ui);
    let title_x = rect.min.x + tree_x(depth) + TREE_CHEVRON_W;
    let max_w = (rect.max.x - title_x - 8.0).max(12.0);
    paint_elided(
        ui,
        egui::pos2(title_x, rect.center().y),
        text,
        FontId::proportional(ui_theme::FONT_CAPTION),
        tokens.text_muted,
        max_w,
    );
}

fn allocate_tree_row(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let width = ui.available_width().max(40.0);
    ui.allocate_exact_size(egui::vec2(width, TREE_ROW_H), egui::Sense::click())
}

fn paint_tree_bg(ui: &egui::Ui, tokens: &Tokens, rect: egui::Rect, hovered: bool, selected: bool) {
    if selected {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(1.0, 0.5)),
            CornerRadius::same(5),
            tokens.accent.gamma_multiply(0.08),
        );
        let bar = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 2.0, rect.max.y));
        ui.painter()
            .rect_filled(bar, CornerRadius::ZERO, tokens.accent);
    } else if hovered {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(1.0, 0.5)),
            CornerRadius::same(5),
            tokens.accent_soft.gamma_multiply(0.55),
        );
    }
}

fn paint_tree_trail(
    ui: &egui::Ui,
    _tokens: &Tokens,
    rect: egui::Rect,
    trail: Option<&TreeTrail>,
) -> f32 {
    let Some(trail) = trail else {
        return 8.0;
    };
    let y = rect.center().y;
    match trail {
        TreeTrail::Status { text, color } => {
            let font = FontId::proportional(ui_theme::FONT_CAPTION);
            let w = measure_text(ui, text, &font);
            ui.painter().text(
                egui::pos2(rect.max.x - 8.0, y),
                egui::Align2::RIGHT_CENTER,
                text,
                font,
                *color,
            );
            w + 16.0
        }
        TreeTrail::Dot(color) => {
            ui.painter()
                .circle_filled(egui::pos2(rect.max.x - 12.0, y), 3.5, *color);
            22.0
        }
    }
}

fn measure_text(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

fn paint_elided(
    ui: &egui::Ui,
    center_left: egui::Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) {
    let format = TextFormat {
        font_id: font,
        color,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append(text, 0.0, format);
    job.wrap.max_width = max_w.max(8.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts(|f| f.layout_job(job));
    let pos = egui::pos2(center_left.x, center_left.y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let n = bytes as f64;
    if n >= MB {
        format!("{:.1}MB", n / MB)
    } else if n >= KB {
        format!("{:.0}KB", n / KB)
    } else {
        format!("{bytes}B")
    }
}

fn format_mtime(mtime: Option<SystemTime>) -> String {
    let Some(t) = mtime else {
        return "-".into();
    };
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%m-%d %H:%M").to_string()
}

fn same_path(a: &Path, b: &Path) -> bool {
    let na = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let nb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    na == nb
}

fn is_markdown(path: &Path) -> bool {
    matches!(file_ext_path(path).as_str(), "md" | "markdown")
}

fn is_wave_file(path: &Path) -> bool {
    matches!(file_ext_path(path).as_str(), "isf" | "wfm" | "csv")
}

fn is_text_file(path: &Path) -> bool {
    matches!(
        file_ext_path(path).as_str(),
        "txt" | "log" | "json" | "jsonl"
    )
}

fn is_image_file(path: &Path) -> bool {
    matches!(
        file_ext_path(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    )
}

fn file_ext_path(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn file_ext(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Per-file open target. PNG/PDF/HTML never jump to wave/log; `.txt` follows the group.
fn open_view_for_file(
    file_view: ArtifactView,
    group_view: ArtifactView,
    name: &str,
) -> ArtifactView {
    match file_ext(name).as_str() {
        "md" | "markdown" => ArtifactView::Report,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "pdf" | "html" | "htm" | "svg" => {
            ArtifactView::External
        }
        "isf" | "wfm" | "csv" => ArtifactView::Wave,
        "log" | "json" | "jsonl" => ArtifactView::Log,
        "txt" => group_view,
        _ => file_view,
    }
}

fn reveal_in_folder(path: &Path) {
    let s = path.to_string_lossy();
    if s.contains('\n') || s.contains('\r') || s.contains('\0') {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").args(["-R"]).arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            let _ = Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

fn read_text_capped(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_PREVIEW_CHARS + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let slice = if bytes.len() > MAX_PREVIEW_CHARS {
        &bytes[..MAX_PREVIEW_CHARS]
    } else {
        &bytes
    };
    let mut text = String::from_utf8_lossy(slice).into_owned();
    if let Some(stripped) = text.strip_prefix('\u{feff}') {
        text = stripped.to_owned();
    }
    if bytes.len() > MAX_PREVIEW_CHARS {
        text.push_str("\n\n...");
    }
    Ok(text)
}

pub(crate) fn open_path_external(path: &Path) -> bool {
    // Prefer argv-separated launchers (no shell string interpolation).
    #[cfg(target_os = "windows")]
    {
        // `cmd /C start "" <path>` is vulnerable when path contains shell metacharacters.
        // Use ShellExecute via `explorer` for dirs, and `cmd /C start` with separate args
        // only after rejecting control chars.
        let s = path.to_string_lossy();
        if s.contains('\n') || s.contains('\r') || s.contains('\0') {
            return false;
        }
        if path.is_dir() {
            Command::new("explorer").arg(path).spawn().is_ok()
        } else {
            Command::new("cmd")
                .args(["/C", "start", "", "/B"])
                .arg(path)
                .spawn()
                .is_ok()
        }
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().is_ok()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn().is_ok()
    }
}

// --- Markdown: parse once -> light block IR -> paint -------------------------------
//
// Design (lightweight + good look):
// - No pulldown-cmark / egui_commonmark (egui 0.31 mismatch + heavier deps).
// - Parse to MdDoc on file open only; each frame only paints widgets.
// - Subset tuned for test reports: headings, para, lists/tasks, quotes,
//   fenced code (``` / ~~~), GFM tables, images ![alt](path), hr;
//   inlines: **bold** *italic* `code` ~~strike~~ and [label](url).

fn parse_md(src: &str) -> MdDoc {
    let lines: Vec<&str> = src
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut blocks = Vec::new();
    let mut i = 0usize;
    let mut para = String::new();

    let flush_para = |para: &mut String, blocks: &mut Vec<MdBlock>| {
        let t = para.trim();
        if !t.is_empty() {
            blocks.push(MdBlock::Paragraph { text: t.to_owned() });
        }
        para.clear();
    };

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Fenced code: ``` or ~~~ (CommonMark; reports often wrap logs in ~~~)
        if let Some((fence_ch, fence_n, lang)) = parse_fence_open(trimmed) {
            flush_para(&mut para, &mut blocks);
            i += 1;
            let mut body = String::new();
            while i < lines.len() {
                if is_fence_close(lines[i].trim(), fence_ch, fence_n) {
                    i += 1;
                    break;
                }
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            blocks.push(MdBlock::Code {
                lang,
                body: body.trim_end().to_owned(),
            });
            continue;
        }

        // Table
        if looks_like_table_row(line) {
            if let Some((consumed, rows)) = parse_markdown_table(&lines[i..]) {
                flush_para(&mut para, &mut blocks);
                blocks.push(MdBlock::Table { rows });
                i += consumed;
                continue;
            }
        }

        // Standalone image line: ![alt](path)
        if let Some((alt, src)) = parse_image_line(trimmed) {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Image { alt, src });
            i += 1;
            continue;
        }

        // HR
        if is_hr(trimmed) {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Rule);
            i += 1;
            continue;
        }

        // Heading
        if let Some((level, body)) = heading_level(line) {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Heading {
                level,
                text: body.to_owned(),
            });
            i += 1;
            continue;
        }

        // Blockquote (merge consecutive)
        if let Some(rest) = trimmed.strip_prefix('>') {
            flush_para(&mut para, &mut blocks);
            let mut q = rest.trim_start().to_owned();
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(r) = t.strip_prefix('>') {
                    if !q.is_empty() {
                        q.push(' ');
                    }
                    q.push_str(r.trim_start());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MdBlock::Quote { text: q });
            continue;
        }

        // Blank -> paragraph break
        if trimmed.is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }

        // Lists / tasks
        let (indent, content) = split_indent(line);
        let depth = (indent / 2).min(6) as u8;
        if let Some((checked, text)) = parse_task_item(content) {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Task {
                depth,
                checked,
                text: text.to_owned(),
            });
            i += 1;
            continue;
        }
        if let Some(text) = content
            .strip_prefix("- ")
            .or_else(|| content.strip_prefix("* "))
            .or_else(|| content.strip_prefix("+ "))
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Bullet {
                depth,
                text: text.to_owned(),
            });
            i += 1;
            continue;
        }
        if let Some((n, text)) = ordered_list_item(content) {
            flush_para(&mut para, &mut blocks);
            blocks.push(MdBlock::Ordered {
                depth,
                n,
                text: text.to_owned(),
            });
            i += 1;
            continue;
        }

        // Soft-wrap into paragraph
        if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(trimmed);
        i += 1;
    }
    flush_para(&mut para, &mut blocks);
    MdDoc { blocks }
}

fn paint_md_doc(
    ui: &mut egui::Ui,
    doc: &MdDoc,
    tokens: &Tokens,
    base_dir: Option<&Path>,
    textures: &mut HashMap<PathBuf, Option<egui::TextureHandle>>,
    open: &mut Option<(PathBuf, ArtifactView, bool)>,
    ctrl_hint: &str,
) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let mut table_id = 0u32;
    for (block_id, block) in doc.blocks.iter().enumerate() {
        match block {
            MdBlock::Heading { level, text } => {
                let size = match *level {
                    1 => 20.0,
                    2 => 16.5,
                    3 => 14.5,
                    _ => ui_theme::FONT_BODY + 1.0,
                };
                ui.add_space(if *level <= 2 { 10.0 } else { 6.0 });
                paint_md_inline(ui, text, size, true, tokens, base_dir, open, ctrl_hint);
                if *level == 1 {
                    ui.add_space(3.0);
                    rule_line(ui, tokens, 1.2);
                    ui.add_space(4.0);
                } else if *level == 2 {
                    ui.add_space(2.0);
                }
            }
            MdBlock::Paragraph { text } => {
                ui.add_space(4.0);
                paint_md_inline(
                    ui,
                    text,
                    ui_theme::FONT_BODY,
                    false,
                    tokens,
                    base_dir,
                    open,
                    ctrl_hint,
                );
                ui.add_space(2.0);
            }
            MdBlock::Bullet { depth, text } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0 + *depth as f32 * 14.0);
                    let bullet = match depth % 3 {
                        0 => "\u{2022}", // •
                        1 => "\u{25E6}", // ◦
                        _ => "\u{25AA}", // ▪
                    };
                    ui.label(
                        RichText::new(bullet)
                            .size(ui_theme::FONT_BODY)
                            .color(tokens.text_muted),
                    );
                    paint_md_inline(
                        ui,
                        text,
                        ui_theme::FONT_BODY,
                        false,
                        tokens,
                        base_dir,
                        open,
                        ctrl_hint,
                    );
                });
            }
            MdBlock::Ordered { depth, n, text } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0 + *depth as f32 * 14.0);
                    ui.label(
                        RichText::new(format!("{n}."))
                            .size(ui_theme::FONT_BODY)
                            .color(tokens.text_muted),
                    );
                    paint_md_inline(
                        ui,
                        text,
                        ui_theme::FONT_BODY,
                        false,
                        tokens,
                        base_dir,
                        open,
                        ctrl_hint,
                    );
                });
            }
            MdBlock::Task {
                depth,
                checked,
                text,
            } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0 + *depth as f32 * 14.0);
                    let mark = if *checked { "\u{2611}" } else { "\u{2610}" }; // ☑ / ☐
                    ui.label(
                        RichText::new(mark)
                            .size(ui_theme::FONT_BODY)
                            .color(if *checked {
                                tokens.accent
                            } else {
                                tokens.text_muted
                            }),
                    );
                    paint_md_inline(
                        ui,
                        text,
                        ui_theme::FONT_BODY,
                        false,
                        tokens,
                        base_dir,
                        open,
                        ctrl_hint,
                    );
                });
            }
            MdBlock::Quote { text } => {
                ui.add_space(4.0);
                let avail = ui.available_width().max(40.0);
                Frame::NONE
                    .fill(tokens.accent_soft)
                    .stroke(Stroke::new(1.0_f32, tokens.divider))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin {
                        left: 12,
                        right: 10,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.set_min_width(avail - 8.0);
                        let r = ui.max_rect();
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(r.min.x - 12.0, r.min.y - 8.0),
                                egui::pos2(r.min.x - 9.0, r.max.y + 8.0),
                            ),
                            CornerRadius::ZERO,
                            tokens.accent,
                        );
                        paint_md_inline(
                            ui,
                            text,
                            ui_theme::FONT_BODY,
                            false,
                            tokens,
                            base_dir,
                            open,
                            ctrl_hint,
                        );
                    });
                ui.add_space(2.0);
            }
            MdBlock::Code { lang, body } => {
                ui.add_space(6.0);
                Frame::NONE
                    .fill(tokens.surface_bg)
                    .stroke(Stroke::new(1.0_f32, tokens.divider))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if !lang.is_empty() {
                            ui.label(
                                RichText::new(lang)
                                    .size(ui_theme::FONT_CAPTION)
                                    .color(tokens.text_muted),
                            );
                            ui.add_space(2.0);
                        }
                        egui::ScrollArea::horizontal()
                            .id_salt(("md_code", block_id))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(body.as_str())
                                        .monospace()
                                        .size(ui_theme::FONT_BODY - 0.5)
                                        .color(tokens.text_primary),
                                );
                            });
                    });
                ui.add_space(4.0);
            }
            MdBlock::Image { alt, src } => {
                ui.add_space(6.0);
                paint_md_image(ui, tokens, base_dir, textures, alt, src, open, ctrl_hint);
                ui.add_space(4.0);
            }
            MdBlock::Table { rows } => {
                table_id = table_id.wrapping_add(1);
                ui.add_space(6.0);
                paint_md_table(ui, tokens, table_id, rows, base_dir, open, ctrl_hint);
                ui.add_space(6.0);
            }
            MdBlock::Rule => {
                ui.add_space(8.0);
                rule_line(ui, tokens, 1.0);
                ui.add_space(8.0);
            }
        }
    }
}

fn rule_line(ui: &mut egui::Ui, tokens: &Tokens, thickness: f32) {
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        Stroke::new(thickness, tokens.divider),
    );
    ui.add_space(thickness + 1.0);
}

fn parse_fence_open(trimmed: &str) -> Option<(char, usize, String)> {
    let mut chars = trimmed.chars();
    let ch = chars.next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let mut n = 1usize;
    for c in chars.by_ref() {
        if c == ch {
            n += 1;
        } else {
            break;
        }
    }
    if n < 3 {
        return None;
    }
    // Remaining after fence markers (chars already consumed one non-fence or EOF).
    let info: String = trimmed
        .chars()
        .skip(n)
        .collect::<String>()
        .trim()
        .to_owned();
    // Info must not contain the fence char (CommonMark).
    if info.contains(ch) {
        return None;
    }
    Some((ch, n, info))
}

fn is_fence_close(trimmed: &str, ch: char, min_n: usize) -> bool {
    let mut n = 0usize;
    for c in trimmed.chars() {
        if c == ch {
            n += 1;
        } else {
            break;
        }
    }
    n >= min_n && trimmed.chars().skip(n).all(|c| c.is_whitespace())
}

fn parse_image_line(trimmed: &str) -> Option<(String, String)> {
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.first() != Some(&'!') {
        return None;
    }
    let (alt, src, next) = parse_md_link(&chars, 1)?;
    if chars[next..].iter().any(|c| !c.is_whitespace()) {
        return None;
    }
    Some((alt, src))
}

/// Collapse `.` / `..` without requiring the path to exist.
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Reports live in `artifacts/reports/`; shots/waves/docs are siblings.
/// Jail at `artifacts/` so `../shots/foo.png` works, while `../../` is blocked.
fn md_asset_jail(base_dir: &Path) -> PathBuf {
    match base_dir.parent() {
        Some(parent) if parent.file_name().is_some_and(|n| n == "artifacts") => {
            parent.to_path_buf()
        }
        _ => base_dir.to_path_buf(),
    }
}

fn decode_md_src(s: &str) -> String {
    let t = s
        .trim()
        .trim_matches(|c| matches!(c, '<' | '>' | '"' | '`'));
    let mut decoded = Vec::new();
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                decoded.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let mut t = String::from_utf8(decoded).unwrap_or_else(|_| t.to_owned());
    if t.contains("\\\\") {
        t = t.replace("\\\\", "\\");
    }
    t
}

fn has_artifact_ext(s: &str) -> bool {
    matches!(
        file_ext(s).as_str(),
        "isf"
            | "wfm"
            | "csv"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "md"
            | "markdown"
            | "pdf"
            | "html"
            | "htm"
            | "txt"
            | "log"
            | "json"
            | "jsonl"
    )
}

fn looks_like_local_file(s: &str) -> bool {
    if s.is_empty() || s.contains('\n') || s.contains('\0') {
        return false;
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return false;
    }
    has_artifact_ext(s)
}

fn file_ref_from_text(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let chars: Vec<char> = t.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == '[' {
            if let Some((_, url, _)) = parse_md_link(&chars, i) {
                let u = decode_md_src(&url);
                if looks_like_local_file(&u) {
                    return Some(u);
                }
            }
        }
    }
    let stripped = decode_md_src(t);
    if looks_like_local_file(&stripped) {
        Some(stripped)
    } else {
        None
    }
}

fn try_follow_md_ref(
    resp: &egui::Response,
    path: &Path,
    open: &mut Option<(PathBuf, ArtifactView, bool)>,
    hint: &str,
) {
    resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clone().on_hover_text(hint);
    if !resp.clicked() {
        return;
    }
    if !path.is_file() {
        return;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let view = open_view_for_file(
        ArtifactView::from_filename(name),
        ArtifactView::from_filename(name),
        name,
    );
    *open = Some((path.to_path_buf(), view, false));
}

fn resolve_md_asset(base_dir: Option<&Path>, src: &str) -> Option<PathBuf> {
    let raw = src.trim();
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let stripped = raw.trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '`');
    let stripped = decode_md_src(stripped);
    if stripped.starts_with("http://") || stripped.starts_with("https://") {
        return None;
    }
    if stripped.contains('\0') || stripped.contains('\n') || stripped.contains('\r') {
        return None;
    }
    let raw = stripped
        .strip_prefix("file:///")
        .or_else(|| stripped.strip_prefix("file://"))
        .unwrap_or(&stripped);
    let p = PathBuf::from(raw);
    let joined = if p.is_absolute() {
        p
    } else if let Some(base) = base_dir {
        base.join(&p)
    } else {
        p
    };
    let resolved = lexical_normalize(&joined);
    if let Some(base) = base_dir {
        let jail = lexical_normalize(&md_asset_jail(base));
        if !resolved.starts_with(&jail) {
            return None;
        }
    }
    let canonical = resolved.canonicalize().unwrap_or(resolved);
    if let Some(base) = base_dir {
        let jail = md_asset_jail(base);
        let jail = jail
            .canonicalize()
            .unwrap_or_else(|_| lexical_normalize(&jail));
        if !canonical.starts_with(jail) {
            return None;
        }
    }
    Some(canonical)
}

fn paint_preview_image(
    ui: &mut egui::Ui,
    path: &Path,
    tokens: &Tokens,
    textures: &mut HashMap<PathBuf, Option<egui::TextureHandle>>,
) {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !textures.contains_key(&key) {
        let tex = load_md_texture(ui.ctx(), path);
        textures.insert(key.clone(), tex);
    }
    let avail = ui.available_size();
    if let Some(Some(tex)) = textures.get(&key) {
        let size = tex.size_vec2();
        let scale = (avail.x / size.x).min(avail.y / size.y).min(1.0).max(0.05);
        let show = egui::vec2(size.x * scale, size.y * scale);
        ui.centered_and_justified(|ui| {
            ui.add(egui::Image::new(tex).fit_to_exact_size(show));
        });
    } else {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(path.display().to_string())
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
        });
    }
}

fn load_md_texture(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 12 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(&bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    let img = if w.max(h) > MAX_MD_IMAGE_EDGE {
        let (tw, th) = if w >= h {
            (
                MAX_MD_IMAGE_EDGE,
                (MAX_MD_IMAGE_EDGE as u64 * h as u64 / w as u64).max(1) as u32,
            )
        } else {
            (
                (MAX_MD_IMAGE_EDGE as u64 * w as u64 / h as u64).max(1) as u32,
                MAX_MD_IMAGE_EDGE,
            )
        };
        image::imageops::thumbnail(&img, tw, th)
    } else {
        img
    };
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img);
    let name = format!("md-img-{}", path.display());
    Some(ctx.load_texture(name, color, Default::default()))
}

fn paint_md_image(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    base_dir: Option<&Path>,
    textures: &mut HashMap<PathBuf, Option<egui::TextureHandle>>,
    alt: &str,
    src: &str,
    open: &mut Option<(PathBuf, ArtifactView, bool)>,
    ctrl_hint: &str,
) {
    let Some(path) = resolve_md_asset(base_dir, src) else {
        ui.label(
            RichText::new(format!("[img blocked] {alt}"))
                .size(ui_theme::FONT_CAPTION)
                .color(tokens.text_muted),
        );
        return;
    };
    if !textures.contains_key(&path) {
        let tex = load_md_texture(ui.ctx(), &path);
        textures.insert(path.clone(), tex);
    }

    let avail_w = ui.available_width().max(40.0);
    if let Some(Some(tex)) = textures.get(&path) {
        let size = tex.size_vec2();
        let scale = ((avail_w - 8.0).max(1.0) / size.x).min(1.0);
        let show = egui::vec2(size.x * scale, size.y * scale);
        Frame::NONE
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                let resp = ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(show)
                        .sense(egui::Sense::click()),
                );
                try_follow_md_ref(&resp, &path, open, ctrl_hint);
                if !alt.is_empty() {
                    ui.add_space(2.0);
                    let cap = ui.add(
                        egui::Label::new(
                            RichText::new(alt)
                                .size(ui_theme::FONT_CAPTION)
                                .color(tokens.text_muted),
                        )
                        .sense(egui::Sense::click()),
                    );
                    try_follow_md_ref(&cap, &path, open, ctrl_hint);
                }
            });
    } else {
        let resp = ui.add(
            egui::Label::new(
                RichText::new(format!("[img missing] {alt}"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            )
            .sense(egui::Sense::click()),
        );
        try_follow_md_ref(&resp, &path, open, ctrl_hint);
    }
}

fn paint_md_table(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    id: u32,
    rows: &[Vec<String>],
    base_dir: Option<&Path>,
    open: &mut Option<(PathBuf, ArtifactView, bool)>,
    ctrl_hint: &str,
) {
    if rows.is_empty() {
        return;
    }
    let cols = rows[0].len().max(1);
    let avail = (ui.available_width() - 4.0).max(80.0);

    let mut weights = vec![1.0_f32; cols];
    for row in rows {
        for (c, cell) in row.iter().enumerate().take(cols) {
            let w = (cell.chars().count() as f32).max(1.0);
            if w > weights[c] {
                weights[c] = w;
            }
        }
    }
    for w in &mut weights {
        *w = w.clamp(4.0, 48.0);
    }
    let sum: f32 = weights.iter().sum::<f32>().max(1.0);
    let col_w: Vec<f32> = weights
        .iter()
        .map(|w| ((avail - 8.0) * (*w / sum)).max(36.0))
        .collect();

    egui::ScrollArea::horizontal()
        .id_salt(("md_table", id))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("md_table_grid", id))
                .striped(true)
                .spacing(egui::vec2(8.0, 8.0))
                .show(ui, |ui| {
                    for (ri, row) in rows.iter().enumerate() {
                        for ci in 0..cols {
                            let cell = row.get(ci).map(String::as_str).unwrap_or("");
                            ui.vertical(|ui| {
                                ui.set_width(col_w[ci]);
                                if let Some(src) =
                                    file_ref_from_text(cell).filter(|_| !cell.contains("]("))
                                {
                                    let resp = ui.add(
                                        egui::Label::new(
                                            RichText::new(decode_md_src(cell))
                                                .color(tokens.accent)
                                                .underline(),
                                        )
                                        .wrap()
                                        .sense(egui::Sense::click()),
                                    );
                                    if let Some(path) = resolve_md_asset(base_dir, &src) {
                                        try_follow_md_ref(&resp, &path, open, ctrl_hint);
                                    }
                                } else {
                                    paint_md_inline(
                                        ui,
                                        cell,
                                        ui_theme::FONT_BODY,
                                        ri == 0,
                                        tokens,
                                        base_dir,
                                        open,
                                        ctrl_hint,
                                    );
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
        });
}

fn split_indent(line: &str) -> (usize, &str) {
    let mut n = 0usize;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 2,
            _ => break,
        }
    }
    let byte_n = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| c.len_utf8())
        .sum::<usize>();
    (n, line[byte_n..].trim_start())
}

fn parse_task_item(content: &str) -> Option<(bool, &str)> {
    let c = content
        .strip_prefix("- ")
        .or_else(|| content.strip_prefix("* "))?;
    if let Some(rest) = c.strip_prefix("[ ] ") {
        return Some((false, rest));
    }
    if let Some(rest) = c.strip_prefix("[x] ").or_else(|| c.strip_prefix("[X] ")) {
        return Some((true, rest));
    }
    None
}

fn is_hr(trimmed: &str) -> bool {
    matches!(trimmed, "---" | "***" | "___" | "* * *" | "- - -")
        || (trimmed.len() >= 3
            && trimmed
                .chars()
                .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
            && trimmed.chars().filter(|c| *c != ' ').count() >= 3)
}

fn looks_like_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim().trim_matches('|');
    if t.is_empty() {
        return false;
    }
    t.chars()
        .all(|c| c == '-' || c == ':' || c == '|' || c.is_whitespace())
        && t.contains('-')
}

fn split_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            chars.next();
            cell.push('|');
        } else if c == '|' {
            cells.push(cell.trim().to_owned());
            cell.clear();
        } else {
            cell.push(c);
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}

fn parse_markdown_table(lines: &[&str]) -> Option<(usize, Vec<Vec<String>>)> {
    if lines.len() < 2 {
        return None;
    }
    if !looks_like_table_row(lines[0]) || !is_table_separator(lines[1]) {
        return None;
    }
    let header = split_table_row(lines[0]);
    if header.is_empty() || header.iter().all(|c| c.is_empty()) {
        return None;
    }
    let cols = header.len();
    let mut rows = vec![header];
    let mut i = 2usize;
    while i < lines.len() && looks_like_table_row(lines[i]) && !is_table_separator(lines[i]) {
        let mut row = split_table_row(lines[i]);
        row.resize(cols, String::new());
        rows.push(row);
        i += 1;
    }
    Some((i, rows))
}

fn heading_level(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut n = 0usize;
    while n < bytes.len() && bytes[n] == b'#' {
        n += 1;
    }
    if n == 0 || n > 6 {
        return None;
    }
    if n >= bytes.len() || bytes[n] != b' ' {
        return None;
    }
    Some((n as u8, line[n + 1..].trim()))
}

fn ordered_list_item(line: &str) -> Option<(u32, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() || bytes[i] != b'.' {
        return None;
    }
    let num: u32 = line[..i].parse().ok()?;
    let rest = line.get(i + 1..)?.strip_prefix(' ')?;
    Some((num, rest))
}

fn paint_md_inline(
    ui: &mut egui::Ui,
    text: &str,
    size: f32,
    strong: bool,
    tokens: &Tokens,
    base_dir: Option<&Path>,
    open: &mut Option<(PathBuf, ArtifactView, bool)>,
    hint: &str,
) {
    let (mut job, links) = md_inline_layout(text, size, strong, tokens);
    job.wrap.max_width = ui.available_width().max(1.0);
    let galley = ui.fonts(|f| f.layout_job(job));
    let resp = ui.add(egui::Label::new(galley.clone()).selectable(true));
    for (li, (section, src)) in links.iter().enumerate() {
        for (ri, row) in galley.rows.iter().enumerate() {
            let rect = row
                .glyphs
                .iter()
                .filter(|g| g.section_index == *section)
                .fold(egui::Rect::NOTHING, |r, g| r.union(g.logical_rect()));
            if !rect.is_positive() {
                continue;
            }
            let hit = ui.interact(
                rect.translate(resp.rect.min.to_vec2()),
                resp.id.with(("md_link", li, ri)),
                egui::Sense::click(),
            );
            if let Some(path) = resolve_md_asset(base_dir, src) {
                try_follow_md_ref(&hit, &path, open, hint);
                if !path.is_file() {
                    hit.on_hover_text(format!("File missing: {}", path.display()));
                }
            } else {
                hit.on_hover_text(format!("Link unavailable: {src}"));
            }
        }
    }
}

fn md_inline_layout(
    text: &str,
    size: f32,
    strong_all: bool,
    tokens: &Tokens,
) -> (LayoutJob, Vec<(u32, String)>) {
    let mut links = Vec::new();
    let mut job = LayoutJob::default();
    let base = TextFormat {
        font_id: FontId::proportional(size),
        color: tokens.text_primary,
        ..Default::default()
    };
    let bold = TextFormat {
        font_id: FontId::proportional(size),
        color: tokens.text_primary,
        extra_letter_spacing: 0.15,
        ..Default::default()
    };
    let italic = TextFormat {
        font_id: FontId::proportional(size),
        color: tokens.text_primary,
        italics: true,
        ..Default::default()
    };
    let strike = TextFormat {
        font_id: FontId::proportional(size),
        color: tokens.text_muted,
        strikethrough: Stroke::new(1.0_f32, tokens.text_muted),
        ..Default::default()
    };
    let code = TextFormat {
        font_id: FontId::monospace(size * 0.92),
        color: tokens.accent,
        background: tokens.surface_bg,
        ..Default::default()
    };
    let link = TextFormat {
        font_id: FontId::proportional(size),
        color: tokens.accent,
        underline: Stroke::new(1.0_f32, tokens.accent.gamma_multiply(0.7)),
        ..Default::default()
    };
    let normal = if strong_all { bold.clone() } else { base };

    let text = text
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    let mut buf = String::new();

    let flush = |job: &mut LayoutJob, buf: &mut String, fmt: &TextFormat| {
        if !buf.is_empty() {
            job.append(buf, 0.0, fmt.clone());
            buf.clear();
        }
    };

    while i < chars.len() {
        // ![alt](url) — keep as accent caption (block form preferred for display)
        if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some((alt, src, next)) = parse_md_link(&chars, i + 1) {
                flush(&mut job, &mut buf, &normal);
                let label = if alt.is_empty() {
                    "[image]".to_owned()
                } else {
                    format!("[{alt}]")
                };
                links.push((job.sections.len() as u32, src));
                job.append(&label, 0.0, link.clone());
                i = next;
                continue;
            }
        }
        // [label](url)
        if chars[i] == '[' {
            if let Some((label, url, next)) = parse_md_link(&chars, i) {
                flush(&mut job, &mut buf, &normal);
                links.push((job.sections.len() as u32, url));
                job.append(&label, 0.0, link.clone());
                i = next;
                continue;
            }
        }
        // `code`
        if chars[i] == '`' {
            flush(&mut job, &mut buf, &normal);
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '`' {
                i += 1;
            }
            let t: String = chars[start..i].iter().collect();
            job.append(&t, 0.0, code.clone());
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        // ~~strike~~ — only when a matching closer exists (avoids ~~~ log fences)
        if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            let mut j = i + 2;
            let mut found = None;
            while j + 1 < chars.len() {
                if chars[j] == '~' && chars[j + 1] == '~' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                flush(&mut job, &mut buf, &normal);
                let t: String = chars[i + 2..end].iter().collect();
                job.append(&t, 0.0, strike.clone());
                i = end + 2;
                continue;
            }
            // Unmatched: emit literal tildes
            buf.push(chars[i]);
            i += 1;
            continue;
        }
        // ***bold italic*** or **bold** or *italic*
        if chars[i] == '*' {
            let marker_len = if i + 2 < chars.len() && chars[i + 1] == '*' && chars[i + 2] == '*' {
                3usize
            } else if i + 1 < chars.len() && chars[i + 1] == '*' {
                2usize
            } else {
                1usize
            };
            let end_pat: Vec<char> = std::iter::repeat('*').take(marker_len).collect();
            let mut j = i + marker_len;
            let mut found = None;
            while j + marker_len <= chars.len() {
                if chars[j..j + marker_len] == end_pat[..] {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                let fmt = if marker_len == 3 {
                    TextFormat {
                        italics: true,
                        extra_letter_spacing: 0.15,
                        ..bold.clone()
                    }
                } else if marker_len == 2 {
                    bold.clone()
                } else {
                    italic.clone()
                };
                flush(&mut job, &mut buf, &normal);
                let t: String = chars[i + marker_len..end].iter().collect();
                job.append(&t, 0.0, fmt);
                i = end + marker_len;
                continue;
            }
            buf.push(chars[i]);
            i += 1;
            continue;
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut job, &mut buf, &normal);
    job.wrap.max_width = f32::INFINITY;
    (job, links)
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut i = start + 1;
    let mut label = String::new();
    while i < chars.len() && chars[i] != ']' {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
        }
        label.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || chars.get(i + 1) != Some(&'(') {
        return None;
    }
    i += 2; // ](
    let url_start = i;
    let angle = chars.get(i) == Some(&'<');
    let mut depth = 0usize;
    while i < chars.len() {
        if angle {
            if chars[i] == '>' && chars.get(i + 1) == Some(&')') {
                i += 1;
                break;
            }
        } else {
            if chars[i] == '(' {
                depth += 1;
            }
            if chars[i] == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
        }
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let url: String = chars[url_start..i].iter().collect();
    Some((label, url, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_browser_only_shows_markdown_and_selects_runs_with_one_click() {
        with_tmp(|root| {
            let make_file = |name: &str| IndexedFile {
                name: name.into(),
                path: root.join(name),
                size: 1,
                mtime: None,
                view: ArtifactView::External,
                output_id: "docs".into(),
            };
            let report = make_file("charge_summary.md");
            fs::write(&report.path, "# 充电检验报告\n\n## 结果\n\n通过").unwrap();
            let evidence: Vec<_> = (0..150)
                .map(|i| make_file(&format!("capture-{i}.isf")))
                .collect();
            let result = |stamp: &str, has_report: bool| {
                let mut files = evidence.clone();
                files.push(make_file("index.jsonl"));
                files.push(make_file("shot.png"));
                if has_report {
                    files.push(report.clone());
                }
                IndexedResult {
                    stamp: stamp.into(),
                    ok: Some(true),
                    groups: vec![wiparse_core::project::IndexedGroup {
                        id: "docs".into(),
                        label: "Documents".into(),
                        label_zh: "文档".into(),
                        view: ArtifactView::External,
                        files,
                    }],
                }
            };
            let mut panel = TestReportPanel::new(&AppConfig::default());
            panel.index.tests.push(wiparse_core::project::IndexedTest {
                id: "charge".into(),
                name: "无线充电检验".into(),
                name_zh: String::new(),
                plugin_id: "charge".into(),
                installed: true,
                results: vec![
                    result("20261016_201300", true),
                    result("20261014_180300", true),
                    result("20261013_170000", false),
                ],
            });
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let tokens = Tokens::dark();
            let mut time = 0.0;
            let mut frame = |panel: &mut TestReportPanel, events| {
                time += 0.1;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(340.0, 850.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.browser_panel(ui, Lang::Zh, &tokens, root, &[], None);
                        });
                    },
                )
            };
            let first = frame(&mut panel, vec![]);
            let texts: Vec<_> = first
                .shapes
                .iter()
                .filter_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        Some(t.galley.job.text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                texts.iter().filter(|s| **s == "总报告").count(),
                1,
                "only latest run starts expanded: {texts:?}"
            );
            assert!(!texts.iter().any(|s| s.contains("capture-")
                || s.contains("index.jsonl")
                || s.contains("shot.png")
                || *s == "文档"));
            if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                crate::ui_audit::export_frame(
                    &ctx,
                    first.clone(),
                    &PathBuf::from(dir),
                    "report-browser",
                    egui::vec2(340.0, 850.0),
                );
            }
            for (stamp, expect_report) in [("10-14  18:03", true), ("10-13  17:00", false)] {
                let out = frame(&mut panel, vec![]);
                let pos = out
                    .shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape {
                            (t.galley.job.text == stamp).then_some(t.pos + t.galley.size() * 0.5)
                        } else {
                            None
                        }
                    })
                    .expect("run row visible without scrolling past evidence");
                frame(&mut panel, vec![egui::Event::PointerMoved(pos)]);
                for pressed in [true, false] {
                    frame(
                        &mut panel,
                        vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        }],
                    );
                }
                if expect_report {
                    assert_eq!(panel.selected.as_ref(), Some(&report.path));
                    assert!(matches!(panel.preview, Some(PreviewKind::Markdown(_))));
                } else {
                    assert!(panel.selected.is_none(), "missing report must never fall back to evidence or display the previous run");
                    assert!(matches!(panel.preview, Some(PreviewKind::EmptyHint(_))));
                    let out = frame(&mut panel, vec![]);
                    assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "尚无 MD 报告")));
                }
            }
        });
    }

    #[test]
    fn data_preview_back_button_returns_to_markdown() {
        with_tmp(|root| {
            let md = root.join("summary.md");
            let json = root.join("checks.json");
            fs::write(&md, "# Report").unwrap();
            fs::write(&json, "{}").unwrap();
            let mut panel = TestReportPanel::new(&AppConfig::default());
            panel.open_markdown(md.clone(), Lang::Zh);
            panel.return_report = Some(md.clone());
            panel.apply_open(
                json,
                ArtifactView::Report,
                false,
                Lang::Zh,
                &mut ReportAction::None,
            );
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let tokens = Tokens::dark();
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.1;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640.0, 400.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.viewer_panel(ui, Lang::Zh, &tokens, None);
                        });
                    },
                )
            };
            let out = frame(vec![]);
            let pos = out
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(t) = &shape.shape {
                        if t.galley.job.text == "返回报告" {
                            return Some(t.pos + t.galley.size() * 0.5);
                        }
                    }
                    None
                })
                .expect("back button visible");
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }]);
            }
            assert_eq!(panel.selected, Some(md));
            assert!(matches!(panel.preview, Some(PreviewKind::Markdown(_))));
            assert!(panel.return_report.is_none());
        });
    }

    #[test]
    fn generated_report_renders_across_window_sizes() {
        with_tmp(|root| {
            let path = if let Ok(path) = std::env::var("WIPARSE_REPORT_AUDIT_MD") {
                PathBuf::from(path)
            } else {
                let path = root.join("summary.md");
                fs::write(&path, "# 检验报告\n\n[数据](checks.json)\n\n| 用例 | 原因 | 数据 |\n|---|---|---|\n| ADP01 | Long diagnostic with enough words to wrap across several lines and remain visible in narrow panels | [检查数据](checks.json) |\n| ADP02 | Normal | [配置](checks.json) |\n").unwrap();
                fs::write(root.join("checks.json"), "{}").unwrap();
                path
            };
            for width in [640.0, 1100.0, 1920.0] {
                let ctx = egui::Context::default();
                crate::fonts::install_cjk_fonts(&ctx);
                let tokens = crate::theme::apply_theme(&ctx, false);
                let mut panel = TestReportPanel::new(&AppConfig::default());
                panel.open_markdown(path.clone(), Lang::Zh);
                assert!(matches!(panel.preview, Some(PreviewKind::Markdown(_))));
                let size = egui::vec2(width, 720.0);
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            panel.ui(ui, Lang::Zh, &tokens, root, &[], None, "default", 0, None);
                        });
                    },
                );
                if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                    crate::ui_audit::export_frame(
                        &ctx,
                        output.clone(),
                        &PathBuf::from(dir),
                        &format!("report-{width:.0}"),
                        size,
                    );
                }
                for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                    assert!(primitive.clip_rect.is_finite());
                    if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                        assert!(mesh.is_valid());
                    }
                }
            }
        });
    }

    #[test]
    fn markdown_destinations_and_escaped_table_cells() {
        for url in ["cap(1).isf", "<截图 (1).png>"] {
            let chars: Vec<_> = format!("[数据]({url}) tail").chars().collect();
            let (_, got, next) = parse_md_link(&chars, 0).unwrap();
            assert_eq!(got, url);
            assert_eq!(chars[next..].iter().collect::<String>(), " tail");
        }
        assert_eq!(
            file_ref_from_text("[检查](checks.json)").as_deref(),
            Some("checks.json")
        );
        assert_eq!(
            decode_md_src("%E6%B3%A2%E5%BD%A2%20%281%29.isf"),
            "波形 (1).isf"
        );
        assert_eq!(
            split_table_row(r"| A | error\| detail | C |"),
            vec!["A", "error| detail", "C"]
        );
    }

    #[test]
    fn report_group_json_is_displayed_as_text_and_images_as_images() {
        with_tmp(|root| {
            let mut panel = TestReportPanel::new(&AppConfig::default());
            for (name, content) in [("checks.json", "{\"value\": 5}"), ("errors.jsonl", "")] {
                let path = root.join(name);
                fs::write(&path, content).unwrap();
                let mut action = ReportAction::None;
                panel.apply_open(path, ArtifactView::Report, false, Lang::En, &mut action);
                assert!(matches!(panel.preview, Some(PreviewKind::Text(_))));
                assert_eq!(action, ReportAction::None);
            }
            let mut action = ReportAction::None;
            panel.apply_open(
                root.join("shot.png"),
                ArtifactView::Report,
                false,
                Lang::En,
                &mut action,
            );
            assert!(matches!(panel.preview, Some(PreviewKind::Image)));
            panel.apply_open(
                root.join("wave.isf"),
                ArtifactView::Report,
                false,
                Lang::En,
                &mut action,
            );
            assert!(matches!(
                action,
                ReportAction::Open {
                    view: ArtifactView::Wave,
                    ..
                }
            ));
        });
    }

    #[test]
    fn wrapped_body_and_table_links_follow_exact_target_without_ctrl() {
        with_tmp(|root| {
            fs::write(root.join("first.json"), "{}").unwrap();
            fs::write(root.join("second.json"), "{}").unwrap();
            for table in [false, true] {
                let ctx = egui::Context::default();
                let tokens = Tokens::dark();
                let mut open = None;
                let mut time = 0.0;
                let text = "before [FIRST](first.json) middle [SECOND LONG LINK WRAPS HERE](second.json) after";
                let mut frame = |events| {
                    time += 0.1;
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(220.0, 600.0),
                            )),
                            time: Some(time),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                if table {
                                    paint_md_table(
                                        ui,
                                        &tokens,
                                        1,
                                        &[vec!["Data".into()], vec![text.into()]],
                                        Some(root),
                                        &mut open,
                                        "click",
                                    );
                                } else {
                                    paint_md_inline(
                                        ui,
                                        text,
                                        14.0,
                                        false,
                                        &tokens,
                                        Some(root),
                                        &mut open,
                                        "click",
                                    );
                                }
                            });
                        },
                    )
                };
                let out = frame(vec![]);
                let target = out
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(t) = &shape.shape {
                            if t.galley.job.text.contains("SECOND LONG") {
                                return t
                                    .galley
                                    .rows
                                    .iter()
                                    .flat_map(|r| &r.glyphs)
                                    .filter(|g| g.section_index == 3)
                                    .last()
                                    .map(|g| g.logical_rect().translate(t.pos.to_vec2()).center());
                            }
                        }
                        None
                    })
                    .expect("rendered link glyph");
                frame(vec![egui::Event::PointerMoved(target)]);
                frame(vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]);
                frame(vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]);
                assert_eq!(
                    open.as_ref().map(|o| o.0.clone()),
                    Some(root.join("second.json").canonicalize().unwrap()),
                    "table={table}"
                );
            }
        });
    }

    #[test]
    fn png_never_jumps_to_wave() {
        assert_eq!(
            open_view_for_file(ArtifactView::Wave, ArtifactView::Wave, "shot.png"),
            ArtifactView::External
        );
    }

    #[test]
    fn txt_follows_output_group() {
        assert_eq!(
            open_view_for_file(ArtifactView::Log, ArtifactView::Report, "notes.txt"),
            ArtifactView::Report
        );
        assert_eq!(
            open_view_for_file(ArtifactView::Log, ArtifactView::Log, "hub.txt"),
            ArtifactView::Log
        );
    }

    #[test]
    fn markdown_stays_in_report() {
        assert_eq!(
            open_view_for_file(ArtifactView::External, ArtifactView::External, "run.md"),
            ArtifactView::Report
        );
    }

    #[test]
    fn isf_opens_as_wave() {
        assert_eq!(
            open_view_for_file(ArtifactView::Wave, ArtifactView::Wave, "cap.isf"),
            ArtifactView::Wave
        );
    }

    #[test]
    fn short_label_strips_prefix_and_stamp() {
        assert_eq!(
            short_artifact_label(
                "tianshu_xinwei_mpp25w_timeout_20260915_143738.isf",
                "tianshu_xinwei_mpp25w",
                "20260915_143758",
            ),
            "timeout 14:37:38"
        );
        assert_eq!(
            short_artifact_label(
                "tianshu_xinwei_mpp25w_summary_20260915_143758.md",
                "tianshu_xinwei_mpp25w",
                "20260915_143758",
            ),
            "summary 14:37:58"
        );
        assert_eq!(
            short_artifact_label("index.jsonl", "lot", "20260915_143758"),
            "index.jsonl"
        );
    }

    #[test]
    fn result_stamp_drops_year() {
        assert_eq!(format_result_stamp("20260915_143758"), "09-15  14:37");
        assert_eq!(
            run_row_titles(&["20260915_143758"]),
            vec!["09-15  14:37".to_owned()]
        );
    }

    #[test]
    fn same_minute_runs_show_seconds() {
        assert_eq!(
            run_row_titles(&["20260915_143758", "20260915_143701"]),
            vec!["09-15  14:37:58".to_owned(), "09-15  14:37:01".to_owned()]
        );
    }

    #[test]
    fn same_second_runs_show_index() {
        assert_eq!(
            run_row_titles(&["20260915_143758", "20260915_143758"]),
            vec![
                "09-15  14:37:58 · 1".to_owned(),
                "09-15  14:37:58 · 2".to_owned()
            ]
        );
    }

    #[test]
    fn test_display_uses_plugin_report_name_not_stamp_or_title() {
        let test = wiparse_core::project::IndexedTest {
            id: "wiparse-dap".into(),
            name: "wiparse-dap".into(),
            name_zh: String::new(),
            plugin_id: "wiparse-dap".into(),
            installed: true,
            results: vec![],
        };
        let plugins = [PluginMeta {
            id: "wiparse-dap".into(),
            name: "CMSIS-DAP Probe".into(),
            name_zh: "CMSIS-DAP 调试通道".into(),
            outputs: vec![],
            file_prefix: "WiparseDap".into(),
            report_name: "WiparseDap".into(),
            report_name_zh: "DAP 通道".into(),
        }];
        assert_eq!(test_display_name(&test, &plugins, Lang::Zh), "DAP 通道");
        assert_eq!(test_display_name(&test, &plugins, Lang::En), "WiparseDap");
        let stamped = wiparse_core::project::IndexedTest {
            id: "20260916_143758".into(),
            name: "20260916_143758".into(),
            name_zh: String::new(),
            plugin_id: "wiparse-dap".into(),
            installed: true,
            results: vec![],
        };
        assert_eq!(
            test_display_name(&stamped, &plugins, Lang::En),
            "WiparseDap"
        );
    }

    #[test]
    fn file_ref_from_backtick_windows_path() {
        let s = "`D:\\\\data\\\\waves\\\\a.isf`";
        assert_eq!(
            file_ref_from_text(s).as_deref(),
            Some(r"D:\data\waves\a.isf")
        );
    }

    #[test]
    fn file_ref_ignores_plain_words() {
        assert!(file_ref_from_text("无").is_none());
        assert!(file_ref_from_text("https://example.com/a.png").is_none());
    }

    fn with_tmp(f: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "wiparse-md-asset-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        f(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn md_image_from_reports_can_reach_sibling_shots() {
        with_tmp(|root| {
            let artifacts = root.join("artifacts");
            let reports = artifacts.join("reports");
            let shots = artifacts.join("shots");
            fs::create_dir_all(&reports).unwrap();
            fs::create_dir_all(&shots).unwrap();
            let png = shots.join("wave.png");
            fs::write(&png, b"x").unwrap();
            let got = resolve_md_asset(Some(&reports), "../shots/wave.png").unwrap();
            assert_eq!(got.canonicalize().unwrap(), png.canonicalize().unwrap());
        });
    }

    #[test]
    fn md_absolute_wave_under_artifacts() {
        with_tmp(|root| {
            let artifacts = root.join("artifacts");
            let reports = artifacts.join("reports");
            let waves = artifacts.join("waves");
            fs::create_dir_all(&reports).unwrap();
            fs::create_dir_all(&waves).unwrap();
            let isf = waves.join("cap.isf");
            fs::write(&isf, b"x").unwrap();
            let got = resolve_md_asset(Some(&reports), isf.to_str().unwrap()).unwrap();
            assert_eq!(got.canonicalize().unwrap(), isf.canonicalize().unwrap());
        });
    }

    #[test]
    fn md_image_cannot_escape_artifacts() {
        with_tmp(|root| {
            let reports = root.join("artifacts").join("reports");
            fs::create_dir_all(&reports).unwrap();
            fs::write(root.join("secret.png"), b"x").unwrap();
            assert!(resolve_md_asset(Some(&reports), "../../secret.png").is_none());
        });
    }

    #[test]
    fn md_image_same_dir_still_works() {
        with_tmp(|root| {
            let png = root.join("inline.png");
            fs::write(&png, b"x").unwrap();
            let got = resolve_md_asset(Some(root), "inline.png").unwrap();
            assert_eq!(got.canonicalize().unwrap(), png.canonicalize().unwrap());
        });
    }
}
