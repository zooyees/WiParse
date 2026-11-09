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

static NAVIGATION_REVISION:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
static NAVIGATION:std::sync::Mutex<Option<Value>>=std::sync::Mutex::new(None);
fn navigation_state()->Value {
    NAVIGATION.lock().ok().and_then(|current|current.clone()).unwrap_or_else(||json!({"owner":std::process::id(),"revision":0,"state":"idle","tab":""}))
}
fn navigation_ack(params:&Value)->InvokeReply {
    let method="ui.navigation.ack_safe";let Some(state)=params["state"].as_str().filter(|state|matches!(*state,"applied"|"blocked")) else{return err(method,"GUI.INVALID_NAVIGATION")};
    let Ok(mut guard)=NAVIGATION.lock() else{return err(method,"GUI.NAVIGATION_UNAVAILABLE")};let Some(current)=guard.as_mut() else{return err(method,"GUI.NAVIGATION_CHANGED")};
    if current["owner"]!=params["owner"]||current["revision"]!=params["revision"]{return err(method,"GUI.NAVIGATION_CHANGED");}
    current["state"]=json!(state);current["reason"]=if state=="blocked"{json!("UNSAVED_EDIT_OR_OPERATION")}else{Value::Null};ok(method,current.clone())
}

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
    pub ai: &'a mut crate::ai::AiPanel,
}

pub fn handle(host: &mut UiHost<'_>, method: &str, params: &Value) -> Option<(InvokeReply, bool)> {
    match method {
        "system.ui.state" | "ui.state" => {host.test_tool.api_poll(*host.lang);Some((ui_state(host), false))},
        "ui.show" => Some(ui_show(host, params)),
        "ui.panels" => Some(ui_panels(host, params)),
        "ui.prefs" => Some(ui_prefs(host, params)),
        "ui.navigation.get_safe" => Some((ok(method,navigation_state()),false)),
        "ui.navigation.ack_safe" => Some((navigation_ack(params),false)),
        "ui.preferences.get_safe" => Some((ok(method,preferences_snapshot(host.cfg)),false)),
        "ui.preferences.set_safe" => Some((preferences_save(host,params),false)),
        "ui.instrument.io_log.get_safe" | "ui.instrument.io_log.clear_safe" => Some((host.instruments.api_io_log(method,params),false)),
        "ui.ai.page_summary_safe" => {let summary=match params["page"].as_str(){Some("serial")=>host.serial.ai_summary(),Some("instruments")=>host.instruments.ai_summary(),Some("calculator")=>host.calculator.ai_summary(),Some("tests")=>host.test_tool.ai_summary(),_=>return Some((err(method,"AI.NATIVE_CONTEXT_REQUIRED"),false))};Some((ok(method,summary),false))},
        "ui.runtime.idle_safe" => Some((if host.test_tool.runtime_busy()||host.serial.runtime_busy()||host.instruments.api_operations_pending()||host.ai.runtime_busy(){err(method,"RUNTIME.ACTIVE_WORK")}else{ok(method,json!({"idle":true}))},false)),
        "ui.market.get_safe" => Some((ok(method,host.test_tool.api_market_snapshot(*host.lang)),false)),
        "ui.market.action_safe" => Some((host.test_tool.api_market_action(params,*host.lang),false)),
        "ui.market.configure_safe" => Some((market_configure(host,params),false)),
        "ui.offline.browser_root_safe" => Some((offline_browser_root(host,params),false)),
        "ui.data.preferences.get_safe" => Some((ok(method,data_preferences_snapshot(host.cfg)),false)),
        "ui.data.preferences.set_safe" => Some((data_preferences_save(host,params),false)),
        "ui.serial.open" => Some((host.serial.api_open_log(params), false)),
        "ui.serial.open_safe" => Some((host.serial.api_open_log_safe(params), false)),
        "ui.serial.close_safe" => Some((host.serial.api_close_tab_safe(params), false)),
        "ui.serial.close" => Some((host.serial.api_close_tab(params), false)),
        "ui.serial.clear" => Some((host.serial.api_clear(), false)),
        "ui.serial.edit_safe" => {
            let reply=if (host.test_tool.running_plugin_id().is_some()||host.serial.is_test_running())&&!matches!(params["action"].as_str(),Some("read"|"discard")){err(method,"RUN.STATION_BUSY")}else{host.serial.api_edit_safe(params)};
            if reply.ok&&params["action"]=="begin"{*host.active=MainTab::Serial;*host.show_serial=true;}
            Some((reply,false))
        },
        "ui.serial.new_log_safe" => Some((if host.test_tool.running_plugin_id().is_some(){err(method,"RUN.STATION_BUSY")}else{host.serial.api_new_live_log_safe(*host.lang,params)},false)),
        "ui.serial.clear_live_safe" => Some((if host.test_tool.running_plugin_id().is_some(){err(method,"RUN.STATION_BUSY")}else{host.serial.api_clear_live_safe(params)},false)),
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
        "ui.ai.settings.get_safe" => Some((host.ai.api_settings_get(host.cfg),false)),
        "ui.ai.settings.set_safe" => Some((if host.test_tool.running_plugin_id().is_some(){err(method,"RUN.STATION_BUSY")}else{host.ai.api_settings_set(host.cfg,params)},false)),
        "ui.ai.test_safe" => Some((if host.test_tool.running_plugin_id().is_some(){err(method,"RUN.STATION_BUSY")}else{host.ai.api_connection_test(host.cfg,params)},false)),
        "ui.ai.chat.submit_safe" => Some((host.ai.api_chat_submit(host.cfg,params),false)),
        "ui.ai.chat.state_safe" => Some((host.ai.api_chat_state(params),false)),
        "ui.instrument.select" => Some((host.instruments.api_select(params), false)),
        "ui.report.browser" => Some((host.test_report.api_set_browser_dir(params), true)),
        "ui.report.open" => Some(ui_report_open(host, params)),
        "ui.project.list" => Some((ui_project_list(host), false)),
        "ui.project.list_safe" => Some((ui_project_list_safe(host), false)),
        "ui.project.select_safe" => Some(ui_project_select_safe(host,params)),
        "ui.project.create_safe" => Some(ui_project_create_safe(host,params)),
        "ui.project.select" => Some(ui_project_select(host, params)),
        "ui.project.create" => Some(ui_project_create(host, params)),
        "ui.project.add_test" => Some(ui_project_add_test(host, params)),
        "ui.project.remove_test" => Some(ui_project_remove_test(host, params)),
        "ui.project.remove_test_safe" => Some(ui_project_remove_test_safe(host, params)),
        "ui.project.add_test_safe" => Some(ui_project_add_test_safe(host, params)),
        "ui.test_tool.browser" => Some((host.test_tool.api_set_plugins_dir(params), true)),
        "ui.test_tool.execution_log_safe" => Some((host.test_tool.api_execution_log(params),false)),
        "ui.test_tool.state_safe" => {host.test_tool.api_poll(*host.lang);Some((ok(method,host.test_tool.api_snapshot()),false))},
        "ui.test_tool.run" => Some((host.test_tool.api_run(params), false)),
        "ui.test_tool.select_safe" => Some((host.test_tool.api_select_plugin_safe(params), false)),
        "ui.test_tool.generic_run_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else if host.serial.is_test_running(){err(method,"RUN.STATION_BUSY")}else{host.test_tool.api_generic_run_safe(params,&host.instruments.live_devices_for_hub())}, false)),
        "ui.test_tool.generic_stop_safe" => Some((host.test_tool.api_generic_stop_safe(params), false)),
        "ui.test_tool.scope_rules_replace_safe" => Some((host.test_tool.api_scope_rules_replace_safe(params),false)),
        "ui.test_tool.scope_rule_enabled_safe" => Some((host.test_tool.api_scope_rule_enabled_safe(params), false)),
        "ui.test_tool.scope_rule_add_safe" => Some((host.test_tool.api_scope_rule_add_safe(params), false)),
        "ui.test_tool.scope_rule_text_safe" => Some((host.test_tool.api_scope_rule_text_safe(params), false)),
        "ui.test_tool.scope_rule_exclusion_safe" => Some((host.test_tool.api_scope_rule_exclusion_safe(params), false)),
        "ui.test_tool.scope_rule_order_safe" => Some((host.test_tool.api_scope_rule_order_safe(params), false)),
        "ui.test_tool.scope_settings_import_safe" => Some((host.test_tool.api_scope_settings_import_safe(params), false)),
        "ui.test_tool.scope_settings_export_safe" => Some((host.test_tool.api_scope_settings_export_safe(params), false)),
        "ui.test_tool.scope_edge_mode_safe" => Some((host.test_tool.api_scope_edge_mode_safe(params), false)),
        "ui.test_tool.scope_serial_settings_safe" => Some((host.test_tool.api_scope_serial_settings_safe(params), false)),
        "ui.test_tool.scope_instrument_safe" => {
            let inventory=host.instruments.api_list(*host.lang,&json!({}));
            let id=params.get("device_id").and_then(Value::as_u64);let resource=params.get("resource").and_then(Value::as_str);
            let device=inventory.get("devices").and_then(Value::as_array).and_then(|devices|devices.iter().find(|device|device.get("device_id").and_then(Value::as_u64)==id&&device.get("resource").and_then(Value::as_str)==resource&&device.get("kind").and_then(Value::as_str)==Some("oscilloscope")));
            match device {
                Some(device) if params.is_object()=>{let mut checked=params.clone();checked["model"]=json!(device.pointer("/identity/model").and_then(Value::as_str).unwrap_or(""));Some((host.test_tool.api_scope_instrument_safe(&checked),false))},
                _=>Some((err(method,"INSTRUMENT.SESSION_CHANGED"),false)),
            }
        },
        "ui.test_tool.scope_preflight_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else{host.test_tool.api_scope_preflight_safe(params)}, false)),
        "ui.test_tool.scope_run_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else if host.serial.is_test_running(){err(method,"RUN.STATION_BUSY")}else{host.test_tool.api_scope_run_safe(params)}, false)),
        "ui.test_tool.scope_stop_safe" => Some((host.test_tool.api_scope_stop_safe(params), false)),
        "ui.test_tool.preflight_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else{host.test_tool.api_fpga_preflight_safe(params)}, false)),
        "ui.test_tool.run_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else{host.test_tool.api_fpga_run_safe(params)}, false)),
        "ui.test_tool.stop_safe" => Some((host.test_tool.api_fpga_stop_safe(params), false)),
        "ui.test_tool.wireless_preflight_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else{host.test_tool.api_wireless_lifecycle_safe(params,false)},false)),
        "ui.test_tool.wireless_run_safe" => Some((if host.instruments.api_operations_pending(){err(method,"INSTRUMENT.OPERATION_IN_PROGRESS")}else{host.test_tool.api_wireless_lifecycle_safe(params,true)},false)),
        "ui.test_tool.wireless_stop_safe" => Some((host.test_tool.api_wireless_stop_safe(params),false)),
        "ui.test_tool.stop" => Some((host.test_tool.api_stop(), false)),
        _ => None,
    }
}

fn snapshot(host: &UiHost<'_>) -> Value {
    let (port, baud) = host.serial.current_port_baud();
    json!({
        "active_tab": host.active.as_id(),
        "navigation":navigation_state(),
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
            "safe_control_version": 1,
            "safe_recording_version": 1,
            "safe_recording_settings_version":1,
            "safe_log_file_version": 1,
            "safe_live_clear_version": 1,
            "safe_new_log_version":1,
            "monitoring": host.serial.is_monitoring(),
            "port": port,
            "baud": baud,
            "status": host.serial.monitor_status_text(),
        },
        "instruments": {
            "safe_scan_version": 1,
            "safe_connect_version": 1,
            "safe_disconnect_version": 1,
            "safe_measure_version": 1,
            "safe_scope_action_version": 1,
            "safe_typed_control_version": 1,
            "safe_acquisition_version":1,
            "safe_source_action_version": 1,
            "safe_load_action_version": 1,
            "safe_capture_version": 1,
            "safe_waveform_source_version": 1,
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
    let revision=NAVIGATION_REVISION.fetch_add(1,std::sync::atomic::Ordering::AcqRel)+1;
    if let Ok(mut guard)=NAVIGATION.lock(){*guard=Some(json!({"owner":std::process::id(),"revision":revision,"tab":next.as_id(),"state":if crate::companion_window(){"requested"}else{"applied"}}));}
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

fn ui_project_list_safe(host:&UiHost<'_>)->InvokeReply {
    let root=host.test_tool.data_root_path();
    let projects=wiparse_core::project::list_projects(&root).into_iter().map(|d|json!({"id":d.id,"name":d.name})).collect::<Vec<_>>();
    ok("ui.project.list_safe",json!({"active_id":host.test_tool.current_project_id(),"projects":projects}))
}

fn ui_project_select_safe(host:&mut UiHost<'_>,params:&Value)->(InvokeReply,bool){
    let method="ui.project.select_safe";
    let Some(id)=params.get("id").and_then(Value::as_str).filter(|id|wiparse_core::project::is_valid_id(id)) else{return (err(method,"PROJECT.INVALID_ID"),false)};
    if let Some(blocker)=host.test_tool.project_switch_blocker(){return (err(method,blocker),false);}
    let root=host.test_tool.data_root_path();
    if !wiparse_core::project::list_projects(&root).iter().any(|project|project.id==id){return (err(method,"PROJECT.NOT_FOUND"),false);}
    let plugins=host.test_tool.plugin_metas();
    match host.test_report.select_project(&root,&plugins,id){
        Ok(selected)=>{host.test_tool.set_project_id(selected);(ok(method,json!({"selected":host.test_tool.current_project_id()})),true)},
        Err(error)=>(err(method,&error),false),
    }
}

fn ui_project_create_safe(host:&mut UiHost<'_>,params:&Value)->(InvokeReply,bool){
    let method="ui.project.create_safe";
    let Some(id)=params.get("id").and_then(Value::as_str).filter(|id|wiparse_core::project::is_valid_id(id)) else{return (err(method,"PROJECT.INVALID_ID"),false)};
    let Some(name)=params.get("name").and_then(Value::as_str).filter(|name|!name.trim().is_empty()&&name.len()<=512&&!name.chars().any(char::is_control)) else{return (err(method,"PROJECT.INVALID_NAME"),false)};
    if let Some(blocker)=host.test_tool.project_switch_blocker(){return (err(method,blocker),false);}
    let root=host.test_tool.data_root_path();
    match wiparse_core::project::create_project_new(&root,id,Some(name)) {
        Ok(_)=>{},Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>return (err(method,"PROJECT.ALREADY_EXISTS"),false),
        Err(error) if error.kind()==std::io::ErrorKind::InvalidInput=>return (err(method,"PROJECT.INVALID_ID"),false),
        Err(error)=>return (err(method,&format!("PROJECT.CREATE_FAILED: {error}; 请刷新并检查目标目录")),false),
    }
    let plugins=host.test_tool.plugin_metas();
    match host.test_report.select_project(&root,&plugins,id){
        Ok(selected)=>{host.test_tool.set_project_id(selected);(ok(method,json!({"selected":host.test_tool.current_project_id()})),true)},
        Err(error)=>(err(method,&format!("PROJECT.CREATED_SELECTION_FAILED: {error}")),false),
    }
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

fn ui_project_add_test_safe(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let method = "ui.project.add_test_safe";
    let Some(project) = params.get("project").and_then(Value::as_str).filter(|id| {
        wiparse_core::project::is_valid_id(id) && id.len() <= 128
    }) else {
        return (err(method, "PROJECT.INVALID_ID"), false);
    };
    let Some(plugin_id) = params.get("plugin_id").and_then(Value::as_str).filter(|id| {
        wiparse_core::project::is_valid_id(id) && id.len() <= 128
    }) else {
        return (err(method, "PLUGIN.INVALID_ID"), false);
    };
    if host.test_tool.current_project_id() != project {
        return (err(method, "PROJECT.CHANGED"), false);
    }
    let plugins = host.test_tool.plugin_metas();
    let Some(plugin) = plugins.iter().find(|plugin| plugin.id == plugin_id) else {
        return (err(method, "PLUGIN.NOT_INSTALLED"), false);
    };
    let entry = wiparse_core::project::test_ref_from_report(&plugin.id, plugin.report_slug());
    if !wiparse_core::project::is_valid_id(&entry.id) || entry.id.len() > 128 {
        return (err(method, "TEST.INVALID_ID"), false);
    }
    if host.test_tool.running_test_id() == Some(entry.id.as_str()) {
        return (err(method, "RUN.TEST_BUSY"), false);
    }
    let data_root = host.test_tool.data_root_path();
    if host.test_report.project_id() != project {
        if let Err(error) = host.test_report.select_project(&data_root, &plugins, project) {
            return (err(method, &error), false);
        }
    }
    match host.test_report.add_report_test(&data_root, &plugins, entry.clone()) {
        Ok(()) => (ok(method, json!({"project": project, "id": entry.id, "plugin_id": plugin_id})), true),
        Err(error) => (err(method, &error), false),
    }
}

fn ui_project_remove_test_safe(host: &mut UiHost<'_>, params: &Value) -> (InvokeReply, bool) {
    let method = "ui.project.remove_test_safe";
    let Some(project) = params.get("project").and_then(Value::as_str).filter(|id| {
        wiparse_core::project::is_valid_id(id) && id.len() <= 128
    }) else {
        return (err(method, "PROJECT.INVALID_ID"), false);
    };
    let Some(id) = params.get("id").and_then(Value::as_str).filter(|id| {
        wiparse_core::project::is_valid_id(id) && id.len() <= 128
    }) else {
        return (err(method, "TEST.INVALID_ID"), false);
    };
    if host.test_tool.current_project_id() != project {
        return (err(method, "PROJECT.CHANGED"), false);
    }
    if host.test_tool.running_test_id() == Some(id) {
        return (err(method, "RUN.TEST_BUSY"), false);
    }
    let data_root = host.test_tool.data_root_path();
    let plugins = host.test_tool.plugin_metas();
    if host.test_report.project_id() != project {
        if let Err(error) = host.test_report.select_project(&data_root, &plugins, project) {
            return (err(method, &error), false);
        }
    }
    match host.test_report.remove_hub_test(&data_root, &plugins, id) {
        Ok(()) => (ok(method, json!({"project": project, "removed": id})), true),
        Err(error) => (err(method, &error), false),
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

fn preferences_fields(cfg:&AppConfig)->Value {
    json!({"theme":cfg.ui.theme,"baudrates":cfg.serial.default_baudrates,"debug_mode":cfg.ui.debug_mode,"language":cfg.ui.language})
}

fn market_configure(host:&mut UiHost<'_>,params:&Value)->InvokeReply{
    let method="ui.market.configure_safe";let snapshot=host.test_tool.api_market_snapshot(*host.lang);
    if snapshot["busy"]==true||host.test_tool.running_plugin_id().is_some(){return err(method,"MARKET.BUSY");}
    if params["revision"]!=snapshot["revision"]{return err(method,"MARKET.STALE");}
    let Some(fields)=params["fields"].as_object().filter(|fields|fields.len()==4&&fields.keys().all(|key|matches!(key.as_str(),"enabled"|"base_url"|"channel"|"install_dir")))else{return err(method,"MARKET.INVALID_SETTINGS")};
    let Some(enabled)=fields["enabled"].as_bool()else{return err(method,"MARKET.INVALID_SETTINGS")};
    let Some(url)=fields["base_url"].as_str().filter(|value|value.len()<=4096&&!value.chars().any(char::is_control))else{return err(method,"MARKET.INVALID_SETTINGS")};
    if !url.is_empty()&&(!url.starts_with("https://")&&!wiparse_core::marketplace::is_loopback_http_url(url)||url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").contains('@')){return err(method,"MARKET.INVALID_URL");}
    let Some(channel)=fields["channel"].as_str().filter(|channel|wiparse_core::marketplace::CHANNELS.contains(channel))else{return err(method,"MARKET.INVALID_CHANNEL")};
    let Some(directory)=fields["install_dir"].as_str().filter(|directory|!directory.is_empty()&&directory.len()<=4096&&!directory.chars().any(char::is_control))else{return err(method,"MARKET.INVALID_DIRECTORY")};
    let mut next=wiparse_core::config::load_config().unwrap_or_else(|_|host.cfg.clone());next.apps.test_tool.marketplace.enabled=enabled;next.apps.test_tool.marketplace.base_url=url.into();next.apps.test_tool.marketplace.channel=channel.into();next.apps.test_tool.marketplace.install_dir=directory.into();
    if let Err(error)=wiparse_core::config::save_config(&next){return err(method,&format!("MARKET.SAVE_FAILED: {error}"));}
    host.test_tool.api_market_apply_settings(&next);*host.cfg=next;ok(method,host.test_tool.api_market_snapshot(*host.lang))
}

fn offline_browser_root(host:&mut UiHost<'_>,params:&Value)->InvokeReply{
    let method="ui.offline.browser_root_safe";let scope=params["scope"].as_str().unwrap_or("");
    if !matches!(scope,"wave"|"data"){return err(method,"BROWSER.INVALID_SCOPE");}
    if let Some(directory)=params.get("directory"){
        let Some(directory)=directory.as_str().filter(|value|value.len()<=4096&&!value.contains('\0'))else{return err(method,"BROWSER.INVALID_PATH")};
        let Ok(path)=std::path::Path::new(directory).canonicalize()else{return err(method,"BROWSER.INVALID_PATH")};if !path.is_dir(){return err(method,"BROWSER.INVALID_PATH");}
        let root=wiparse_core::paths::app_root().canonicalize().unwrap_or_else(|_|wiparse_core::paths::app_root());let value=match path.strip_prefix(&root){Ok(relative) if relative.as_os_str().is_empty()=>".".into(),Ok(relative)=>relative.to_string_lossy().into_owned(),Err(_)=>path.to_string_lossy().into_owned()};
        let mut next=host.cfg.clone();if scope=="wave"{next.apps.instruments.waveform_browser_dir=value;}else{next.apps.data_analysis.browser_dir=value;}
        if let Err(error)=wiparse_core::config::save_config(&next){return err(method,&format!("BROWSER.SAVE_FAILED: {error}"));}*host.cfg=next;
    }
    let directory=if scope=="wave"{&host.cfg.apps.instruments.waveform_browser_dir}else{&host.cfg.apps.data_analysis.browser_dir};
    ok(method,json!({"directory":directory,"resolved":if directory.is_empty(){Value::Null}else{json!(wiparse_core::paths::project_path(directory))}}))
}

fn data_preferences_snapshot(cfg:&AppConfig)->Value{
    let mut fields=serde_json::to_value(&cfg.apps.data_analysis).unwrap_or(Value::Null);if let Some(fields)=fields.as_object_mut(){fields.remove("browser_dir");}
    json!({"revision":preferences_revision(&fields),"fields":fields})
}
fn data_preferences_save(host:&mut UiHost<'_>,params:&Value)->InvokeReply{
    let method="ui.data.preferences.set_safe";
    let snapshot=data_preferences_snapshot(host.cfg);
    if params["revision"]!=snapshot["revision"]{return err(method,"DATA.PREFERENCES_STALE");}
    let Some(fields)=params["fields"].as_object().filter(|fields|fields.len()<=20&&fields.keys().all(|key|snapshot["fields"].get(key).is_some()))else{return err(method,"DATA.INVALID_PREFERENCES")};
    let mut combined=snapshot["fields"].clone();for (key,value) in fields{combined[key]=value.clone();}
    let Ok(mut config)=serde_json::from_value::<wiparse_core::config::DataAnalysisConfig>(combined)else{return err(method,"DATA.INVALID_PREFERENCES")};
    if !(1..=8).contains(&config.series_count)||config.filters.len()>8||config.kinds.len()>8||config.frame_header.len()>256||config.filters.iter().any(|value|value.len()>256||value.contains('\0'))||config.kinds.iter().any(|value|!matches!(value.as_str(),"voltage"|"current"|"temperature"))||!matches!(config.source.as_str(),"file"|"serial")||config.browser_dir.len()>4096||config.browser_dir.contains('\0')||[config.threshold_voltage,config.threshold_current,config.threshold_temperature].iter().any(|value|!value.is_finite()||value.abs()>1e12){return err(method,"DATA.INVALID_PREFERENCES");}
    config.browser_dir=host.cfg.apps.data_analysis.browser_dir.clone();
    let mut next=host.cfg.clone();next.apps.data_analysis=config;
    if let Err(error)=wiparse_core::config::save_config(&next){return err(method,&format!("DATA.SAVE_FAILED: {error}"));}
    *host.cfg=next;ok(method,data_preferences_snapshot(host.cfg))
}
fn preferences_revision(fields:&Value)->String {
    use std::hash::{Hash,Hasher};let mut hash=std::collections::hash_map::DefaultHasher::new();
    fields.to_string().hash(&mut hash);format!("{:016x}",hash.finish())
}
fn preferences_snapshot(cfg:&AppConfig)->Value {
    let fields=preferences_fields(cfg);json!({"revision":preferences_revision(&fields),"fields":fields})
}
fn preferences_save(host:&mut UiHost<'_>,params:&Value)->InvokeReply {
    let method="ui.preferences.set_safe";
    if host.test_tool.running_plugin_id().is_some(){return err(method,"RUN.STATION_BUSY");}
    if params["revision"].as_str()!=Some(preferences_revision(&preferences_fields(host.cfg)).as_str()){return err(method,"PREFERENCES.STALE");}
    let Some(fields)=params["fields"].as_object().filter(|fields|(2..=4).contains(&fields.len())&&fields.keys().all(|key|matches!(key.as_str(),"theme"|"baudrates"|"debug_mode"|"language"))) else{return err(method,"PREFERENCES.INVALID_FIELDS")};
    let Some(theme)=fields.get("theme").and_then(Value::as_str).filter(|theme|matches!(*theme,"light"|"dark")) else{return err(method,"PREFERENCES.INVALID_THEME")};
    let Some(bauds)=fields.get("baudrates").and_then(Value::as_array).filter(|values|!values.is_empty()&&values.len()<=32) else{return err(method,"PREFERENCES.INVALID_BAUDRATES")};
    let mut next=host.cfg.clone();let mut baudrates=Vec::new();
    for value in bauds {let Some(value)=value.as_str().filter(|value|value.len()<=10&&value.parse::<u32>().is_ok_and(|value|value>0&&value<=12000000)) else{return err(method,"PREFERENCES.INVALID_BAUDRATES")};if !baudrates.iter().any(|stored|stored==value){baudrates.push(value.to_string());}}
    next.ui.theme=theme.to_string();next.serial.default_baudrates=baudrates;
    if let Some(value)=fields.get("language"){let Some(language)=value.as_str().filter(|language|matches!(*language,"zh"|"en")) else{return err(method,"PREFERENCES.INVALID_LANGUAGE")};next.ui.language=language.to_string();}

    if let Some(value)=fields.get("debug_mode") {let Some(enabled)=value.as_bool() else{return err(method,"PREFERENCES.INVALID_DEBUG_MODE")};if enabled!=host.cfg.ui.debug_mode&&(host.instruments.api_operations_pending()||host.serial.is_test_running()){return err(method,"INSTRUMENT.OPERATION_IN_PROGRESS");}next.ui.debug_mode=enabled;}

    if let Err(error)=wiparse_core::config::save_config(&next){return err(method,&format!("PREFERENCES.SAVE_FAILED: {error}"));}
    *host.lang=parse_lang(&next.ui.language);
    host.instruments.apply_debug_mode(next.ui.debug_mode,*host.lang);
    host.serial.api_apply_default_baudrates(&next.serial.default_baudrates);*host.cfg=next;
    ok(method,preferences_snapshot(host.cfg))
}
