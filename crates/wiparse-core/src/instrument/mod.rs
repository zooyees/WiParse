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
    detect_profile, ControlCommand, InstrumentDevice, InstrumentProfile, MeasureFunction, Reading,
    WaveformTrace,
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
    instrument: VisaInstrument,
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
        Ok(Self { instrument })
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
                });
            }
        }
    }
    resources.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(resources)
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
}
