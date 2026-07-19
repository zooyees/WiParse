//! Generic VISA/SCPI instrument support.
//!
//! The transport is intentionally separated from device commands so drivers can
//! be tested without a VISA runtime or physical instruments.

pub mod drivers;
pub mod recording;

use crate::scope::visa::{Instrument as VisaInstrument, ResourceManager, VisaError};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use thiserror::Error;

pub use drivers::{
    classify_instrument_kind, detect_profile, estimate_dc_source_channels,
    format_human_scope_reading, humanize_scope_reading_text, ControlCommand, InstrumentDevice,
    InstrumentProfile, MeasureFunction, Reading, ScopeMeasType, WaveformTrace,
};
pub use recording::{export_csv, AcquisitionBuffer, Sample};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    Oscilloscope,
    DcSource,
    ElectronicLoad,
    Multimeter,
    Generic,
}

impl InstrumentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Oscilloscope => "Oscilloscope",
            Self::DcSource => "DC Source",
            Self::ElectronicLoad => "Electronic Load",
            Self::Multimeter => "Multimeter",
            Self::Generic => "Generic SCPI",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub raw: String,
}

impl Identity {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let mut parts = raw.split(',').map(str::trim);
        Self {
            manufacturer: parts.next().unwrap_or_default().to_owned(),
            model: parts.next().unwrap_or_default().to_owned(),
            serial: parts.next().unwrap_or_default().to_owned(),
            firmware: parts.collect::<Vec<_>>().join(","),
            raw,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub screenshot: bool,
    pub waveform: bool,
    pub channels: u8,
    pub source_output: bool,
    pub source_protection: bool,
    pub load_modes: Vec<String>,
    pub measure_functions: Vec<String>,
    pub range_control: bool,
    pub nplc_control: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    pub address: String,
    pub transport: String,
    /// Detected instrument class after optional `*IDN?` probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InstrumentKind>,
    /// Parsed identity from a successful probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    /// Probe failure reason when the resource was found but not identified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum InstrumentError {
    #[error(transparent)]
    Visa(#[from] VisaError),
    #[error("SCPI I/O: {0}")]
    Io(String),
    #[error("invalid response for {command}: {response}")]
    InvalidResponse { command: String, response: String },
    #[error("value {value} outside safe range {min}..={max} {unit}")]
    OutOfRange {
        value: f64,
        min: f64,
        max: f64,
        unit: &'static str,
    },
    #[error("operation is not supported by {0}")]
    Unsupported(String),
}

pub trait Transport: Send {
    fn write(&mut self, command: &str) -> Result<(), InstrumentError>;
    fn query(&mut self, command: &str) -> Result<String, InstrumentError>;
    fn read_raw(&mut self) -> Result<Vec<u8>, InstrumentError>;
}

pub struct VisaTransport {
    /// Closed before `_rm` (declaration order). VISA sessions opened from a
    /// Resource Manager become invalid once that manager is closed — keep both
    /// alive for the lifetime of this transport.
    instrument: VisaInstrument,
    _rm: ResourceManager,
}

impl VisaTransport {
    pub fn open(resource: &str, timeout_ms: u32) -> Result<Self, InstrumentError> {
        Self::open_with_library(resource, timeout_ms, None)
    }

    pub fn open_with_library(
        resource: &str,
        timeout_ms: u32,
        library: Option<&str>,
    ) -> Result<Self, InstrumentError> {
        let rm = ResourceManager::new_with_library(library)?;
        let instrument = rm.open(resource, timeout_ms)?;
        Ok(Self {
            instrument,
            _rm: rm,
        })
    }
}

impl Transport for VisaTransport {
    fn write(&mut self, command: &str) -> Result<(), InstrumentError> {
        self.instrument.write_str(command)?;
        Ok(())
    }

    fn query(&mut self, command: &str) -> Result<String, InstrumentError> {
        Ok(self.instrument.query_str(command)?)
    }

    fn read_raw(&mut self) -> Result<Vec<u8>, InstrumentError> {
        Ok(self.instrument.read_raw()?)
    }
}

pub struct ScpiSession {
    transport: Box<dyn Transport>,
}

impl ScpiSession {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self { transport }
    }

    pub fn open(resource: &str, timeout_ms: u32) -> Result<Self, InstrumentError> {
        Self::open_with_library(resource, timeout_ms, None)
    }

    pub fn open_with_library(
        resource: &str,
        timeout_ms: u32,
        library: Option<&str>,
    ) -> Result<Self, InstrumentError> {
        Ok(Self::new(Box::new(VisaTransport::open_with_library(
            resource, timeout_ms, library,
        )?)))
    }

    pub fn write(&mut self, command: &str) -> Result<(), InstrumentError> {
        self.transport.write(command)
    }

    pub fn query(&mut self, command: &str) -> Result<String, InstrumentError> {
        self.transport.query(command)
    }

    pub fn query_f64(&mut self, command: &str) -> Result<f64, InstrumentError> {
        let response = self.query(command)?;
        response
            .trim()
            .parse()
            .map_err(|_| InstrumentError::InvalidResponse {
                command: command.to_owned(),
                response,
            })
    }

    pub fn read_raw(&mut self) -> Result<Vec<u8>, InstrumentError> {
        self.transport.read_raw()
    }

    pub fn identify(&mut self) -> Result<Identity, InstrumentError> {
        Ok(Identity::parse(self.query("*IDN?")?))
    }

    pub fn clear(&mut self) -> Result<(), InstrumentError> {
        self.write("*CLS")
    }

    pub fn reset(&mut self) -> Result<(), InstrumentError> {
        self.write("*RST")
    }

    pub fn next_error(&mut self) -> Result<String, InstrumentError> {
        self.query("SYST:ERR?")
    }
}

pub fn list_resources() -> Result<Vec<ResourceInfo>, InstrumentError> {
    list_resources_with_library(None)
}

pub fn list_resources_with_library(
    library: Option<&str>,
) -> Result<Vec<ResourceInfo>, InstrumentError> {
    let rm = ResourceManager::new_with_library(library)?;
    let mut seen = HashSet::new();
    let mut resources = Vec::new();
    for (expression, transport) in [
        ("USB?*INSTR", "USB"),
        ("TCPIP?*INSTR", "TCPIP"),
        ("TCPIP?*SOCKET", "TCPIP"),
    ] {
        for address in rm.list_resources(expression)? {
            if seen.insert(address.clone()) {
                resources.push(ResourceInfo {
                    address,
                    transport: transport.to_owned(),
                    kind: None,
                    identity: None,
                    probe_error: None,
                });
            }
        }
    }
    resources.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(resources)
}

/// Discover VISA resources and classify each by probing `*IDN?`.
///
/// Classification uses [`detect_profile`] so USB/LAN results can be routed to the
/// matching instrument card (scope / source / load / DMM).
pub fn discover_resources_with_library(
    library: Option<&str>,
    timeout_ms: u32,
) -> Result<Vec<ResourceInfo>, InstrumentError> {
    let mut resources = list_resources_with_library(library)?;
    let probe_timeout = timeout_ms.clamp(500, 10_000);
    for resource in &mut resources {
        match ScpiSession::open_with_library(&resource.address, probe_timeout, library) {
            Ok(mut session) => match session.identify() {
                Ok(identity) => {
                    let profile = detect_profile(&identity, None);
                    resource.kind = Some(profile.kind);
                    resource.identity = Some(identity);
                    resource.probe_error = None;
                }
                Err(error) => {
                    resource.kind = None;
                    resource.identity = None;
                    resource.probe_error = Some(error.to_string());
                }
            },
            Err(error) => {
                resource.kind = None;
                resource.identity = None;
                resource.probe_error = Some(error.to_string());
            }
        }
    }
    resources.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.address.cmp(&b.address))
    });
    Ok(resources)
}

fn kind_rank(kind: Option<InstrumentKind>) -> u8 {
    match kind {
        Some(InstrumentKind::Oscilloscope) => 0,
        Some(InstrumentKind::DcSource) => 1,
        Some(InstrumentKind::ElectronicLoad) => 2,
        Some(InstrumentKind::Multimeter) => 3,
        Some(InstrumentKind::Generic) => 4,
        None => 5,
    }
}

#[derive(Default)]
pub struct MockTransport {
    responses: VecDeque<(String, String)>,
    pub writes: Vec<String>,
}

impl MockTransport {
    pub fn scripted(
        items: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self {
            responses: items
                .into_iter()
                .map(|(command, response)| (command.into(), response.into()))
                .collect(),
            writes: Vec::new(),
        }
    }
}

impl Transport for MockTransport {
    fn write(&mut self, command: &str) -> Result<(), InstrumentError> {
        self.writes.push(command.to_owned());
        Ok(())
    }

    fn query(&mut self, command: &str) -> Result<String, InstrumentError> {
        self.writes.push(command.to_owned());
        let Some((expected, response)) = self.responses.pop_front() else {
            return Err(InstrumentError::Io(format!(
                "no mock response for {command}"
            )));
        };
        if expected != command {
            return Err(InstrumentError::Io(format!(
                "expected {expected}, received {command}"
            )));
        }
        Ok(response)
    }

    fn read_raw(&mut self) -> Result<Vec<u8>, InstrumentError> {
        Ok(Vec::new())
    }
}

/// Soft demo instrument for UI debugging without VISA hardware.
///
/// Answers common SCPI queries with stable fake readings and returns a tiny
/// PNG / synthetic waveform block for scope capture paths.
pub struct DemoTransport {
    kind: InstrumentKind,
    idn: String,
    pending_raw: Option<Vec<u8>>,
    wave_points: usize,
}

impl DemoTransport {
    pub fn for_kind(kind: InstrumentKind) -> Self {
        let idn = match kind {
            InstrumentKind::Oscilloscope => {
                "RIGOL TECHNOLOGIES,DS1054Z,DEMO001,00.04.04.SP4".into()
            }
            InstrumentKind::DcSource => "RIGOL TECHNOLOGIES,DP832,DEMO001,00.01.14".into(),
            InstrumentKind::ElectronicLoad => "ITECH,IT8511A+,DEMO001,1.00".into(),
            InstrumentKind::Multimeter => {
                "Keysight Technologies,34461A,DEMO001,A.03.00-02.40".into()
            }
            InstrumentKind::Generic => "WiParse,GENERIC-DEMO,DEMO001,1.0".into(),
        };
        Self {
            kind,
            idn,
            pending_raw: None,
            wave_points: 512,
        }
    }

    fn normalize(command: &str) -> String {
        command.trim().trim_end_matches('\n').to_ascii_uppercase()
    }

    fn ieee_block(payload: &[u8]) -> Vec<u8> {
        let len = payload.len().to_string();
        let mut out = Vec::with_capacity(2 + len.len() + payload.len());
        out.push(b'#');
        out.push(b'0' + len.len() as u8);
        out.extend_from_slice(len.as_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Minimal valid 1×1 RGB PNG for screenshot preview paths.
    fn demo_png() -> &'static [u8] {
        &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0xD4, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    fn demo_waveform_bytes(&self) -> Vec<u8> {
        let n = self.wave_points.clamp(64, 4096);
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let phase = (i as f64 / n as f64) * std::f64::consts::TAU * 3.0;
            let v = 128.0 + 40.0 * phase.sin();
            samples.push(v.clamp(0.0, 255.0) as u8);
        }
        Self::ieee_block(&samples)
    }

    fn arm_raw_for_write(&mut self, command: &str) {
        let cmd = Self::normalize(command);
        if cmd.contains("HARDCOPY")
            || cmd.contains("DISPLAY:DATA")
            || cmd.contains(":DISPLAY:DATA")
            || cmd.contains("SAVE:IMAGE")
        {
            self.pending_raw = Some(Self::ieee_block(Self::demo_png()));
        } else if cmd.contains("WAV:DATA") || cmd.contains("CURVE") {
            self.pending_raw = Some(self.demo_waveform_bytes());
        }
    }

    fn query_response(&self, command: &str) -> String {
        let cmd = Self::normalize(command);
        if cmd == "*IDN?" {
            return self.idn.clone();
        }
        if cmd.starts_with("SYST:ERR") || cmd.starts_with("SYSTEM:ERROR") {
            return "0,\"No error\"".into();
        }
        if cmd.contains("WAV:XINC") || cmd.contains("WFMOUTPRE:XINCR") {
            return "1.0E-6".into();
        }
        if cmd.contains("WAV:XOR") || cmd.contains("WFMOUTPRE:XZERO") {
            return "0".into();
        }
        if cmd.contains("WAV:YINC") || cmd.contains("WFMOUTPRE:YMULT") {
            return "0.01".into();
        }
        if cmd.contains("WAV:YOR") || cmd.contains("WFMOUTPRE:YOFF") {
            return "128".into();
        }
        if cmd.contains("WAV:YREF") || cmd.contains("WFMOUTPRE:YZERO") {
            return "0".into();
        }
        if cmd.contains("IMMED:UNIT") || cmd.contains("IMMED:UNITS") {
            return match self.kind {
                InstrumentKind::Oscilloscope => "Hz".into(),
                _ => "".into(),
            };
        }
        if cmd.contains("MEAS:VOLT") {
            return "5.000".into();
        }
        if cmd.contains("MEAS:CURR") {
            return "0.250".into();
        }
        if cmd.contains("MEAS:POW") {
            return "1.250".into();
        }
        if cmd.contains("MEASURE:ITEM") || cmd.contains("MEASU:ITEM") || cmd.contains(":MEASURE:ITEM")
        {
            return "1.000E+03".into();
        }
        if cmd.contains("IMMED:VALUE") || cmd.contains("IMMED:VAL") {
            return "1.000E+03".into();
        }
        if cmd == "READ?" || cmd.ends_with(":READ?") {
            return match self.kind {
                InstrumentKind::Multimeter => "3.300".into(),
                _ => "0".into(),
            };
        }
        if cmd.ends_with('?') {
            // Numeric-friendly default for unknown queries.
            return "0".into();
        }
        String::new()
    }
}

impl Transport for DemoTransport {
    fn write(&mut self, command: &str) -> Result<(), InstrumentError> {
        self.arm_raw_for_write(command);
        // Some instruments issue capture/waveform as a write that expects a following read_raw.
        // Others use query(); arm on write covers the write→read_raw path.
        let cmd = Self::normalize(command);
        if let Some(points) = cmd
            .strip_prefix(":WAV:POIN ")
            .or_else(|| cmd.strip_prefix("WAV:POIN "))
            .or_else(|| cmd.strip_prefix("DATA:STOP "))
        {
            if let Ok(n) = points.trim().parse::<usize>() {
                self.wave_points = n.clamp(64, 4096);
            }
        }
        if let Some(stop) = cmd
            .strip_prefix(":WAV:STOP ")
            .or_else(|| cmd.strip_prefix("WAV:STOP "))
        {
            if let Ok(n) = stop.trim().parse::<usize>() {
                self.wave_points = n.clamp(64, 4096);
            }
        }
        Ok(())
    }

    fn query(&mut self, command: &str) -> Result<String, InstrumentError> {
        self.arm_raw_for_write(command);
        Ok(self.query_response(command))
    }

    fn read_raw(&mut self) -> Result<Vec<u8>, InstrumentError> {
        Ok(self
            .pending_raw
            .take()
            .unwrap_or_else(|| match self.kind {
                InstrumentKind::Oscilloscope => self.demo_waveform_bytes(),
                _ => Vec::new(),
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identity_and_mock_query() {
        let mock = MockTransport::scripted([("*IDN?", "RIGOL,DP832,SN1,1.0")]);
        let mut session = ScpiSession::new(Box::new(mock));
        let id = session.identify().unwrap();
        assert_eq!(id.manufacturer, "RIGOL");
        assert_eq!(id.model, "DP832");
        assert_eq!(id.serial, "SN1");
    }

    #[test]
    fn rejects_bad_float() {
        let mock = MockTransport::scripted([("MEAS?", "not-a-number")]);
        let mut session = ScpiSession::new(Box::new(mock));
        assert!(matches!(
            session.query_f64("MEAS?"),
            Err(InstrumentError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn demo_transport_identifies_and_measures() {
        let mut session = ScpiSession::new(Box::new(DemoTransport::for_kind(
            InstrumentKind::Multimeter,
        )));
        let id = session.identify().unwrap();
        assert!(id.model.contains("34461A"));
        assert_eq!(session.query("READ?").unwrap(), "3.300");
    }
}
