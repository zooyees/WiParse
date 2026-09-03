//! GUI page switch / control handlers (stateful, UI thread).

use super::dispatch::{err, ok};
use super::InvokeReply;
use crate::app::MainTab;
use crate::calculator::CalculatorPanel;
use crate::instrument_control::InstrumentControlPanel;
use crate::serial_tool::SerialToolPanel;
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
    pub serial: &'a mut SerialToolPanel,
    pub instruments: &'a mut InstrumentControlPanel,
    pub calculator: &'a mut CalculatorPanel,
    pub waveform: &'a mut WaveformAnalysisPanel,
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
        },
        "waveform": host.waveform.api_snapshot(),
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
        return (err("ui.show", "missing tab (serial|calculator|instruments|waveform)"), false);
    };
    let Some(next) = MainTab::from_id(id) else {
        return (err("ui.show", &format!("unknown tab '{id}'")), false);
    };
    match next {
        MainTab::Serial => *host.show_serial = true,
        MainTab::Calculator => *host.show_calculator = true,
        MainTab::Instruments => *host.show_instruments = true,
        MainTab::Waveform => *host.show_waveform = true,
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
    if !any {
        return (err("ui.panels", "set at least one of serial/calculator/instruments/waveform"), false);
    }
    if !*host.show_serial && !*host.show_calculator && !*host.show_instruments && !*host.show_waveform
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
