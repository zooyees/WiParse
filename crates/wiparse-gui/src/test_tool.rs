//! Testing Hub — discover Node.js plugins and run them via WiParse CLI / HTTP.

use crate::market_jobs::{
    installed_map_from_registry, marketplace_cli_path, spawn_catalog_refresh, spawn_pull_install,
    spawn_uninstall, CatalogRow, HubMode, MarketJob, MarketJobEvent,
};
use crate::theme::{self as ui_theme, Tokens};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, CornerRadius, FontId, Frame, Margin, RichText, Stroke};
use std::collections::{HashMap, HashSet};
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
use wiparse_core::marketplace::CHANNELS;

const SIDE_W: f32 = 248.0;
const PANEL_GAP: f32 = 8.0;
const CARD_MARGIN_X: i8 = 8;
const MAX_LOG_CHARS: usize = 200_000;
const LABEL_COL_MIN: f32 = 72.0;
const LABEL_COL_MAX: f32 = 168.0;
const BTN_W: f32 = 72.0;
const PATH_BROWSE_W: f32 = 32.0;
const PARAM_GAP_X: f32 = 8.0;
const PLUGIN_ROW_H: f32 = 44.0;
const HUD_H: f32 = 26.0;
/// Config / Output horizontal split inside the runner card.
const RUNNER_SPLIT_GAP: f32 = 8.0;
const CONFIG_FRAC: f32 = 0.42;
const CONFIG_MIN_W: f32 = 300.0;
const OUTPUT_MIN_W: f32 = 260.0;

/// Live instrument row for generic `type: device` params. Hub does not
/// special-case oscilloscopes — plugins filter with `filter.kind`.
#[derive(Debug, Clone)]
pub struct LiveDevice {
    pub device_id: u64,
    pub resource: String,
    pub kind: String,
    pub model: String,
    pub manufacturer: String,
    pub serial: String,
}

impl LiveDevice {
    pub fn field(&self, key: &str) -> String {
        match key.trim().to_ascii_lowercase().as_str() {
            "device_id" | "id" => self.device_id.to_string(),
            "resource" => self.resource.clone(),
            "kind" => self.kind.clone(),
            "model" | "identity.model" => self.model.clone(),
            "manufacturer" | "identity.manufacturer" => self.manufacturer.clone(),
            "serial" | "identity.serial" => self.serial.clone(),
            _ => String::new(),
        }
    }

    pub fn label(&self) -> String {
        let model = if self.model.is_empty() {
            "device"
        } else {
            self.model.as_str()
        };
        if self.resource.is_empty() {
            format!("{model}  id={}", self.device_id)
        } else {
            format!("{model}  {}", self.resource)
        }
    }
}

#[derive(Debug, Clone)]
struct PluginParam {
    name: String,
    type_name: String,
    default: String,
    path: String,
    label: String,
    label_zh: String,
    help: String,
    /// Optional kind filter for `type: device` (`oscilloscope`, `dc_source`, …).
    filter_kind: String,
    /// If set, use this sibling param’s current value as the kind filter.
    filter_from: String,
    /// Map live-device fields → other param names when the user picks a device.
    fills: Vec<(String, String)>,
    /// `type: enum` choices: (value, label, label_zh).
    options: Vec<(String, String, String)>,
    hidden: bool,
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
    /// "bundled" | "marketplace"
    source: String,
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
    /// After Stop: force-kill child once this instant is reached (graceful window).
    kill_after: Option<Instant>,
}

/// Compact HUD state (mirrors scope-serial-monitor hud.ps1).
#[derive(Debug, Clone, Default)]
struct LoopHud {
    step: String,
    hint: String,
    cycle: Option<u64>,
    trigger: String,
    elapsed_s: Option<u64>,
    filename: String,
}

#[derive(Debug, Clone)]
enum PathPickTarget {
    PluginsDir,
    Param(String),
}

pub struct TestToolPanel {
    plugins_dir: String,
    cli_path: String,
    node_path: String,
    data_root: String,
    marketplace_install_dir: String,
    marketplace_enabled: bool,
    marketplace_base_url: String,
    marketplace_channel: String,
    hub_mode: HubMode,
    catalog: Vec<CatalogRow>,
    catalog_selected: Option<String>,
    /// Version chosen in Market detail (defaults to latest).
    catalog_selected_version: Option<String>,
    catalog_status: String,
    catalog_busy: bool,
    catalog_autoload: bool,
    market_job: Option<MarketJob>,
    plugins: Vec<PluginInfo>,
    selected: Option<String>,
    type_filter: String,
    status: String,
    loop_hint: String,
    loop_hud: LoopHud,
    log: String,
    pending_pick: Option<PathPickTarget>,
    job: Option<RunningJob>,
    /// Extra free-form args after form params.
    extra_args: String,
    /// Values for selected plugin params (name → value).
    param_values: HashMap<String, String>,
    preflight_only: bool,
    /// Defer station.json merge to the next frame so plugin clicks stay snappy.
    pending_station_merge: bool,
    /// Cached station.json per plugin id (mtime + value).
    station_cache: HashMap<String, (Option<std::time::SystemTime>, serde_json::Value)>,
    /// Connected instruments from the Instruments panel (this frame).
    live_devices: Vec<LiveDevice>,
    /// Params the user edited this selection (preflight autofill skips these).
    param_touched: HashSet<String>,
    serial_ports: Vec<String>,
    serial_ports_at: Option<Instant>,
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
        let marketplace_install_dir = cfg.apps.test_tool.marketplace.install_dir.clone();
        let marketplace_enabled = cfg.apps.test_tool.marketplace.enabled;
        let marketplace_base_url = if cfg.apps.test_tool.marketplace.base_url.trim().is_empty() {
            "http://127.0.0.1:8787".into()
        } else {
            cfg.apps.test_tool.marketplace.base_url.clone()
        };
        let marketplace_channel = if cfg.apps.test_tool.marketplace.channel.trim().is_empty() {
            "stable".into()
        } else {
            cfg.apps.test_tool.marketplace.channel.clone()
        };
        let mut panel = Self {
            plugins_dir,
            cli_path,
            node_path,
            data_root,
            marketplace_install_dir,
            marketplace_enabled,
            marketplace_base_url,
            marketplace_channel,
            hub_mode: HubMode::Plugins,
            catalog: Vec::new(),
            catalog_selected: None,
            catalog_selected_version: None,
            catalog_status: String::new(),
            catalog_busy: false,
            catalog_autoload: false,
            market_job: None,
            plugins: Vec::new(),
            selected: None,
            type_filter: String::new(),
            status: String::new(),
            loop_hint: String::new(),
            loop_hud: LoopHud::default(),
            log: String::new(),
            pending_pick: None,
            job: None,
            extra_args: String::new(),
            param_values: HashMap::new(),
            preflight_only: false,
            pending_station_merge: false,
            station_cache: HashMap::new(),
            live_devices: Vec::new(),
            param_touched: HashSet::new(),
            serial_ports: Vec::new(),
            serial_ports_at: None,
        };
        panel.refresh_plugins();
        panel
    }

    pub fn status_text(&self) -> &str {
        if self.hub_mode == HubMode::Market && !self.catalog_status.trim().is_empty() {
            &self.catalog_status
        } else {
            &self.status
        }
    }

    pub fn status_tone(&self) -> crate::theme::StatusTone {
        use crate::theme::StatusTone;
        if self.job.is_some() || self.catalog_busy {
            return StatusTone::Busy;
        }
        let s = self.status_text();
        let low = s.to_ascii_lowercase();
        if low.contains("fail")
            || low.contains("error")
            || s.contains("失败")
            || s.contains("错误")
            || low.contains("e_")
        {
            StatusTone::Error
        } else if s.contains("已安装") || low.contains("installed") || low.contains("ok") {
            StatusTone::Ok
        } else {
            StatusTone::Neutral
        }
    }

    pub fn api_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "plugins_dir": self.plugins_dir,
            "cli_path": self.cli_path,
            "node_path": self.node_path,
            "data_root": self.data_root,
            "marketplace_install_dir": self.marketplace_install_dir,
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
        self.select_plugin(id.to_owned());
        // API run needs station values immediately (no UI frame to defer into).
        self.merge_station_into_params();
        if let Some(obj) = params.get("params").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    self.param_values.insert(k.clone(), s.to_owned());
                    self.param_touched.insert(k.clone());
                } else if let Some(n) = v.as_i64() {
                    self.param_values.insert(k.clone(), n.to_string());
                    self.param_touched.insert(k.clone());
                } else if let Some(b) = v.as_bool() {
                    self.param_values
                        .insert(k.clone(), if b { "true" } else { "false" }.into());
                    self.param_touched.insert(k.clone());
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
        self.loop_hud = LoopHud {
            step: "stopped".into(),
            ..LoopHud::default()
        };
        ok("ui.test_tool.stop", self.api_snapshot())
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        devices: &[LiveDevice],
    ) {
        self.live_devices.clear();
        self.live_devices.extend(devices.iter().cloned());
        if self.status.is_empty() {
            self.status = tr(lang, "test_tool.status_ready");
        }
        // Plugin rows are painter-hit-tested; never let galley text-selection steal clicks.
        ui.style_mut().interaction.selectable_labels = false;
        // Apply deferred station.json AFTER the click frame, so selection feels instant.
        if self.pending_station_merge {
            self.pending_station_merge = false;
            self.merge_station_into_params();
        }
        self.poll_job(lang);
        self.poll_market_job(lang);
        if self.job.is_some() || self.market_job.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        if let Some(target) = self.pending_pick.take() {
            self.run_path_pick(target);
        }

        if !self.catalog_autoload
            && !self.catalog_busy
            && !self.marketplace_base_url.trim().is_empty()
        {
            self.catalog_autoload = true;
            self.refresh_catalog(lang);
        }

        let avail = ui.available_size();
        let (full, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
        if !full.is_positive() {
            return;
        }

        let side_w = SIDE_W
            .min(full.width() * 0.30)
            .max(200.0)
            .min((full.width() - 280.0).max(180.0));
        let view_w = (full.width() - side_w - PANEL_GAP).max(220.0);
        let side_rect = egui::Rect::from_min_size(full.min, egui::vec2(side_w, full.height()));
        let view_rect = egui::Rect::from_min_size(
            egui::pos2(full.min.x + side_w + PANEL_GAP, full.min.y),
            egui::vec2(view_w, full.height()),
        );

        match self.hub_mode {
            HubMode::Plugins => {
                panel_in_rect(ui, side_rect, |ui| self.browser_panel(ui, lang, tokens));
                panel_in_rect(ui, view_rect, |ui| self.runner_panel(ui, lang, tokens));
            }
            HubMode::Market => {
                panel_in_rect(ui, side_rect, |ui| self.market_browser_panel(ui, lang, tokens));
                panel_in_rect(ui, view_rect, |ui| self.market_detail_panel(ui, lang, tokens));
            }
        }
    }

    fn paint_hub_mode_tabs(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        width: f32,
    ) {
        let plugins_sel = self.hub_mode == HubMode::Plugins;
        let left = tr(lang, "test_tool.plugins");
        let right = tr(lang, "test_tool.market");
        let changed = ui_theme::segmented_two(
            ui,
            tokens,
            &left,
            &right,
            plugins_sel,
            egui::vec2(width, ui_theme::CTRL_H),
        );
        if let Some(want_plugins) = changed {
            if want_plugins {
                self.hub_mode = HubMode::Plugins;
            } else {
                self.hub_mode = HubMode::Market;
                if self.catalog.is_empty() && !self.catalog_busy {
                    self.refresh_catalog(lang);
                }
            }
        }
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
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = ui.available_width();

                self.paint_hub_mode_tabs(ui, lang, tokens, ctrl_w);

                let refresh_lbl = tr(lang, "btn.refresh_browser");
                let search_hint = tr(lang, "test_tool.search");
                let (_, refresh) = hub_edit_with_btn(
                    ui,
                    tokens,
                    &mut self.type_filter,
                    search_hint,
                    &refresh_lbl,
                    ctrl_w,
                );
                if refresh {
                    self.refresh_plugins();
                }

                egui::CollapsingHeader::new(
                    RichText::new(tr(lang, "test_tool.folder"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                )
                .default_open(false)
                .show(ui, |ui| {
                    let browse_lbl = tr(lang, "test_tool.browse");
                    let hint = tr(lang, "test_tool.plugins_hint");
                    let (path_edit, browse) = hub_edit_with_btn(
                        ui,
                        tokens,
                        &mut self.plugins_dir,
                        hint,
                        &browse_lbl,
                        ui.available_width().min(ctrl_w),
                    );
                    if browse {
                        self.pending_pick = Some(PathPickTarget::PluginsDir);
                        ui.ctx().request_repaint();
                    }
                    if path_edit.lost_focus() {
                        self.refresh_plugins();
                        self.persist_config();
                    }
                });

                let search = self.type_filter.trim().to_ascii_lowercase();
                let visible_idxs: Vec<usize> = self
                    .plugins
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        search.is_empty()
                            || p.id.to_ascii_lowercase().contains(&search)
                            || p.name.to_ascii_lowercase().contains(&search)
                            || p.name_zh.to_ascii_lowercase().contains(&search)
                            || p.type_name.to_ascii_lowercase().contains(&search)
                    })
                    .map(|(i, _)| i)
                    .collect();

                ui.add_space(2.0);
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, tokens.divider),
                );
                ui.add_space(6.0);

                if visible_idxs.is_empty() {
                    ui.label(
                        RichText::new(tr(lang, "test_tool.empty_cta"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    if ui_theme::primary_btn_sized(
                        ui,
                        tokens,
                        tr(lang, "test_tool.market"),
                        egui::vec2(inner_w.min(120.0), ui_theme::CTRL_H),
                    )
                    .clicked()
                    {
                        self.hub_mode = HubMode::Market;
                        if self.catalog.is_empty() && !self.catalog_busy {
                            self.refresh_catalog(lang);
                        }
                    }
                } else {
                    let list_h = ui.available_height().max(72.0);
                    let selected_id = self.selected.clone();
                    let update_ids: std::collections::HashSet<String> = self
                        .catalog
                        .iter()
                        .filter(|r| r.has_update())
                        .map(|r| r.id.clone())
                        .collect();
                    let mut pick: Option<String> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("test_tool_plugins")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ctrl_w);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            for &idx in &visible_idxs {
                                let Some(p) = self.plugins.get(idx) else {
                                    continue;
                                };
                                let title = if matches!(lang, Lang::Zh) && !p.name_zh.is_empty() {
                                    p.name_zh.clone()
                                } else {
                                    p.name.clone()
                                };
                                let source_tag = if p.source == "marketplace" {
                                    tr(lang, "test_tool.source_market")
                                } else {
                                    tr(lang, "test_tool.source_local")
                                };
                                let type_ver = format!("v{} · {}", p.version, source_tag);
                                let id = p.id.clone();
                                let is_sel = selected_id.as_deref() == Some(id.as_str());
                                let badge_text = tr(lang, "test_tool.market_update_badge");
                                let badge = if update_ids.contains(&id) {
                                    Some((badge_text.as_str(), tokens.warning))
                                } else {
                                    None
                                };
                                let tooltip = format!(
                                    "{}\nv{}\n{}",
                                    p.id, p.version, p.description
                                );
                                let resp = ui.push_id(&id, |ui| {
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(ctrl_w, PLUGIN_ROW_H),
                                        egui::Sense::click(),
                                    );
                                    if ui.is_rect_visible(rect) {
                                        paint_plugin_row(
                                            ui,
                                            tokens,
                                            rect,
                                            resp.hovered(),
                                            is_sel,
                                            &title,
                                            &type_ver,
                                            badge,
                                        );
                                    }
                                    resp
                                })
                                .inner;
                                if resp.clicked() {
                                    pick = Some(id);
                                }
                                resp.on_hover_text(tooltip);
                            }
                        });
                    if let Some(id) = pick {
                        self.select_plugin(id);
                        ui.ctx().request_repaint();
                    }
                }
            });
    }

    fn runner_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);

                let sel_idx = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.plugins.iter().position(|p| p.id == *id));

                self.paint_runner_header(ui, lang, tokens, sel_idx);

                // Left: config  |  Right: output
                let body_h = ui.available_height().max(120.0);
                let body_w = ui.available_width();
                let gap = RUNNER_SPLIT_GAP * 2.0; // space around divider
                let cfg_w = if body_w < CONFIG_MIN_W + OUTPUT_MIN_W + gap {
                    (body_w * 0.45).max(160.0)
                } else {
                    (body_w * CONFIG_FRAC).clamp(CONFIG_MIN_W, body_w - OUTPUT_MIN_W - gap)
                };

                ui.horizontal(|ui| {
                    ui.set_min_height(body_h);
                    ui.allocate_ui_with_layout(
                        egui::vec2(cfg_w, body_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(body_h);
                            ui.set_max_width(cfg_w);
                            self.paint_runner_config(ui, lang, tokens, sel_idx);
                        },
                    );
                    ui.add_space(RUNNER_SPLIT_GAP);
                    let div_x = ui.cursor().left();
                    let div_y = ui.max_rect().y_range();
                    ui.painter()
                        .vline(div_x, div_y, Stroke::new(1.0_f32, tokens.divider));
                    ui.add_space(RUNNER_SPLIT_GAP + 1.0);
                    let out_avail = ui.available_width().max(120.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(out_avail, body_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(body_h);
                            ui.set_min_width(out_avail);
                            self.paint_runner_output(ui, lang, tokens);
                        },
                    );
                });
            });
    }

    fn paint_runner_header(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        sel_idx: Option<usize>,
    ) {
        let running = self.job.is_some();
        let run_lbl = if running {
            tr(lang, "test_tool.stop")
        } else {
            tr(lang, "test_tool.run")
        };
        let pre_lbl = tr(lang, "test_tool.preflight");
        let clear_lbl = tr(lang, "test_tool.clear_log");
        let run_w = text_btn_w(ui, &run_lbl);
        let pre_w = if running { 0.0 } else { text_btn_w(ui, &pre_lbl) };
        let clear_w = text_btn_w(ui, &clear_lbl);
        let gap = 6.0;
        let cluster_w = run_w
            + clear_w
            + gap
            + if pre_w > 0.0 { pre_w + gap } else { 0.0 };

        let row_w = ui.available_width().max(cluster_w + 80.0);
        let row_h = ui_theme::CTRL_H;
        let (row, _) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::hover());
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(row.max.x - cluster_w, row.min.y),
            egui::vec2(cluster_w, row_h),
        );
        let left_rect = egui::Rect::from_min_max(
            row.min,
            egui::pos2((btn_rect.min.x - 8.0).max(row.min.x), row.max.y),
        );

        let title = if let Some(i) = sel_idx {
            let p = &self.plugins[i];
            if matches!(lang, Lang::Zh) && !p.name_zh.is_empty() {
                p.name_zh.clone()
            } else {
                p.name.clone()
            }
        } else {
            tr(lang, "test_tool.runner_title")
        };
        let version = sel_idx.map(|i| format!("v{}", self.plugins[i].version));

        place_in_rect(
            ui,
            left_rect,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(&title)
                            .size(ui_theme::FONT_TITLE)
                            .strong()
                            .color(tokens.text_primary),
                    )
                    .truncate()
                    .selectable(false),
                );
                if let Some(v) = version {
                    ui.add(
                        egui::Label::new(
                            RichText::new(v)
                                .size(ui_theme::FONT_CAPTION)
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                }
                self.paint_status_chip(ui, lang, tokens);
            },
        );

        let mut run_clicked = false;
        let mut pre_clicked = false;
        let mut stop_clicked = false;
        let mut clear_clicked = false;
        place_in_rect(
            ui,
            btn_rect,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let can_run = sel_idx.is_some() && !running;
                if running {
                    stop_clicked = ui_theme::stop_btn_sized(
                        ui,
                        tokens,
                        run_lbl,
                        egui::vec2(run_w, row_h),
                    )
                    .clicked();
                } else {
                    run_clicked = ui_theme::primary_btn_sized_enabled(
                        ui,
                        tokens,
                        run_lbl,
                        egui::vec2(run_w, row_h),
                        can_run,
                    )
                    .clicked();
                    pre_clicked = ui_theme::secondary_btn_sized_enabled(
                        ui,
                        tokens,
                        pre_lbl,
                        egui::vec2(pre_w, row_h),
                        can_run,
                    )
                    .on_hover_text(tr(lang, "test_tool.preflight_help"))
                    .clicked();
                }
                clear_clicked = ui_theme::secondary_btn_sized(
                    ui,
                    tokens,
                    clear_lbl,
                    egui::vec2(clear_w, row_h),
                )
                .clicked();
            },
        );

        if clear_clicked {
            self.log.clear();
        }
        if stop_clicked {
            self.stop_job();
            self.status = tr(lang, "test_tool.status_stopped");
            self.loop_hint.clear();
            self.loop_hud = LoopHud {
                step: "stopped".into(),
                ..LoopHud::default()
            };
        } else if run_clicked {
            self.preflight_only = false;
            if let Err(e) = self.start_selected(lang) {
                self.append_log(LogKind::System, &format!("ERROR: {e}\n"));
                self.status = e;
            }
        } else if pre_clicked {
            self.preflight_only = true;
            if let Err(e) = self.start_selected(lang) {
                self.append_log(LogKind::System, &format!("ERROR: {e}\n"));
                self.status = e;
            }
        }
    }

    fn paint_runner_config(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        sel_idx: Option<usize>,
    ) {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);

        if self.job.is_some() || !self.loop_hud.step.is_empty() {
            self.paint_loop_hud(ui, lang, tokens);
        }

        let Some(sel_idx) = sel_idx else {
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr(lang, "test_tool.select_hint"))
                    .size(ui_theme::FONT_BODY)
                    .color(tokens.text_muted),
            );
            return;
        };

        let param_idxs: Vec<usize> = self.plugins[sel_idx]
            .params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name != "preflight_only" && !p.hidden)
            .map(|(i, _)| i)
            .collect();

        if param_idxs.is_empty() {
            return;
        }

        // Same CTRL_H row as the Output header so the two columns line up.
        let cap_w = ui.available_width();
        let (cap, _) =
            ui.allocate_exact_size(egui::vec2(cap_w, ui_theme::CTRL_H), egui::Sense::hover());
        ui.painter_at(cap).text(
            egui::pos2(cap.min.x, cap.center().y),
            Align2::LEFT_CENTER,
            tr(lang, "test_tool.params"),
            FontId::proportional(ui_theme::FONT_CAPTION),
            tokens.text_muted,
        );

        let label_w = self.measure_param_label_col(ui, lang, sel_idx, &param_idxs);
        let param_h = ui.available_height().max(72.0);
        let mut pending_param: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("test_tool_params")
            .max_height(param_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                // Preserve plugin.json order; widgets sit in a pre-computed row rect.
                for &pi in &param_idxs {
                    self.paint_param_field(
                        ui,
                        lang,
                        tokens,
                        sel_idx,
                        pi,
                        label_w,
                        &mut pending_param,
                    );
                }
            });
        if let Some(name) = pending_param {
            self.pending_pick = Some(PathPickTarget::Param(name));
            ui.ctx().request_repaint();
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new(
            RichText::new(tr(lang, "test_tool.advanced"))
                .size(ui_theme::FONT_CAPTION)
                .strong()
                .color(tokens.text_muted),
        )
        .default_open(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let full_w = ui.available_width();

            ui.label(
                RichText::new(tr(lang, "test_tool.node_path"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.node_path)
                    .desired_width(full_w)
                    .hint_text("node")
                    .margin(egui::vec2(6.0, 3.0)),
            );
            if r.lost_focus() {
                self.persist_config();
            }

            ui.label(
                RichText::new(tr(lang, "test_tool.cli_path"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.cli_path)
                    .desired_width(full_w)
                    .hint_text("wiparse")
                    .margin(egui::vec2(6.0, 3.0)),
            );
            if r.lost_focus() {
                self.persist_config();
            }

            ui.label(
                RichText::new(tr(lang, "test_tool.data_root"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.data_root)
                    .desired_width(full_w)
                    .hint_text(".")
                    .margin(egui::vec2(6.0, 3.0)),
            );
            if r.lost_focus() {
                self.persist_config();
            }

            ui.label(
                RichText::new(tr(lang, "test_tool.extra_args"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.extra_args)
                    .desired_width(full_w)
                    .hint_text("--flag value")
                    .margin(egui::vec2(6.0, 3.0)),
            );
            if r.lost_focus() {
                self.persist_config();
            }

            ui.label(
                RichText::new(tr(lang, "test_tool.market_url"))
                    .size(ui_theme::FONT_CAPTION)
                    .color(tokens.text_muted),
            );
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.marketplace_base_url)
                    .desired_width(full_w)
                    .hint_text("http://127.0.0.1:8787")
                    .margin(egui::vec2(6.0, 3.0)),
            );
            if r.lost_focus() {
                self.persist_config();
            }
        });
    }

    fn paint_runner_output(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
        let cap_w = ui.available_width();
        let (cap, _) =
            ui.allocate_exact_size(egui::vec2(cap_w, ui_theme::CTRL_H), egui::Sense::hover());
        ui.painter_at(cap).text(
            egui::pos2(cap.min.x, cap.center().y),
            Align2::LEFT_CENTER,
            tr(lang, "test_tool.log"),
            FontId::proportional(ui_theme::FONT_CAPTION),
            tokens.text_muted,
        );

        let log_h = ui.available_height().max(80.0);
        Frame::NONE
            .fill(tokens.canvas_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_min_height(log_h - 4.0);
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::both()
                    .id_salt("test_tool_log")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let wrap_w = (ui.available_width() - 4.0).max(80.0);
                        ui.set_min_width(wrap_w);
                        if self.log.is_empty() {
                            ui.label(
                                RichText::new(tr(lang, "test_tool.log_empty"))
                                    .size(ui_theme::FONT_CAPTION)
                                    .color(tokens.text_muted),
                            );
                        } else {
                            let job = hub_log_layout_job(&self.log, tokens, wrap_w);
                            ui.add(egui::Label::new(job).wrap().selectable(true));
                        }
                    });
            });
    }

    fn paint_param_field(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        sel_idx: usize,
        param_idx: usize,
        label_w: f32,
        pending_param: &mut Option<String>,
    ) {
        let Some(p) = self.plugins.get(sel_idx).and_then(|pl| pl.params.get(param_idx)) else {
            return;
        };
        let name = p.name.clone();
        let type_name = p.type_name.trim().to_ascii_lowercase();
        let label = if matches!(lang, Lang::Zh) && !p.label_zh.is_empty() {
            p.label_zh.clone()
        } else if !p.label.is_empty() {
            p.label.clone()
        } else {
            p.name.clone()
        };
        let hint = if p.help.is_empty() {
            p.default.clone()
        } else {
            p.help.clone()
        };
        let help = p.help.clone();
        let is_path = is_path_param(p);
        let is_bool = type_name == "boolean" || type_name == "bool";
        let is_device = type_name == "device";
        let is_enum = type_name == "enum";
        let is_serial = type_name == "serial_port" || type_name == "serial";
        let filter_kind = self.device_filter_kind(p);
        let options = p.options.clone();

        if is_serial {
            self.refresh_serial_ports_if_needed();
        }

        // Place label / field / browse from the row rect. Never subtract
        // `available_width` inside a horizontal layout — item_spacing makes
        // that arithmetic drift and wraps a second empty control.
        let row_w = ui.available_width().max(120.0);
        let row_h = ui_theme::CTRL_H;
        let (row, _) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::hover());
        let gap = PARAM_GAP_X;
        let label_w = label_w.min((row_w * 0.45).max(LABEL_COL_MIN));
        let browse_w = if is_path { PATH_BROWSE_W } else { 0.0 };
        let label_rect = egui::Rect::from_min_size(row.min, egui::vec2(label_w, row_h));
        let btn_rect = if is_path {
            egui::Rect::from_min_size(
                egui::pos2(row.max.x - browse_w, row.min.y),
                egui::vec2(browse_w, row_h),
            )
        } else {
            egui::Rect::NOTHING
        };
        let field_left = label_rect.max.x + gap;
        let field_right = if is_path {
            (btn_rect.min.x - gap).max(field_left + 48.0)
        } else {
            row.max.x
        };
        let field_rect = egui::Rect::from_min_max(
            egui::pos2(field_left, row.min.y),
            egui::pos2(field_right, row.max.y),
        );

        let painter = ui.painter_at(label_rect);
        painter.text(
            egui::pos2(label_rect.min.x, label_rect.center().y),
            Align2::LEFT_CENTER,
            &label,
            FontId::proportional(ui_theme::FONT_CAPTION),
            tokens.text_muted,
        );
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if label_rect.contains(pos) && label.chars().count() > 14 {
                ui.interact(
                    label_rect,
                    ui.id().with(("param_label", &name)),
                    egui::Sense::hover(),
                )
                .on_hover_text(&label);
            }
        }

        let hover_resp: Option<egui::Response>;
        if is_bool {
            let mut on = is_truthy(self.param_values.get(&name).map(|s| s.as_str()).unwrap_or(""));
            let resp = place_in_rect(
                ui,
                field_rect,
                egui::Layout::top_down(egui::Align::Min),
                |ui| ui.checkbox(&mut on, ""),
            );
            if resp.changed() {
                self.param_values
                    .insert(name.clone(), if on { "true" } else { "false" }.into());
                self.param_touched.insert(name.clone());
            }
            hover_resp = Some(resp);
        } else if is_device {
            let current = self
                .param_values
                .get(&name)
                .cloned()
                .unwrap_or_default();
            let filtered: Vec<LiveDevice> = self
                .live_devices
                .iter()
                .filter(|d| kind_allowed(&filter_kind, &d.kind))
                .cloned()
                .collect();
            let selected_label = filtered
                .iter()
                .find(|d| d.device_id.to_string() == current)
                .map(|d| d.label())
                .unwrap_or_else(|| {
                    if current.trim().is_empty() {
                        tr(lang, "test_tool.device_auto")
                    } else {
                        current.clone()
                    }
                });
            let mut picked = current.clone();
            let resp = place_in_rect(
                ui,
                field_rect,
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ComboBox::from_id_salt(("hub_device", &name))
                        .width(field_rect.width())
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut picked,
                                String::new(),
                                tr(lang, "test_tool.device_auto"),
                            );
                            if filtered.is_empty() {
                                ui.label(
                                    RichText::new(tr(lang, "test_tool.device_none"))
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                );
                            }
                            for d in &filtered {
                                ui.selectable_value(
                                    &mut picked,
                                    d.device_id.to_string(),
                                    d.label(),
                                );
                            }
                        })
                        .response
                },
            );
            if picked != current {
                self.apply_device_selection(&name, &picked);
            }
            hover_resp = Some(resp);
        } else if is_enum && !options.is_empty() {
            let current = self
                .param_values
                .get(&name)
                .cloned()
                .unwrap_or_default();
            let selected_label = options
                .iter()
                .find(|(v, _, _)| v == &current)
                .map(|(v, en, zh)| {
                    if matches!(lang, Lang::Zh) && !zh.is_empty() {
                        zh.clone()
                    } else if !en.is_empty() {
                        en.clone()
                    } else {
                        v.clone()
                    }
                })
                .unwrap_or_else(|| current.clone());
            let mut picked = current.clone();
            let resp = place_in_rect(
                ui,
                field_rect,
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ComboBox::from_id_salt(("hub_enum", &name))
                        .width(field_rect.width())
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (value, en, zh) in &options {
                                let lab = if matches!(lang, Lang::Zh) && !zh.is_empty() {
                                    zh.as_str()
                                } else if !en.is_empty() {
                                    en.as_str()
                                } else {
                                    value.as_str()
                                };
                                ui.selectable_value(&mut picked, value.clone(), lab);
                            }
                        })
                        .response
                },
            );
            if picked != current {
                self.param_values.insert(name.clone(), picked);
                self.param_touched.insert(name.clone());
            }
            hover_resp = Some(resp);
        } else if is_serial {
            let current = self
                .param_values
                .get(&name)
                .cloned()
                .unwrap_or_default();
            let mut ports = self.serial_ports.clone();
            if !current.trim().is_empty() && !ports.iter().any(|p| p == &current) {
                ports.insert(0, current.clone());
            }
            let mut picked = current.clone();
            let selected_text = if current.trim().is_empty() {
                tr(lang, "test_tool.serial_none")
            } else {
                current.clone()
            };
            let resp = place_in_rect(
                ui,
                field_rect,
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ComboBox::from_id_salt(("hub_serial", &name))
                        .width(field_rect.width())
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            if ports.is_empty() {
                                ui.label(
                                    RichText::new(tr(lang, "test_tool.serial_none"))
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                );
                            }
                            for p in &ports {
                                ui.selectable_value(&mut picked, p.clone(), p);
                            }
                        })
                        .response
                },
            );
            if picked != current {
                self.param_values.insert(name.clone(), picked);
                self.param_touched.insert(name.clone());
            }
            hover_resp = Some(resp);
        } else {
            let entry = self.param_values.entry(name.clone()).or_default();
            let resp = hub_text_edit(ui, field_rect, entry, &hint);
            if resp.changed() {
                self.param_touched.insert(name.clone());
            }
            hover_resp = Some(resp);
        }
        if let Some(resp) = hover_resp {
            if !help.is_empty() {
                resp.on_hover_text(help);
            }
        }
        if is_path {
            let clicked = place_in_rect(
                ui,
                btn_rect,
                egui::Layout::top_down(egui::Align::Center),
                |ui| ui_theme::secondary_btn_sized(ui, tokens, "…", btn_rect.size()).clicked(),
            );
            if clicked {
                *pending_param = Some(name);
            }
        }
    }

    fn device_filter_kind(&self, p: &PluginParam) -> String {
        if !p.filter_from.is_empty() {
            if let Some(v) = self.param_values.get(&p.filter_from) {
                if !v.trim().is_empty() {
                    return v.clone();
                }
            }
        }
        p.filter_kind.clone()
    }

    fn apply_device_selection(&mut self, param_name: &str, device_id: &str) {
        self.param_values
            .insert(param_name.to_owned(), device_id.to_owned());
        self.param_touched.insert(param_name.to_owned());
        if device_id.trim().is_empty() {
            return;
        }
        let Some(dev) = self
            .live_devices
            .iter()
            .find(|d| d.device_id.to_string() == device_id)
            .cloned()
        else {
            return;
        };
        let fills = self
            .selected_plugin()
            .and_then(|p| p.params.iter().find(|x| x.name == param_name))
            .map(|x| x.fills.clone())
            .unwrap_or_default();
        for (target, field) in fills {
            let val = dev.field(&field);
            if val.is_empty() {
                continue;
            }
            self.param_values.insert(target.clone(), val);
            self.param_touched.insert(target);
        }
    }

    fn refresh_serial_ports_if_needed(&mut self) {
        let stale = self
            .serial_ports_at
            .map(|t| t.elapsed() > Duration::from_secs(3))
            .unwrap_or(true);
        if !stale {
            return;
        }
        self.serial_ports_at = Some(Instant::now());
        self.serial_ports = wiparse_core::serial::list_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.device)
            .collect();
    }

    fn apply_suggested_params(&mut self, obj: &serde_json::Value) {
        let Some(map) = obj.get("suggested_params").and_then(|x| x.as_object()) else {
            return;
        };
        if map.is_empty() {
            return;
        }
        let policy = obj
            .get("suggested_params_policy")
            .and_then(|x| x.as_str())
            .unwrap_or("untouched");
        let known: HashSet<String> = self
            .selected_plugin()
            .map(|p| p.params.iter().map(|x| x.name.clone()).collect())
            .unwrap_or_default();
        let mut filled = Vec::new();
        for (k, v) in map {
            if !known.contains(k) {
                continue;
            }
            let s = json_to_string(Some(v));
            if s.trim().is_empty() {
                continue;
            }
            let cur = self
                .param_values
                .get(k)
                .map(|x| x.trim().to_string())
                .unwrap_or_default();
            let apply = match policy {
                "always" => true,
                "empty" => cur.is_empty(),
                _ => !self.param_touched.contains(k),
            };
            if apply && cur != s {
                self.param_values.insert(k.clone(), s.clone());
                filled.push(format!("{k}={s}"));
            }
        }
        if !filled.is_empty() {
            self.append_log(
                LogKind::System,
                &format!("[hub] auto-fill {}\n", filled.join("  ")),
            );
        }
    }

    fn measure_param_label_col(
        &self,
        ui: &egui::Ui,
        lang: Lang,
        sel_idx: usize,
        idxs: &[usize],
    ) -> f32 {
        let font = FontId::proportional(ui_theme::FONT_CAPTION);
        let mut w = LABEL_COL_MIN;
        let Some(plugin) = self.plugins.get(sel_idx) else {
            return w;
        };
        for &i in idxs {
            let Some(p) = plugin.params.get(i) else {
                continue;
            };
            let label = if matches!(lang, Lang::Zh) && !p.label_zh.is_empty() {
                p.label_zh.as_str()
            } else if !p.label.is_empty() {
                p.label.as_str()
            } else {
                p.name.as_str()
            };
            let gw = ui.fonts(|f| {
                f.layout_no_wrap(label.to_owned(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            });
            w = w.max(gw);
        }
        (w + 4.0).clamp(LABEL_COL_MIN, LABEL_COL_MAX)
    }

    /// Compact status pill in the header (theme-aware; replaces the giant Idle bar).
    fn paint_status_chip(&self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let step = if self.loop_hud.step.is_empty() {
            if self.job.is_some() {
                "armed"
            } else {
                "idle"
            }
        } else {
            self.loop_hud.step.as_str()
        };
        let (dot, label) = match step {
            "wait" => (tokens.success, tr(lang, "test_tool.hud_wait")),
            "processing" => (tokens.warning, tr(lang, "test_tool.hud_processing")),
            "captured" => (tokens.accent, tr(lang, "test_tool.hud_captured")),
            "stopped" => (tokens.stop_bg, tr(lang, "test_tool.hud_stopped")),
            "armed" => (tokens.text_muted, tr(lang, "test_tool.hud_armed")),
            _ => (tokens.text_muted, tr(lang, "test_tool.hud_idle")),
        };
        Frame::NONE
            .fill(tokens.input_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 3.5, dot);
                    ui.label(
                        RichText::new(label)
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_primary),
                    );
                });
            });
    }

    fn paint_loop_hud(&self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let avail_w = ui.available_width();
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(avail_w, HUD_H), egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }

        let step = if self.loop_hud.step.is_empty() {
            if self.job.is_some() {
                "armed"
            } else {
                "idle"
            }
        } else {
            self.loop_hud.step.as_str()
        };

        let strip_c = match step {
            "wait" => tokens.success,
            "processing" => tokens.warning,
            "captured" => tokens.accent,
            "stopped" => tokens.stop_bg,
            "armed" => tokens.text_muted,
            _ => tokens.divider,
        };
        let bg = tokens.input_bg;
        let fg = tokens.text_primary;
        let muted = tokens.text_muted;

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(5), bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(5),
            Stroke::new(1.0_f32, tokens.divider),
            egui::StrokeKind::Inside,
        );
        let strip = egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height()));
        painter.rect_filled(
            strip,
            CornerRadius {
                nw: 5,
                ne: 0,
                sw: 5,
                se: 0,
            },
            strip_c,
        );

        let state = match step {
            "wait" => tr(lang, "test_tool.hud_wait"),
            "processing" => tr(lang, "test_tool.hud_processing"),
            "captured" => tr(lang, "test_tool.hud_captured"),
            "stopped" => tr(lang, "test_tool.hud_stopped"),
            "armed" => tr(lang, "test_tool.hud_armed"),
            _ => tr(lang, "test_tool.hud_idle"),
        };

        let mut meta = String::new();
        match step {
            "wait" => {
                if let Some(c) = self.loop_hud.cycle {
                    meta.push_str(&format!("#{c}"));
                }
                let trig = self.loop_hud.trigger.trim();
                if !trig.is_empty() {
                    if !meta.is_empty() {
                        meta.push_str("  ");
                    }
                    meta.push_str(trig);
                }
            }
            "processing" => {
                if let Some(c) = self.loop_hud.cycle {
                    meta.push_str(&format!("#{c}"));
                }
                let who = self.loop_hud.trigger.trim();
                if !who.is_empty() {
                    if !meta.is_empty() {
                        meta.push_str("  ");
                    }
                    meta.push_str(who);
                }
            }
            "captured" => {
                if !self.loop_hud.filename.is_empty() {
                    meta = self.loop_hud.filename.clone();
                } else if matches!(lang, Lang::Zh) {
                    meta = "下一轮".into();
                } else {
                    meta = "next".into();
                }
            }
            "stopped" => {}
            _ => {
                meta = self.loop_hud.hint.clone();
            }
        }
        if meta.chars().count() > 72 {
            meta = meta.chars().take(69).collect::<String>() + "…";
        }

        let elapsed = match (step, self.loop_hud.elapsed_s) {
            ("wait", Some(s)) => {
                let s = s.min(u64::from(u32::MAX));
                format!("{:01}:{:02}", s / 60, s % 60)
            }
            _ => String::new(),
        };

        let inner = rect.shrink2(egui::vec2(10.0, 0.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(false)),
            |ui| {
                ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                ui.set_min_height(inner.height());
                ui.set_min_width(inner.width());
                ui.label(
                    RichText::new(state)
                        .size(ui_theme::FONT_CAPTION)
                        .strong()
                        .color(fg),
                );
                ui.add_space(8.0);
                let time_w = if elapsed.is_empty() { 0.0 } else { 44.0 };
                let meta_w = (ui.available_width() - time_w - 4.0).max(40.0);
                ui.add_sized(
                    egui::vec2(meta_w, inner.height()),
                    egui::Label::new(RichText::new(meta).size(ui_theme::FONT_CAPTION).color(muted))
                        .truncate(),
                );
                if !elapsed.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(elapsed)
                                .monospace()
                                .size(ui_theme::FONT_CAPTION)
                                .color(muted),
                        );
                    });
                }
            },
        );
    }

    fn apply_hud_to_status_bar(&mut self, lang: Lang) {
        let step = if self.loop_hud.step.is_empty() {
            return;
        } else {
            self.loop_hud.step.as_str()
        };
        let label = match step {
            "wait" => tr(lang, "test_tool.hud_wait"),
            "processing" => tr(lang, "test_tool.hud_processing"),
            "captured" => tr(lang, "test_tool.hud_captured"),
            "stopped" => tr(lang, "test_tool.hud_stopped"),
            "armed" => tr(lang, "test_tool.hud_armed"),
            "idle" => tr(lang, "test_tool.hud_idle"),
            other => other.to_owned(),
        };
        self.status = if let Some(c) = self.loop_hud.cycle {
            format!("{label} #{c}")
        } else {
            label
        };
    }

    fn selected_plugin(&self) -> Option<&PluginInfo> {
        let id = self.selected.as_deref()?;
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Instant selection: fill param defaults from plugin.json only (no disk I/O).
    fn select_plugin(&mut self, id: String) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        self.selected = Some(id);
        self.apply_param_defaults_fast();
        self.pending_station_merge = true;
    }

    fn apply_param_defaults_fast(&mut self) {
        self.param_values.clear();
        self.param_touched.clear();
        let defaults: Vec<(String, String)> = self
            .selected_plugin()
            .map(|p| {
                p.params
                    .iter()
                    .map(|param| (param.name.clone(), param.default.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (name, default) in defaults {
            self.param_values.insert(name, default);
        }
        if let Some(v) = self.param_values.get("preflight_only") {
            self.preflight_only = is_truthy(v);
        }
    }

    /// Overlay station.json values onto param_values (cached by mtime).
    fn merge_station_into_params(&mut self) {
        let Some(sel) = self.selected.clone() else {
            return;
        };
        let Some(idx) = self.plugins.iter().position(|p| p.id == sel) else {
            return;
        };
        let station = self.station_value_cached(idx);
        let overlays: Vec<(String, String)> = self.plugins[idx]
            .params
            .iter()
            .filter(|p| !p.path.is_empty())
            .filter_map(|p| station_get(&station, &p.path).map(|v| (p.name.clone(), v)))
            .collect();
        for (name, val) in overlays {
            self.param_values.insert(name, val);
        }
        if let Some(v) = self.param_values.get("preflight_only") {
            self.preflight_only = is_truthy(v);
        }
    }

    fn station_value_cached(&mut self, idx: usize) -> serde_json::Value {
        let plugin = &self.plugins[idx];
        let id = plugin.id.clone();
        let path = plugin.dir.join(&plugin.config);
        let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        let need_reload = match self.station_cache.get(&id) {
            Some((cached_mt, _)) => *cached_mt != mtime,
            None => true,
        };
        if need_reload {
            let value = fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(serde_json::Value::Null);
            self.station_cache.insert(id.clone(), (mtime, value));
        }
        self.station_cache
            .get(&id)
            .map(|(_, v)| v.clone())
            .unwrap_or(serde_json::Value::Null)
    }

    fn load_param_defaults_for_selection(&mut self) {
        self.apply_param_defaults_fast();
        self.merge_station_into_params();
    }

    fn refresh_plugins(&mut self) {
        let mut by_id: HashMap<String, PluginInfo> = HashMap::new();
        for p in scan_plugins(Path::new(self.plugins_dir.trim())) {
            by_id.insert(p.id.clone(), p);
        }
        let mkt_root = self.resolve_marketplace_root();
        for p in scan_marketplace_active_plugins(&mkt_root) {
            by_id.insert(p.id.clone(), p);
        }
        let mut plugins: Vec<PluginInfo> = by_id.into_values().collect();
        plugins.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
        self.plugins = plugins;
        self.station_cache
            .retain(|id, _| self.plugins.iter().any(|p| p.id == *id));
        if let Some(sel) = &self.selected {
            if !self.plugins.iter().any(|p| p.id == *sel) {
                self.selected = None;
            }
        }
        if self.selected.is_none() {
            self.selected = self.plugins.first().map(|p| p.id.clone());
        }
        self.load_param_defaults_for_selection();
        self.pending_station_merge = false;
    }

    fn resolve_marketplace_root(&self) -> PathBuf {
        if !self.marketplace_install_dir.trim().is_empty() {
            return resolve_dir(self.marketplace_install_dir.trim());
        }
        let data_root = if self.data_root.trim().is_empty() {
            project_path("")
        } else {
            resolve_dir(self.data_root.trim())
        };
        wiparse_core::marketplace::default_marketplace_root(&data_root)
    }

    fn run_path_pick(&mut self, target: PathPickTarget) {
        match target {
            PathPickTarget::PluginsDir => {
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
            PathPickTarget::Param(name) => {
                let start = self
                    .param_values
                    .get(&name)
                    .map(|s| resolve_dir(s.trim()))
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| project_path(""));
                let mut dialog = rfd::FileDialog::new().set_title("Select folder");
                if start.is_dir() {
                    dialog = dialog.set_directory(&start);
                }
                let Some(path) = dialog.pick_folder() else {
                    return;
                };
                if !path.is_dir() {
                    return;
                }
                self.param_values
                    .insert(name.clone(), path.display().to_string());
                self.param_touched.insert(name);
            }
        }
    }

    fn persist_config(&self) {
        if let Ok(mut cfg) = wiparse_core::config::load_config() {
            cfg.apps.test_tool.plugins_dir = self.plugins_dir.clone();
            cfg.apps.test_tool.cli_path = self.cli_path.clone();
            cfg.apps.test_tool.node_path = self.node_path.clone();
            cfg.apps.test_tool.data_root = self.data_root.clone();
            cfg.apps.test_tool.marketplace.install_dir = self.marketplace_install_dir.clone();
            cfg.apps.test_tool.marketplace.enabled = self.marketplace_enabled;
            cfg.apps.test_tool.marketplace.base_url = self.marketplace_base_url.clone();
            cfg.apps.test_tool.marketplace.channel = self.marketplace_channel.clone();
            let _ = wiparse_core::config::save_config(&cfg);
        }
    }

    fn data_root_resolved(&self) -> String {
        if self.data_root.trim().is_empty() {
            project_path("").display().to_string()
        } else {
            self.data_root.trim().to_owned()
        }
    }

    fn refresh_catalog(&mut self, lang: Lang) {
        if self.catalog_busy {
            return;
        }
        let url = self.marketplace_base_url.trim().to_owned();
        if url.is_empty() {
            self.catalog_status = tr(lang, "test_tool.market_no_url").to_string();
            return;
        }
        if !self.marketplace_enabled {
            self.marketplace_enabled = true;
            self.persist_config();
        }
        self.catalog_busy = true;
        self.catalog_status = tr(lang, "test_tool.market_loading").to_string();
        let installed =
            installed_map_from_registry(&self.resolve_marketplace_root());
        self.market_job = Some(spawn_catalog_refresh(
            url,
            self.marketplace_channel.clone(),
            installed,
        ));
    }

    fn install_selected_catalog_plugin(&mut self, lang: Lang) {
        let Some(id) = self.catalog_selected.clone() else {
            return;
        };
        let Some(row) = self.catalog.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        let version = self
            .catalog_selected_version
            .clone()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| row.versions.first().cloned())
            .unwrap_or_else(|| row.latest_version.clone());
        let url = self.marketplace_base_url.trim().to_owned();
        if url.is_empty() {
            self.catalog_status = tr(lang, "test_tool.market_no_url").to_string();
            return;
        }
        let node = self.node_path.trim().to_owned();
        let cli = marketplace_cli_path();
        if !cli.is_file() {
            self.catalog_status = format!("{} {}", tr(lang, "test_tool.market_missing_cli"), cli.display());
            return;
        }
        self.catalog_busy = true;
        self.catalog_status = tr(lang, "test_tool.market_installing")
            .replace("{id}", &id)
            .replace("{version}", &version);
        self.append_log(
            LogKind::System,
            &format!("—— marketplace install {id}@{version} ——\n"),
        );
        self.market_job = Some(spawn_pull_install(
            node,
            cli,
            url,
            id,
            version,
            self.data_root_resolved(),
            self.resolve_marketplace_root().display().to_string(),
        ));
    }

    fn uninstall_selected_catalog_plugin(&mut self, lang: Lang) {
        let Some(id) = self.catalog_selected.clone() else {
            return;
        };
        let Some(row) = self.catalog.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        let Some(version) = row.installed_version.clone() else {
            self.catalog_status = tr(lang, "test_tool.market_not_installed").to_string();
            return;
        };
        let node = self.node_path.trim().to_owned();
        let cli = marketplace_cli_path();
        self.catalog_busy = true;
        self.catalog_status = tr(lang, "test_tool.market_uninstalling")
            .replace("{id}", &id)
            .replace("{version}", &version);
        self.market_job = Some(spawn_uninstall(
            node,
            cli,
            id,
            version,
            self.data_root_resolved(),
            self.resolve_marketplace_root().display().to_string(),
        ));
    }

    fn poll_market_job(&mut self, lang: Lang) {
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(job) = self.market_job.as_ref() {
            loop {
                match job.rx.try_recv() {
                    Ok(ev) => events.push(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        } else {
            return;
        }

        let mut done = disconnected;
        for ev in events {
            match ev {
                MarketJobEvent::Log(t) => {
                    self.append_log(LogKind::System, &t);
                }
                MarketJobEvent::CatalogOk(rows) => {
                    self.catalog = rows;
                    if self.catalog_selected.is_none() {
                        self.catalog_selected = self.catalog.first().map(|r| r.id.clone());
                    }
                    self.catalog_status = tr(lang, "test_tool.market_catalog_count")
                        .replace("{n}", &self.catalog.len().to_string());
                    // Keep detail version selection coherent with selected row.
                    if let Some(sel) = self.catalog_selected.clone() {
                        if let Some(row) = self.catalog.iter().find(|r| r.id == sel) {
                            if self.catalog_selected_version.as_deref()
                                .map(|v| !row.versions.iter().any(|x| x == v) && v != row.latest_version.as_str())
                                .unwrap_or(true)
                            {
                                self.catalog_selected_version = Some(row.latest_version.clone());
                            }
                        }
                    }
                    self.catalog_busy = false;
                    done = true;
                }
                MarketJobEvent::CatalogErr(e) => {
                    self.catalog_status = e.clone();
                    self.append_log(LogKind::System, &format!("[marketplace] catalog error: {e}\n"));
                    self.catalog_busy = false;
                    done = true;
                }
                MarketJobEvent::InstallOk { id, version, dir } => {
                    self.append_log(
                        LogKind::System,
                        &format!("[marketplace] installed {id}@{version} → {dir}\n"),
                    );
                    self.catalog_status = tr(lang, "test_tool.market_installed_ok")
                        .replace("{id}", &id)
                        .replace("{version}", &version);
                    self.catalog_busy = false;
                    done = true;
                    self.refresh_plugins();
                    self.selected = Some(id.clone());
                    self.load_param_defaults_for_selection();
                    self.hub_mode = HubMode::Plugins;
                    let installed = installed_map_from_registry(&self.resolve_marketplace_root());
                    for row in &mut self.catalog {
                        if let Some((_, v)) = installed.iter().find(|(i, _)| i == &row.id) {
                            row.installed = true;
                            row.installed_version = Some(v.clone());
                        }
                    }
                }
                MarketJobEvent::InstallErr { id, message } => {
                    self.append_log(
                        LogKind::System,
                        &format!("[marketplace] install failed {id}: {message}\n"),
                    );
                    self.catalog_status = message;
                    self.catalog_busy = false;
                    done = true;
                }
                MarketJobEvent::UninstallOk { id, version } => {
                    self.append_log(
                        LogKind::System,
                        &format!("[marketplace] uninstalled {id}@{version}\n"),
                    );
                    self.catalog_status = tr(lang, "test_tool.market_uninstalled_ok")
                        .replace("{id}", &id)
                        .replace("{version}", &version);
                    self.catalog_busy = false;
                    done = true;
                    self.refresh_plugins();
                    for row in &mut self.catalog {
                        if row.id == id {
                            row.installed = false;
                            row.installed_version = None;
                        }
                    }
                }
                MarketJobEvent::UninstallErr { id, message } => {
                    self.append_log(
                        LogKind::System,
                        &format!("[marketplace] uninstall failed {id}: {message}\n"),
                    );
                    self.catalog_status = message;
                    self.catalog_busy = false;
                    done = true;
                }
            }
        }
        if done {
            self.market_job = None;
        }
    }

    fn market_browser_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let card_w = ui.available_width();
        let inner_w = (card_w - f32::from(CARD_MARGIN_X) * 2.0).max(80.0);
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(CARD_MARGIN_X, 10))
            .show(ui, |ui| {
                ui.set_max_width(inner_w);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                let ctrl_w = ui.available_width();

                self.paint_hub_mode_tabs(ui, lang, tokens, ctrl_w);

                let refresh_lbl = tr(lang, "btn.refresh_browser");
                let search_hint = tr(lang, "test_tool.search");
                let (_, refresh) = hub_edit_with_btn(
                    ui,
                    tokens,
                    &mut self.type_filter,
                    search_hint,
                    &refresh_lbl,
                    ctrl_w,
                );
                if refresh && !self.catalog_busy {
                    self.refresh_catalog(lang);
                }

                egui::CollapsingHeader::new(
                    RichText::new(tr(lang, "test_tool.market_connect"))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                )
                .default_open(self.marketplace_base_url.trim().is_empty())
                .show(ui, |ui| {
                    let url_w = ui.available_width().max(80.0);
                    let url_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.marketplace_base_url)
                            .desired_width(url_w)
                            .clip_text(true)
                            .hint_text("http://127.0.0.1:8787")
                            .margin(egui::vec2(6.0, 4.0)),
                    );
                    if url_edit.lost_focus() {
                        self.persist_config();
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(
                            RichText::new(tr(lang, "test_tool.market_channel"))
                                .size(ui_theme::FONT_CAPTION)
                                .color(tokens.text_muted),
                        );
                        let mut ch = if self.marketplace_channel.trim().is_empty() {
                            "stable".to_string()
                        } else {
                            self.marketplace_channel.clone()
                        };
                        let before = ch.clone();
                        let combo_w = ui.available_width().max(80.0);
                        egui::ComboBox::from_id_salt("market_channel")
                            .width(combo_w)
                            .selected_text(ch.clone())
                            .show_ui(ui, |ui| {
                                for c in CHANNELS {
                                    ui.selectable_value(&mut ch, (*c).to_string(), *c);
                                }
                            });
                        if ch != before {
                            self.marketplace_channel = ch;
                            self.persist_config();
                            self.refresh_catalog(lang);
                        }
                    });
                });

                if !self.catalog_status.is_empty() {
                    let err = self.catalog_status.to_ascii_lowercase().contains("fail")
                        || self.catalog_status.contains("错误")
                        || self.catalog_status.contains("失败")
                        || self.catalog_status.contains("missing")
                        || self.catalog_status.contains("E_");
                    let color = if err {
                        tokens.stop_bg
                    } else if self.catalog_busy {
                        tokens.warning
                    } else {
                        tokens.text_muted
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.catalog_status)
                                .size(ui_theme::FONT_CAPTION)
                                .color(color),
                        )
                        .truncate()
                        .selectable(false),
                    );
                }

                let search = self.type_filter.trim().to_ascii_lowercase();
                let rows: Vec<(String, String, String, bool, bool)> = self
                    .catalog
                    .iter()
                    .filter(|row| {
                        if search.is_empty() {
                            return true;
                        }
                        row.id.to_ascii_lowercase().contains(&search)
                            || row.name.to_ascii_lowercase().contains(&search)
                            || row.name_zh.to_ascii_lowercase().contains(&search)
                            || row.type_name.to_ascii_lowercase().contains(&search)
                    })
                    .map(|row| {
                        let title = if matches!(lang, Lang::Zh) && !row.name_zh.is_empty() {
                            row.name_zh.clone()
                        } else {
                            row.name.clone()
                        };
                        (
                            row.id.clone(),
                            title,
                            format!("v{} · {}", row.latest_version, row.type_name),
                            row.installed,
                            row.has_update(),
                        )
                    })
                    .collect();
                let selected = self.catalog_selected.clone();

                egui::ScrollArea::vertical()
                    .id_salt("test_tool_market")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ctrl_w);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        if rows.is_empty() {
                            ui.label(
                                RichText::new(tr(lang, "test_tool.market_empty"))
                                    .size(ui_theme::FONT_CAPTION)
                                    .color(tokens.text_muted),
                            );
                            return;
                        }
                        let mut clicked: Option<String> = None;
                        for (id, title, type_ver, installed, update) in &rows {
                            let is_sel = selected.as_deref() == Some(id.as_str());
                            let badge_owned;
                            let badge = if *update {
                                badge_owned = tr(lang, "test_tool.market_update_badge");
                                Some((badge_owned.as_str(), tokens.warning))
                            } else if *installed {
                                badge_owned = tr(lang, "test_tool.market_installed");
                                Some((badge_owned.as_str(), tokens.success))
                            } else {
                                badge_owned = tr(lang, "test_tool.market_available");
                                Some((badge_owned.as_str(), tokens.text_muted))
                            };
                            let resp = ui.push_id(id, |ui| {
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(ctrl_w, PLUGIN_ROW_H),
                                    egui::Sense::click(),
                                );
                                if ui.is_rect_visible(rect) {
                                    paint_plugin_row(
                                        ui,
                                        tokens,
                                        rect,
                                        resp.hovered(),
                                        is_sel,
                                        title,
                                        type_ver,
                                        badge,
                                    );
                                }
                                resp
                            })
                            .inner;
                            if resp.clicked() {
                                clicked = Some(id.clone());
                            }
                        }
                        if let Some(id) = clicked {
                            if let Some(row) = self.catalog.iter().find(|r| r.id == id) {
                                self.catalog_selected_version = Some(row.latest_version.clone());
                            }
                            self.catalog_selected = Some(id);
                        }
                    });
            });
    }

    fn market_detail_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);

                let Some(id) = self.catalog_selected.clone() else {
                    ui.label(
                        RichText::new(tr(lang, "test_tool.market_select_hint"))
                            .size(ui_theme::FONT_CAPTION)
                            .color(tokens.text_muted),
                    );
                    return;
                };
                let Some(row) = self.catalog.iter().find(|r| r.id == id).cloned() else {
                    ui.label(tr(lang, "test_tool.market_empty"));
                    return;
                };
                let title = if matches!(lang, Lang::Zh) && !row.name_zh.is_empty() {
                    row.name_zh.clone()
                } else {
                    row.name.clone()
                };
                let update_available = row.has_update();
                let busy = self.catalog_busy;

                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&title)
                                .size(ui_theme::FONT_TITLE)
                                .strong()
                                .color(tokens.text_primary),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    let (pill, color) = if update_available {
                        (tr(lang, "test_tool.market_update_badge"), tokens.warning)
                    } else if row.installed {
                        (tr(lang, "test_tool.market_installed"), tokens.success)
                    } else {
                        (tr(lang, "test_tool.market_available"), tokens.text_muted)
                    };
                    ui.label(
                        RichText::new(pill)
                            .size(ui_theme::FONT_CAPTION)
                            .color(color),
                    );
                });
                ui.label(
                    RichText::new(format!("v{} · {}", row.latest_version, row.id))
                        .size(ui_theme::FONT_CAPTION)
                        .color(tokens.text_muted),
                );

                let body_h = ui.available_height().max(120.0);
                let body_w = ui.available_width();
                let gap = RUNNER_SPLIT_GAP * 2.0;
                let cfg_w = if body_w < CONFIG_MIN_W + OUTPUT_MIN_W + gap {
                    (body_w * 0.48).max(160.0)
                } else {
                    (body_w * CONFIG_FRAC).clamp(CONFIG_MIN_W, body_w - OUTPUT_MIN_W - gap)
                };

                ui.horizontal(|ui| {
                    ui.set_min_height(body_h);
                    ui.allocate_ui_with_layout(
                        egui::vec2(cfg_w, body_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(body_h);
                            ui.set_max_width(cfg_w);
                            ui.spacing_mut().item_spacing.y = 8.0;
                            if !row.publisher.is_empty() {
                                ui.label(
                                    RichText::new(&row.publisher)
                                        .size(ui_theme::FONT_CAPTION)
                                        .color(tokens.text_muted),
                                );
                            }
                            if !row.description.is_empty() {
                                ui.label(
                                    RichText::new(&row.description)
                                        .size(ui_theme::FONT_BODY)
                                        .color(tokens.text_primary),
                                );
                            }
                            ui.add_space(4.0);

                            let primary = if update_available {
                                tr(lang, "test_tool.market_update")
                                    .replace("{version}", &row.latest_version)
                            } else if row.installed {
                                tr(lang, "test_tool.market_open_plugins")
                            } else {
                                tr(lang, "test_tool.market_install")
                            };
                            let primary_w = text_btn_w(ui, &primary).max(72.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                if ui_theme::primary_btn_sized_enabled(
                                    ui,
                                    tokens,
                                    primary,
                                    egui::vec2(primary_w, ui_theme::CTRL_H),
                                    !busy || (row.installed && !update_available),
                                )
                                .clicked()
                                {
                                    if row.installed && !update_available {
                                        self.selected = Some(row.id.clone());
                                        self.load_param_defaults_for_selection();
                                        self.hub_mode = HubMode::Plugins;
                                    } else if !busy {
                                        self.catalog_selected_version =
                                            Some(row.latest_version.clone());
                                        self.install_selected_catalog_plugin(lang);
                                    }
                                }
                                if row.installed
                                    && ui_theme::ghost_btn_sized_enabled(
                                        ui,
                                        tokens,
                                        tr(lang, "test_tool.market_uninstall"),
                                        egui::vec2(64.0, ui_theme::CTRL_H),
                                        !busy,
                                    )
                                    .clicked()
                                    && !busy
                                {
                                    self.uninstall_selected_catalog_plugin(lang);
                                }
                            });
                        },
                    );
                    ui.add_space(RUNNER_SPLIT_GAP);
                    let div_x = ui.cursor().left();
                    let div_y = ui.max_rect().y_range();
                    ui.painter()
                        .vline(div_x, div_y, Stroke::new(1.0_f32, tokens.divider));
                    ui.add_space(RUNNER_SPLIT_GAP + 1.0);
                    let out_avail = ui.available_width().max(120.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(out_avail, body_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(body_h);
                            ui.set_min_width(out_avail);
                            self.paint_runner_output(ui, lang, tokens);
                        },
                    );
                });
            });
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
        let mut stop = station_get(&station, "paths.stop_file").map(PathBuf::from);
        let mut status = station_get(&station, "paths.status_file").map(PathBuf::from);

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

        // Keep status HUD beside ISF dir when the form overrides isf_dir (or always
        // prefer isf_dir/_loop_status.json after expansion — matches plugin-contract).
        let isf_raw = self
            .param_values
            .get("isf_dir")
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .or_else(|| station_get(&station, "paths.isf_dir"));
        if let Some(isf) = isf_raw {
            let isf_path = expand(&isf);
            status = Some(isf_path.join("_loop_status.json"));
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
        args.push("--plugins-root".into());
        args.push(self.plugins_dir.trim().to_owned());
        args.push("--marketplace-dir".into());
        args.push(self.resolve_marketplace_root().display().to_string());

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
        self.loop_hud = LoopHud::default();

        let mut child = Command::new(&node);
        child
            .args(&args)
            .current_dir(project_path("test-tools"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        hide_console_window(&mut child);
        let mut child = child
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
            kill_after: None,
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
            // Honor graceful-stop deadline without releasing `job` early (keeps Run busy).
            if let Some(deadline) = job.kill_after {
                if Instant::now() >= deadline {
                    let _ = job.child.kill();
                    job.kill_after = None;
                }
            }
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
            if let Some(obj) = extract_json_value(&line.text) {
                self.apply_suggested_params(&obj);
            }
            self.append_log(line.kind, &line.text);
        }

        if let Some(text) = status_snapshot {
            // While graceful-stopping, keep the Stopped chip; still drain logs.
            let stopping = self
                .job
                .as_ref()
                .and_then(|j| j.kill_after)
                .is_some();
            if !stopping {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let step = v
                    .get("step")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                let hint = v
                    .get("hint")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                let cycle = v.get("cycle").and_then(|x| x.as_u64());
                let elapsed_s = v
                    .get("elapsed_s")
                    .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)));
                let filename = v
                    .get("filename")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                let mut trigger = v
                    .get("trigger")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                if trigger.is_empty() {
                    if let Some(arr) = v.get("triggers").and_then(|x| x.as_array()) {
                        let labels: Vec<String> = arr
                            .iter()
                            .filter_map(|t| {
                                if let Some(s) = t.as_str() {
                                    Some(s.to_owned())
                                } else if let Some(l) = t.get("label").and_then(|x| x.as_str()) {
                                    Some(l.to_owned())
                                } else {
                                    t.get("id").and_then(|x| x.as_str()).map(|s| s.to_owned())
                                }
                            })
                            .take(4)
                            .collect();
                        trigger = labels.join("  ");
                    }
                }
                if !hint.is_empty() {
                    self.loop_hint = if let Some(c) = cycle {
                        format!("[{step} #{c}] {hint}")
                    } else {
                        format!("[{step}] {hint}")
                    };
                }
                self.loop_hud = LoopHud {
                    step,
                    hint,
                    cycle,
                    trigger,
                    elapsed_s,
                    filename,
                };
                self.apply_hud_to_status_bar(lang);
            }
            }
        }

        if let Some(e) = wait_err {
            self.append_log(LogKind::System, &format!("wait error: {e}\n"));
            self.job = None;
            self.loop_hud = LoopHud {
                step: "stopped".into(),
                ..LoopHud::default()
            };
            self.status = tr(lang, "test_tool.status_fail");
            return;
        }

        if let Some((id, status)) = finished {
            self.job = None;
            let code = status.code().unwrap_or(-1);
            if status.success() {
                self.append_log(LogKind::System, &format!("—— done {id} (exit 0) ——\n"));
                self.status = format!("{} · {}", tr(lang, "test_tool.status_ok"), id);
                // Preflight / short runs never enter a capture cycle — do not show "captured".
                if matches!(self.loop_hud.step.as_str(), "" | "armed") {
                    self.loop_hud = LoopHud {
                        step: "idle".into(),
                        ..LoopHud::default()
                    };
                }
            } else {
                self.append_log(
                    LogKind::System,
                    &format!("—— failed {id} (exit {code}) ——\n"),
                );
                self.status =
                    format!("{} · {} ({code})", tr(lang, "test_tool.status_fail"), id);
                if matches!(self.loop_hud.step.as_str(), "" | "armed" | "idle") {
                    self.loop_hud = LoopHud {
                        step: "stopped".into(),
                        ..LoopHud::default()
                    };
                }
            }
        }
    }

    fn stop_job(&mut self) {
        let marketplace_dir = self.resolve_marketplace_root().display().to_string();
        let plugins_dir = self.plugins_dir.trim().to_owned();
        let node = self.node_path.trim().to_owned();
        let data_root = if self.data_root.trim().is_empty() {
            project_path("").display().to_string()
        } else {
            self.data_root.trim().to_owned()
        };
        let Some(job) = self.job.as_mut() else {
            return;
        };
        if job.kill_after.is_some() {
            // Stop already requested; keep waiting / force deadline.
            return;
        }
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
            let runner = project_path("test-tools/runner.mjs");
            if !node.is_empty() && runner.is_file() {
                let mut stop_cmd = Command::new(&node);
                stop_cmd
                    .args([
                        runner.display().to_string(),
                        "--plugin".into(),
                        job.plugin_id.clone(),
                        "--lifecycle".into(),
                        "stop".into(),
                        "--data-root".into(),
                        data_root,
                        "--plugins-root".into(),
                        plugins_dir,
                        "--marketplace-dir".into(),
                        marketplace_dir,
                    ])
                    .current_dir(project_path("test-tools"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .stdin(Stdio::null());
                hide_console_window(&mut stop_cmd);
                let _ = stop_cmd.spawn();
            }
        }
        // Keep `self.job` so Run stays busy and Output keeps draining until exit.
        job.kill_after = Some(Instant::now() + Duration::from_secs(30));
        self.append_log(LogKind::System, "—— stop requested (grace ≤30s) ——\n");
        self.loop_hint.clear();
        self.loop_hud = LoopHud {
            step: "stopped".into(),
            ..LoopHud::default()
        };
    }

    fn append_log(&mut self, kind: LogKind, text: &str) {
        let prefix = match kind {
            LogKind::Stdout => "",
            LogKind::Stderr => "[err] ",
            LogKind::System => "[sys] ",
        };
        for chunk in text.split_inclusive('\n') {
            let formatted = format_hub_log_line(chunk);
            if formatted.is_empty() {
                continue;
            }
            if formatted.starts_with('[')
                || prefix.is_empty()
                || formatted.starts_with("[err]")
                || formatted.starts_with("[sys]")
            {
                self.log.push_str(&formatted);
            } else {
                self.log.push_str(prefix);
                self.log.push_str(&formatted);
            }
        }
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
        // Panel teardown: no more poll frames — force-kill immediately.
        if let Some(mut job) = self.job.take() {
            if let Some(ref stop) = job.stop_file {
                let _ = fs::write(stop, b"host exit\n");
            }
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
    }
}

fn is_path_param(p: &PluginParam) -> bool {
    let t = p.type_name.trim().to_ascii_lowercase();
    matches!(t.as_str(), "path" | "dir" | "directory" | "folder")
        || p.name.ends_with("_dir")
        || p.name.ends_with("_path")
}

fn normalize_kind(s: &str) -> String {
    match s
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "scope" | "osc" | "oscilloscope" => "oscilloscope".into(),
        "psu" | "dcsource" | "dc_source" | "source" | "power" => "dc_source".into(),
        "load" | "electronic_load" | "eload" => "electronic_load".into(),
        "dmm" | "multimeter" | "meter" => "multimeter".into(),
        other => other.to_string(),
    }
}

fn kind_allowed(filter: &str, device_kind: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    let device = normalize_kind(device_kind);
    f.split(|c| c == ',' || c == '|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|part| normalize_kind(part) == device)
}

fn extract_json_value(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    let json_str = if let Some(rest) = t.strip_prefix("[runner] result ") {
        rest.trim()
    } else if t.starts_with('{') {
        t
    } else {
        return None;
    };
    serde_json::from_str(json_str).ok()
}

fn parse_filter_kind(p: &serde_json::Value) -> String {
    p.get("filter")
        .and_then(|f| f.get("kind"))
        .and_then(|x| x.as_str())
        .or_else(|| p.get("filter_kind").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_owned()
}

fn parse_fills(p: &serde_json::Value) -> Vec<(String, String)> {
    let Some(obj) = p.get("fills").and_then(|x| x.as_object()) else {
        return Vec::new();
    };
    obj.iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
        .collect()
}

fn parse_options(p: &serde_json::Value) -> Vec<(String, String, String)> {
    let Some(arr) = p.get("options").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|o| {
            if let Some(s) = o.as_str() {
                return Some((s.to_owned(), s.to_owned(), String::new()));
            }
            let value = o.get("value")?.as_str()?.to_owned();
            let label = o
                .get("label")
                .and_then(|x| x.as_str())
                .unwrap_or(&value)
                .to_owned();
            let label_zh = o
                .get("label_zh")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            Some((value, label, label_zh))
        })
        .collect()
}

fn text_btn_w(ui: &egui::Ui, label: &str) -> f32 {
    let font = FontId::proportional(ui_theme::FONT_TITLE);
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_owned(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    (text_w + 20.0).clamp(BTN_W, 120.0)
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

fn place_in_rect<R>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    layout: egui::Layout,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(layout.with_main_wrap(false)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            add(ui)
        },
    )
    .inner
}

/// Text field clipped to `rect`. `desired_width` + margin must not spill out.
fn hub_text_edit(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &mut String,
    hint: &str,
) -> egui::Response {
    place_in_rect(
        ui,
        rect,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_sized(
                rect.size(),
                egui::TextEdit::singleline(text)
                    .desired_width(rect.width())
                    .clip_text(true)
                    .hint_text(hint),
            )
        },
    )
}

/// One row: text field on the left, trailing button on the right. No wrap/overlap.
fn hub_edit_with_btn(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    text: &mut String,
    hint: impl Into<String>,
    btn_label: &str,
    row_w: f32,
) -> (egui::Response, bool) {
    let btn_w = text_btn_w(ui, btn_label);
    let gap = 8.0;
    let row_h = ui_theme::CTRL_H;
    let (row, _) = ui.allocate_exact_size(egui::vec2(row_w.max(1.0), row_h), egui::Sense::hover());
    let btn_w = btn_w.min((row.width() - 40.0).max(1.0));
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(row.max.x - btn_w, row.min.y),
        egui::vec2(btn_w, row_h),
    );
    let edit_rect = egui::Rect::from_min_max(
        row.min,
        egui::pos2((btn_rect.min.x - gap).max(row.min.x + 32.0), row.max.y),
    );
    let hint = hint.into();
    let edit = hub_text_edit(ui, edit_rect, text, &hint);
    let clicked = place_in_rect(
        ui,
        btn_rect,
        egui::Layout::top_down(egui::Align::Center),
        |ui| ui_theme::ghost_btn_sized(ui, tokens, btn_label, btn_rect.size(), false).clicked(),
    );
    (edit, clicked)
}

fn paint_plugin_row(
    ui: &egui::Ui,
    tokens: &Tokens,
    rect: egui::Rect,
    hovered: bool,
    selected: bool,
    title: &str,
    type_ver: &str,
    badge: Option<(&str, egui::Color32)>,
) {
    let fill = if selected {
        tokens.accent.linear_multiply(0.18)
    } else if hovered {
        tokens.accent.linear_multiply(0.08)
    } else {
        egui::Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    let inner = rect.shrink2(egui::vec2(8.0, 6.0));
    let clip = ui.painter_at(inner);
    let title_c = if selected {
        tokens.accent
    } else {
        tokens.text_primary
    };
    if let Some((text, color)) = badge {
        clip.text(
            egui::pos2(inner.max.x, inner.min.y + 1.0),
            Align2::RIGHT_TOP,
            text,
            FontId::proportional(ui_theme::FONT_CAPTION),
            color,
        );
    }
    clip.text(
        egui::pos2(inner.min.x, inner.min.y + 1.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(ui_theme::FONT_BODY),
        title_c,
    );
    clip.text(
        egui::pos2(inner.min.x, inner.max.y - 1.0),
        Align2::LEFT_BOTTOM,
        type_ver,
        FontId::proportional(ui_theme::FONT_CAPTION),
        tokens.text_muted,
    );
}

fn hub_log_layout_job(log: &str, tokens: &Tokens, wrap_w: f32) -> LayoutJob {
    let font = FontId::monospace(ui_theme::FONT_CAPTION);
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap_w,
            max_rows: usize::MAX,
            break_anywhere: false,
            overflow_character: Some('…'),
        },
        ..LayoutJob::default()
    };
    for line in log.split_inclusive('\n') {
        let color = if line.starts_with("[err]") || line.contains(" FAIL") || line.contains(" fatal")
        {
            tokens.stop_bg
        } else if line.starts_with("[sys]") {
            tokens.text_muted
        } else {
            tokens.text_primary
        };
        job.append(
            line,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..TextFormat::default()
            },
        );
    }
    job
}

fn format_hub_log_line(raw: &str) -> String {
    let had_nl = raw.ends_with('\n');
    let body = raw.trim_end_matches(['\r', '\n']).trim();
    if body.is_empty() {
        return String::new();
    }
    if body.starts_with("HTTP invoke ") {
        return String::new();
    }
    let formatted = if let Some(rest) = body.strip_prefix("[runner] result ") {
        format_runner_result_line(rest)
    } else if body.starts_with('{') {
        format_json_log_line(body).unwrap_or_else(|| body.to_owned())
    } else {
        body.to_owned()
    };
    if formatted.is_empty() {
        return String::new();
    }
    if had_nl {
        format!("{formatted}\n")
    } else {
        formatted
    }
}

fn format_runner_result_line(rest: &str) -> String {
    let trimmed = rest.trim();
    if let Some(pretty) = format_json_log_line(trimmed) {
        if pretty.is_empty() {
            return String::new();
        }
        return format!("[runner] {pretty}");
    }
    format!("[runner] {trimmed}")
}

fn format_json_log_line(s: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let obj = v.as_object()?;
    if obj.get("type").and_then(|x| x.as_str()) == Some("wiparse.plugin_result") {
        return Some(String::new());
    }
    if let Some(checks) = obj.get("checks").and_then(|x| x.as_array()) {
        let ok = obj.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        let step = obj.get("step").and_then(|x| x.as_str()).unwrap_or("preflight");
        let mut out = format!("{step} {}", if ok { "OK" } else { "FAIL" });
        for c in checks {
            let id = c.get("id").and_then(|x| x.as_str()).unwrap_or("check");
            let cok = c.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
            let detail = c.get("detail").and_then(|x| x.as_str()).unwrap_or("");
            let mark = if cok { "OK" } else { "X " };
            if detail.is_empty() {
                out.push_str(&format!("\n  {mark} {id}"));
            } else {
                out.push_str(&format!("\n  {mark} {id}  {detail}"));
            }
        }
        return Some(out);
    }
    if let Some(step) = obj.get("step").and_then(|x| x.as_str()) {
        let mut head = step.to_owned();
        if let Some(c) = obj.get("cycle").and_then(|x| x.as_u64()) {
            head = format!("{step} #{c}");
        }
        if let Some(hint) = obj.get("hint").and_then(|x| x.as_str()).filter(|h| !h.is_empty()) {
            return Some(format!("[{head}] {hint}"));
        }
        if let Some(err) = obj.get("error").and_then(|x| x.as_str()).filter(|e| !e.is_empty()) {
            return Some(format!("[{head}] {err}"));
        }
        if let Some(ids) = obj.get("ids") {
            return Some(format!("[{head}] {ids}"));
        }
        return Some(format!("[{head}]"));
    }
    if let Some(ok) = obj.get("ok").and_then(|x| x.as_bool()) {
        if obj.contains_key("lifecycle") || obj.contains_key("summary_md") {
            let life = obj.get("lifecycle").and_then(|x| x.as_str()).unwrap_or("run");
            let mut out = format!("{life} {}", if ok { "ok" } else { "FAIL" });
            if let Some(err) = obj.get("error").and_then(|x| x.as_str()) {
                out.push_str(&format!("  {err}"));
            }
            if let Some(md) = obj.get("summary_md").and_then(|x| x.as_str()) {
                out.push_str(&format!("  md={md}"));
            }
            return Some(out);
        }
    }
    None
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
        project_path("WiParse-CLI.exe"),
        project_path("wiparse.exe"),
        project_path("bin/WiParse-CLI.exe"),
        project_path("bin/wiparse.exe"),
        project_path("bin/wiparse"),
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
        if let Some(info) = read_plugin_info(&path) {
            out.push(info);
        }
    }
    out.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
    out
}

fn scan_marketplace_active_plugins(marketplace_root: &Path) -> Vec<PluginInfo> {
    let mut out = Vec::new();
    let registry = marketplace_root.join("registry.json");
    let Ok(text) = fs::read_to_string(&registry) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    let Some(plugins) = v.get("plugins").and_then(|x| x.as_object()) else {
        return out;
    };
    for (_id, entry) in plugins {
        let Some(active) = entry.get("active").and_then(|x| x.as_str()) else {
            continue;
        };
        let dir = entry
            .get("versions")
            .and_then(|vers| vers.get(active))
            .and_then(|ver| ver.get("dir"))
            .and_then(|d| d.as_str())
            .map(PathBuf::from);
        let Some(dir) = dir else {
            continue;
        };
        if let Some(mut info) = read_plugin_info(&dir) {
            info.source = "marketplace".into();
            out.push(info);
        }
    }
    out
}

fn read_plugin_info(path: &Path) -> Option<PluginInfo> {
    let manifest = path.join("plugin.json");
    if !manifest.is_file() {
        return None;
    }
    let text = fs::read_to_string(&manifest).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
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
        return None;
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
                        filter_kind: parse_filter_kind(p),
                        filter_from: p
                            .get("filter_from")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_owned(),
                        fills: parse_fills(p),
                        options: parse_options(p),
                        hidden: p.get("hidden").and_then(|x| x.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PluginInfo {
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
        dir: path.to_path_buf(),
        source: "bundled".into(),
        params,
    })
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

/// Prevent a console / PowerShell window flash when spawning Node (or other
/// console-subsystem) children from the GUI. stdout/stderr stay piped to Output.
fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
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

