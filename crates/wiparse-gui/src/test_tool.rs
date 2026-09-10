//! Testing Hub — discover Node.js plugins and run them via WiParse CLI / HTTP.

use crate::theme::{self as ui_theme, Tokens};
use egui::{CornerRadius, Frame, Margin, RichText, Stroke};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::paths::project_path;

const SIDE_W: f32 = 268.0;
const PANEL_GAP: f32 = 8.0;
const CARD_MARGIN_X: i8 = 12;
const MAX_LOG_CHARS: usize = 200_000;
const LABEL_COL_W: f32 = 108.0;
const BTN_W: f32 = 76.0;

#[derive(Debug, Clone)]
struct PluginParam {
    name: String,
    type_name: String,
    default: String,
    path: String,
    label: String,
    label_zh: String,
    help: String,
}

#[derive(Debug, Clone)]
struct PluginInfo {
    id: String,
    name: String,
    name_zh: String,
    type_name: String,
    version: String,
    description: String,
    entry: String,
    config: String,
    dir: PathBuf,
    params: Vec<PluginParam>,
}

#[derive(Debug, Clone)]
enum LogKind {
    Stdout,
    Stderr,
    System,
}

struct LogLine {
    kind: LogKind,
    text: String,
}

struct RunningJob {
    child: Child,
    rx: Receiver<LogLine>,
    plugin_id: String,
    stop_file: Option<PathBuf>,
    status_file: Option<PathBuf>,
    last_status_read: Instant,
    last_status_mtime: Option<std::time::SystemTime>,
}

pub struct TestToolPanel {
    plugins_dir: String,
    cli_path: String,
    node_path: String,
    data_root: String,
    plugins: Vec<PluginInfo>,
    selected: Option<String>,
    type_filter: String,
    status: String,
    loop_hint: String,
    log: String,
    pending_pick_dir: bool,
    job: Option<RunningJob>,
    /// Extra free-form args after form params.
    extra_args: String,
    /// Values for selected plugin params (name → value).
    param_values: HashMap<String, String>,
    preflight_only: bool,
}

impl TestToolPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let mut plugins_dir = cfg.apps.test_tool.plugins_dir.clone();
        if plugins_dir.trim().is_empty() {
            plugins_dir = project_path("test-tools/plugins").display().to_string();
        }
        let mut cli_path = cfg.apps.test_tool.cli_path.clone();
        if cli_path.trim().is_empty() {
            cli_path = default_cli_path();
        }
        let mut node_path = cfg.apps.test_tool.node_path.clone();
        if node_path.trim().is_empty() {
            node_path = "node".into();
        }
        let data_root = cfg.apps.test_tool.data_root.clone();
        let mut panel = Self {
            plugins_dir,
            cli_path,
            node_path,
            data_root,
            plugins: Vec::new(),
            selected: None,
            type_filter: String::new(),
            status: String::new(),
            loop_hint: String::new(),
            log: String::new(),
            pending_pick_dir: false,
            job: None,
            extra_args: String::new(),
            param_values: HashMap::new(),
            preflight_only: false,
        };
        panel.refresh_plugins();
        panel
    }

    pub fn status_text(&self) -> &str {
        if !self.loop_hint.is_empty() {
            &self.loop_hint
        } else {
            &self.status
        }
    }

    pub fn api_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "plugins_dir": self.plugins_dir,
            "cli_path": self.cli_path,
            "node_path": self.node_path,
            "data_root": self.data_root,
            "plugin_count": self.plugins.len(),
            "selected": self.selected,
            "running": self.job.is_some(),
            "status": self.status,
            "loop_hint": self.loop_hint,
            "params": self.param_values,
            "plugins": self.plugins.iter().map(|p| serde_json::json!({
                "id": p.id,
                "name": p.name,
                "name_zh": p.name_zh,
                "type": p.type_name,
                "version": p.version,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn api_set_plugins_dir(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let dir = params
            .get("dir")
            .or_else(|| params.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if dir.is_empty() {
            return err("ui.test_tool.browser", "missing dir");
        }
        self.plugins_dir = dir.to_owned();
        self.refresh_plugins();
        self.persist_config();
        ok("ui.test_tool.browser", self.api_snapshot())
    }

    pub fn api_run(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = params
            .get("id")
            .or_else(|| params.get("plugin"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if id.is_empty() {
            return err("ui.test_tool.run", "missing plugin id");
        }
        self.selected = Some(id.to_owned());
        self.load_param_defaults_for_selection();
        if let Some(obj) = params.get("params").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    self.param_values.insert(k.clone(), s.to_owned());
                } else if let Some(n) = v.as_i64() {
                    self.param_values.insert(k.clone(), n.to_string());
                } else if let Some(b) = v.as_bool() {
                    self.param_values
                        .insert(k.clone(), if b { "true" } else { "false" }.into());
                }
            }
        }
        if let Some(extra) = params.get("args").and_then(|v| v.as_str()) {
            self.extra_args = extra.to_owned();
        }
        if let Some(pf) = params.get("preflight_only").and_then(|v| v.as_bool()) {
            self.preflight_only = pf;
        }
        match self.start_selected(Lang::En) {
            Ok(()) => ok("ui.test_tool.run", self.api_snapshot()),
            Err(e) => err("ui.test_tool.run", &e),
        }
    }

    pub fn api_stop(&mut self) -> crate::backend::InvokeReply {
        use crate::backend::invoke_ok as ok;
        self.stop_job();
        self.status = "stopped".into();
        ok("ui.test_tool.stop", self.api_snapshot())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        if self.status.is_empty() {
            self.status = tr(lang, "test_tool.status_ready");
        }
        self.poll_job(lang);
        if self.job.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        if self.pending_pick_dir {
            self.pending_pick_dir = false;
            self.run_pick_plugins_dir();
        }

        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        if !full.is_positive() {
            return;
        }

        let side_w = SIDE_W
            .min(full.width() * 0.34)
            .max(232.0)
            .min((full.width() - 220.0).max(200.0));
        let view_w = (full.width() - side_w - PANEL_GAP).max(200.0);
        let side_rect = egui::Rect::from_min_size(full.min, egui::vec2(side_w, full.height()));
        let view_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, full.min.y),
            egui::vec2(view_w, full.height()),
        );

        panel_in_rect(ui, side_rect, |ui| self.browser_panel(ui, lang, tokens));
        panel_in_rect(ui, view_rect, |ui| self.runner_panel(ui, lang, tokens));
    }

    fn browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 12))
            .show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = inner_w;

                ui.label(
                    RichText::new(tr(lang, "test_tool.plugins"))
                        .size(ui_theme::FONT_TITLE)
                        .strong()
                        .color(tokens.text_primary),
                );

                let gap = 6.0;
                let browse_w = if matches!(lang, Lang::Zh) { 72.0 } else { 72.0 };
                let refresh_w = if matches!(lang, Lang::Zh) { 48.0 } else { 64.0 };
                let path_w = (ctrl_w - gap * 2.0 - browse_w - refresh_w).max(48.0);
                ui.horizontal(|ui| {
                    ui.set_max_width(ctrl_w);
                    ui.spacing_mut().item_spacing.x = gap;
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.plugins_dir)
                            .desired_width(path_w)
                            .hint_text(tr(lang, "test_tool.plugins_hint"))
                            .margin(egui::vec2(6.0, 4.0)),
                    );
                    if edit.lost_focus() {
                        self.refresh_plugins();
                        self.persist_config();
                    }
                    if ui_theme::secondary_btn_sized(
                        ui,
                        tokens,
                        if matches!(lang, Lang::Zh) {
                            "浏览"
                        } else {
                            "Browse"
                        },
                        egui::vec2(browse_w, ui_theme::CTRL_H),
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
                        egui::vec2(refresh_w, ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.refresh_plugins();
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(lang, "test_tool.type_filter"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.type_filter)
                            .desired_width((ctrl_w - 48.0).max(80.0))
                            .hint_text("smoke / capture_loop")
                            .margin(egui::vec2(6.0, 3.0)),
                    );
                });

                let filter = self.type_filter.trim().to_ascii_lowercase();
                let visible: Vec<&PluginInfo> = self
                    .plugins
                    .iter()
                    .filter(|p| {
                        filter.is_empty() || p.type_name.to_ascii_lowercase().contains(&filter)
                    })
                    .collect();

                let count_label = if matches!(lang, Lang::Zh) {
                    format!("{} {}", tr(lang, "test_tool.count"), visible.len())
                } else {
                    format!("{} plugins", visible.len())
                };
                ui.label(
                    RichText::new(count_label)
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );

                ui.add_space(2.0);
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, tokens.divider),
                );
                ui.add_space(6.0);

                if visible.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "test_tool.empty"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                } else {
                    let list_h = ui.available_height().max(72.0);
                    let selected = self.selected.clone();
                    let mut pick: Option<String> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("test_tool_plugins")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ctrl_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for p in visible {
                                let title = if matches!(lang, Lang::Zh) && !p.name_zh.is_empty() {
                                    p.name_zh.as_str()
                                } else {
                                    p.name.as_str()
                                };
                                let is_sel = selected.as_deref() == Some(p.id.as_str());
                                let resp = ui.allocate_ui_with_layout(
                                    egui::vec2(ctrl_w, 40.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let fill = if is_sel {
                                            tokens.accent.linear_multiply(0.18)
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        };
                                        Frame::NONE
                                            .fill(fill)
                                            .corner_radius(CornerRadius::same(6))
                                            .inner_margin(Margin::symmetric(8, 6))
                                            .show(ui, |ui| {
                                                ui.set_min_width((ctrl_w - 8.0).max(40.0));
                                                ui.label(
                                                    RichText::new(title)
                                                        .size(ui_theme::FONT_BODY)
                                                        .strong()
                                                        .color(if is_sel {
                                                            tokens.accent
                                                        } else {
                                                            tokens.text_primary
                                                        }),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{} · v{}",
                                                        p.type_name, p.version
                                                    ))
                                                    .size(ui_theme::FONT_CAPTION)
                                                    .color(tokens.text_muted),
                                                );
                                            })
                                            .response
                                    },
                                );
                                let click = resp.response.interact(egui::Sense::click());
                                if click.clicked() {
                                    pick = Some(p.id.clone());
                                }
                                click.on_hover_text(format!(
                                    "{}\nv{}\n{}",
                                    p.id, p.version, p.description
                                ));
                            }
                        });
                    if let Some(id) = pick {
                        if self.selected.as_deref() != Some(id.as_str()) {
                            self.selected = Some(id);
                            self.load_param_defaults_for_selection();
                        }
                    }
                }
            });
    }

    fn runner_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);

                // --- Header: title + actions ---
                ui.horizontal(|ui| {
                    let title = if let Some(p) = self.selected_plugin() {
                        if matches!(lang, Lang::Zh) && !p.name_zh.is_empty() {
                            p.name_zh.clone()
                        } else {
                            p.name.clone()
                        }
                    } else {
                        tr(lang, "test_tool.runner_title")
                    };
                    ui.label(
                        RichText::new(title)
                            .size(ui_theme::FONT_TITLE)
                            .strong()
                            .color(tokens.text_primary),
                    );
                    if let Some(p) = self.selected_plugin() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!("{} · v{}", p.type_name, p.version))
                                .size(ui_theme::FONT_CAPTION)
                                .color(tokens.text_muted),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let running = self.job.is_some();
                        if running {
                            if ui_theme::secondary_btn_sized(
                                ui,
                                tokens,
                                tr(lang, "test_tool.stop"),
                                egui::vec2(BTN_W, ui_theme::CTRL_H),
                            )
                            .clicked()
                            {
                                self.stop_job();
                                self.status = tr(lang, "test_tool.status_stopped");
                                self.loop_hint.clear();
                            }
                        } else {
                            if ui_theme::primary_btn_sized(
                                ui,
                                tokens,
                                tr(lang, "test_tool.run"),
                                egui::vec2(BTN_W, ui_theme::CTRL_H),
                            )
                            .clicked()
                            {
                                self.preflight_only = false;
                                if let Err(e) = self.start_selected(lang) {
                                    self.append_log(LogKind::System, &format!("ERROR: {e}\n"));
                                    self.status = e;
                                }
                            }
                            if ui_theme::secondary_btn_sized(
                                ui,
                                tokens,
                                tr(lang, "test_tool.preflight"),
                                egui::vec2(BTN_W, ui_theme::CTRL_H),
                            )
                            .clicked()
                            {
                                self.preflight_only = true;
                                if let Err(e) = self.start_selected(lang) {
                                    self.append_log(LogKind::System, &format!("ERROR: {e}\n"));
                                    self.status = e;
                                }
                            }
                        }
                    });
                });

                // Status line
                let status_line = if !self.loop_hint.is_empty() {
                    self.loop_hint.as_str()
                } else if self.selected.is_none() {
                    ""
                } else {
                    self.status.as_str()
                };
                if !status_line.is_empty() {
                    ui.label(
                        RichText::new(status_line)
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                }

                if self.selected_plugin().is_none() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(tr(lang, "test_tool.select_hint"))
                            .size(ui_theme::FONT_BODY)
                            .color(tokens.text_muted),
                    );
                } else {
                    // --- Params (two-column rows) ---
                    let params = self
                        .selected_plugin()
                        .map(|p| p.params.clone())
                        .unwrap_or_default();
                    let visible_params: Vec<&PluginParam> = params
                        .iter()
                        .filter(|p| p.name != "preflight_only")
                        .collect();
                    if !visible_params.is_empty() {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(tr(lang, "test_tool.params"))
                                .size(ui_theme::FONT_CAPTION)
                                .strong()
                                .color(tokens.text_muted),
                        );
                        let param_h = ((visible_params.len() as f32) * (ui_theme::CTRL_H + 8.0))
                            .min(168.0)
                            .max(48.0);
                        egui::ScrollArea::vertical()
                            .id_salt("test_tool_params")
                            .max_height(param_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                for p in &visible_params {
                                    let label =
                                        if matches!(lang, Lang::Zh) && !p.label_zh.is_empty() {
                                            p.label_zh.as_str()
                                        } else if !p.label.is_empty() {
                                            p.label.as_str()
                                        } else {
                                            p.name.as_str()
                                        };
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(ui_theme::CTRL_H);
                                        ui.add_sized(
                                            egui::vec2(LABEL_COL_W, ui_theme::CTRL_H),
                                            egui::Label::new(
                                                RichText::new(label)
                                                    .size(ui_theme::FONT_CAPTION)
                                                    .color(tokens.text_muted),
                                            )
                                            .truncate(),
                                        );
                                        let entry =
                                            self.param_values.entry(p.name.clone()).or_default();
                                        let field_w =
                                            (ui.available_width() - 4.0).max(80.0);
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(entry)
                                                .desired_width(field_w)
                                                .hint_text(if p.help.is_empty() {
                                                    p.default.as_str()
                                                } else {
                                                    p.help.as_str()
                                                })
                                                .margin(egui::vec2(6.0, 3.0)),
                                        );
                                        if !p.help.is_empty() {
                                            resp.on_hover_text(&p.help);
                                        }
                                    });
                                }
                            });
                    }

                    // --- Advanced (collapsed; egui remembers open state by id) ---
                    ui.add_space(2.0);
                    let mut persist = false;
                    egui::CollapsingHeader::new(
                        RichText::new(tr(lang, "test_tool.advanced"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    )
                    .id_salt("test_tool_advanced")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let data_hint = project_path("").display().to_string();
                        if labeled_path_row(
                            ui,
                            tokens,
                            &tr(lang, "test_tool.data_root"),
                            &mut self.data_root,
                            &data_hint,
                        ) {
                            persist = true;
                        }
                        if labeled_path_row(
                            ui,
                            tokens,
                            &tr(lang, "test_tool.cli_path"),
                            &mut self.cli_path,
                            "",
                        ) {
                            persist = true;
                        }
                        if labeled_path_row(
                            ui,
                            tokens,
                            &tr(lang, "test_tool.node_path"),
                            &mut self.node_path,
                            "node",
                        ) {
                            persist = true;
                        }
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                egui::vec2(LABEL_COL_W, ui_theme::CTRL_H),
                                egui::Label::new(
                                    RichText::new(tr(lang, "test_tool.extra_args"))
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                )
                                .truncate(),
                            );
                            let field_w = (ui.available_width() - 4.0).max(80.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.extra_args)
                                    .desired_width(field_w)
                                    .hint_text("--custom flag")
                                    .margin(egui::vec2(6.0, 3.0)),
                            );
                        });
                    });
                    if persist {
                        self.persist_config();
                    }
                }

                ui.add_space(4.0);
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, tokens.divider),
                );
                ui.add_space(6.0);

                // --- Log fills rest ---
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr(lang, "test_tool.log"))
                            .size(ui_theme::FONT_CAPTION)
                            .strong()
                            .color(tokens.text_muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui_theme::secondary_btn_sized(
                            ui,
                            tokens,
                            tr(lang, "test_tool.clear_log"),
                            egui::vec2(64.0, ui_theme::CTRL_H),
                        )
                        .clicked()
                        {
                            self.log.clear();
                        }
                    });
                });

                let log_h = ui.available_height().max(96.0);
                Frame::NONE
                    .fill(tokens.canvas_bg)
                    .stroke(Stroke::new(1.0_f32, tokens.divider))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_min_height(log_h - 4.0);
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("test_tool_log")
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width((ui.available_width() - 4.0).max(80.0));
                                if self.log.is_empty() {
                                    ui.label(
                                        RichText::new(tr(lang, "test_tool.log_empty"))
                                            .size(ui_theme::FONT_CAPTION)
                                            .color(tokens.text_muted),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new(self.log.as_str())
                                            .monospace()
                                            .size(ui_theme::FONT_CAPTION)
                                            .color(tokens.text_primary),
                                    );
                                }
                            });
                    });
            });
    }

    fn selected_plugin(&self) -> Option<&PluginInfo> {
        let id = self.selected.as_deref()?;
        self.plugins.iter().find(|p| p.id == id)
    }

    fn load_param_defaults_for_selection(&mut self) {
        self.param_values.clear();
        let Some(p) = self.selected_plugin().cloned() else {
            return;
        };
        // Prefer station.json values when path is set; else param default.
        let station = load_station_json(&p);
        for param in &p.params {
            let mut val = param.default.clone();
            if !param.path.is_empty() {
                if let Some(v) = station_get(&station, &param.path) {
                    val = v;
                }
            }
            self.param_values.insert(param.name.clone(), val);
        }
        if let Some(v) = self.param_values.get("preflight_only") {
            self.preflight_only = is_truthy(v);
        }
    }

    fn refresh_plugins(&mut self) {
        self.plugins = scan_plugins(Path::new(self.plugins_dir.trim()));
        if let Some(sel) = &self.selected {
            if !self.plugins.iter().any(|p| p.id == *sel) {
                self.selected = None;
            }
        }
        if self.selected.is_none() {
            self.selected = self.plugins.first().map(|p| p.id.clone());
        }
        self.load_param_defaults_for_selection();
    }

    fn run_pick_plugins_dir(&mut self) {
        let start = resolve_dir(self.plugins_dir.trim());
        let mut dialog = rfd::FileDialog::new().set_title("Select plugins directory");
        if start.is_dir() {
            dialog = dialog.set_directory(&start);
        }
        let Some(path) = dialog.pick_folder() else {
            return;
        };
        if !path.is_dir() {
            return;
        }
        self.plugins_dir = path.display().to_string();
        self.refresh_plugins();
        self.persist_config();
    }

    fn persist_config(&self) {
        if let Ok(mut cfg) = wiparse_core::config::load_config() {
            cfg.apps.test_tool.plugins_dir = self.plugins_dir.clone();
            cfg.apps.test_tool.cli_path = self.cli_path.clone();
            cfg.apps.test_tool.node_path = self.node_path.clone();
            cfg.apps.test_tool.data_root = self.data_root.clone();
            let _ = wiparse_core::config::save_config(&cfg);
        }
    }

    fn build_plugin_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (k, v) in &self.param_values {
            if k == "preflight_only" {
                continue;
            }
            if v.trim().is_empty() {
                continue;
            }
            out.push(format!("--{k}"));
            out.push(v.clone());
        }
        out.extend(split_args(self.extra_args.trim()));
        out
    }

    fn resolve_runtime_paths(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let Some(p) = self.selected_plugin() else {
            return (None, None);
        };
        let station = load_station_json(p);
        // Apply form overrides onto a lightweight view for stop/status paths.
        let mut stop = station_get(&station, "paths.stop_file").map(PathBuf::from);
        let mut status = station_get(&station, "paths.status_file").map(PathBuf::from);
        if let Some(v) = self.param_values.get("isf_dir") {
            if !v.is_empty() {
                // status often under isf parent — keep station status if absolute
                let _ = v;
            }
        }
        // Expand simple templates for stop/status when still templated
        let data_root = if self.data_root.trim().is_empty() {
            project_path("")
        } else {
            PathBuf::from(self.data_root.trim())
        };
        let product = self
            .param_values
            .get("product")
            .cloned()
            .or_else(|| station_get(&station, "station.product"))
            .unwrap_or_default();
        let expand = |s: &str| -> PathBuf {
            let t = s
                .replace("{data_root}", &data_root.display().to_string())
                .replace("{product}", &product)
                .replace("{plugin_dir}", &p.dir.display().to_string());
            let pb = PathBuf::from(t);
            if pb.is_absolute() {
                pb
            } else {
                p.dir.join(pb)
            }
        };
        if let Some(ref s) = stop {
            let raw = s.display().to_string();
            stop = Some(expand(&raw));
        }
        if let Some(ref s) = status {
            let raw = s.display().to_string();
            status = Some(expand(&raw));
        }
        (stop, status)
    }

    fn start_selected(&mut self, lang: Lang) -> Result<(), String> {
        if self.job.is_some() {
            return Err(tr(lang, "test_tool.status_busy"));
        }
        let plugin = self
            .selected_plugin()
            .cloned()
            .ok_or_else(|| tr(lang, "test_tool.select_hint"))?;
        if !plugin
            .id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            || !plugin
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(format!("invalid plugin id: {}", plugin.id));
        }

        let runner = project_path("test-tools/runner.mjs");
        if !runner.is_file() {
            return Err(format!(
                "{}: {}",
                tr(lang, "test_tool.status_no_runner"),
                runner.display()
            ));
        }

        let node = self.node_path.trim().to_owned();
        if node.is_empty() {
            return Err(tr(lang, "test_tool.status_no_node"));
        }

        let mut args = vec![
            runner.display().to_string(),
            "--plugin".into(),
            plugin.id.clone(),
            "--lifecycle".into(),
            if self.preflight_only {
                "preflight".into()
            } else {
                "run".into()
            },
        ];
        let cli = self.cli_path.trim().to_owned();
        if !cli.is_empty() {
            args.push("--cli".into());
            args.push(cli);
        }
        let data_root = if self.data_root.trim().is_empty() {
            project_path("").display().to_string()
        } else {
            self.data_root.trim().to_owned()
        };
        args.push("--data-root".into());
        args.push(data_root);

        let extra = self.build_plugin_args();
        if !extra.is_empty() {
            args.push("--".into());
            args.extend(extra);
        }

        let (stop_file, status_file) = self.resolve_runtime_paths();
        if let Some(ref sf) = stop_file {
            let _ = fs::remove_file(sf);
        }

        let banner = format!(
            "\n—— run {} ({}) ——\n$ {} {}\n",
            plugin.id,
            plugin.type_name,
            node,
            args.join(" ")
        );
        self.append_log(LogKind::System, &banner);
        self.loop_hint.clear();

        let mut child = Command::new(&node)
            .args(&args)
            .current_dir(project_path("test-tools"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", tr(lang, "test_tool.status_spawn_err")))?;

        let (tx, rx) = mpsc::channel::<LogLine>();
        if let Some(out) = child.stdout.take() {
            let txo = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(out);
                for line in reader.lines().flatten() {
                    let _ = txo.send(LogLine {
                        kind: LogKind::Stdout,
                        text: line + "\n",
                    });
                }
            });
        }
        if let Some(err) = child.stderr.take() {
            let txe = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines().flatten() {
                    let _ = txe.send(LogLine {
                        kind: LogKind::Stderr,
                        text: line + "\n",
                    });
                }
            });
        }
        drop(tx);

        self.job = Some(RunningJob {
            child,
            rx,
            plugin_id: plugin.id.clone(),
            stop_file,
            status_file,
            last_status_read: Instant::now() - Duration::from_secs(1),
            last_status_mtime: None,
        });
        self.status = format!("{} · {}", tr(lang, "test_tool.status_running"), plugin.id);
        Ok(())
    }

    fn poll_job(&mut self, lang: Lang) {
        let mut pending = Vec::new();
        let mut finished: Option<(String, std::process::ExitStatus)> = None;
        let mut wait_err: Option<String> = None;
        let mut status_snapshot: Option<String> = None;

        if let Some(job) = self.job.as_mut() {
            loop {
                match job.rx.try_recv() {
                    Ok(line) => pending.push(line),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            match job.child.try_wait() {
                Ok(Some(status)) => {
                    let id = job.plugin_id.clone();
                    while let Ok(line) = job.rx.try_recv() {
                        pending.push(line);
                    }
                    finished = Some((id, status));
                }
                Ok(None) => {}
                Err(e) => wait_err = Some(e.to_string()),
            }

            // Throttle status-file IO: every 400ms, and only when mtime changes.
            if let Some(ref path) = job.status_file {
                if job.last_status_read.elapsed() >= Duration::from_millis(400) {
                    job.last_status_read = Instant::now();
                    let mtime = fs::metadata(path).ok().and_then(|m| m.modified().ok());
                    let changed = match (mtime, job.last_status_mtime) {
                        (Some(a), Some(b)) => a != b,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    if changed {
                        job.last_status_mtime = mtime;
                        if let Ok(text) = fs::read_to_string(path) {
                            status_snapshot = Some(text);
                        }
                    }
                }
            }
        }

        for line in pending {
            self.append_log(line.kind, &line.text);
        }

        if let Some(text) = status_snapshot {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let step = v.get("step").and_then(|x| x.as_str()).unwrap_or("");
                let hint = v.get("hint").and_then(|x| x.as_str()).unwrap_or("");
                let cycle = v.get("cycle").and_then(|x| x.as_u64());
                if !hint.is_empty() {
                    self.loop_hint = if let Some(c) = cycle {
                        format!("[{step} #{c}] {hint}")
                    } else {
                        format!("[{step}] {hint}")
                    };
                }
            }
        }

        if let Some(e) = wait_err {
            self.append_log(LogKind::System, &format!("wait error: {e}\n"));
            self.job = None;
            self.status = tr(lang, "test_tool.status_fail");
            return;
        }

        if let Some((id, status)) = finished {
            self.job = None;
            let code = status.code().unwrap_or(-1);
            if status.success() {
                self.append_log(LogKind::System, &format!("—— done {id} (exit 0) ——\n"));
                self.status = format!("{} · {}", tr(lang, "test_tool.status_ok"), id);
            } else {
                self.append_log(
                    LogKind::System,
                    &format!("—— failed {id} (exit {code}) ——\n"),
                );
                self.status =
                    format!("{} · {} ({code})", tr(lang, "test_tool.status_fail"), id);
            }
        }
    }

    fn stop_job(&mut self) {
        if let Some(mut job) = self.job.take() {
            let wrote_stop = if let Some(ref stop) = job.stop_file {
                if let Some(parent) = stop.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(stop, b"host stop\n").is_ok()
            } else {
                false
            };
            // Fallback only when host could not write the stop_file itself.
            if !wrote_stop {
                let node = self.node_path.trim().to_owned();
                let runner = project_path("test-tools/runner.mjs");
                if !node.is_empty() && runner.is_file() {
                    let data_root = if self.data_root.trim().is_empty() {
                        project_path("").display().to_string()
                    } else {
                        self.data_root.trim().to_owned()
                    };
                    let _ = Command::new(&node)
                        .args([
                            runner.display().to_string(),
                            "--plugin".into(),
                            job.plugin_id.clone(),
                            "--lifecycle".into(),
                            "stop".into(),
                            "--data-root".into(),
                            data_root,
                        ])
                        .current_dir(project_path("test-tools"))
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .stdin(Stdio::null())
                        .spawn();
                }
            }
            // Never block the UI thread: wait briefly off-thread then kill.
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(800);
                loop {
                    match job.child.try_wait() {
                        Ok(Some(_)) => return,
                        Ok(None) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(40));
                        }
                        _ => {
                            let _ = job.child.kill();
                            let _ = job.child.wait();
                            return;
                        }
                    }
                }
            });
            self.append_log(LogKind::System, "—— stop requested ——\n");
            self.loop_hint.clear();
        }
    }

    fn append_log(&mut self, kind: LogKind, text: &str) {
        let prefix = match kind {
            LogKind::Stdout => "",
            LogKind::Stderr => "[err] ",
            LogKind::System => "[sys] ",
        };
        self.log.push_str(prefix);
        self.log.push_str(text);
        if self.log.len() > MAX_LOG_CHARS {
            let keep = MAX_LOG_CHARS / 2;
            let mut drain = self.log.len() - keep;
            while drain < self.log.len() && !self.log.is_char_boundary(drain) {
                drain += 1;
            }
            self.log.drain(..drain);
            self.log.insert_str(0, "…\n");
        }
    }
}

impl Drop for TestToolPanel {
    fn drop(&mut self) {
        self.stop_job();
    }
}

fn labeled_path_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    value: &mut String,
    hint: &str,
) -> bool {
    let mut lost = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(LABEL_COL_W, ui_theme::CTRL_H),
            egui::Label::new(
                RichText::new(label)
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            )
            .truncate(),
        );
        let field_w = (ui.available_width() - 4.0).max(80.0);
        let resp = ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(field_w)
                .hint_text(hint)
                .margin(egui::vec2(6.0, 3.0)),
        );
        lost = resp.lost_focus();
    });
    lost
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

fn resolve_dir(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        project_path(raw)
    }
}

fn default_cli_path() -> String {
    let candidates = [
        project_path("dist/WiParse-CLI.exe"),
        project_path("dist/wiparse-cli.exe"),
        project_path("dist/wiparse.exe"),
        project_path("target/release/wiparse.exe"),
        project_path("target/debug/wiparse.exe"),
    ];
    for c in candidates {
        if c.is_file() {
            return c.display().to_string();
        }
    }
    if cfg!(windows) {
        "WiParse-CLI.exe".into()
    } else {
        "wiparse".into()
    }
}

fn scan_plugins(root: &Path) -> Vec<PluginInfo> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return out;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("plugin.json");
        if !manifest.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let id = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("plugin")
            })
            .to_owned();
        if id.trim().is_empty() {
            continue;
        }
        let params = v
            .get("params")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let name = p.get("name")?.as_str()?.to_owned();
                        Some(PluginParam {
                            name,
                            type_name: p
                                .get("type")
                                .and_then(|x| x.as_str())
                                .unwrap_or("string")
                                .to_owned(),
                            default: json_to_string(p.get("default")),
                            path: p
                                .get("path")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            label: p
                                .get("label")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            label_zh: p
                                .get("label_zh")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            help: p
                                .get("help")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(PluginInfo {
            id: id.clone(),
            name: v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or(&id)
                .to_owned(),
            name_zh: v
                .get("name_zh")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned(),
            type_name: v
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("custom")
                .to_owned(),
            version: v
                .get("version")
                .and_then(|x| x.as_str())
                .unwrap_or("0.0.0")
                .to_owned(),
            description: v
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned(),
            entry: v
                .get("entry")
                .and_then(|x| x.as_str())
                .unwrap_or("index.mjs")
                .to_owned(),
            config: v
                .get("config")
                .and_then(|x| x.as_str())
                .unwrap_or("station.json")
                .to_owned(),
            dir: path,
            params,
        });
    }
    out.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
    out
}

fn load_station_json(plugin: &PluginInfo) -> serde_json::Value {
    let path = plugin.dir.join(&plugin.config);
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn station_get(v: &serde_json::Value, dotted: &str) -> Option<String> {
    let mut cur = v;
    for part in dotted.split('.') {
        cur = cur.get(part)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn json_to_string(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn is_truthy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            ch if ch.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            ch => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

