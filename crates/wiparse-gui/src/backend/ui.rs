//! GUI page switch / control handlers (stateful, UI thread).

use super::dispatch::{err, ok};
use super::InvokeReply;
use crate::app::MainTab;
use crate::calculator::CalculatorPanel;
use crate::data_analysis::DataAnalysisPanel;
use crate::instrument_control::InstrumentControlPanel;
use crate::serial_tool::SerialToolPanel;
use crate::test_report::TestReportPanel;
use crate::test_tool::TestToolPanel;
use crate::waveform_analysis::WaveformAnalysisPanel;
use serde_json::{json, Value};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::{parse_lang, Lang};

pub struct UiHost<'a> {
    pub cfg: &'a mut AppConfig,
    pub lang: &'a mut Lang,
    pub active: &'a mut MainTab,
    pub show_serial: &'a mut bool,
    pub show_calculator: &'a mut bool,
    pub show_instruments: &'a mut bool,
    pub show_waveform: &'a mut bool,
    pub show_data_analysis: &'a mut bool,
    pub show_test_report: &'a mut bool,
    pub show_test_tool: &'a mut bool,
    pub serial: &'a mut SerialToolPanel,
    pub instruments: &'a mut InstrumentControlPanel,
    pub calculator: &'a mut CalculatorPanel,
    pub waveform: &'a mut WaveformAnalysisPanel,
    pub data_analysis: &'a mut DataAnalysisPanel,
    pub test_report: &'a mut TestReportPanel,
    pub test_tool: &'a mut TestToolPanel,
}

pub fn handle(host: &mut UiHost<'_>, method: &str, params: &Value) -> Option<(InvokeReply, bool)> {
    match method {
        "system.ui.state" | "ui.state" => Some((ui_state(host), false)),
        "ui.show" => Some(ui_show(host, params)),
        "ui.panels" => Some(ui_panels(host, params)),
        "ui.prefs" => Some(ui_prefs(host, params)),
        "ui.serial.open" => Some((host.serial.api_open_log(params), false)),
        "ui.serial.close" => Some((host.serial.api_close_tab(params), false)),
        "ui.serial.clear" => Some((host.serial.api_clear(), false)),
        "ui.serial.filter" => Some((host.serial.api_filter(params), false)),
        "ui.serial.tab" => Some((host.serial.api_activate_tab(params), false)),
        "ui.serial.name" => Some((host.serial.api_set_live_name(*host.lang, params), false)),
        "ui.serial.browser" => Some((host.serial.api_set_browser_dir(params), false)),
        "ui.wave.open" => Some((host.waveform.api_open(*host.lang, params), false)),
        "ui.wave.close" => Some((host.waveform.api_close(*host.lang), false)),
        "ui.wave.select" => Some((host.waveform.api_select(params), false)),
        "ui.wave.browser" => Some((host.waveform.api_set_browser_dir(params), false)),
        "ui.wave.bus" => Some((host.waveform.api_bus(params), false)),
        "ui.wave.cursor" => Some((host.waveform.api_cursors(params), false)),
        "ui.wave.fit" => Some((host.waveform.api_fit(), false)),
        "ui.calc.get" => Some((ok(method, host.calculator.api_get()), false)),
        "ui.calc.set" => Some((host.calculator.api_set(params), false)),
        "ui.instrument.select" => Some((host.instruments.api_select(params), false)),
        "ui.report.browser" => Some((host.test_report.api_set_browser_dir(params), true)),
        "ui.report.open" => Some(ui_report_open(host, params)),
        "ui.project.list" => Some((ui_project_list(host), false)),
        "ui.project.select" => Some(ui_project_select(host, params)),
        "ui.project.create" => Some(ui_project_create(host, params)),
        "ui.project.add_test" => Some(ui_project_add_test(host, params)),
        "ui.project.remove_test" => Some(ui_project_remove_test(host, params)),
        "ui.test_tool.browser" => Some((host.test_tool.api_set_plugins_dir(params), true)),
        "ui.test_tool.run" => Some((host.test_tool.api_run(params), false)),
        "ui.test_tool.stop" => Some((host.test_tool.api_stop(), false)),
        _ => None,
    }
}

fn snapshot(host: &UiHost<'_>) -> Value {
    let (port, baud) = host.serial.current_port_baud();
    json!({
        "active_tab": host.active.as_id(),
        "language": match *host.lang { Lang::Zh => "zh", Lang::En => "en" },
        "theme": host.cfg.ui.theme,
        "debug": host.cfg.ui.debug_mode,
        "panels": {
            "serial": *host.show_serial,
            "calculator": *host.show_calculator,
            "instruments": *host.show_instruments,
            "waveform": *host.show_waveform,
            "data_analysis": *host.show_data_analysis,
            "test_report": *host.show_test_report,
            "test_tool": *host.show_test_tool,
        },
        "serial": {
            "monitoring": host.serial.is_monitoring(),
            "port": port,
            "baud": baud,
            "status": host.serial.monitor_status_text(),
        },
        "instruments": {
            "devices": host.instruments.device_count(),
            "selected_id": host.instruments.selected_device_id(),
            "overview_open": host.instruments.overview_open(),
            "scanning": host.instruments.is_scanning(),
        },
        "waveform": host.waveform.api_snapshot(),
        "data_analysis": host.data_analysis.api_snapshot(),
        "test_report": host.test_report.api_snapshot(),
        "test_tool": host.test_tool.api_snapshot(),
        "calculator": host.calculator.api_get(),
    })
}

fn ui_state(host: &UiHost<'_>) -> InvokeReply {
    ok("ui.state", snapshot(host))
}

fn ui_show(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let tab = params
        .get("tab")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("page").and_then(|v| v.as_str()));
    let Some(id) = tab else {
        return (
            err(
                "ui.show",
                "missing tab (serial|calculator|instruments|waveform|data_analysis|test_report|test_tool)",
            ),
            false,
        );
    };
    let Some(next) = MainTab::from_id(id) else {
        return (err("ui.show", &format!("unknown tab '{id}'")), false);
    };
    match next {
        MainTab::Serial => *host.show_serial = true,
        MainTab::Calculator => *host.show_calculator = true,
        MainTab::Instruments => *host.show_instruments = true,
        MainTab::Waveform => *host.show_waveform = true,
        MainTab::DataAnalysis => *host.show_data_analysis = true,
        MainTab::TestReport => *host.show_test_report = true,
        MainTab::TestTool => *host.show_test_tool = true,
    }
    *host.active = next;
    (ok("ui.show", snapshot(host)), true)
}

fn ui_panels(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let mut any = false;
    if let Some(v) = params.get("serial").and_then(|x| x.as_bool()) {
        *host.show_serial = v;
        any = true;
    }
    if let Some(v) = params.get("calculator").and_then(|x| x.as_bool()) {
        *host.show_calculator = v;
        any = true;
    }
    if let Some(v) = params.get("instruments").and_then(|x| x.as_bool()) {
        *host.show_instruments = v;
        any = true;
    }
    if let Some(v) = params.get("waveform").and_then(|x| x.as_bool()) {
        *host.show_waveform = v;
        any = true;
    }
    if let Some(v) = params
        .get("data_analysis")
        .or_else(|| params.get("data"))
        .and_then(|x| x.as_bool())
    {
        *host.show_data_analysis = v;
        any = true;
    }
    if let Some(v) = params
        .get("test_report")
        .or_else(|| params.get("report"))
        .and_then(|x| x.as_bool())
    {
        *host.show_test_report = v;
        any = true;
    }
    if let Some(v) = params
        .get("test_tool")
        .or_else(|| params.get("tools"))
        .and_then(|x| x.as_bool())
    {
        *host.show_test_tool = v;
        any = true;
    }
    if !any {
        return (
            err(
                "ui.panels",
                "set at least one of serial/calculator/instruments/waveform/data_analysis/test_report/test_tool",
            ),
            false,
        );
    }
    if !*host.show_serial
        && !*host.show_calculator
        && !*host.show_instruments
        && !*host.show_waveform
        && !*host.show_data_analysis
        && !*host.show_test_report
        && !*host.show_test_tool
    {
        *host.show_serial = true;
    }
    (ok("ui.panels", snapshot(host)), true)
}

fn ui_prefs(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let mut any = false;
    if let Some(lang) = params.get("language").and_then(|v| v.as_str()) {
        *host.lang = parse_lang(lang);
        host.cfg.ui.language = match *host.lang {
            Lang::Zh => "zh".into(),
            Lang::En => "en".into(),
        };
        any = true;
    }
    if let Some(theme) = params.get("theme").and_then(|v| v.as_str()) {
        let t = theme.trim().to_ascii_lowercase();
        if t != "dark" && t != "light" {
            return (err("ui.prefs", "theme must be dark or light"), false);
        }
        host.cfg.ui.theme = t;
        any = true;
    }
    if let Some(debug) = params.get("debug").and_then(|v| v.as_bool()) {
        if host.cfg.ui.debug_mode != debug {
            host.cfg.ui.debug_mode = debug;
            host.instruments.apply_debug_mode(debug, *host.lang);
            if debug {
                *host.show_instruments = true;
                *host.active = MainTab::Instruments;
            }
        }
        any = true;
    }
    if !any {
        return (err("ui.prefs", "set language, theme, and/or debug"), false);
    }
    (ok("ui.prefs", snapshot(host)), true)
}

fn project_ctx<'a>(
    host: &'a mut UiHost<'_>,
) -> (std::path::PathBuf, Vec<wiparse_core::project::PluginMeta>) {
    (host.test_tool.data_root_path(), host.test_tool.plugin_metas())
}

fn ui_project_list(host: &mut UiHost<'_>) -> InvokeReply {
    let data_root = host.test_tool.data_root_path();
    let plugins = host.test_tool.plugin_metas();
    let id = host.test_report.project_id().to_owned();
    let _ = host.test_report.select_project(&data_root, &plugins, &id);
    let docs = wiparse_core::project::list_projects(&data_root);
    let list: Vec<Value> = docs
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "tests": d.tests.iter().map(|t| &t.id).collect::<Vec<_>>(),
            })
        })
        .collect();
    ok(
        "ui.project.list",
        json!({
            "active_id": host.test_report.project_id(),
            "projects": list,
            "tree": host.test_report.api_snapshot(),
        }),
    )
}

fn ui_project_select(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let Some(id) = params.get("id").and_then(|v| v.as_str()) else {
        return (err("ui.project.select", "missing id"), false);
    };
    let (data_root, plugins) = project_ctx(host);
    match host.test_report.select_project(&data_root, &plugins, id) {
        Ok(id) => {
            host.test_tool.set_project_id(id);
            (ok("ui.project.select", host.test_report.api_snapshot()), true)
        }
        Err(e) => (err("ui.project.select", &e), false),
    }
}

fn ui_project_create(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let Some(id) = params.get("id").and_then(|v| v.as_str()) else {
        return (err("ui.project.create", "missing id"), false);
    };
    let name = params.get("name").and_then(|v| v.as_str());
    let (data_root, plugins) = project_ctx(host);
    match host.test_report.create_project(&data_root, &plugins, id, name) {
        Ok(id) => {
            host.test_tool.set_project_id(id);
            (ok("ui.project.create", host.test_report.api_snapshot()), true)
        }
        Err(e) => (err("ui.project.create", &e), false),
    }
}

fn ui_project_add_test(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let Some(id) = params
        .get("id")
        .or_else(|| params.get("plugin"))
        .and_then(|v| v.as_str())
    else {
        return (err("ui.project.add_test", "missing id"), false);
    };
    let (data_root, plugins) = project_ctx(host);
    match host.test_report.add_hub_test(&data_root, &plugins, id) {
        Ok(()) => (ok("ui.project.add_test", host.test_report.api_snapshot()), true),
        Err(e) => (err("ui.project.add_test", &e), false),
    }
}

fn ui_project_remove_test(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let Some(id) = params
        .get("id")
        .or_else(|| params.get("plugin"))
        .and_then(|v| v.as_str())
    else {
        return (err("ui.project.remove_test", "missing id"), false);
    };
    let (data_root, plugins) = project_ctx(host);
    match host.test_report.remove_hub_test(&data_root, &plugins, id) {
        Ok(()) => (
            ok("ui.project.remove_test", host.test_report.api_snapshot()),
            true,
        ),
        Err(e) => (err("ui.project.remove_test", &e), false),
    }
}

fn ui_report_open(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
        return (err("ui.report.open", "missing path"), false);
    };
    let view = params.get("view").and_then(|v| v.as_str());
    let overlay = params.get("overlay").and_then(|v| v.as_bool()).unwrap_or(false);
    let data_root = host.test_tool.data_root_path();
    let pb = std::path::Path::new(path);
    match host.test_report.prepare_open(&data_root, pb, view) {
        Ok((path, view)) => {
            use wiparse_core::project::ArtifactView;
            let meta = json!({
                "path": path.display().to_string(),
                "view": view.as_str(),
                "overlay": overlay,
                "project": host.test_report.project_id(),
            });
            match view {
                ArtifactView::Log => {
                    *host.show_test_report = true;
                    *host.active = MainTab::TestReport;
                    host.test_report.open_in_pane(path, view, overlay, *host.lang);
                }
                ArtifactView::Wave => {
                    let _ = host.waveform.open_path_maybe_overlay(&path, *host.lang, overlay);
                    *host.show_test_report = true;
                    *host.active = MainTab::TestReport;
                    host.test_report.open_in_pane(path, view, overlay, *host.lang);
                }
                ArtifactView::Report => {
                    *host.show_test_report = true;
                    *host.active = MainTab::TestReport;
                    host.test_report.open_markdown(path, *host.lang);
                }
                ArtifactView::External => {
                    let _ = crate::test_report::open_path_external(&path);
                }
            }
            (ok("ui.report.open", meta), true)
        }
        Err(e) => (err("ui.report.open", &e), false),
    }
}
