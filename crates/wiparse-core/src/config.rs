//! Application config with deep-merge defaults (mirrors Python `config.py`).

use crate::i18n::normalize_language;
use crate::paths::{config_file, default_config_file};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub system: SystemConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub alerts: AlertsConfig,
    #[serde(default)]
    pub serial: SerialConfig,
    #[serde(default)]
    pub log_monitor: LogMonitorConfig,
    #[serde(default)]
    pub apps: AppsConfig,
    #[serde(default)]
    pub update: UpdateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    #[serde(default = "default_db_name")]
    pub db_name: String,
    #[serde(default = "default_log_file")]
    pub log_file: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_commit_interval")]
    pub db_commit_interval_sec: f64,
    #[serde(default = "default_batch")]
    pub db_commit_batch_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub panels: PanelFlags,
    #[serde(default = "default_chart_points")]
    pub chart_max_points: u32,
    /// When true, the instrument workbench shows every instrument card (and demo sessions).
    #[serde(default)]
    pub debug_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelFlags {
    #[serde(default = "default_true")]
    pub serial_tool: bool,
    /// `waveform_scope` is accepted for migration from pre-calculator configs.
    #[serde(default = "default_true", alias = "waveform_scope")]
    pub calculator: bool,
    #[serde(default = "default_true", alias = "tektronix_scope")]
    pub instrument_control: bool,
    #[serde(default = "default_true")]
    pub waveform_analysis: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertsConfig {
    #[serde(default = "default_temp_warn")]
    pub temp_warning_threshold: f64,
    #[serde(default = "default_ovp")]
    pub ovp_threshold: f64,
    #[serde(default = "default_ocp")]
    pub ocp_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    #[serde(default = "default_bauds")]
    pub default_baudrates: Vec<String>,
    #[serde(default)]
    pub demo_mode: bool,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default = "default_reconnect_sec")]
    pub reconnect_interval_sec: f64,
    #[serde(default = "default_max_reconnect")]
    pub max_reconnect_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMonitorConfig {
    #[serde(default = "default_log_name")]
    pub default_filename: String,
    #[serde(default = "default_log_dir")]
    pub save_dir: String,
    #[serde(default = "default_ext")]
    pub file_extension: String,
    /// Absolute paths of file tabs to restore on next launch (Python parity).
    #[serde(default)]
    pub open_log_files: Vec<String>,
    /// Directory last used by the Open Log dialog.
    #[serde(default)]
    pub last_open_dir: String,
    /// Root directory for the serial sidebar log-folder browser (subfolders with `.txt`).
    #[serde(default)]
    pub log_browser_dir: String,
    /// When true, live serial RX is written to the save-dir file (flushed every 5s).
    #[serde(default = "default_true")]
    pub save_live_to_disk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppsConfig {
    #[serde(default)]
    pub tektronix_scope: TektronixScopeConfig,
    #[serde(default)]
    pub instruments: InstrumentControlConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TektronixScopeConfig {
    #[serde(default = "default_scope_dir")]
    pub save_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentControlConfig {
    #[serde(default)]
    pub visa_library: String,
    #[serde(default = "default_instrument_timeout")]
    pub timeout_ms: u32,
    #[serde(default = "default_sample_interval")]
    pub sample_interval_ms: u64,
    #[serde(default = "default_instrument_points")]
    pub max_points: usize,
    #[serde(default = "default_instrument_dir")]
    pub save_dir: String,
    #[serde(default)]
    pub known_tcpip_resources: Vec<String>,
    /// Root directory for the waveform-analysis sidebar folder browser
    /// (first-level subfolders containing CSV / ISF / TXT sources).
    #[serde(default)]
    pub waveform_browser_dir: String,
    /// Default directory for `instrument.waveform_source` (ISF), independent of
    /// `waveform_browser_dir`. Empty = caller must pass `dir`.
    #[serde(default)]
    pub waveform_source_dir: String,
}

/// Online update settings (HTTPS manifest + optional auto-check).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// HTTPS URL to `latest.json` (channel manifest).
    #[serde(default = "default_update_manifest_url")]
    pub manifest_url: String,
    #[serde(default = "default_update_channel")]
    pub channel: String,
    /// Background check interval; `0` = startup only.
    #[serde(default = "default_update_check_hours")]
    pub check_interval_hours: u32,
    /// Download in background when an update is found (user still confirms install).
    #[serde(default)]
    pub auto_download: bool,
}

fn default_update_manifest_url() -> String {
    String::new()
}
fn default_update_channel() -> String {
    "stable".into()
}
fn default_update_check_hours() -> u32 {
    24
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            manifest_url: default_update_manifest_url(),
            channel: default_update_channel(),
            check_interval_hours: default_update_check_hours(),
            auto_download: false,
        }
    }
}

fn default_db_name() -> String {
    "charging_data.db".into()
}
fn default_log_file() -> String {
    "monitor.log".into()
}
fn default_log_level() -> String {
    "INFO".into()
}
fn default_commit_interval() -> f64 {
    1.0
}
fn default_batch() -> u32 {
    100
}
fn default_lang() -> String {
    "en".into()
}
fn default_theme() -> String {
    "dark".into()
}
fn default_chart_points() -> u32 {
    500
}
fn default_true() -> bool {
    true
}
fn default_temp_warn() -> f64 {
    60.0
}
fn default_ovp() -> f64 {
    25.0
}
fn default_ocp() -> f64 {
    3.0
}
fn default_bauds() -> Vec<String> {
    vec!["115200".into(), "1000000".into(), "2000000".into()]
}
fn default_reconnect_sec() -> f64 {
    3.0
}
fn default_max_reconnect() -> u32 {
    5
}
fn default_log_name() -> String {
    "Live Packet Log".into()
}
fn default_log_dir() -> String {
    "log".into()
}
fn default_ext() -> String {
    "txt".into()
}
fn default_scope_dir() -> String {
    "scope_captures".into()
}
fn default_instrument_timeout() -> u32 {
    30_000
}
fn default_sample_interval() -> u64 {
    1_000
}
fn default_instrument_points() -> usize {
    10_000
}
fn default_instrument_dir() -> String {
    "instrument_data".into()
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            db_name: default_db_name(),
            log_file: default_log_file(),
            log_level: default_log_level(),
            db_commit_interval_sec: default_commit_interval(),
            db_commit_batch_size: default_batch(),
        }
    }
}

impl Default for PanelFlags {
    fn default() -> Self {
        Self {
            serial_tool: true,
            calculator: true,
            instrument_control: true,
            waveform_analysis: true,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: default_lang(),
            theme: default_theme(),
            panels: PanelFlags::default(),
            chart_max_points: default_chart_points(),
            debug_mode: false,
        }
    }
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            temp_warning_threshold: default_temp_warn(),
            ovp_threshold: default_ovp(),
            ocp_threshold: default_ocp(),
        }
    }
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            default_baudrates: default_bauds(),
            demo_mode: false,
            auto_reconnect: true,
            reconnect_interval_sec: default_reconnect_sec(),
            max_reconnect_attempts: default_max_reconnect(),
        }
    }
}

impl Default for LogMonitorConfig {
    fn default() -> Self {
        Self {
            default_filename: default_log_name(),
            save_dir: default_log_dir(),
            file_extension: default_ext(),
            open_log_files: Vec::new(),
            last_open_dir: String::new(),
            log_browser_dir: String::new(),
            save_live_to_disk: true,
        }
    }
}

impl Default for TektronixScopeConfig {
    fn default() -> Self {
        Self {
            save_dir: default_scope_dir(),
        }
    }
}

impl Default for InstrumentControlConfig {
    fn default() -> Self {
        Self {
            visa_library: String::new(),
            timeout_ms: default_instrument_timeout(),
            sample_interval_ms: default_sample_interval(),
            max_points: default_instrument_points(),
            save_dir: default_instrument_dir(),
            known_tcpip_resources: Vec::new(),
            waveform_browser_dir: String::new(),
            waveform_source_dir: String::new(),
        }
    }
}

impl Default for AppsConfig {
    fn default() -> Self {
        Self {
            tektronix_scope: TektronixScopeConfig::default(),
            instruments: InstrumentControlConfig::default(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig::default(),
            ui: UiConfig::default(),
            alerts: AlertsConfig::default(),
            serial: SerialConfig::default(),
            log_monitor: LogMonitorConfig::default(),
            apps: AppsConfig::default(),
            update: UpdateConfig::default(),
        }
    }
}

/// Deep-merge JSON objects (`override` wins on leaf keys).
pub fn deep_merge(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = Map::new();
            for (k, v) in b {
                out.insert(k.clone(), v.clone());
            }
            for (k, v) in o {
                if let Some(existing) = out.get(k) {
                    out.insert(k.clone(), deep_merge(existing, v));
                } else {
                    out.insert(k.clone(), v.clone());
                }
            }
            Value::Object(out)
        }
        (_, o) => o.clone(),
    }
}

fn default_config_value() -> Value {
    serde_json::to_value(AppConfig::default()).unwrap_or_else(|_| json!({}))
}

fn migrate_legacy_panel_key(value: &mut Value) {
    let Some(panels) = value
        .pointer_mut("/ui/panels")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(legacy) = panels.remove("waveform_scope") {
        panels.entry("calculator").or_insert(legacy);
    }
    if let Some(legacy) = panels.remove("tektronix_scope") {
        panels.entry("instrument_control").or_insert(legacy);
    }
}

pub fn load_config() -> Result<AppConfig, ConfigError> {
    let mut merged = default_config_value();

    // Optional packaged defaults file
    if let Ok(text) = fs::read_to_string(default_config_file()) {
        if let Ok(mut file_cfg) = serde_json::from_str::<Value>(&text) {
            migrate_legacy_panel_key(&mut file_cfg);
            merged = deep_merge(&merged, &file_cfg);
        }
    }

    let path = config_file();
    if path.is_file() {
        let text = fs::read_to_string(&path)?;
        let mut file_cfg: Value = serde_json::from_str(&text)?;
        migrate_legacy_panel_key(&mut file_cfg);
        merged = deep_merge(&merged, &file_cfg);
    }

    let mut cfg: AppConfig = serde_json::from_value(merged)?;
    cfg.ui.language = normalize_language(&cfg.ui.language).to_string();
    Ok(cfg)
}

pub fn save_config(cfg: &AppConfig) -> Result<(), ConfigError> {
    let path = config_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg)?;
    fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge_nested() {
        let base = json!({"a": {"x": 1, "y": 2}, "b": 3});
        let over = json!({"a": {"y": 9}, "c": 4});
        let m = deep_merge(&base, &over);
        assert_eq!(m["a"]["x"], 1);
        assert_eq!(m["a"]["y"], 9);
        assert_eq!(m["c"], 4);
    }

    #[test]
    fn default_loads() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.ui.theme, "dark");
        assert!(cfg.ui.panels.serial_tool);
        assert!(cfg.ui.panels.calculator);
    }

    #[test]
    fn migrates_legacy_waveform_panel_flag() {
        let mut value = json!({"ui": {"panels": {"waveform_scope": true}}});
        migrate_legacy_panel_key(&mut value);
        assert_eq!(value["ui"]["panels"]["calculator"], true);
        assert!(value["ui"]["panels"].get("waveform_scope").is_none());
    }

    #[test]
    fn migrates_tektronix_panel_to_instrument_control() {
        let mut value = json!({"ui": {"panels": {"tektronix_scope": false}}});
        migrate_legacy_panel_key(&mut value);
        assert_eq!(value["ui"]["panels"]["instrument_control"], false);
        assert!(value["ui"]["panels"].get("tektronix_scope").is_none());
    }

    #[test]
    fn explicit_calculator_false_survives_roundtrip() {
        let mut cfg = AppConfig::default();
        cfg.ui.panels.calculator = false;
        let json = serde_json::to_string(&cfg).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert!(!restored.ui.panels.calculator);
    }

    #[test]
    fn missing_calculator_field_defaults_visible() {
        let flags: PanelFlags = serde_json::from_value(json!({
            "serial_tool": true,
            "tektronix_scope": true
        }))
        .unwrap();
        assert!(flags.calculator);
    }
}
