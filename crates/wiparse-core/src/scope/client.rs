//! Tektronix USB scope client — mirrors Python `TektronixScopeClient`.

use crate::config::load_config;
use crate::paths::project_path;
use crate::scope::binary::{
    downsample_minmax, nearest_step, parse_ieee_block, SCALE_STEPS_S, SCALE_STEPS_V,
};
use crate::scope::visa::{Instrument, ResourceManager, VisaError};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const TEK_VID: u16 = 0x0699;

/// Match Tektronix USB resource strings across NI-VISA / TekVISA formatting variants.
fn is_tek_usb_resource(addr: &str) -> bool {
    let u = addr.to_uppercase();
    if !u.contains("USB") {
        return false;
    }
    u.contains("0X0699") || u.contains("0X699") || u.contains("::1689::") || u.contains("::0699::")
}

fn list_candidate_resources(rm: &ResourceManager) -> Result<(Vec<String>, String), ScopeError> {
    // Probe several expressions — NI-VISA returns INV_OBJECT for empty USB?*INSTR.
    let mut notes = Vec::new();
    let mut usb = Vec::new();
    let mut all = Vec::new();
    for expr in ["USB?*INSTR", "USB?*", "?*INSTR", "?*"] {
        match rm.list_resources(expr) {
            Ok(v) => {
                notes.push(format!("{expr}: {} found", v.len()));
                if expr.starts_with("USB") {
                    if !v.is_empty() && usb.is_empty() {
                        usb = v;
                    }
                } else if !v.is_empty() && all.is_empty() {
                    all = v;
                }
            }
            Err(e) => notes.push(format!("{expr}: {e}")),
        }
    }
    let best = if !usb.is_empty() { usb } else { all };
    Ok((best, notes.join("; ")))
}

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("{code}: {message}")]
    Coded {
        code: &'static str,
        message: String,
        hint: Option<String>,
    },
    #[error(transparent)]
    Visa(#[from] VisaError),
}

impl ScopeError {
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self::Coded {
            code,
            message: message.into(),
            hint: None,
        }
    }
    pub fn with_hint(
        code: &'static str,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self::Coded {
            code,
            message: message.into(),
            hint: Some(hint.into()),
        }
    }

    /// Message + hint for UI (Python panel logs both).
    pub fn user_message(&self) -> String {
        match self {
            Self::Coded {
                message,
                hint: Some(h),
                ..
            } => format!("{message} — {h}"),
            Self::Coded { message, .. } => message.clone(),
            Self::Visa(e) => e.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeInfo {
    pub index: usize,
    pub resource: String,
    pub description: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waveform {
    pub channel: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub points: usize,
    pub x_unit: String,
    pub y_unit: String,
    pub resource: String,
    pub idn: String,
}

pub fn default_save_dir() -> PathBuf {
    let rel = load_config()
        .ok()
        .map(|c| c.apps.tektronix_scope.save_dir)
        .unwrap_or_else(|| "scope_captures".into());
    let path = project_path(rel);
    let _ = fs::create_dir_all(&path);
    path
}

type Prefloat = HashMap<String, f64>;
type Cache = HashMap<usize, HashMap<String, Prefloat>>;

pub struct TektronixScopeClient {
    rm: Option<ResourceManager>,
    scopes: Vec<Instrument>,
    idns: Vec<String>,
    resources: Vec<String>,
    wfm_cache: Cache,
    units_cache: HashMap<(usize, String), String>,
}

impl Default for TektronixScopeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TektronixScopeClient {
    pub fn new() -> Self {
        Self {
            rm: None,
            scopes: Vec::new(),
            idns: Vec::new(),
            resources: Vec::new(),
            wfm_cache: HashMap::new(),
            units_cache: HashMap::new(),
        }
    }

    pub fn connected(&self) -> bool {
        !self.scopes.is_empty()
    }

    pub fn scope_count(&self) -> usize {
        self.scopes.len()
    }

    pub fn idn(&self, index: usize) -> Result<&str, ScopeError> {
        self.require(index)?;
        Ok(&self.idns[index])
    }

    pub fn resource(&self, index: usize) -> Result<&str, ScopeError> {
        self.require(index)?;
        Ok(&self.resources[index])
    }

    pub fn close(&mut self) {
        self.scopes.clear();
        self.idns.clear();
        self.resources.clear();
        self.wfm_cache.clear();
        self.units_cache.clear();
        self.rm = None;
    }

    fn require(&self, index: usize) -> Result<(), ScopeError> {
        if self.scopes.is_empty() {
            return Err(ScopeError::with_hint(
                "SCOPE_NOT_FOUND",
                "Scope not connected",
                "Connect first",
            ));
        }
        if index >= self.scopes.len() {
            return Err(ScopeError::coded(
                "INVALID_ARGS",
                format!("Scope index {index} out of range"),
            ));
        }
        Ok(())
    }

    fn handle(&self, index: usize) -> Result<&Instrument, ScopeError> {
        self.require(index)?;
        Ok(&self.scopes[index])
    }

    pub fn write(&self, cmd: &str, index: usize) -> Result<(), ScopeError> {
        self.handle(index)?
            .write_str(cmd)
            .map_err(|e| ScopeError::coded("SCOPE_IO", format!("Write failed: {cmd}: {e}")))
    }

    pub fn query(&self, cmd: &str, index: usize) -> Result<String, ScopeError> {
        self.handle(index)?
            .query_str(cmd)
            .map_err(|e| ScopeError::coded("SCOPE_IO", format!("Query failed: {cmd}: {e}")))
    }

    pub fn query_float(&self, cmd: &str, index: usize) -> Result<f64, ScopeError> {
        let s = self.query(cmd, index)?;
        s.parse::<f64>()
            .map_err(|e| ScopeError::coded("SCOPE_IO", format!("Bad float from {cmd}: {e}")))
    }

    pub fn list_scopes() -> Result<Vec<ScopeInfo>, ScopeError> {
        let rm = ResourceManager::new().map_err(|e| {
            ScopeError::with_hint(
                "SCOPE_NOT_FOUND",
                format!("VISA ResourceManager unavailable: {e}"),
                "Install NI-VISA or TekVISA drivers (match Python visa32.dll / PYVISA_LIBRARY)",
            )
        })?;
        let (resources, notes) = list_candidate_resources(&rm)?;
        tracing::info!("VISA {} — {notes}", rm.library_source());
        let mut results = Vec::new();
        let mut last_err = String::new();
        for addr in resources {
            if !is_tek_usb_resource(&addr) {
                continue;
            }
            let instr = match rm.open(&addr, 5000) {
                Ok(i) => i,
                Err(e) => {
                    last_err = format!("{addr}: {e}");
                    continue;
                }
            };
            let idn = match instr.query_str("*IDN?") {
                Ok(s) => s,
                Err(e) => {
                    last_err = format!("{addr} *IDN?: {e}");
                    continue;
                }
            };
            if !idn.to_uppercase().contains("TEKTRONIX") {
                continue;
            }
            results.push(ScopeInfo {
                index: results.len(),
                resource: addr,
                description: idn,
                vendor_id: Some(TEK_VID),
                product_id: None,
            });
        }
        if results.is_empty() && !last_err.is_empty() {
            tracing::warn!("Tektronix probe failed: {last_err}");
        }
        Ok(results)
    }

    pub fn connect(&mut self, resource: Option<&str>, index: usize) -> Result<Value, ScopeError> {
        self.close();
        let rm = ResourceManager::new().map_err(|e| {
            ScopeError::with_hint(
                "SCOPE_NOT_FOUND",
                format!("VISA unavailable: {e}"),
                "Install NI-VISA or TekVISA drivers (match Python visa32.dll / PYVISA_LIBRARY)",
            )
        })?;
        let lib = rm.library_source().to_string();
        let (resources, notes) = list_candidate_resources(&rm)?;
        tracing::info!("VISA connect via {lib} — {notes}");

        let mut candidates: Vec<(String, Instrument, String)> = Vec::new();
        let mut last_err = String::new();
        let mut seen_non_tek = Vec::new();
        for addr in &resources {
            if !is_tek_usb_resource(addr) {
                if addr.to_uppercase().contains("USB") {
                    seen_non_tek.push(addr.clone());
                }
                continue;
            }
            if let Some(want) = resource {
                if addr != want {
                    continue;
                }
            }
            let instr = match rm.open(addr, 30_000) {
                Ok(i) => i,
                Err(e) => {
                    last_err = format!("{addr}: {e}");
                    continue;
                }
            };
            let idn = match instr.query_str("*IDN?") {
                Ok(s) => s,
                Err(e) => {
                    last_err = format!("{addr} *IDN?: {e}");
                    continue;
                }
            };
            if !idn.to_uppercase().contains("TEKTRONIX") {
                last_err = format!("{addr} *IDN?={idn} (not TEKTRONIX)");
                continue;
            }
            candidates.push((addr.clone(), instr, idn));
        }

        if candidates.is_empty() {
            self.close();
            let mut hint = format!("lib={lib}; {notes}");
            if !last_err.is_empty() {
                hint.push_str("; ");
                hint.push_str(&last_err);
            }
            if !seen_non_tek.is_empty() {
                hint.push_str(&format!("; other USB: {}", seen_non_tek.join(", ")));
            }
            if resources.is_empty() {
                hint.push_str("; no VISA INSTR resources — plug USB and install Tek/NI-VISA");
            }
            return Err(ScopeError::with_hint(
                "SCOPE_NOT_FOUND",
                "No Tektronix USB scope found",
                hint,
            ));
        }
        // Python Connect always uses index 0; clamp rather than failing when
        // the UI still shows a stale Scope 2 slot before discovery finishes.
        let index = index.min(candidates.len() - 1);

        self.rm = Some(rm);
        for (addr, instr, idn) in candidates {
            self.resources.push(addr);
            self.idns.push(idn);
            self.scopes.push(instr);
        }

        Ok(json!({
            "resource": self.resources[index],
            "idn": self.idns[index],
            "index": index,
            "count": self.scopes.len(),
        }))
    }

    pub fn capture_png(
        &mut self,
        out_path: Option<&Path>,
        index: usize,
    ) -> Result<Value, ScopeError> {
        if self.scopes.is_empty() {
            let _ = self.connect(None, index)?;
        }
        self.require(index)?;
        let handle = &self.scopes[index];
        handle
            .write_str("SAVe:IMAGe:FILEFormat PNG")
            .map_err(|e| ScopeError::coded("SCOPE_CAPTURE_FAILED", e.to_string()))?;
        handle
            .write_str("SAVe:IMAGe:INKSaver ON")
            .map_err(|e| ScopeError::coded("SCOPE_CAPTURE_FAILED", e.to_string()))?;
        handle
            .write_str("HARDCopy STARt")
            .map_err(|e| ScopeError::coded("SCOPE_CAPTURE_FAILED", e.to_string()))?;
        let data = handle
            .read_raw()
            .map_err(|e| ScopeError::coded("SCOPE_CAPTURE_FAILED", e.to_string()))?;

        let path = match out_path {
            Some(p) if p.as_os_str() != "-" => {
                let path = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    project_path(p)
                };
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                path
            }
            _ => {
                let stamp = Local::now().format("%Y%m%d_%H%M%S");
                default_save_dir().join(format!("{stamp}.png"))
            }
        };
        fs::write(&path, &data)
            .map_err(|e| ScopeError::coded("SCOPE_CAPTURE_FAILED", e.to_string()))?;

        Ok(json!({
            "resource": self.resources[index],
            "idn": self.idns[index],
            "path": path.canonicalize().unwrap_or(path).to_string_lossy(),
            "bytes": data.len(),
            "format": "png",
        }))
    }

    // ── Acquisition ─────────────────────────────────────────────────────

    pub fn run(&self, index: usize) -> Result<(), ScopeError> {
        self.write("ACQuire:STOPAfter RUNSTop", index)?;
        self.write("ACQuire:STATE RUN", index)
    }

    pub fn stop(&self, index: usize) -> Result<(), ScopeError> {
        self.write("ACQuire:STATE STOP", index)
    }

    pub fn single(&self, index: usize) -> Result<(), ScopeError> {
        self.write("ACQuire:STOPAfter SEQuence", index)?;
        self.write("ACQuire:STATE RUN", index)
    }

    pub fn force_trigger(&self, index: usize) -> Result<(), ScopeError> {
        self.write("TRIGger FORCe", index)
    }

    pub fn autoset(&self, index: usize) -> Result<(), ScopeError> {
        self.write("AUTOSet EXECute", index)
    }

    pub fn default_setup(&self, index: usize) -> Result<(), ScopeError> {
        self.write("FACtory", index)
    }

    pub fn set_acquire_mode(&self, mode: &str, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("ACQuire:MODe {mode}"), index)
    }

    pub fn set_record_length(&self, length: i32, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("HORizontal:RECOrdlength {length}"), index)
    }

    // ── Horizontal ──────────────────────────────────────────────────────

    pub fn nudge_horizontal_scale(
        &mut self,
        direction: i32,
        fine: bool,
        index: usize,
    ) -> Result<f64, ScopeError> {
        let cur = self.query_float("HORizontal:SCAle?", index)?;
        let nxt = if fine {
            if direction > 0 {
                cur * 1.05
            } else {
                cur / 1.05
            }
        } else {
            nearest_step(cur, SCALE_STEPS_S, direction)
        };
        self.write(&format!("HORizontal:SCAle {nxt}"), index)?;
        self.invalidate_waveform_cache(Some(index));
        Ok(nxt)
    }

    pub fn nudge_horizontal_position(
        &self,
        direction: i32,
        fine: bool,
        index: usize,
    ) -> Result<f64, ScopeError> {
        let step = if fine { 0.5 } else { 2.0 };
        let cur = self.query_float("HORizontal:POSition?", index)?;
        let nxt = (cur + direction as f64 * step).clamp(0.0, 100.0);
        self.write(&format!("HORizontal:POSition {nxt}"), index)?;
        Ok(nxt)
    }

    pub fn center_horizontal_position(&self, index: usize) -> Result<(), ScopeError> {
        match self.query("HORizontal:DELay:MODe?", index) {
            Ok(delay) => {
                let d = delay.to_uppercase();
                if d.contains("ON") || d.trim() == "1" {
                    self.write("HORizontal:DELay:TIMe 0.0", index)
                } else {
                    self.write("HORizontal:POSition 10.0", index)
                }
            }
            Err(_) => self.write("HORizontal:POSition 50.0", index),
        }
    }

    // ── Vertical ────────────────────────────────────────────────────────

    pub fn select_channel(&self, channel: &str, on: bool, index: usize) -> Result<(), ScopeError> {
        let ch = channel.to_uppercase();
        let state = if on { "ON" } else { "OFF" };
        self.write(&format!("SELect:{ch} {state}"), index)
    }

    pub fn channel_selected(&self, channel: &str, index: usize) -> Result<bool, ScopeError> {
        let val = self
            .query(&format!("SELect:{}?", channel.to_uppercase()), index)?
            .to_uppercase();
        Ok(val == "1" || val == "ON")
    }

    pub fn nudge_channel_scale(
        &mut self,
        channel: &str,
        direction: i32,
        fine: bool,
        index: usize,
    ) -> Result<f64, ScopeError> {
        let ch = channel.to_uppercase();
        let cur = self.query_float(&format!("{ch}:SCAle?"), index)?;
        let nxt = if fine {
            if direction > 0 {
                cur * 1.05
            } else {
                cur / 1.05
            }
        } else {
            nearest_step(cur, SCALE_STEPS_V, direction)
        };
        self.write(&format!("{ch}:SCAle {nxt}"), index)?;
        self.invalidate_waveform_cache(Some(index));
        Ok(nxt)
    }

    pub fn nudge_channel_position(
        &self,
        channel: &str,
        direction: i32,
        fine: bool,
        index: usize,
    ) -> Result<f64, ScopeError> {
        let ch = channel.to_uppercase();
        let step = if fine { 0.05 } else { 0.25 };
        let cur = self.query_float(&format!("{ch}:POSition?"), index)?;
        let nxt = cur + direction as f64 * step;
        self.write(&format!("{ch}:POSition {nxt}"), index)?;
        Ok(nxt)
    }

    pub fn center_channel_position(&self, channel: &str, index: usize) -> Result<(), ScopeError> {
        let ch = channel.to_uppercase();
        self.write(&format!("{ch}:POSition 0.0"), index)
    }

    pub fn set_channel_coupling(
        &self,
        channel: &str,
        coupling: &str,
        index: usize,
    ) -> Result<(), ScopeError> {
        self.write(
            &format!("{}:COUPling {coupling}", channel.to_uppercase()),
            index,
        )
    }

    // ── Trigger ─────────────────────────────────────────────────────────

    pub fn nudge_trigger_level(
        &self,
        direction: i32,
        fine: bool,
        index: usize,
    ) -> Result<f64, ScopeError> {
        let vdiv = self.query_float("CH1:SCAle?", index).unwrap_or(0.5);
        let step = if fine { vdiv * 0.02 } else { vdiv * 0.1 };
        let cur = self.query_float("TRIGger:A:LEVel?", index)?;
        let nxt = cur + direction as f64 * step;
        self.write(&format!("TRIGger:A:LEVel {nxt}"), index)?;
        Ok(nxt)
    }

    pub fn set_trigger_level_50pct(&self, index: usize) -> Result<(), ScopeError> {
        self.write("TRIGger:A SETLevel", index)
    }

    pub fn set_trigger_source(&self, source: &str, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("TRIGger:A:EDGE:SOUrce {source}"), index)
    }

    pub fn set_trigger_slope(&self, slope: &str, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("TRIGger:A:EDGE:SLOpe {slope}"), index)
    }

    pub fn set_trigger_mode(&self, mode: &str, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("TRIGger:A:MODe {mode}"), index)
    }

    pub fn set_zoom(&self, on: bool, index: usize) -> Result<(), ScopeError> {
        let state = if on { "ON" } else { "OFF" };
        self.write(&format!("ZOOm:MODe {state}"), index)
            .or_else(|_| self.write(&format!("ZOOm:STATE {state}"), index))
    }

    pub fn set_intensity_waveform(&self, percent: f64, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("DISplay:INTENSITy:WAVEform {percent}"), index)
    }

    pub fn set_intensity_graticule(&self, percent: f64, index: usize) -> Result<(), ScopeError> {
        self.write(&format!("DISplay:INTENSITy:GRAticule {percent}"), index)
    }

    pub fn cursors_set_on(&self, on: bool, index: usize) -> Result<(), ScopeError> {
        if on {
            self.write("CURSor:FUNCtion WAVEform", index)
                .or_else(|_| self.write("CURSor:FUNCtion HBArs", index))
        } else {
            self.write("CURSor:FUNCtion OFF", index)
        }
    }

    pub fn invalidate_waveform_cache(&mut self, index: Option<usize>) {
        match index {
            None => self.wfm_cache.clear(),
            Some(i) => {
                self.wfm_cache.remove(&i);
            }
        }
    }

    pub fn read_status(&self, index: usize, light: bool) -> Result<Value, ScopeError> {
        self.require(index)?;
        let mut status = json!({
            "index": index,
            "idn": self.idns[index],
        });
        if light {
            status["acquire_state"] = json!(self.query("ACQuire:STATE?", index).ok());
            let mut channels = serde_json::Map::new();
            for ch in ["CH1", "CH2", "CH3", "CH4"] {
                let selected = self.channel_selected(ch, index).unwrap_or(false);
                channels.insert(ch.into(), json!({ "selected": selected }));
            }
            status["channels"] = Value::Object(channels);
            return Ok(status);
        }

        for (key, cmd) in [
            ("acquire_state", "ACQuire:STATE?"),
            ("horizontal_scale", "HORizontal:SCAle?"),
            ("horizontal_position", "HORizontal:POSition?"),
            ("record_length", "HORizontal:RECOrdlength?"),
            ("sample_rate", "HORizontal:SAMPLERate?"),
            ("trigger_level", "TRIGger:A:LEVel?"),
            ("trigger_source", "TRIGger:A:EDGE:SOUrce?"),
            ("trigger_slope", "TRIGger:A:EDGE:SLOpe?"),
            ("trigger_mode", "TRIGger:A:MODe?"),
        ] {
            status[key] = json!(self.query(cmd, index).ok());
        }
        let mut channels = serde_json::Map::new();
        for ch in ["CH1", "CH2", "CH3", "CH4"] {
            let selected = self.channel_selected(ch, index).unwrap_or(false);
            let scale = self.query_float(&format!("{ch}:SCAle?"), index).ok();
            let position = self.query_float(&format!("{ch}:POSition?"), index).ok();
            channels.insert(
                ch.into(),
                json!({ "selected": selected, "scale": scale, "position": position }),
            );
        }
        status["channels"] = Value::Object(channels);
        Ok(status)
    }

    fn get_wfm_preamble(
        &mut self,
        channel: &str,
        index: usize,
        force: bool,
    ) -> Result<(Prefloat, String), ScopeError> {
        let ch = channel.to_uppercase();
        if !force {
            if let Some(pre) = self.wfm_cache.get(&index).and_then(|m| m.get(&ch)) {
                let unit = self
                    .units_cache
                    .get(&(index, ch.clone()))
                    .cloned()
                    .unwrap_or_else(|| "V".into());
                return Ok((pre.clone(), unit));
            }
        }
        let blob = self.handle(index)?.query_str("WFMOutpre?").ok();
        let mut pre = Prefloat::new();
        if let Some(blob) = blob {
            for part in blob
                .replace(":WFMOUTPRE:", "")
                .replace("WFMOUTPRE:", "")
                .split(';')
            {
                let part_s = part.replace(',', " ");
                let bits: Vec<&str> = part_s.split_whitespace().collect();
                if bits.len() >= 2 {
                    let key = bits[0].to_uppercase();
                    if let Ok(fval) = bits[bits.len() - 1].parse::<f64>() {
                        if key.starts_with("XINCR") {
                            pre.insert("xincr".into(), fval);
                        } else if key.starts_with("XZERO") {
                            pre.insert("xzero".into(), fval);
                        } else if key.starts_with("PT_OFF") || key == "PT_OFF" {
                            pre.insert("pt_off".into(), fval);
                        } else if key.starts_with("YMULT") {
                            pre.insert("ymult".into(), fval);
                        } else if key.starts_with("YZERO") {
                            pre.insert("yzero".into(), fval);
                        } else if key.starts_with("YOFF") {
                            pre.insert("yoff".into(), fval);
                        }
                    }
                }
            }
        }
        let yunit = self
            .handle(index)?
            .query_str("WFMOutpre:YUNit?")
            .unwrap_or_else(|_| "V".into())
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        for (key, q) in [
            ("xincr", "WFMOutpre:XINcr?"),
            ("xzero", "WFMOutpre:XZEro?"),
            ("pt_off", "WFMOutpre:PT_Off?"),
            ("ymult", "WFMOutpre:YMUlt?"),
            ("yzero", "WFMOutpre:YZEro?"),
            ("yoff", "WFMOutpre:YOFf?"),
        ] {
            if pre.contains_key(key) {
                continue;
            }
            if let Ok(v) = self.query_float(q, index) {
                pre.insert(key.into(), v);
            } else {
                pre.insert(key.into(), 0.0);
            }
        }
        self.wfm_cache
            .entry(index)
            .or_default()
            .insert(ch.clone(), pre.clone());
        self.units_cache.insert((index, ch), yunit.clone());
        Ok((pre, yunit))
    }

    pub fn read_waveform(
        &mut self,
        channel: &str,
        index: usize,
        points: Option<u32>,
        display_points: Option<usize>,
        use_cache: bool,
    ) -> Result<Waveform, ScopeError> {
        self.require(index)?;
        let ch = channel.to_uppercase();
        let max_xfer = points.unwrap_or(10_000).clamp(100, 100_000);

        self.write(&format!("DATa:SOUrce {ch}"), index)?;
        self.write("DATa:ENCdg RIBINARY", index)?;
        self.write("DATa:WIDth 1", index)?;
        self.write("DATa:STARt 1", index)?;
        self.write(&format!("DATa:STOP {max_xfer}"), index)?;

        let (pre, yunit) = self.get_wfm_preamble(&ch, index, !use_cache)?;
        self.write("CURVe?", index)?;
        let raw = self
            .handle(index)?
            .read_raw()
            .map_err(|e| ScopeError::coded("SCOPE_WAVE_FAILED", format!("CURVe? failed: {e}")))?;
        let payload = parse_ieee_block(&raw);

        let ymult = *pre.get("ymult").unwrap_or(&1.0);
        let yoff = *pre.get("yoff").unwrap_or(&0.0);
        let yzero = *pre.get("yzero").unwrap_or(&0.0);
        let xincr = *pre.get("xincr").unwrap_or(&1.0);
        let xzero = *pre.get("xzero").unwrap_or(&0.0);
        let pt_off = *pre.get("pt_off").unwrap_or(&0.0);

        let mut y: Vec<f64> = payload
            .iter()
            .map(|&b| {
                let v = b as i8 as f64;
                (v - yoff) * ymult + yzero
            })
            .collect();
        let mut x: Vec<f64> = (0..y.len())
            .map(|i| xzero + xincr * (i as f64 - pt_off))
            .collect();

        if let Some(dp) = display_points {
            if y.len() > dp {
                let (nx, ny) = downsample_minmax(&x, &y, dp);
                x = nx;
                y = ny;
            }
        }
        let n = y.len();
        Ok(Waveform {
            channel: ch,
            x,
            y,
            points: n,
            x_unit: "s".into(),
            y_unit: if yunit.is_empty() { "V".into() } else { yunit },
            resource: self.resources[index].clone(),
            idn: self.idns[index].clone(),
        })
    }

    pub fn read_waveforms(
        &mut self,
        channels: Option<&[String]>,
        index: usize,
        points: u32,
        display_points: usize,
    ) -> Result<Vec<Waveform>, ScopeError> {
        self.require(index)?;
        let chans: Vec<String> = if let Some(cs) = channels {
            cs.to_vec()
        } else {
            let mut v = Vec::new();
            for ch in ["CH1", "CH2", "CH3", "CH4"] {
                if self.channel_selected(ch, index).unwrap_or(false) {
                    v.push(ch.to_string());
                }
            }
            if v.is_empty() {
                v.push("CH1".into());
            }
            v
        };
        self.write("DATa:ENCdg RIBINARY", index)?;
        self.write("DATa:WIDth 1", index)?;
        self.write("DATa:STARt 1", index)?;
        self.write(&format!("DATa:STOP {points}"), index)?;

        let mut waves = Vec::new();
        for ch in chans {
            match self.read_waveform(&ch, index, Some(points), Some(display_points), true) {
                Ok(w) => waves.push(w),
                Err(e) => tracing::warn!("wave {ch}: {e}"),
            }
        }
        Ok(waves)
    }
}

// ── Module-level API used by CLI ────────────────────────────────────────

pub fn list_scopes() -> Result<Vec<ScopeInfo>, ScopeError> {
    TektronixScopeClient::list_scopes()
}

pub fn scope_capabilities() -> Value {
    json!({
        "list": true,
        "shot": true,
        "wave": true,
        "front_panel": true,
        "cursors": true,
        "live": true,
        "note": "Requires NI-VISA or TekVISA (visa64.dll) at runtime",
        "tek_vid": format!("0x{TEK_VID:04X}"),
    })
}

pub fn capture_shot(index: usize, out: Option<&Path>) -> Result<Value, ScopeError> {
    let mut client = TektronixScopeClient::new();
    client.capture_png(out, index)
}

pub fn read_waveform_json(
    index: usize,
    channel: &str,
    points: Option<u32>,
) -> Result<Value, ScopeError> {
    let mut client = TektronixScopeClient::new();
    client.connect(None, index)?;
    let w = client.read_waveform(channel, index, points, Some(2500), false)?;
    Ok(json!({
        "channel": w.channel,
        "points": w.points,
        "x_unit": w.x_unit,
        "y_unit": w.y_unit,
        "x": w.x,
        "y": w.y,
        "resource": w.resource,
        "idn": w.idn,
    }))
}
