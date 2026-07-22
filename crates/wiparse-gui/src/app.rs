//! Main shell — Python MonitorWindow: tabs + Settings on one header row.
//!
//! Settings menu mirrors Python `QToolButton#settings_menu_btn` + cascading `QMenu`:
//! root items with ▶ flyouts (Panels / Language / Theme), accent hover, checkmarks.

use crate::calculator::CalculatorPanel;
use crate::instrument_control::InstrumentControlPanel;
use crate::serial_tool::SerialToolPanel;
use crate::theme::{self as ui_theme, Tokens};
use egui::{Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Sense, Stroke, Vec2};
use wiparse_core::config::{save_config, AppConfig};
use wiparse_core::i18n::{parse_lang, tr, Lang};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Serial,
    Calculator,
    Instruments,
}

fn initial_tab(show_serial: bool, show_instruments: bool) -> MainTab {
    if show_serial {
        MainTab::Serial
    } else if show_instruments {
        MainTab::Instruments
    } else {
        MainTab::Calculator
    }
}

/// Open cascading submenu (Python QMenu flyout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSub {
    None,
    Panels,
    Language,
    Theme,
}

pub struct WiParseApp {
    cfg: AppConfig,
    lang: Lang,
    active: MainTab,
    show_serial: bool,
    show_calculator: bool,
    show_instruments: bool,
    calculator: CalculatorPanel,
    serial: SerialToolPanel,
    instruments: InstrumentControlPanel,
    settings_open: bool,
    settings_sub: SettingsSub,
    /// Anchor Y for the open submenu (top of parent row).
    settings_sub_anchor_y: f32,
    settings_btn_rect: Option<Rect>,
    settings_root_rect: Option<Rect>,
    settings_sub_rect: Option<Rect>,
    settings_leave_since: Option<f64>,
    about_open: bool,
    tokens: Tokens,
    theme_applied_light: Option<bool>,
    taskbar_icon_applied: bool,
    taskbar_icon_attempts: u8,
}

impl WiParseApp {
    pub fn new(cfg: AppConfig) -> Self {
        let lang = parse_lang(&cfg.ui.language);
        let show_serial = cfg.ui.panels.serial_tool;
        let show_calculator = cfg.ui.panels.calculator;
        let show_instruments = cfg.ui.panels.instrument_control;
        let active = initial_tab(show_serial, show_instruments);
        let light = cfg.ui.theme == "light";
        Self {
            serial: SerialToolPanel::new(&cfg, lang),
            instruments: {
                let mut panel = InstrumentControlPanel::new(&cfg);
                if cfg.ui.debug_mode {
                    panel.apply_debug_mode(true, lang);
                }
                panel
            },
            cfg,
            lang,
            active,
            show_serial,
            show_calculator,
            show_instruments,
            calculator: CalculatorPanel::new(),
            settings_open: false,
            settings_sub: SettingsSub::None,
            settings_sub_anchor_y: 0.0,
            settings_btn_rect: None,
            settings_root_rect: None,
            settings_sub_rect: None,
            settings_leave_since: None,
            about_open: false,
            tokens: if light {
                Tokens::light()
            } else {
                Tokens::dark()
            },
            theme_applied_light: None,
            taskbar_icon_applied: false,
            taskbar_icon_attempts: 0,
        }
    }

    fn ensure_visible_tab(&mut self) {
        let visible = match self.active {
            MainTab::Serial => self.show_serial,
            MainTab::Calculator => self.show_calculator,
            MainTab::Instruments => self.show_instruments,
        };
        if visible {
            return;
        }
        if self.show_serial {
            self.active = MainTab::Serial;
        } else if self.show_instruments {
            self.active = MainTab::Instruments;
        } else if self.show_calculator {
            self.active = MainTab::Calculator;
        } else {
            self.show_serial = true;
            self.active = MainTab::Serial;
        }
    }

    fn persist_ui_prefs(&mut self) {
        self.cfg.ui.panels.serial_tool = self.show_serial;
        self.cfg.ui.panels.calculator = self.show_calculator;
        self.cfg.ui.panels.instrument_control = self.show_instruments;
        let _ = save_config(&self.cfg);
    }

    fn set_debug_mode(&mut self, enabled: bool) {
        if self.cfg.ui.debug_mode == enabled {
            return;
        }
        self.cfg.ui.debug_mode = enabled;
        self.instruments.apply_debug_mode(enabled, self.lang);
        if enabled {
            self.show_instruments = true;
            self.active = MainTab::Instruments;
        }
        self.persist_ui_prefs();
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings_sub = SettingsSub::None;
        self.settings_leave_since = None;
        self.settings_root_rect = None;
        self.settings_sub_rect = None;
    }

    fn settings_over_any(&self, pos: Pos2) -> bool {
        self.settings_btn_rect.is_some_and(|r| r.contains(pos))
            || self.settings_root_rect.is_some_and(|r| r.contains(pos))
            || self.settings_sub_rect.is_some_and(|r| r.contains(pos))
    }

    /// Root Settings menu — Python `_view_menu` items with flyout arrows.
    fn settings_root_ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        // Compact popup — avoid stretching across empty horizontal space.
        const ROOT_W: f32 = 124.0;
        ui.set_width(ROOT_W);
        ui.set_max_width(ROOT_W);
        ui.set_min_width(ROOT_W);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().item_spacing.x = 0.0;

        let panels_l = tr(self.lang, "menu.panels");
        let lang_l = tr(self.lang, "menu.language");
        let theme_l = tr(self.lang, "menu.theme");

        let panels = menu_cascade_row(
            ui,
            t,
            &panels_l,
            self.settings_sub == SettingsSub::Panels,
            ROOT_W,
        );
        if panels.hovered() || panels.clicked() {
            self.settings_sub = SettingsSub::Panels;
            self.settings_sub_anchor_y = panels.rect.top();
        }

        menu_separator(ui, t, ROOT_W);

        let lang_row = menu_cascade_row(
            ui,
            t,
            &lang_l,
            self.settings_sub == SettingsSub::Language,
            ROOT_W,
        );
        if lang_row.hovered() || lang_row.clicked() {
            self.settings_sub = SettingsSub::Language;
            self.settings_sub_anchor_y = lang_row.rect.top();
        }

        let theme_row = menu_cascade_row(
            ui,
            t,
            &theme_l,
            self.settings_sub == SettingsSub::Theme,
            ROOT_W,
        );
        if theme_row.hovered() || theme_row.clicked() {
            self.settings_sub = SettingsSub::Theme;
            self.settings_sub_anchor_y = theme_row.rect.top();
        }

        menu_separator(ui, t, ROOT_W);

        let debug_l = tr(self.lang, "menu.debug_mode");
        if menu_check_row(ui, t, &debug_l, self.cfg.ui.debug_mode, ROOT_W).clicked() {
            let enabled = !self.cfg.ui.debug_mode;
            self.set_debug_mode(enabled);
            self.close_settings();
        }

        menu_separator(ui, t, ROOT_W);

        let about_l = tr(self.lang, "menu.about");
        if menu_action_row(ui, t, &about_l, ROOT_W).clicked() {
            self.about_open = true;
            self.close_settings();
        }
    }

    fn settings_sub_ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        // Fit longest panel label ("Instrument Control" / "仪表控制") + checkbox.
        const SUB_W: f32 = 174.0;
        ui.set_width(SUB_W);
        ui.set_max_width(SUB_W);
        ui.set_min_width(SUB_W);
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut dirty = false;
        let mut close_after = false;

        match self.settings_sub {
            SettingsSub::Panels => {
                let serial_name = tr(self.lang, "tool.serial_tool.name");
                let calculator_name = tr(self.lang, "tool.calculator.name");
                let instrument_name = tr(self.lang, "tool.instrument_control.name");

                if menu_check_row(ui, t, &serial_name, self.show_serial, SUB_W).clicked() {
                    self.show_serial = !self.show_serial;
                    if self.show_serial {
                        self.active = MainTab::Serial;
                    }
                    dirty = true;
                }
                if menu_check_row(ui, t, &calculator_name, self.show_calculator, SUB_W).clicked() {
                    self.show_calculator = !self.show_calculator;
                    if self.show_calculator {
                        self.active = MainTab::Calculator;
                    }
                    dirty = true;
                }
                if menu_check_row(ui, t, &instrument_name, self.show_instruments, SUB_W).clicked() {
                    self.show_instruments = !self.show_instruments;
                    if self.show_instruments {
                        self.active = MainTab::Instruments;
                    }
                    dirty = true;
                }
                if !self.show_serial && !self.show_calculator && !self.show_instruments {
                    self.show_serial = true;
                    self.active = MainTab::Serial;
                }
            }
            SettingsSub::Language => {
                if menu_check_row(ui, t, "中文", self.lang == Lang::Zh, SUB_W).clicked() {
                    self.lang = Lang::Zh;
                    self.cfg.ui.language = "zh".into();
                    dirty = true;
                    close_after = true;
                }
                if menu_check_row(ui, t, "English", self.lang == Lang::En, SUB_W).clicked() {
                    self.lang = Lang::En;
                    self.cfg.ui.language = "en".into();
                    dirty = true;
                    close_after = true;
                }
            }
            SettingsSub::Theme => {
                let theme_dark = tr(self.lang, "menu.theme_dark");
                let theme_light = tr(self.lang, "menu.theme_light");
                if menu_check_row(ui, t, &theme_dark, self.cfg.ui.theme == "dark", SUB_W).clicked()
                {
                    self.cfg.ui.theme = "dark".into();
                    self.theme_applied_light = None; // force reapply
                    dirty = true;
                    close_after = true;
                }
                if menu_check_row(ui, t, &theme_light, self.cfg.ui.theme == "light", SUB_W)
                    .clicked()
                {
                    self.cfg.ui.theme = "light".into();
                    self.theme_applied_light = None;
                    dirty = true;
                    close_after = true;
                }
            }
            SettingsSub::None => {}
        }

        if dirty {
            self.persist_ui_prefs();
        }
        if close_after {
            self.close_settings();
        }
    }

    fn paint_settings_menus(&mut self, ctx: &egui::Context, t: &Tokens) {
        let btn = self.settings_btn_rect.unwrap_or_else(|| {
            Rect::from_min_size(
                Pos2::new(ui_right_fallback(ctx) - 80.0, 4.0),
                Vec2::new(72.0, 32.0),
            )
        });

        // Root menu flush under Settings button (right-aligned like Python corner menu).
        let root_anchor = Pos2::new(btn.right(), btn.bottom() + 2.0);
        let mut root_rect = None;
        egui::Area::new(egui::Id::new("settings_menu_root"))
            .order(egui::Order::Foreground)
            .fixed_pos(root_anchor)
            .pivot(egui::Align2::RIGHT_TOP)
            .interactable(true)
            .show(ctx, |ui| {
                menu_frame(t).show(ui, |ui| {
                    self.settings_root_ui(ui, t);
                    root_rect = Some(ui.min_rect().expand(4.0));
                });
            });
        self.settings_root_rect = root_rect;

        // Cascading flyout to the LEFT of the root (screen is right-edge).
        self.settings_sub_rect = None;
        if self.settings_sub != SettingsSub::None {
            let root = self.settings_root_rect.unwrap_or(Rect::NOTHING);
            // Bridge gap between root and sub so the pointer can travel without leave-close.
            let sub_anchor = Pos2::new(root.left() + 2.0, self.settings_sub_anchor_y);
            let mut sub_rect = None;
            egui::Area::new(egui::Id::new("settings_menu_sub"))
                .order(egui::Order::Foreground)
                .fixed_pos(sub_anchor)
                .pivot(egui::Align2::RIGHT_TOP)
                .interactable(true)
                .show(ctx, |ui| {
                    menu_frame(t).show(ui, |ui| {
                        self.settings_sub_ui(ui, t);
                        sub_rect = Some(ui.min_rect().expand(4.0));
                    });
                });
            // Expand hit area to include the 2px bridge toward the root.
            if let Some(mut r) = sub_rect {
                if root.is_positive() {
                    r = r.union(Rect::from_min_max(
                        Pos2::new(r.right().min(root.left()), r.top()),
                        Pos2::new(root.left().max(r.right()), r.bottom()),
                    ));
                }
                self.settings_sub_rect = Some(r);
            }
        }

        // Leave-to-close with grace; click outside closes immediately.
        let pointer = ctx
            .pointer_interact_pos()
            .or_else(|| ctx.pointer_hover_pos());
        let now = ctx.input(|i| i.time);
        if let Some(pos) = pointer {
            if self.settings_over_any(pos) {
                self.settings_leave_since = None;
            } else {
                let pressed = ctx.input(|i| i.pointer.any_pressed());
                if pressed {
                    let on_btn = self.settings_btn_rect.is_some_and(|r| r.contains(pos));
                    if !on_btn {
                        self.close_settings();
                    }
                } else {
                    let since = self.settings_leave_since.get_or_insert(now);
                    if now - *since > 0.28 {
                        self.close_settings();
                    } else {
                        ctx.request_repaint();
                    }
                }
            }
        }
    }

    fn paint_about_dialog(&mut self, ctx: &egui::Context, t: &Tokens) {
        let mut open = self.about_open;
        let mut close_requested = false;
        egui::Window::new(tr(self.lang, "about.title"))
            .id(egui::Id::new("about_dialog"))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
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
                ui.set_min_width(280.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("WiParse")
                            .size(22.0)
                            .strong()
                            .color(t.text_primary),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            tr(self.lang, "about.version"),
                            APP_VERSION
                        ))
                        .size(13.0)
                        .color(t.text_muted),
                    );
                    ui.add_space(14.0);
                    if ui.button(tr(self.lang, "about.close")).clicked() {
                        close_requested = true;
                    }
                });
            });
        self.about_open = open && !close_requested;
    }
}

impl eframe::App for WiParseApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.taskbar_icon_applied && self.taskbar_icon_attempts < 10 {
            self.taskbar_icon_attempts += 1;
            self.taskbar_icon_applied = crate::windows_icon::apply_embedded_icon();
            if !self.taskbar_icon_applied {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let serial_visible = self.active == MainTab::Serial && self.show_serial;
        let live_filters = serial_visible && self.serial.active_live_visible();
        if live_filters {
            self.serial.sync_visible_live_filters();
        }
        // Live filters only need refresh when serial UI shows the live tab.
        self.serial.drain_events(live_filters);
        // Instrument workers can deliver large waveform/image payloads. Do not
        // deserialize, rebuild plots, or schedule its live repaint loop while
        // the user is working in another tool.
        if self.active == MainTab::Instruments && self.show_instruments {
            self.instruments.pump(ctx);
        }

        let pump_serial = self.serial.is_monitoring() || self.serial.has_background_io();
        let pump_instruments = self.active == MainTab::Instruments
            && self.show_instruments
            && self.instruments.live_active();
        if pump_serial || pump_instruments {
            let ms = if serial_visible {
                33
            } else if self.active == MainTab::Instruments && self.show_instruments && pump_instruments
            {
                33
            } else {
                500
            };
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        }

        let light = self.cfg.ui.theme == "light";
        if self.theme_applied_light != Some(light) {
            self.tokens = ui_theme::apply_theme(ctx, light);
            self.theme_applied_light = Some(light);
        }
        let t = self.tokens;
        self.ensure_visible_tab();

        egui::TopBottomPanel::top("main_header")
            .frame(
                Frame::NONE
                    .fill(t.header_bg)
                    .inner_margin(Margin::symmetric(10, 0))
                    .stroke(Stroke::new(1.0_f32, t.border)),
            )
            .exact_height(40.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;

                    main_tab(
                        ui,
                        &t,
                        &mut self.active,
                        MainTab::Serial,
                        self.show_serial,
                        &tr(self.lang, "tool.serial_tool.name"),
                    );
                    main_tab(
                        ui,
                        &t,
                        &mut self.active,
                        MainTab::Calculator,
                        self.show_calculator,
                        &tr(self.lang, "tool.calculator.name"),
                    );
                    main_tab(
                        ui,
                        &t,
                        &mut self.active,
                        MainTab::Instruments,
                        self.show_instruments,
                        &tr(self.lang, "tool.instrument_control.name"),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Python QToolButton#settings_menu_btn: transparent, hover tint.
                        let settings_label = tr(self.lang, "menu.settings");
                        let hovered_fill = Color32::from_rgba_unmultiplied(2, 132, 199, 36);
                        let open_fill = Color32::from_rgba_unmultiplied(2, 132, 199, 55);
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(
                                ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        settings_label.clone(),
                                        FontId::proportional(13.0),
                                        t.text_muted,
                                    )
                                    .size()
                                    .x
                                }) + 24.0,
                                28.0,
                            ),
                            Sense::click(),
                        );
                        let fill = if self.settings_open {
                            open_fill
                        } else if resp.hovered() {
                            hovered_fill
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
                        let fg = if resp.hovered() || self.settings_open {
                            t.text_primary
                        } else {
                            t.text_muted
                        };
                        let galley = ui.fonts(|f| {
                            f.layout_no_wrap(settings_label, FontId::proportional(13.0), fg)
                        });
                        let text_pos = Pos2::new(
                            rect.center().x - galley.size().x * 0.5,
                            rect.center().y - galley.size().y * 0.5,
                        );
                        ui.painter().galley(text_pos, galley, fg);
                        self.settings_btn_rect = Some(rect.expand(2.0));
                        if resp.clicked() {
                            if self.settings_open {
                                self.close_settings();
                            } else {
                                self.settings_open = true;
                                self.settings_sub = SettingsSub::None;
                                self.settings_leave_since = None;
                            }
                        }
                    });
                });
            });

        if self.settings_open {
            self.paint_settings_menus(ctx, &t);
        }
        if self.about_open {
            self.paint_about_dialog(ctx, &t);
        }

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .frame(
                Frame::NONE
                    .fill(t.header_bg)
                    .inner_margin(Margin::symmetric(10, 4))
                    .stroke(Stroke::new(1.0_f32, t.border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.active == MainTab::Serial {
                        ui.label(
                            egui::RichText::new(self.serial.status_text())
                                .size(12.0)
                                .color(t.text_muted),
                        );
                    } else if self.active == MainTab::Instruments {
                        ui.label(
                            egui::RichText::new(self.instruments.status_text())
                                .size(12.0)
                                .color(t.text_muted),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(tr(self.lang, "status.ready"))
                                .size(12.0)
                                .color(t.text_muted),
                        );
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t.canvas_bg).inner_margin(Margin::same(8)))
            .show(ctx, |ui| match self.active {
                MainTab::Serial if self.show_serial => {
                    self.serial.ui(ui, self.lang, &t);
                }
                MainTab::Calculator if self.show_calculator => {
                    self.calculator.ui(ui, self.lang, &t);
                }
                MainTab::Instruments if self.show_instruments => {
                    self.instruments.ui(ui, self.lang, &t);
                }
                _ => {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Enable a panel in Settings").color(t.text_muted),
                        );
                    });
                }
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.serial.on_exit();
        self.instruments.on_exit();
    }
}

// ── Settings menu chrome (Python menu_popup_stylesheet parity) ─────────────

fn menu_frame(t: &Tokens) -> Frame {
    Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0_f32, t.border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(0, 4))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(80),
        })
}

fn menu_separator(ui: &mut egui::Ui, t: &Tokens, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 9.0), Sense::hover());
    let y = rect.center().y;
    ui.painter().hline(
        (rect.left() + 8.0)..=(rect.right() - 8.0),
        y,
        Stroke::new(1.0_f32, t.border),
    );
}

fn menu_cascade_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    open: bool,
    width: f32,
) -> egui::Response {
    let h = 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let hot = resp.hovered() || open;
    if hot {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, t.accent);
    }
    let fg = if hot { t.accent_text } else { t.text_primary };
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(13.0), fg));
    let text_pos = Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, fg);

    // Flyout arrow on the trailing edge — matches QMenu cascade.
    let arrow = "▸";
    let ag = ui.fonts(|f| f.layout_no_wrap(arrow.to_string(), FontId::proportional(12.0), fg));
    ui.painter().galley(
        Pos2::new(
            rect.right() - 14.0 - ag.size().x,
            rect.center().y - ag.size().y * 0.5,
        ),
        ag,
        fg,
    );
    resp
}

fn menu_action_row(ui: &mut egui::Ui, t: &Tokens, label: &str, width: f32) -> egui::Response {
    let h = 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, t.accent);
    }
    let fg = if hot { t.accent_text } else { t.text_primary };
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(13.0), fg));
    let text_pos = Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, fg);
    resp
}

fn menu_check_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    label: &str,
    checked: bool,
    width: f32,
) -> egui::Response {
    let h = 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, t.accent);
    }
    let fg = if hot { t.accent_text } else { t.text_primary };

    // Indicator box — Python QMenu::indicator (14px).
    let box_size = 14.0;
    let box_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 14.0, rect.center().y),
        Vec2::splat(box_size),
    );
    let box_fill = if checked { t.accent } else { t.input_bg };
    let box_stroke = if checked {
        Stroke::new(1.0_f32, t.accent)
    } else {
        Stroke::new(1.0_f32, t.border)
    };
    // When row is hot + checked, use white checkbox.
    let box_fill = if hot && checked {
        Color32::from_rgba_unmultiplied(255, 255, 255, 230)
    } else {
        box_fill
    };
    let box_stroke = if hot && !checked {
        Stroke::new(1.0_f32, t.accent_text)
    } else if hot && checked {
        Stroke::new(1.0_f32, Color32::WHITE)
    } else {
        box_stroke
    };
    ui.painter()
        .rect_filled(box_rect, CornerRadius::same(3), box_fill);
    ui.painter().rect_stroke(
        box_rect,
        CornerRadius::same(3),
        box_stroke,
        egui::StrokeKind::Outside,
    );
    if checked {
        let check_color = if hot { t.accent } else { t.accent_text };
        let p1 = Pos2::new(box_rect.left() + 3.0, box_rect.center().y);
        let p2 = Pos2::new(box_rect.center().x - 0.5, box_rect.bottom() - 3.5);
        let p3 = Pos2::new(box_rect.right() - 3.0, box_rect.top() + 3.0);
        ui.painter()
            .line_segment([p1, p2], Stroke::new(1.6_f32, check_color));
        ui.painter()
            .line_segment([p2, p3], Stroke::new(1.6_f32, check_color));
    }

    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(13.0), fg));
    let text_pos = Pos2::new(rect.left() + 28.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, fg);
    resp
}

fn ui_right_fallback(ctx: &egui::Context) -> f32 {
    ctx.screen_rect().right() - 12.0
}

fn main_tab(
    ui: &mut egui::Ui,
    t: &Tokens,
    active: &mut MainTab,
    tab: MainTab,
    visible: bool,
    label: &str,
) {
    if !visible {
        return;
    }
    let selected = *active == tab;
    let (fill, fg, top_accent) = if selected {
        (t.panel_bg, t.text_primary, t.accent)
    } else {
        (t.tab_inactive_bg, t.tab_inactive_text, Color32::TRANSPARENT)
    };

    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(13.5), fg));
    let pad_x = 16.0;
    let size = Vec2::new(galley.size().x + pad_x * 2.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
        if selected {
            let accent_rect =
                Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), rect.top() + 3.0));
            ui.painter()
                .rect_filled(accent_rect, CornerRadius::ZERO, top_accent);
        }
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, fg);
        if !selected {
            ui.painter().line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1.0_f32, t.border),
            );
        }
    }

    if resp.hovered() && !selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(2, 132, 199, 18),
        );
    }
    if resp.clicked() {
        *active = tab;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_visibility_does_not_force_startup_tab() {
        assert_eq!(initial_tab(true, true), MainTab::Serial);
        assert_eq!(initial_tab(false, true), MainTab::Instruments);
        assert_eq!(initial_tab(false, false), MainTab::Calculator);
    }
}
