//! Main shell — Python MonitorWindow: tabs + Settings on one header row.
//!
//! Settings menu mirrors Python `QToolButton#settings_menu_btn` + cascading `QMenu`:
//! root items with ▶ flyouts (Panels / Language / Theme), accent hover, checkmarks.

use crate::backend::{self, ApiBridge};
use crate::calculator::CalculatorPanel;
use crate::data_analysis::DataAnalysisPanel;
use crate::instrument_control::InstrumentControlPanel;
use crate::serial_tool::SerialToolPanel;
use crate::test_report::TestReportPanel;
use crate::test_tool::TestToolPanel;
use crate::theme::{self as ui_theme, Tokens};
use crate::update::UpdateController;
use crate::waveform_analysis::WaveformAnalysisPanel;
use egui::{Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Sense, Stroke, Vec2};
use wiparse_core::config::{save_config, AppConfig};
use wiparse_core::i18n::{parse_lang, tr, Lang};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MainTab {
    Serial,
    Calculator,
    Instruments,
    Waveform,
    DataAnalysis,
    TestReport,
    TestTool,
}

impl MainTab {
    pub(crate) fn as_id(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Calculator => "calculator",
            Self::Instruments => "instruments",
            Self::Waveform => "waveform",
            Self::DataAnalysis => "data_analysis",
            Self::TestReport => "test_report",
            Self::TestTool => "test_tool",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "serial" | "serial_tool" | "log" => Some(Self::Serial),
            "calculator" | "calc" => Some(Self::Calculator),
            "instruments" | "instrument" | "instrument_control" | "scope" => {
                Some(Self::Instruments)
            }
            "waveform" | "wave" | "waveform_analysis" => Some(Self::Waveform),
            "data" | "data_analysis" | "analysis" => Some(Self::DataAnalysis),
            "report" | "test_report" | "reports" => Some(Self::TestReport),
            "test_tool" | "testtool" | "tools" | "plugins" | "testing_hub" | "hub" => {
                Some(Self::TestTool)
            }
            _ => None,
        }
    }
}

fn page_guide(lang: Lang, tab: MainTab) -> crate::ai::PageGuide {
    let zh = matches!(lang, Lang::Zh);
    let (key, role, purpose, prompts): (&str, &str, &str, [&str; 3]) = match tab {
        MainTab::Serial => (
            "tool.serial_tool.name",
            if zh {
                "根据最近日志解释异常、超时和校验问题。不会打开串口或发送数据。"
            } else {
                "Explain exceptions, timeouts and checksum issues in the recent log. It will not open a port or send data."
            },
            "Explain the attached serial-tool log. Cite line evidence. Do not claim a port was opened or bytes were sent.",
            [
                if zh { "最近日志里有哪些异常，各自可能是什么原因？" } else { "Which exceptions are in the recent log, and what could cause each?" },
                if zh { "这些内容更像超时、校验错误，还是对端没有应答？" } else { "Does this look like a timeout, a checksum error, or no reply?" },
                if zh { "下一步应在串口页核对哪些设置？" } else { "Which serial-page settings should be checked next?" },
            ],
        ),
        MainTab::Waveform => (
            "tool.waveform_analysis.name",
            if zh {
                "依据已计算的测量值和最多 20 帧解码摘要判断波形。不会上传原始采样。"
            } else {
                "Judge the waveform from cached measurements and at most 20 decoded frames. Raw samples are not uploaded."
            },
            "Interpret the attached waveform measurements and decoded-frame summaries. Do not invent raw samples that were not included.",
            [
                if zh { "根据测量值，这路波形有什么异常？" } else { "From the measurements, what looks abnormal on this waveform?" },
                if zh { "已解码的帧里，哪些字段值得怀疑？" } else { "Which fields in the decoded frames look suspicious?" },
                if zh { "两支光标之间应重点看哪些指标？" } else { "Which measurements matter between the two cursors?" },
            ],
        ),
        MainTab::TestReport => (
            "tool.test_report.name",
            if zh {
                "汇总当前项目的运行结果，并阅读正在打开的报告摘录。不会改写结论。"
            } else {
                "Summarize this project's runs and the excerpt of the open report. It will not rewrite a verdict."
            },
            "Summarize the attached test-report index and open excerpt. Do not invent verdicts for runs that are not listed.",
            [
                if zh { "当前项目里，最近哪些测试没有通过？" } else { "Which recent tests in this project did not pass?" },
                if zh { "这些失败有没有共同的线索？" } else { "Do these failures share a clue?" },
                if zh { "打开的这份报告里，哪些结论还需要重测？" } else { "Which conclusions in the open report still need a retest?" },
            ],
        ),
        MainTab::Instruments => (
            "tool.instrument_control.name",
            if zh {
                "根据已连接会话的状态和最近日志说明下一步。不会下发仪器命令。"
            } else {
                "Explain the next step from connected-session status and the recent log. It will not send an instrument command."
            },
            "Explain the attached instrument-session status. Do not claim a command was sent or a measurement was taken.",
            [
                if zh { "按当前会话状态，下一步应该先确认什么？" } else { "Given the current sessions, what should be confirmed first?" },
                if zh { "这个报错通常对应连接问题还是设置问题？" } else { "Does this error usually mean a connection problem or a setup problem?" },
                if zh { "开始采集前还应核对哪些条件？" } else { "What should be checked before acquisition starts?" },
            ],
        ),
        MainTab::DataAnalysis => (
            "tool.data_analysis.name",
            if zh {
                "看曲线的点数、单位、比例和阈值是否合理。不会上传原始采样或文件路径。"
            } else {
                "Check whether series length, units, scale and thresholds make sense. Raw samples and file paths stay local."
            },
            "Interpret the attached data-analysis series, scales and thresholds. Do not invent sample values.",
            [
                if zh { "这些曲线的尺度和点数是否合理？" } else { "Do the scale and point counts of these series look reasonable?" },
                if zh { "当前阈值会不会漏报或误报？" } else { "Could the current thresholds miss events or raise false alarms?" },
                if zh { "应如何核对单位和数据来源？" } else { "How should the units and data source be checked?" },
            ],
        ),
        MainTab::TestTool => (
            "tool.test_tool.name",
            if zh {
                "结合当前插件、测试项目、选择和矩阵统计说明优先检查什么。不会启动测试或改结论。"
            } else {
                "Use the current plugin, project, selection and matrix counts to say what to check first. It will not start a test or change a verdict."
            },
            "Explain the attached test-project status, selection and matrix counts. Do not start a run or change a verdict.",
            [
                if zh { "按当前项目和矩阵统计，应优先检查什么？" } else { "From this project and the matrix counts, what should be checked first?" },
                if zh { "失败项可能和哪些配置或仪器状态有关？" } else { "Which configuration or instrument state could relate to the failures?" },
                if zh { "预检之前还应确认哪些条件？" } else { "What else should be confirmed before preflight?" },
            ],
        ),
        MainTab::Calculator => (
            "tool.calculator.name",
            if zh {
                "核对页面上的输入、单位和已算出的结果。不会把计算当成实测。"
            } else {
                "Check the inputs, units and results already on the page. A calculation is not a measurement."
            },
            "Check the attached calculator inputs and results against the named formulas. State units and assumptions. Do not claim a measurement was taken.",
            [
                if zh { "这些输入和结果的单位、适用条件对吗？" } else { "Are the units and assumptions of these inputs and results correct?" },
                if zh { "帮我核对谐振、滤波或 RC 的公式是否用对了。" } else { "Check whether the resonance, filter or RC formula was applied correctly." },
                if zh { "这个 CRC 或进制换算结果有没有可疑之处？" } else { "Does this CRC or radix conversion look suspicious?" },
            ],
        ),
    };
    crate::ai::PageGuide {
        title: tr(lang, key),
        role: role.into(),
        purpose: purpose.into(),
        prompts: prompts.into_iter().map(str::to_owned).collect(),
    }
}

fn initial_tab(
    show_serial: bool,
    show_instruments: bool,
    show_waveform: bool,
    show_data_analysis: bool,
    show_test_report: bool,
    show_test_tool: bool,
) -> MainTab {
    if show_serial {
        MainTab::Serial
    } else if show_instruments {
        MainTab::Instruments
    } else if show_waveform {
        MainTab::Waveform
    } else if show_data_analysis {
        MainTab::DataAnalysis
    } else if show_test_tool {
        MainTab::TestTool
    } else if show_test_report {
        MainTab::TestReport
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
    ai: crate::ai::AiPanel,
    lang: Lang,
    active: MainTab,
    last_logged_tab: Option<MainTab>,
    show_serial: bool,
    show_calculator: bool,
    show_instruments: bool,
    show_waveform: bool,
    show_data_analysis: bool,
    show_test_report: bool,
    show_test_tool: bool,
    calculator: CalculatorPanel,
    serial: SerialToolPanel,
    instruments: InstrumentControlPanel,
    waveform: WaveformAnalysisPanel,
    data_analysis: DataAnalysisPanel,
    test_report: TestReportPanel,
    test_tool: TestToolPanel,
    api: ApiBridge,
    settings_open: bool,
    settings_sub: SettingsSub,
    /// Anchor Y for the open submenu (top of parent row).
    settings_sub_anchor_y: f32,
    settings_btn_rect: Option<Rect>,
    settings_root_rect: Option<Rect>,
    settings_sub_rect: Option<Rect>,
    settings_leave_since: Option<f64>,
    about_open: bool,
    updater: UpdateController,
    update_startup_checked: bool,
    tokens: Tokens,
    theme_applied_light: Option<bool>,
    taskbar_icon_applied: bool,
    taskbar_icon_attempts: u8,
    startup_warning: Option<String>,
    exit_log_prompt:bool,
    exit_log_confirmed:bool,
}

impl WiParseApp {
    pub fn new(cfg: AppConfig) -> Self {
        Self::new_with_warning(cfg, None)
    }

    pub fn new_with_warning(cfg: AppConfig, startup_warning: Option<String>) -> Self {
        let lang = parse_lang(&cfg.ui.language);
        let show_serial = cfg.ui.panels.serial_tool;
        let show_calculator = cfg.ui.panels.calculator;
        let show_instruments = cfg.ui.panels.instrument_control;
        let show_waveform = cfg.ui.panels.waveform_analysis;
        let show_data_analysis = cfg.ui.panels.data_analysis;
        let show_test_report = cfg.ui.panels.test_report;
        let show_test_tool = cfg.ui.panels.test_tool;
        let active = initial_tab(
            show_serial,
            show_instruments,
            show_waveform,
            show_data_analysis,
            show_test_report,
            show_test_tool,
        );
        let light = cfg.ui.theme == "light";
        let api = ApiBridge::new();
        if let Err(e) = backend::start(api.clone()) {
            tracing::error!("Failed to start WiParse API: {e}");
            if crate::companion_window() {
                std::process::exit(2);
            }
        }
        let updater = UpdateController::new(cfg.update.clone());
        Self {
            ai: crate::ai::AiPanel::new(&cfg.ai),
            serial: SerialToolPanel::new(&cfg, lang),
            instruments: {
                let mut panel = InstrumentControlPanel::new(&cfg);
                if cfg.ui.debug_mode {
                    panel.apply_debug_mode(true, lang);
                }
                panel
            },
            waveform: WaveformAnalysisPanel::new(&cfg),
            data_analysis: DataAnalysisPanel::new(&cfg),
            test_report: TestReportPanel::new(&cfg),
            test_tool: TestToolPanel::new(&cfg),
            cfg,
            lang,
            active,
            last_logged_tab: None,
            show_serial,
            show_calculator,
            show_instruments,
            show_waveform,
            show_data_analysis,
            show_test_report,
            show_test_tool,
            calculator: CalculatorPanel::new(),
            api,
            settings_open: false,
            settings_sub: SettingsSub::None,
            settings_sub_anchor_y: 0.0,
            settings_btn_rect: None,
            settings_root_rect: None,
            settings_sub_rect: None,
            settings_leave_since: None,
            about_open: false,
            updater,
            update_startup_checked: false,
            tokens: if light {
                Tokens::light()
            } else {
                Tokens::dark()
            },
            theme_applied_light: None,
            taskbar_icon_applied: false,
            taskbar_icon_attempts: 0,
            startup_warning,
            exit_log_prompt:false,
            exit_log_confirmed:false,
        }
    }

    fn ensure_visible_tab(&mut self) {
        let visible = match self.active {
            MainTab::Serial => self.show_serial,
            MainTab::Calculator => self.show_calculator,
            MainTab::Instruments => self.show_instruments,
            MainTab::Waveform => self.show_waveform,
            MainTab::DataAnalysis => self.show_data_analysis,
            MainTab::TestReport => self.show_test_report,
            MainTab::TestTool => self.show_test_tool,
        };
        if visible {
            return;
        }
        if self.show_serial {
            self.active = MainTab::Serial;
        } else if self.show_instruments {
            self.active = MainTab::Instruments;
        } else if self.show_waveform {
            self.active = MainTab::Waveform;
        } else if self.show_data_analysis {
            self.active = MainTab::DataAnalysis;
        } else if self.show_test_tool {
            self.active = MainTab::TestTool;
        } else if self.show_test_report {
            self.active = MainTab::TestReport;
        } else if self.show_calculator {
            self.active = MainTab::Calculator;
        } else {
            self.show_serial = true;
            self.active = MainTab::Serial;
        }
    }

    fn open_report_artifact(
        &mut self,
        path: std::path::PathBuf,
        view: wiparse_core::project::ArtifactView,
        overlay: bool,
    ) {
        use wiparse_core::project::ArtifactView;
        match view {
            ArtifactView::Log => {
                self.test_report
                    .open_in_pane(path, view, overlay, self.lang);
            }
            ArtifactView::Wave => {
                let _ = self
                    .waveform
                    .open_path_maybe_overlay(&path, self.lang, overlay);
                if self.active != MainTab::TestReport {
                    self.show_waveform = true;
                    self.active = MainTab::Waveform;
                }
            }
            ArtifactView::Report => {
                self.test_report.open_markdown(path, self.lang);
            }
            ArtifactView::External => {
                let _ = crate::test_report::open_path_external(&path);
            }
        }
    }

    fn persist_ui_prefs(&mut self) {
        self.cfg.ui.panels.serial_tool = self.show_serial;
        self.cfg.ui.panels.calculator = self.show_calculator;
        self.cfg.ui.panels.instrument_control = self.show_instruments;
        self.cfg.ui.panels.waveform_analysis = self.show_waveform;
        self.cfg.ui.panels.data_analysis = self.show_data_analysis;
        self.cfg.ui.panels.test_report = self.show_test_report;
        self.cfg.ui.panels.test_tool = self.show_test_tool;
        self.cfg.ui.language = match self.lang {
            Lang::Zh => "zh".into(),
            Lang::En => "en".into(),
        };
        if let Err(e) = save_config(&self.cfg) {
            self.startup_warning = Some(format!("配置保存失败 / Failed to save config: {e}"));
        }
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

        if menu_action_row(ui, t, "AI", ROOT_W).clicked() {
            self.ai.open_assistant();
            self.close_settings();
        }
        let about_l = tr(self.lang, "menu.about");
        if menu_action_row(ui, t, &about_l, ROOT_W).clicked() {
            self.about_open = true;
            self.close_settings();
        }
    }

    fn settings_sub_ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        // Fit longest panel label ("Waveform Analysis" / "数据分析") + checkbox.
        const SUB_W: f32 = 196.0;
        ui.set_width(SUB_W);
        ui.set_max_width(SUB_W);
        ui.set_min_width(SUB_W);
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut dirty = false;
        let mut close_after = false;

        match self.settings_sub {
            SettingsSub::Panels => {
                let serial_name = tr(self.lang, "tool.serial_tool.name");
                let instrument_name = tr(self.lang, "tool.instrument_control.name");
                let waveform_name = tr(self.lang, "tool.waveform_analysis.name");
                let data_name = tr(self.lang, "tool.data_analysis.name");
                let report_name = tr(self.lang, "tool.test_report.name");
                let test_tool_name = tr(self.lang, "tool.test_tool.name");
                let calculator_name = tr(self.lang, "tool.calculator.name");

                if menu_check_row(ui, t, &serial_name, self.show_serial, SUB_W).clicked() {
                    self.show_serial = !self.show_serial;
                    if self.show_serial {
                        self.active = MainTab::Serial;
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
                if menu_check_row(ui, t, &waveform_name, self.show_waveform, SUB_W).clicked() {
                    self.show_waveform = !self.show_waveform;
                    if self.show_waveform {
                        self.active = MainTab::Waveform;
                    }
                    dirty = true;
                }
                if menu_check_row(ui, t, &data_name, self.show_data_analysis, SUB_W).clicked() {
                    self.show_data_analysis = !self.show_data_analysis;
                    if self.show_data_analysis {
                        self.active = MainTab::DataAnalysis;
                    }
                    dirty = true;
                }
                if menu_check_row(ui, t, &test_tool_name, self.show_test_tool, SUB_W).clicked() {
                    self.show_test_tool = !self.show_test_tool;
                    if self.show_test_tool {
                        self.active = MainTab::TestTool;
                    }
                    dirty = true;
                }
                if menu_check_row(ui, t, &report_name, self.show_test_report, SUB_W).clicked() {
                    self.show_test_report = !self.show_test_report;
                    if self.show_test_report {
                        self.active = MainTab::TestReport;
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
                if !self.show_serial
                    && !self.show_calculator
                    && !self.show_instruments
                    && !self.show_waveform
                    && !self.show_data_analysis
                    && !self.show_test_report
                    && !self.show_test_tool
                {
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
        use crate::update::UpdatePhase;

        self.updater.poll();
        let mut open = self.about_open;
        let mut close_requested = false;
        const DIALOG_W: f32 = 420.0;
        const BTN: Vec2 = Vec2::new(88.0, 28.0);

        egui::Window::new(tr(self.lang, "about.title"))
            .id(egui::Id::new("about_dialog"))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(DIALOG_W)
            .open(&mut open)
            .frame(
                Frame::window(&ctx.style())
                    .fill(t.panel_bg)
                    .stroke(Stroke::new(1.0_f32, t.border))
                    .corner_radius(CornerRadius::same(ui_theme::RADIUS_DIALOG))
                    .inner_margin(Margin::symmetric(18, 16)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(DIALOG_W);
                ui.set_max_width(DIALOG_W);

                // Header: product identity + version badge
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("WiParse")
                                .size(ui_theme::FONT_SECTION + 2.0)
                                .strong()
                                .color(t.text_primary),
                        );
                        ui.label(
                            egui::RichText::new(tr(self.lang, "about.subtitle"))
                                .size(ui_theme::FONT_BODY)
                                .color(t.text_muted),
                        );
                    });
                    ui.with_layout(
                        egui::Layout::top_down(egui::Align::Max).with_cross_align(egui::Align::Max),
                        |ui| {
                            Frame::NONE
                                .fill(t.surface_bg)
                                .stroke(Stroke::new(1.0_f32, t.divider))
                                .corner_radius(CornerRadius::same(ui_theme::RADIUS_CTRL))
                                .inner_margin(Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("v{APP_VERSION}"))
                                            .monospace()
                                            .size(ui_theme::FONT_BODY)
                                            .color(t.accent),
                                    );
                                });
                        },
                    );
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Simplified release notes for this build
                Frame::NONE
                    .fill(t.surface_bg)
                    .stroke(Stroke::new(1.0_f32, t.divider))
                    .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(tr(self.lang, "about.notes_section"))
                                .size(ui_theme::FONT_TITLE)
                                .strong()
                                .color(t.text_primary),
                        );
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(148.0)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(tr(self.lang, "about.notes"))
                                        .size(ui_theme::FONT_BODY)
                                        .color(t.text_muted),
                                );
                            });
                    });

                ui.add_space(10.0);

                // Update section card
                Frame::NONE
                    .fill(t.surface_bg)
                    .stroke(Stroke::new(1.0_f32, t.divider))
                    .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(tr(self.lang, "about.update_section"))
                                .size(ui_theme::FONT_TITLE)
                                .strong()
                                .color(t.text_primary),
                        );
                        ui.add_space(8.0);

                        let status_h = 52.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), status_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| match self.updater.phase() {
                                    UpdatePhase::Idle => {
                                        ui.label(
                                        egui::RichText::new("—").size(12.0).color(t.text_muted),
                                        );
                                    }
                                    UpdatePhase::Checking => {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(tr(self.lang, "update.checking"));
                                        });
                                    }
                                    UpdatePhase::UpToDate => {
                                        ui.horizontal(|ui| {
                                            ui_theme::status_line(
                                                ui,
                                                t,
                                                ui_theme::StatusTone::Ok,
                                                &tr(self.lang, "update.up_to_date"),
                                            );
                                        });
                                    }
                                    UpdatePhase::Available { manifest, .. } => {
                                        ui.label(format!(
                                            "{}: v{}",
                                            tr(self.lang, "update.available"),
                                            manifest.version
                                        ));
                                        if let Some(notes) = manifest.notes.as_deref() {
                                            ui.add_space(2.0);
                                        egui::ScrollArea::vertical().max_height(36.0).show(
                                            ui,
                                            |ui| {
                                                    ui.label(
                                                        egui::RichText::new(notes)
                                                            .size(11.0)
                                                            .color(t.text_muted),
                                                    );
                                            },
                                        );
                                        }
                                    }
                                    UpdatePhase::Downloading { received, total } => {
                                        ui.label(tr(self.lang, "update.downloading"));
                                        let frac = if *total > 0 {
                                            *received as f32 / *total as f32
                                        } else {
                                            0.0
                                        };
                                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                                    }
                                    UpdatePhase::Ready(_) => {
                                        ui.label(
                                            egui::RichText::new(tr(self.lang, "update.ready"))
                                                .color(t.text_primary),
                                        );
                                    }
                                    UpdatePhase::Error(msg) => {
                                    ui.colored_label(Color32::from_rgb(0xFF, 0x6B, 0x6B), msg);
                                }
                            },
                        );

                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if ui
                                .add(egui::Button::new(tr(self.lang, "update.check")).min_size(BTN))
                                .clicked()
                            {
                                self.updater.check_now();
                            }
                            if matches!(self.updater.phase(), UpdatePhase::Available { .. })
                                && ui
                                    .add(
                                        egui::Button::new(tr(self.lang, "update.download"))
                                            .min_size(BTN)
                                            .fill(t.accent)
                                            .stroke(Stroke::NONE),
                                    )
                                    .clicked()
                            {
                                self.updater.download_available();
                            }
                            if matches!(self.updater.phase(), UpdatePhase::Ready(_))
                                && ui
                                    .add(
                                        egui::Button::new(tr(self.lang, "update.install"))
                                            .min_size(BTN)
                                            .fill(t.accent)
                                            .stroke(Stroke::NONE),
                                    )
                                    .clicked()
                            {
                                let _ = self.updater.apply_ready_and_exit();
                            }
                        });
                    });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(tr(self.lang, "about.close"))
                                .min_size(egui::vec2(72.0, 28.0)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                });
            });
        self.about_open = open && !close_requested;
    }

    fn dispatch_test_instrument_job(&mut self, job: crate::serial_tool::TestInstrumentJob) {
        use crate::serial_tool::TestInstrumentJob;
        let fallback = self.instruments.first_oscilloscope_id();
        match job {
            TestInstrumentJob::Capture {
                tag,
                save,
                device_id,
            } => {
                let id = device_id.or(fallback);
                let Some(device_id) = id else {
                    self.serial.notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "capture".into(),
                        device_id: None,
                        detail: serde_json::json!({ "error": "no connected oscilloscope", "tag": tag }),
                    });
                    return;
                };
                let mut params = serde_json::json!({
                    "device_id": device_id,
                    "wait": save,
                });
                if save {
                    if let Some(dir) = self.serial.evidence_dir() {
                        let path = dir.join("scope").join(format!("{tag}.png"));
                        params["path"] = serde_json::json!(path.display().to_string());
                    }
                }
                let reply = self.instruments.api_capture(&params);
                if !reply.ok {
                    let msg = reply.error.unwrap_or_else(|| "capture rejected".into());
                    self.serial
                        .notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "capture".into(),
                        device_id: Some(device_id),
                        detail: serde_json::json!({ "error": msg, "tag": tag }),
                    });
                }
            }
            TestInstrumentJob::WaveformSource {
                device_id,
                dir,
                filename,
                overwrite,
            } => {
                let id = device_id.or(fallback);
                let Some(device_id) = id else {
                    self.serial
                        .notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "waveform_source".into(),
                        device_id: None,
                        detail: serde_json::json!({ "error": "no connected oscilloscope" }),
                    });
                    return;
                };
                let mut params = serde_json::json!({
                    "device_id": device_id,
                    "overwrite": overwrite,
                });
                if !dir.is_empty() {
                    params["dir"] = serde_json::json!(dir);
                }
                if let Some(name) = filename {
                    params["filename"] = serde_json::json!(name);
                }
                let reply = self.instruments.api_waveform_source(&params);
                if !reply.ok {
                    let msg = reply
                        .error
                        .unwrap_or_else(|| "waveform_source rejected".into());
                    self.serial
                        .notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "waveform_source".into(),
                        device_id: Some(device_id),
                        detail: serde_json::json!({ "error": msg }),
                    });
                }
            }
            TestInstrumentJob::Command { device_id, command } => {
                let id = device_id.or(fallback);
                let Some(device_id) = id else {
                    self.serial
                        .notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "command".into(),
                        device_id: None,
                        detail: serde_json::json!({ "error": "no connected instrument" }),
                    });
                    return;
                };
                let reply = self.instruments.api_command(&serde_json::json!({
                    "device_id": device_id,
                    "command": command,
                }));
                if !reply.ok {
                    let msg = reply.error.unwrap_or_else(|| "command rejected".into());
                    self.serial
                        .notify_test_job(&crate::instrument_control::InstrumentJobResult {
                        job_id: 0,
                        ok: false,
                        kind: "command".into(),
                        device_id: Some(device_id),
                        detail: serde_json::json!({ "error": msg }),
                    });
                }
            }
        }
    }
}

impl eframe::App for WiParseApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input|input.viewport().close_requested())&&!self.exit_log_confirmed&&self.serial.has_pending_log_edits(){
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);self.exit_log_prompt=true;
        }
        if self.exit_log_prompt {
            let zh=self.lang==Lang::Zh;let mut discard=false;let mut stay=false;
            egui::Window::new(if zh{"日志仍在编辑"}else{"Log editing is still active"}).collapsible(false).resizable(false).show(ctx,|ui|{
                ui.label(if zh{"日志编辑缓冲尚未结束。请返回保存或放弃编辑，也可以明确放弃未保存内容并退出。"}else{"A log editor is still active. Return to save or discard it, or explicitly discard unsaved edits and exit."});
                ui.horizontal(|ui|{if ui.button(if zh{"返回处理"}else{"Return"}).clicked(){stay=true;}if ui.button(if zh{"放弃并退出"}else{"Discard and exit"}).clicked(){discard=true;}});
            });
            if stay{self.exit_log_prompt=false;}
            if discard{self.exit_log_confirmed=true;self.exit_log_prompt=false;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
        }
        if crate::companion_window() {
            let detached = crate::companion_detached();
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(detached||self.exit_log_prompt));
            if detached {
                if self.test_tool.companion_shutdown_blocked() || self.serial.is_monitoring() || self.serial.has_pending_log_edits()
                    || self.serial.is_test_running() || self.instruments.api_operations_pending() {
                    // Keep the one actual device owner alive. The operator can
                    // finish/stop operations through the retained host.
                    if self.startup_warning.is_none() {
                        self.startup_warning = Some("新界面已关闭。采集、测试或未保存的配置仍在处理中，设备宿主已保留；请完成或停止任务后正常关闭。".into());
                    }
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        self.ai.install(ctx);
        self.test_tool.workspace_close_guard(ctx, self.lang);
        self.serial.attach_repaint_context(ctx);
        self.api.attach_egui_ctx(ctx);
        if let Some(message) = self.startup_warning.clone() {
            egui::Window::new("WiParse 配置 / Configuration")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("知道了 / Dismiss").clicked() {
                        self.startup_warning = None;
                    }
                });
        }
        if !self.update_startup_checked {
            self.update_startup_checked = true;
            self.updater.maybe_check_on_startup();
        }
        self.updater.poll();
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
        let serial_log_versions=self.serial.log_versions();
        // Drain Agent/CLI API requests on the UI thread (shared serial/instrument/UI state).
        let persist = backend::drain_api_requests(
            &self.api,
            backend::UiHost {
                cfg: &mut self.cfg,
                lang: &mut self.lang,
                active: &mut self.active,
                show_serial: &mut self.show_serial,
                show_calculator: &mut self.show_calculator,
                show_instruments: &mut self.show_instruments,
                show_waveform: &mut self.show_waveform,
                show_data_analysis: &mut self.show_data_analysis,
                show_test_report: &mut self.show_test_report,
                show_test_tool: &mut self.show_test_tool,
                serial: &mut self.serial,
                instruments: &mut self.instruments,
                calculator: &mut self.calculator,
                waveform: &mut self.waveform,
                data_analysis: &mut self.data_analysis,
                test_report: &mut self.test_report,
                test_tool: &mut self.test_tool,
                ai: &mut self.ai,
            },
        );
        if persist {
            self.persist_ui_prefs();
            self.ensure_visible_tab();
        }
        // Live filters only need refresh when serial UI shows the live tab.
        let serial_stream_visible=self.api.events.has_serial_log_subscribers();
        self.serial.set_live_view_visible(serial_visible||serial_stream_visible);
        self.serial
            .drain_events_with_bus(live_filters, Some(&self.api));
        self.serial.publish_log_changes(&serial_log_versions,&self.api);
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("wiparse-ai-platform-bridge"),self.api.clone());
            data.insert_temp(egui::Id::new("wiparse-ai-project-root"),self.test_tool.data_root_path());
        });
        self.serial.tick_ai(ctx, self.lang, self.test_tool.current_project_id(), &self.cfg.ai, self.test_tool.running_plugin_id().is_some());
        // Data Analysis serial-live: pull new live lines after serial drain.
        if !crate::companion_window() && self.data_analysis.wants_live_serial() {
            let dropped = self.serial.live_dropped();
            let total = self.serial.live_abs_len();
            self.data_analysis.prepare_live_poll(total, dropped);
            let from = self.data_analysis.live_line_cursor();
            if total > from {
                let lines = self.serial.live_lines_since(from, total - from);
                let next = from.max(dropped).saturating_add(lines.len());
                self.data_analysis
                    .ingest_live_lines(&lines, next, self.lang);
            }
        }
        for tag in self.serial.take_scope_captures() {
            let id = self.instruments.first_oscilloscope_id();
            let ok = if let Some(device_id) = id {
                self.instruments
                    .api_capture(&serde_json::json!({ "device_id": device_id }))
                    .ok
            } else {
                false
            };
            self.serial.note_scope_capture(&tag, id, ok);
        }
        for job in self.serial.take_test_instrument_jobs() {
            self.dispatch_test_instrument_job(job);
        }
        // Instrument workers can deliver large waveform/image payloads. Do not
        // deserialize, rebuild plots, or schedule its live repaint loop while
        // the user is working in another tool.
        // Always drain instrument worker so Agent API jobs complete; UI widgets
        // for heavy scope views only render on the Instruments tab.
        self.instruments.pump_with_bus(ctx, Some(&self.api.events));
        for result in self.instruments.take_job_results() {
            self.serial.notify_test_job(&result);
        }

        let pump_serial = self.serial.has_pending_rx() || self.serial.has_background_io();
        let pump_instruments = self.active == MainTab::Instruments
            && self.show_instruments
            && self.instruments.live_active();
        let pump_data = self.active == MainTab::DataAnalysis
            && self.show_data_analysis
            && self.data_analysis.needs_repaint();
        if pump_serial || pump_instruments || pump_data {
            let ms = if serial_visible || self.serial.has_pending_rx() {
                33
            } else if self.active == MainTab::Instruments
                && self.show_instruments
                && pump_instruments
            {
                33
            } else if pump_data {
                if self.data_analysis.is_loading() {
                    8
                } else {
                    16
                }
            } else {
                500
            };
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        } else {
            // Keep draining Agent/MCP invoke while idle; otherwise stateful
            // methods wait 15s and time out.
            let idle_ms = if self.serial.is_monitoring() && !serial_visible {
                500
            } else {
                200
            };
            ctx.request_repaint_after(std::time::Duration::from_millis(idle_ms));
        }

        let light = self.cfg.ui.theme == "light";
        if self.theme_applied_light != Some(light) {
            let tokens = ui_theme::tokens_for_preference(light);
            self.tokens = ui_theme::apply_tokens(ctx, tokens);
            self.theme_applied_light = Some(light);
        }
        let t = self.tokens;
        self.ensure_visible_tab();
        let reveal_active_tab = self.last_logged_tab != Some(self.active);
        if reveal_active_tab {
            tracing::info!(tab = self.active.as_id(), "active WiParse page");
            self.last_logged_tab = Some(self.active);
        }

        egui::TopBottomPanel::top("main_header")
            .frame(
                Frame::NONE
                    .fill(t.header_bg)
                    .inner_margin(Margin::symmetric(10, 0))
                    .stroke(Stroke::new(1.0_f32, t.divider)),
            )
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let brand_w = if ui.available_width() >= 820.0 { 112.0 } else { 30.0 };
                    let (brand, _) = ui.allocate_exact_size(egui::vec2(brand_w, 32.0), Sense::hover());
                    let icon = egui::Rect::from_center_size(
                        egui::pos2(brand.left() + 14.0, brand.center().y), egui::vec2(24.0, 24.0));
                    ui.painter().rect_filled(icon, CornerRadius::same(7), t.accent);
                    let center = icon.center();
                    let points = [center + egui::vec2(-7.0, 1.0), center + egui::vec2(-3.0, 1.0),
                        center + egui::vec2(-1.0, -5.0), center + egui::vec2(2.0, 5.0),
                        center + egui::vec2(4.0, -1.0), center + egui::vec2(7.0, -1.0)];
                    ui.painter().add(egui::Shape::line(points.to_vec(), Stroke::new(1.6_f32, t.accent_text)));
                    if brand_w > 30.0 {
                        ui.painter().text(egui::pos2(brand.left() + 34.0, brand.center().y),
                            egui::Align2::LEFT_CENTER, "WiParse", FontId::proportional(16.0), t.text_primary);
                    }
                    let gear_w = 36.0;
                    let tabs_w = (ui.available_width() - gear_w - ui.spacing().item_spacing.x).max(1.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(tabs_w, ui.available_height()),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            egui::ScrollArea::horizontal()
                                .id_salt("main-nav")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .show(ui, |ui| {
                                    ui.horizontal_centered(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::Serial,
                                            self.show_serial,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.serial_tool.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::Instruments,
                                            self.show_instruments,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.instrument_control.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::Waveform,
                                            self.show_waveform,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.waveform_analysis.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::DataAnalysis,
                                            self.show_data_analysis,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.data_analysis.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::TestTool,
                                            self.show_test_tool,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.test_tool.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::TestReport,
                                            self.show_test_report,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.test_report.name"),
                                        );
                                        main_tab(
                                            ui,
                                            &t,
                                            &mut self.active,
                                            MainTab::Calculator,
                                            self.show_calculator,
                                            reveal_active_tab,
                                            &tr(self.lang, "tool.calculator.name"),
                                        );
                                    });
                                });
                        },
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Gear icon settings — quieter than text label.
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(32.0, 28.0), Sense::click());
                        let fill = if self.settings_open {
                            t.accent_soft
                        } else if resp.hovered() {
                            t.accent_soft
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(ui_theme::RADIUS_CTRL),
                            fill,
                        );
                        let fg = if resp.hovered() || self.settings_open {
                            t.text_primary
                        } else {
                            t.text_muted
                        };
                        // Draw with primitives so the icon does not depend on a symbol font.
                        ui.painter()
                            .circle_stroke(rect.center(), 6.0, Stroke::new(1.6_f32, fg));
                        ui.painter()
                            .circle_stroke(rect.center(), 2.0, Stroke::new(1.6_f32, fg));
                        for n in 0..8 {
                            let angle = n as f32 * std::f32::consts::TAU / 8.0;
                            let dir = Vec2::angled(angle);
                            ui.painter().line_segment(
                                [rect.center() + dir * 7.0, rect.center() + dir * 9.0],
                                Stroke::new(1.6_f32, fg),
                            );
                        }
                        self.settings_btn_rect = Some(rect.expand(2.0));
                        let resp = resp.on_hover_text(tr(self.lang, "menu.settings"));
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

        let ai_active = self.active;
        let ai_waveform = &self.waveform;
        let ai_report = &self.test_report;
        if self.serial.ai_alert_ui(ctx,self.lang) {self.active=MainTab::Serial; self.show_serial=true;}
        if self.serial.take_ai_settings_request() { self.ai.open_settings(); }
        let ai_serial = &self.serial;
        let ai_instruments = &self.instruments;
        let ai_data = &self.data_analysis;
        let ai_tool = &self.test_tool;
        let ai_calculator = &self.calculator;
        let guide = page_guide(self.lang, ai_active);
        self.ai.ui(ctx, self.lang, &mut self.cfg, &guide, || {
            let data = match ai_active {
                MainTab::Waveform => ai_waveform.ai_summary(),
                MainTab::TestReport => ai_report.ai_summary(),
                MainTab::Serial => ai_serial.ai_summary(),
                MainTab::Instruments => ai_instruments.ai_summary(),
                MainTab::DataAnalysis => ai_data.ai_summary(),
                MainTab::TestTool => ai_tool.ai_summary(),
                MainTab::Calculator => ai_calculator.ai_summary(),
            };
            format!(
                "WiParse {} · {}\n{}",
                APP_VERSION,
                ai_active.as_id(),
                serde_json::to_string_pretty(&data).unwrap_or_default()
            )
        });

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .frame(
                Frame::NONE
                    .fill(t.header_bg)
                    .inner_margin(Margin::symmetric(10, 4))
                    .stroke(Stroke::new(1.0_f32, t.divider)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (status, tone) = if self.active == MainTab::Serial {
                        (
                            self.serial.status_text().to_string(),
                            ui_theme::tone_from_status(self.serial.status_text()),
                        )
                    } else if self.active == MainTab::Instruments {
                        (
                            self.instruments.status_text().to_string(),
                            ui_theme::tone_from_status(self.instruments.status_text()),
                        )
                    } else if self.active == MainTab::Waveform {
                        (
                            self.waveform.status_text().to_string(),
                            ui_theme::tone_from_status(self.waveform.status_text()),
                        )
                    } else if self.active == MainTab::DataAnalysis {
                        (
                            self.data_analysis.status_text().to_string(),
                            ui_theme::tone_from_status(self.data_analysis.status_text()),
                        )
                    } else if self.active == MainTab::TestReport {
                        (
                            self.test_report.status_text().to_string(),
                            ui_theme::tone_from_status(self.test_report.status_text()),
                        )
                    } else if self.active == MainTab::TestTool {
                        (
                            self.test_tool.status_text().to_string(),
                            self.test_tool.status_tone(),
                        )
                    } else {
                        let status = self.calculator.status_line(self.lang);
                        let tone = if status == tr(self.lang, "status.ready") {
                            ui_theme::StatusTone::Ok
                        } else {
                            ui_theme::StatusTone::Error
                        };
                        (status, tone)
                    };
                    ui_theme::status_line(ui, &t, tone, &status);
                });
            });

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t.canvas_bg).inner_margin(Margin::same(ui_theme::SHELL_INSET)))
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
                MainTab::Waveform if self.show_waveform => {
                    self.waveform.ui(ui, self.lang, &t);
                }
                MainTab::DataAnalysis if self.show_data_analysis => {
                    self.data_analysis.ui(ui, self.lang, &t);
                    self.data_analysis.clear_live_plot_dirty();
                }
                MainTab::TestReport if self.show_test_report => {
                    let plugins = self.test_tool.plugin_metas();
                    let running = self.test_tool.running_test_id().map(str::to_owned);
                    let data_root = self.test_tool.data_root_path();
                    let hub_project = self.test_tool.current_project_id();
                    let hub_epoch = self.test_tool.artifact_epoch();
                    match self.test_report.ui(
                        ui,
                        self.lang,
                        &t,
                        &data_root,
                        &plugins,
                        running.as_deref(),
                        &hub_project,
                        hub_epoch,
                        Some(&mut self.waveform),
                    ) {
                        crate::test_report::ReportAction::None => {}
                        crate::test_report::ReportAction::GoHub => {
                            self.show_test_tool = true;
                            self.active = MainTab::TestTool;
                        }
                        crate::test_report::ReportAction::ProjectChanged(id) => {
                            self.test_tool.set_project_id(id);
                        }
                        crate::test_report::ReportAction::Open {
                            path,
                            view,
                            overlay,
                        } => {
                            self.open_report_artifact(path, view, overlay);
                        }
                    }
                }
                MainTab::TestTool if self.show_test_tool => {
                    self.test_tool
                        .ui(ui, self.lang, &t, &self.instruments.live_devices_for_hub());
                }
                _ => {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(tr(self.lang, "shell.enable_panel")).color(t.text_muted),
                        );
                    });
                }
            });
        if let Some(path) = self.test_tool.take_workspace_result() {
            let root = self.test_tool.data_root_path();
            if let Ok((path, view)) = self.test_report.prepare_open(&root, &path, None) {
                self.open_report_artifact(path, view, false);
            }
        }
        if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("hub.open_ai_settings"))).unwrap_or(false) {
            self.ai.open_settings();
        }
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
        .corner_radius(CornerRadius::same(ui_theme::RADIUS_CTRL))
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
    reveal_selected: bool,
    label: &str,
) {
    if !visible {
        return;
    }
    let selected = *active == tab;
    let fg = if selected {
        t.text_primary
    } else {
        t.tab_inactive_text
    };

    let galley = ui.fonts(|f| {
        f.layout_no_wrap(
            label.to_string(),
            FontId::proportional(ui_theme::FONT_TITLE),
            fg,
        )
    });
    let pad_x = 14.0;
    let size = Vec2::new(galley.size().x + pad_x * 2.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if selected && reveal_selected {
        resp.scroll_to_me(Some(egui::Align::Center));
    }

    if ui.is_rect_visible(rect) {
        if resp.hovered() || selected {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(ui_theme::RADIUS_CTRL),
                t.accent_soft,
            );
        }
        let text_pos = Pos2::new(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5 - 1.0,
        );
        ui.painter().galley(text_pos, galley, fg);
        // Bottom indicator bar (Apple-style text tabs).
        let bar_y = rect.bottom() - 2.0;
        if selected {
            ui.painter().hline(
                (rect.left() + 10.0)..=(rect.right() - 10.0),
                bar_y,
                Stroke::new(2.0_f32, t.accent),
            );
        }
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
        assert_eq!(
            initial_tab(true, true, true, true, true, true),
            MainTab::Serial
        );
        assert_eq!(
            initial_tab(false, true, true, true, true, true),
            MainTab::Instruments
        );
        assert_eq!(
            initial_tab(false, false, true, true, true, true),
            MainTab::Waveform
        );
        assert_eq!(
            initial_tab(false, false, false, true, true, true),
            MainTab::DataAnalysis
        );
        assert_eq!(
            initial_tab(false, false, false, false, true, true),
            MainTab::TestTool
        );
        assert_eq!(
            initial_tab(false, false, false, false, false, true),
            MainTab::TestTool
        );
        assert_eq!(
            initial_tab(false, false, false, false, true, false),
            MainTab::TestReport
        );
        assert_eq!(
            initial_tab(false, false, false, false, false, false),
            MainTab::Calculator
        );
    }
}
