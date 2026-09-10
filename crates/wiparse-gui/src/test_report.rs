//! Test Report tool — browse Markdown reports by folder and preview on the right.
//!
//! HTML/PDF are intentionally unsupported: proper in-app rendering needs a
//! browser/PDF engine, which is out of scope for this egui shell.

use crate::theme::{self as ui_theme, Tokens};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::paths::project_path;

const SIDE_W: f32 = 300.0;
const PANEL_GAP: f32 = 8.0;
const CARD_MARGIN_X: i8 = 10;
const MAX_PREVIEW_CHARS: usize = 400_000;
const MAX_MD_IMAGE_EDGE: u32 = 1600;
const ROOT_FOLDER_KEY: &str = "__root__";

#[derive(Debug, Clone)]
struct ReportEntry {
    name: String,
    path: PathBuf,
    size: u64,
    mtime: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct ReportFolder {
    name: String,
    files: Vec<ReportEntry>,
}

#[derive(Debug, Clone)]
enum PreviewKind {
    /// Parsed once on open — paint is O(blocks), not re-parse each frame.
    Markdown(std::sync::Arc<MdDoc>),
    EmptyHint(String),
}

/// Lightweight block IR for report Markdown (CommonMark subset + GFM tables).
#[derive(Debug, Clone)]
struct MdDoc {
    blocks: Vec<MdBlock>,
}

#[derive(Debug, Clone)]
enum MdBlock {
    Heading { level: u8, text: String },
    Paragraph { text: String },
    Bullet { depth: u8, text: String },
    Ordered { depth: u8, n: u32, text: String },
    Task { depth: u8, checked: bool, text: String },
    Quote { text: String },
    Code { lang: String, body: String },
    Image { alt: String, src: String },
    Table { rows: Vec<Vec<String>> },
    Rule,
}

pub struct TestReportPanel {
    browser_dir: String,
    scanned_dir: String,
    folders: Vec<ReportFolder>,
    selected: Option<PathBuf>,
    preview_title: String,
    preview: Option<PreviewKind>,
    status: String,
    /// Defer folder dialog to end-of-frame to avoid egui/rfd re-entrancy freezes.
    pending_pick_dir: bool,
    /// Textures for `![alt](path)` relative to the open report file.
    image_textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl TestReportPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let mut browser_dir = cfg.apps.test_report.browser_dir.clone();
        if browser_dir.trim().is_empty() {
            browser_dir = project_path("evidence").display().to_string();
        }
        let mut panel = Self {
            browser_dir,
            scanned_dir: String::new(),
            folders: Vec::new(),
            selected: None,
            preview_title: String::new(),
            preview: None,
            status: String::new(),
            pending_pick_dir: false,
            image_textures: HashMap::new(),
        };
        panel.refresh_browser();
        panel
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    fn report_count(&self) -> usize {
        self.folders.iter().map(|f| f.files.len()).sum()
    }

    pub fn api_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "browser_dir": self.browser_dir,
            "report_count": self.report_count(),
            "folder_count": self.folders.len(),
            "selected": self.selected.as_ref().map(|p| p.display().to_string()),
            "status": self.status,
        })
    }

    pub fn api_set_browser_dir(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let dir = params
            .get("dir")
            .or_else(|| params.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if dir.is_empty() {
            return err("ui.report.browser", "missing dir");
        }
        self.browser_dir = dir.to_owned();
        self.refresh_browser();
        self.persist_browser_dir();
        ok("ui.report.browser", self.api_snapshot())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        if self.status.is_empty() {
            self.status = tr(lang, "report.status_ready");
        }

        if self.pending_pick_dir {
            self.pending_pick_dir = false;
            self.run_pick_browser_dir();
        }

        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        if !full.is_positive() {
            return;
        }

        let side_w = SIDE_W
            .min(full.width() * 0.38)
            .max(250.0)
            .min((full.width() - 180.0).max(200.0));
        let view_w = (full.width() - side_w - PANEL_GAP).max(160.0);
        let side_rect = egui::Rect::from_min_size(full.min, egui::vec2(side_w, full.height()));
        let view_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, full.min.y),
            egui::vec2(view_w, full.height()),
        );

        panel_in_rect(ui, side_rect, |ui| self.browser_panel(ui, lang, tokens));
        panel_in_rect(ui, view_rect, |ui| self.viewer_panel(ui, lang, tokens));
    }

    fn browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
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
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = inner_w;

                ui.label(
                    RichText::new(tr(lang, "report.browser_dir"))
                        .size(ui_theme::FONT_TITLE)
                        .strong()
                        .color(tokens.text_primary),
                );
                let browser_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.browser_dir)
                        .desired_width(ctrl_w)
                        .hint_text(tr(lang, "report.browser_hint"))
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if browser_edit.lost_focus() {
                    self.refresh_browser();
                    self.persist_browser_dir();
                }

                let gap = 6.0;
                let btn_w = ((ctrl_w - gap) * 0.5).max(48.0);
                ui.horizontal(|ui| {
                    ui.set_max_width(ctrl_w);
                    ui.spacing_mut().item_spacing.x = gap;
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "btn.browse_dir"),
                        egui::vec2(btn_w, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.pending_pick_dir = true;
                        ui.ctx().request_repaint();
                    }
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "btn.refresh_browser"),
                        egui::vec2(btn_w, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.refresh_browser();
                    }
                });

                self.ensure_browser_fresh();

                let n = self.report_count();
                if n > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} · {}    {} · {}",
                            tr(lang, "report.folder_count"),
                            self.folders.len(),
                            tr(lang, "report.count"),
                            n
                        ))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                    );
                }

                if self.browser_dir.trim().is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "report.browser_hint"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                } else if self.folders.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "report.browser_empty"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                } else {
                    let list_h = ui.available_height().max(72.0);
                    let mut open_path: Option<PathBuf> = None;
                    let selected = self.selected.clone();
                    ui.spacing_mut().scroll.floating = false;
                    egui::ScrollArea::vertical()
                        .id_salt("report_browser_folders")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ctrl_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for folder in &self.folders {
                                let folder_label = if folder.name == ROOT_FOLDER_KEY {
                                    tr(lang, "report.folder_root")
                                } else {
                                    folder.name.clone()
                                };
                                let header =
                                    format!("{}  ({})", folder_label, folder.files.len());
                                egui::CollapsingHeader::new(
                                    RichText::new(header)
                                        .size(ui_theme::FONT_BODY)
                                        .strong()
                                        .color(tokens.text_primary),
                                )
                                .id_salt(("report-folder", folder.name.as_str()))
                                .default_open(self.folders.len() <= 3)
                                .show(ui, |ui| {
                                    ui.set_max_width(ctrl_w);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(tr(lang, "report.col_name"))
                                                .size(ui_theme::FONT_CAPTION)
                                                .color(tokens.text_muted),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(tr(lang, "report.col_mtime"))
                                                        .size(ui_theme::FONT_CAPTION)
                                                        .color(tokens.text_muted),
                                                );
                                                ui.add_space(8.0);
                                                ui.label(
                                                    RichText::new(tr(lang, "report.col_size"))
                                                        .size(ui_theme::FONT_CAPTION)
                                                        .color(tokens.text_muted),
                                                );
                                            },
                                        );
                                    });
                                    for file in &folder.files {
                                        let is_sel = selected
                                            .as_ref()
                                            .is_some_and(|p| same_path(p, &file.path));
                                        let row = file_row(
                                            ui,
                                            tokens,
                                            ctrl_w,
                                            &file.name,
                                            &format_size(file.size),
                                            &format_mtime(file.mtime),
                                            is_sel,
                                        );
                                        if row.clicked() {
                                            open_path = Some(file.path.clone());
                                        }
                                        row.on_hover_text(file.path.display().to_string());
                                    }
                                });
                            }
                            ui.add_space(8.0);
                        });
                    if let Some(path) = open_path {
                        self.open_report(path, lang);
                    }
                }
            });
    }

    fn viewer_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    let title = if self.preview_title.is_empty() {
                        tr(lang, "report.viewer_title")
                    } else {
                        self.preview_title.clone()
                    };
                    ui.label(
                        RichText::new(title)
                            .size(ui_theme::FONT_TITLE)
                            .strong()
                            .color(tokens.text_primary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.selected.is_some()
                            && ui_theme::secondary_btn_sized(
                                ui,
                                tokens,
                                tr(lang, "report.open_external"),
                                egui::vec2(120.0, ui_theme::CTRL_H),
                            )
                            .clicked()
                        {
                            self.open_external(lang);
                        }
                    });
                });
                ui.add_space(6.0);
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, tokens.divider),
                );
                ui.add_space(8.0);

                let content_h = ui.available_height().max(40.0);
                let mut open_ext = false;
                let preview = self.preview.clone();
                let base_dir = self
                    .selected
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                Frame::NONE
                    .fill(tokens.canvas_bg)
                    .stroke(Stroke::new(1.0_f32, tokens.divider))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_min_height(content_h - 4.0);
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
                                        ui.set_min_width((ui.available_width() - 4.0).max(80.0));
                                        paint_md_doc(
                                            ui,
                                            doc,
                                            tokens,
                                            base_dir.as_deref(),
                                            &mut self.image_textures,
                                        );
                                    });
                            }
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
                if open_ext {
                    self.open_external(lang);
                }
            });
    }

    fn ensure_browser_fresh(&mut self) {
        let cur = self.browser_dir.trim().to_owned();
        if cur != self.scanned_dir {
            self.refresh_browser();
        }
    }

    /// First-level folders (and root) with `.md` files — same model as serial log browser.
    fn refresh_browser(&mut self) {
        let raw = self.browser_dir.trim().to_owned();
        self.scanned_dir = raw.clone();
        self.folders.clear();
        if raw.is_empty() {
            return;
        }
        let root = resolve_dir(&raw);
        if !root.is_dir() {
            return;
        }

        let mut folders = Vec::new();

        if let Ok(entries) = fs::read_dir(&root) {
            let mut root_files = Vec::new();
            for ent in entries.flatten() {
                let path = ent.path();
                if path.is_file() && is_markdown(&path) {
                    if let Some(file) = report_entry_from_path(&path) {
                        root_files.push(file);
                    }
                }
            }
            if !root_files.is_empty() {
                root_files.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                });
                folders.push(ReportFolder {
                    name: ROOT_FOLDER_KEY.into(),
                    files: root_files,
                });
            }
        }

        if let Ok(entries) = fs::read_dir(&root) {
            let mut subdirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subdirs.sort_by(|a, b| {
                a.file_name()
                    .map(|n| n.to_ascii_lowercase())
                    .cmp(&b.file_name().map(|n| n.to_ascii_lowercase()))
            });
            for dir in subdirs {
                let name_lc = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    name_lc.as_str(),
                    "node_modules" | ".git" | "target" | "cache" | ".cache" | "dist"
                ) {
                    continue;
                }
                let Ok(files_iter) = fs::read_dir(&dir) else {
                    continue;
                };
                let mut md_files = Vec::new();
                for file in files_iter.flatten() {
                    let path = file.path();
                    if path.is_file() && is_markdown(&path) {
                        if let Some(entry) = report_entry_from_path(&path) {
                            md_files.push(entry);
                        }
                    }
                }
                if md_files.is_empty() {
                    continue;
                }
                md_files.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                });
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.display().to_string());
                folders.push(ReportFolder {
                    name,
                    files: md_files,
                });
            }
        }

        folders.sort_by(|a, b| {
            let a_root = a.name == ROOT_FOLDER_KEY;
            let b_root = b.name == ROOT_FOLDER_KEY;
            match (a_root, b_root) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase()),
            }
        });
        self.folders = folders;
    }

    fn run_pick_browser_dir(&mut self) {
        let start = resolve_dir(self.browser_dir.trim());
        let mut dialog = rfd::FileDialog::new().set_title("Select report directory");
        if start.is_dir() {
            dialog = dialog.set_directory(&start);
        }
        let Some(path) = dialog.pick_folder() else {
            return;
        };
        if !path.is_dir() {
            return;
        }
        self.browser_dir = path.display().to_string();
        self.refresh_browser();
        self.persist_browser_dir();
    }

    fn persist_browser_dir(&self) {
        if let Ok(mut cfg) = wiparse_core::config::load_config() {
            cfg.apps.test_report.browser_dir = self.browser_dir.clone();
            let _ = wiparse_core::config::save_config(&cfg);
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
            self.status = format!(
                "{} · {}",
                tr(lang, "report.open_external"),
                path.display()
            );
        } else {
            self.status = tr(lang, "report.status_open_err");
        }
    }
}

fn panel_in_rect(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.set_min_size(rect.size());
            add(ui);
        },
    );
}

fn file_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    width: f32,
    name: &str,
    size: &str,
    mtime: &str,
    selected: bool,
) -> egui::Response {
    let row_h = 22.0_f32;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, row_h), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(3),
            tokens.accent.gamma_multiply(0.18),
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(3),
            tokens.accent_soft.gamma_multiply(0.55),
        );
    }
    let y = rect.center().y;
    ui.painter().text(
        egui::pos2(rect.min.x + 4.0, y),
        egui::Align2::LEFT_CENTER,
        elide_chars(name, 28),
        FontId::proportional(ui_theme::FONT_BODY),
        tokens.text_primary,
    );
    ui.painter().text(
        egui::pos2(rect.max.x - 4.0, y),
        egui::Align2::RIGHT_CENTER,
        format!("{size}  {mtime}"),
        FontId::proportional(ui_theme::FONT_CAPTION),
        tokens.text_muted,
    );
    resp
}

fn elide_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_owned();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('\u{2026}'); // …
    out
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

fn resolve_dir(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        project_path(raw)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let na = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let nb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    na == nb
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "md" || e == "markdown"
        })
        .unwrap_or(false)
}

fn report_entry_from_path(path: &Path) -> Option<ReportEntry> {
    let meta = fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta.and_then(|m| m.modified().ok());
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some(ReportEntry {
        name,
        path: path.to_path_buf(),
        size,
        mtime,
    })
}

fn read_text_capped(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
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

fn open_path_external(path: &Path) -> bool {
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
            blocks.push(MdBlock::Paragraph {
                text: t.to_owned(),
            });
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
) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let mut table_id = 0u32;
    for block in &doc.blocks {
        match block {
            MdBlock::Heading { level, text } => {
                let size = match *level {
                    1 => 20.0,
                    2 => 16.5,
                    3 => 14.5,
                    _ => ui_theme::FONT_BODY + 1.0,
                };
                ui.add_space(if *level <= 2 { 10.0 } else { 6.0 });
                ui.add(
                    egui::Label::new(md_inline_job(text, size, true, tokens))
                        .wrap()
                        .selectable(false),
                );
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
                ui.add(
                    egui::Label::new(md_inline_job(text, ui_theme::FONT_BODY, false, tokens))
                        .wrap()
                        .selectable(true),
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
                    ui.add(
                        egui::Label::new(md_inline_job(text, ui_theme::FONT_BODY, false, tokens))
                            .wrap(),
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
                    ui.add(
                        egui::Label::new(md_inline_job(text, ui_theme::FONT_BODY, false, tokens))
                            .wrap(),
                    );
                });
            }
            MdBlock::Task { depth, checked, text } => {
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
                    ui.add(
                        egui::Label::new(md_inline_job(text, ui_theme::FONT_BODY, false, tokens))
                            .wrap(),
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
                        ui.add(
                            egui::Label::new(
                                RichText::new(text.as_str())
                                    .italics()
                                    .size(ui_theme::FONT_BODY)
                                    .color(tokens.text_muted),
                            )
                            .wrap(),
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
                            .id_salt(("md_code", body.len() as u64))
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
                paint_md_image(ui, tokens, base_dir, textures, alt, src);
                ui.add_space(4.0);
            }
            MdBlock::Table { rows } => {
                table_id = table_id.wrapping_add(1);
                ui.add_space(6.0);
                paint_md_table(ui, tokens, table_id, rows);
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
    let info: String = trimmed.chars().skip(n).collect::<String>().trim().to_owned();
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

fn resolve_md_asset(base_dir: Option<&Path>, src: &str) -> Option<PathBuf> {
    let raw = src.trim();
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let raw = raw
        .strip_prefix("file:///")
        .or_else(|| raw.strip_prefix("file://"))
        .unwrap_or(raw);
    let p = PathBuf::from(raw);
    let resolved = if p.is_absolute() {
        p
    } else if let Some(base) = base_dir {
        base.join(&p)
    } else {
        p
    };
    // Jail relative assets under the report directory when possible.
    if let Some(base) = base_dir {
        let base_c = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
        let res_c = resolved.canonicalize().unwrap_or(resolved.clone());
        if !res_c.starts_with(&base_c) {
            return None;
        }
        return Some(res_c);
    }
    Some(resolved)
}

fn load_md_texture(
    ctx: &egui::Context,
    path: &Path,
) -> Option<egui::TextureHandle> {
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
        let scale = (avail_w / size.x).min(1.0);
        let show = egui::vec2(size.x * scale, size.y * scale);
        Frame::NONE
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.add(egui::Image::new(tex).fit_to_exact_size(show));
                if !alt.is_empty() {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(alt)
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                }
            });
    } else {
        ui.label(
            RichText::new(format!("[img missing] {alt}"))
                .size(ui_theme::FONT_CAPTION)
                .color(tokens.text_muted),
        );
    }
}

fn paint_md_table(ui: &mut egui::Ui, tokens: &Tokens, id: u32, rows: &[Vec<String>]) {
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

    Frame::NONE
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.0_f32, tokens.divider))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_min_width(avail);
            for (ri, row) in rows.iter().enumerate() {
                let row_bg = if ri == 0 {
                    tokens.accent.gamma_multiply(0.10)
                } else if ri % 2 == 0 {
                    tokens.canvas_bg.gamma_multiply(0.55)
                } else {
                    Color32::TRANSPARENT
                };
                let row_h_guess = if ri == 0 { 26.0 } else { 22.0 };
                let (row_rect, _) = ui.allocate_exact_size(
                    egui::vec2(avail - 4.0, row_h_guess),
                    egui::Sense::hover(),
                );
                if row_bg.a() > 0 {
                    ui.painter()
                        .rect_filled(row_rect, CornerRadius::same(3), row_bg);
                }
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(row_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_min_height(row_h_guess);
                        for (ci, cell) in row.iter().enumerate().take(cols) {
                            let w = col_w.get(ci).copied().unwrap_or(48.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(w, row_h_guess),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if ri == 0 {
                                        ui.label(
                                            RichText::new(cell.as_str())
                                                .size(ui_theme::FONT_BODY)
                                                .strong()
                                                .color(tokens.text_primary),
                                        );
                                    } else {
                                        ui.add(
                                            egui::Label::new(md_inline_job(
                                                cell,
                                                ui_theme::FONT_BODY,
                                                false,
                                                tokens,
                                            ))
                                            .wrap(),
                                        );
                                    }
                                },
                            );
                        }
                    },
                );
                if ri == 0 {
                    ui.painter().hline(
                        row_rect.x_range(),
                        row_rect.max.y,
                        Stroke::new(1.0_f32, tokens.divider),
                    );
                }
                let _ = id;
            }
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
    let c = content.strip_prefix("- ").or_else(|| content.strip_prefix("* "))?;
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
    inner.split('|').map(|c| c.trim().to_owned()).collect()
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

fn md_inline_job(text: &str, size: f32, strong_all: bool, tokens: &Tokens) -> LayoutJob {
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
            if let Some((alt, _src, next)) = parse_md_link(&chars, i + 1) {
                flush(&mut job, &mut buf, &normal);
                let label = if alt.is_empty() {
                    "[image]".to_owned()
                } else {
                    format!("[{alt}]")
                };
                job.append(&label, 0.0, link.clone());
                i = next;
                continue;
            }
        }
        // [label](url)
        if chars[i] == '[' {
            if let Some((label, _url, next)) = parse_md_link(&chars, i) {
                flush(&mut job, &mut buf, &normal);
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
            let marker_len = if i + 2 < chars.len()
                && chars[i + 1] == '*'
                && chars[i + 2] == '*'
            {
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
    job
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut i = start + 1;
    let label_start = i;
    while i < chars.len() && chars[i] != ']' {
        i += 1;
    }
    if i >= chars.len() || chars.get(i + 1) != Some(&'(') {
        return None;
    }
    let label: String = chars[label_start..i].iter().collect();
    i += 2; // ](
    let url_start = i;
    while i < chars.len() && chars[i] != ')' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let url: String = chars[url_start..i].iter().collect();
    Some((label, url, i + 1))
}
