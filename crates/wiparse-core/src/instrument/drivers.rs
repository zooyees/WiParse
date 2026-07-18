//! Capability-driven SCPI drivers for common bench instruments.

use super::{Capabilities, Identity, InstrumentError, InstrumentKind, ScpiSession, Transport};
use crate::scope::binary::{downsample_minmax, parse_ieee_block};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct InstrumentProfile {
    pub name: String,
    pub kind: InstrumentKind,
    pub capabilities: Capabilities,
    pub voltage_limit: (f64, f64),
    pub current_limit: (f64, f64),
    pub power_limit: (f64, f64),
}

impl InstrumentProfile {
    fn generic(kind: InstrumentKind, vendor: &str) -> Self {
        let mut capabilities = Capabilities::default();
        match kind {
            InstrumentKind::Oscilloscope => {
                capabilities.channels = 4;
                capabilities.screenshot = vendor.contains("TEKTRONIX");
                capabilities.waveform = true;
            }
            InstrumentKind::DcSource => {
                capabilities.source_output = true;
                capabilities.source_protection = true;
            }
            InstrumentKind::ElectronicLoad => {
                capabilities.load_modes = ["CC", "CV", "CR", "CP"].map(str::to_owned).to_vec();
            }
            InstrumentKind::Multimeter => {
                capabilities.measure_functions =
                    ["VOLT:DC", "VOLT:AC", "CURR:DC", "CURR:AC", "RES", "FREQ"]
                        .map(str::to_owned)
                        .to_vec();
                capabilities.range_control = true;
                capabilities.nplc_control = true;
            }
            InstrumentKind::Generic => {}
        }
        Self {
            name: format!("{} {}", vendor.trim(), kind.label())
                .trim()
                .to_owned(),
            kind,
            capabilities,
            // Conservative defaults. Profiles can only broaden these after model validation.
            voltage_limit: (0.0, 60.0),
            current_limit: (0.0, 20.0),
            power_limit: (0.0, 300.0),
        }
    }
}

pub fn detect_profile(identity: &Identity, requested: Option<InstrumentKind>) -> InstrumentProfile {
    let vendor = identity.manufacturer.to_uppercase();
    let model = identity.model.to_uppercase();
    let kind = requested.unwrap_or_else(|| {
        if vendor.contains("TEKTRONIX")
            || model.starts_with("MDO")
            || model.starts_with("MSO")
            || model.starts_with("DS")
        {
            InstrumentKind::Oscilloscope
        } else if model.starts_with("DP")
            || model.starts_with("E36")
            || model.starts_with("N67")
            || model.contains("SOURCE")
        {
            InstrumentKind::DcSource
        } else if model.starts_with("IT8")
            || model.starts_with("PLZ")
            || model.starts_with("N33")
            || model.contains("LOAD")
        {
            InstrumentKind::ElectronicLoad
        } else if vendor.contains("KEITHLEY")
            || model.starts_with("34")
            || model.starts_with("DM")
            || model.contains("DMM")
        {
            InstrumentKind::Multimeter
        } else {
            InstrumentKind::Generic
        }
    });
    let mut profile = InstrumentProfile::generic(kind, &identity.manufacturer);
    profile.name = if vendor.contains("TEKTRONIX") {
        "Tektronix oscilloscope".into()
    } else if vendor.contains("RIGOL") {
        format!("RIGOL {}", kind.label())
    } else if vendor.contains("KEYSIGHT") || vendor.contains("AGILENT") {
        format!("Keysight {}", kind.label())
    } else if vendor.contains("ITECH") {
        format!("ITECH {}", kind.label())
    } else if vendor.contains("CHROMA") {
        format!("Chroma {}", kind.label())
    } else if vendor.contains("KEITHLEY") {
        format!("Keithley {}", kind.label())
    } else {
        profile.name
    };
    if kind == InstrumentKind::Oscilloscope && vendor.contains("RIGOL") {
        profile.capabilities.screenshot = true;
    }
    profile
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasureFunction {
    DcVoltage,
    AcVoltage,
    DcCurrent,
    AcCurrent,
    Resistance,
    Frequency,
}

impl MeasureFunction {
    fn scpi(self) -> &'static str {
        match self {
            Self::DcVoltage => "VOLT:DC",
            Self::AcVoltage => "VOLT:AC",
            Self::DcCurrent => "CURR:DC",
            Self::AcCurrent => "CURR:AC",
            Self::Resistance => "RES",
            Self::Frequency => "FREQ",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::DcVoltage | Self::AcVoltage => "V",
            Self::DcCurrent | Self::AcCurrent => "A",
            Self::Resistance => "Ω",
            Self::Frequency => "Hz",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ControlCommand {
    Reset,
    Clear,
    RawWrite(String),
    RawQuery(String),
    ScopeRun,
    ScopeStop,
    ScopeSingle,
    ScopeAutoset,
    ScopeChannel {
        channel: u8,
        enabled: bool,
    },
    ScopeScale {
        channel: u8,
        volts_per_div: f64,
    },
    ScopeOffset {
        channel: u8,
        volts: f64,
    },
    ScopeTimebase(f64),
    ScopeTrigger {
        source: String,
        level: f64,
        slope: String,
    },
    SourceVoltage(f64),
    SourceCurrent(f64),
    SourceOvp(f64),
    SourceOcp(f64),
    SourceOutput(bool),
    LoadMode(String),
    LoadLevel {
        mode: String,
        value: f64,
    },
    LoadInput(bool),
    DmmFunction(MeasureFunction),
    DmmAutoRange {
        function: MeasureFunction,
        enabled: bool,
    },
    DmmRange {
        function: MeasureFunction,
        value: f64,
    },
    DmmNplc {
        function: MeasureFunction,
        value: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reading {
    pub channel: String,
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformTrace {
    pub channel: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub x_unit: String,
    pub y_unit: String,
}

pub struct InstrumentDevice {
    pub resource: String,
    pub identity: Identity,
    pub profile: InstrumentProfile,
    session: ScpiSession,
    dmm_function: MeasureFunction,
}

impl InstrumentDevice {
    pub fn connect(
        resource: impl Into<String>,
        timeout_ms: u32,
        requested: Option<InstrumentKind>,
    ) -> Result<Self, InstrumentError> {
        Self::connect_with_library(resource, timeout_ms, requested, None)
    }

    pub fn connect_with_library(
        resource: impl Into<String>,
        timeout_ms: u32,
        requested: Option<InstrumentKind>,
        library: Option<&str>,
    ) -> Result<Self, InstrumentError> {
        let resource = resource.into();
        let mut session = ScpiSession::open_with_library(&resource, timeout_ms, library)?;
        let identity = session.identify()?;
        Ok(Self::from_session(resource, session, identity, requested))
    }

    pub fn from_transport(
        resource: impl Into<String>,
        transport: Box<dyn Transport>,
        requested: Option<InstrumentKind>,
    ) -> Result<Self, InstrumentError> {
        let resource = resource.into();
        let mut session = ScpiSession::new(transport);
        let identity = session.identify()?;
        Ok(Self::from_session(resource, session, identity, requested))
    }

    fn from_session(
        resource: String,
        session: ScpiSession,
        identity: Identity,
        requested: Option<InstrumentKind>,
    ) -> Self {
        let profile = detect_profile(&identity, requested);
        Self {
            resource,
            identity,
            profile,
            session,
            dmm_function: MeasureFunction::DcVoltage,
        }
    }

    pub fn execute(&mut self, command: ControlCommand) -> Result<Option<String>, InstrumentError> {
        use ControlCommand::*;
        let result = match command {
            Reset => self.session.reset().map(|_| None),
            Clear => self.session.clear().map(|_| None),
            RawWrite(command) => self.session.write(&command).map(|_| None),
            RawQuery(command) => self.session.query(&command).map(Some),
            ScopeRun => self
                .require(InstrumentKind::Oscilloscope, "run")?
                .write("RUN")
                .map(|_| None),
            ScopeStop => self
                .require(InstrumentKind::Oscilloscope, "stop")?
                .write("STOP")
                .map(|_| None),
            ScopeSingle => self
                .require(InstrumentKind::Oscilloscope, "single")?
                .write("SINGLE")
                .map(|_| None),
            ScopeAutoset => self
                .require(InstrumentKind::Oscilloscope, "autoset")?
                .write("AUTOSET EXECUTE")
                .map(|_| None),
            ScopeChannel { channel, enabled } => self
                .require(InstrumentKind::Oscilloscope, "channel")?
                .write(&format!("SELECT:CH{channel} {}", on_off(enabled)))
                .map(|_| None),
            ScopeScale {
                channel,
                volts_per_div,
            } => {
                validate(volts_per_div, 1e-6, 1_000.0, "V/div")?;
                self.require(InstrumentKind::Oscilloscope, "vertical scale")?
                    .write(&format!("CH{channel}:SCALE {volts_per_div}"))
                    .map(|_| None)
            }
            ScopeOffset { channel, volts } => {
                validate(volts, -1_000.0, 1_000.0, "V")?;
                self.require(InstrumentKind::Oscilloscope, "vertical offset")?
                    .write(&format!("CH{channel}:OFFSET {volts}"))
                    .map(|_| None)
            }
            ScopeTimebase(seconds) => {
                validate(seconds, 1e-9, 1_000.0, "s/div")?;
                self.require(InstrumentKind::Oscilloscope, "timebase")?
                    .write(&format!("HORIZONTAL:SCALE {seconds}"))
                    .map(|_| None)
            }
            ScopeTrigger {
                source,
                level,
                slope,
            } => {
                validate(level, -1_000.0, 1_000.0, "V")?;
                let session = self.require(InstrumentKind::Oscilloscope, "trigger")?;
                session.write(&format!("TRIGGER:A:EDGE:SOURCE {source}"))?;
                session.write(&format!("TRIGGER:A:EDGE:SLOPE {slope}"))?;
                session
                    .write(&format!("TRIGGER:A:LEVEL {level}"))
                    .map(|_| None)
            }
            SourceVoltage(value) => {
                validate_range(value, self.profile.voltage_limit, "V")?;
                self.require(InstrumentKind::DcSource, "set voltage")?
                    .write(&format!("VOLT {value}"))
                    .map(|_| None)
            }
            SourceCurrent(value) => {
                validate_range(value, self.profile.current_limit, "A")?;
                self.require(InstrumentKind::DcSource, "set current")?
                    .write(&format!("CURR {value}"))
                    .map(|_| None)
            }
            SourceOvp(value) => {
                validate_range(value, self.profile.voltage_limit, "V")?;
                self.require(InstrumentKind::DcSource, "OVP")?
                    .write(&format!("VOLT:PROT {value}"))
                    .map(|_| None)
            }
            SourceOcp(value) => {
                validate_range(value, self.profile.current_limit, "A")?;
                self.require(InstrumentKind::DcSource, "OCP")?
                    .write(&format!("CURR:PROT {value}"))
                    .map(|_| None)
            }
            SourceOutput(enabled) => self
                .require(InstrumentKind::DcSource, "output")?
                .write(&format!("OUTP {}", on_off(enabled)))
                .map(|_| None),
            LoadMode(mode) => {
                let mode = normalize_load_mode(&self.profile, &mode)?;
                self.require(InstrumentKind::ElectronicLoad, "load mode")?
                    .write(&format!("FUNC {mode}"))
                    .map(|_| None)
            }
            LoadLevel { mode, value } => {
                let mode = normalize_load_mode(&self.profile, &mode)?;
                let (range, unit, command) = match mode {
                    "CC" => (self.profile.current_limit, "A", "CURR"),
                    "CV" => (self.profile.voltage_limit, "V", "VOLT"),
                    "CR" => ((0.001, 1_000_000.0), "Ω", "RES"),
                    "CP" => (self.profile.power_limit, "W", "POW"),
                    _ => unreachable!(),
                };
                validate_range(value, range, unit)?;
                self.require(InstrumentKind::ElectronicLoad, "load level")?
                    .write(&format!("{command} {value}"))
                    .map(|_| None)
            }
            LoadInput(enabled) => self
                .require(InstrumentKind::ElectronicLoad, "load input")?
                .write(&format!("INP {}", on_off(enabled)))
                .map(|_| None),
            DmmFunction(function) => {
                self.dmm_function = function;
                self.require(InstrumentKind::Multimeter, "measure function")?
                    .write(&format!("CONF:{}", function.scpi()))
                    .map(|_| None)
            }
            DmmAutoRange { function, enabled } => self
                .require(InstrumentKind::Multimeter, "auto range")?
                .write(&format!(
                    "SENS:{}:RANG:AUTO {}",
                    function.scpi(),
                    on_off(enabled)
                ))
                .map(|_| None),
            DmmRange { function, value } => {
                validate(value, 0.0, 1e12, function.unit())?;
                self.require(InstrumentKind::Multimeter, "range")?
                    .write(&format!("SENS:{}:RANG {value}", function.scpi()))
                    .map(|_| None)
            }
            DmmNplc { function, value } => {
                validate(value, 0.001, 100.0, "NPLC")?;
                self.require(InstrumentKind::Multimeter, "NPLC")?
                    .write(&format!("SENS:{}:NPLC {value}", function.scpi()))
                    .map(|_| None)
            }
        };
        result
    }

    pub fn read_measurements(&mut self) -> Result<Vec<Reading>, InstrumentError> {
        match self.profile.kind {
            InstrumentKind::Oscilloscope => {
                let value = self.session.query_f64("MEASUREMENT:IMMED:VALUE?")?;
                Ok(vec![Reading {
                    channel: "Immediate".into(),
                    value,
                    unit: "V".into(),
                }])
            }
            InstrumentKind::DcSource | InstrumentKind::ElectronicLoad => {
                let voltage = self.session.query_f64("MEAS:VOLT?")?;
                let current = self.session.query_f64("MEAS:CURR?")?;
                Ok(vec![
                    Reading {
                        channel: "Voltage".into(),
                        value: voltage,
                        unit: "V".into(),
                    },
                    Reading {
                        channel: "Current".into(),
                        value: current,
                        unit: "A".into(),
                    },
                    Reading {
                        channel: "Power".into(),
                        value: voltage * current,
                        unit: "W".into(),
                    },
                ])
            }
            InstrumentKind::Multimeter => Ok(vec![Reading {
                channel: self.dmm_function.scpi().into(),
                value: self.session.query_f64("READ?")?,
                unit: self.dmm_function.unit().into(),
            }]),
            InstrumentKind::Generic => Ok(vec![Reading {
                channel: "Reading".into(),
                value: self.session.query_f64("READ?")?,
                unit: String::new(),
            }]),
        }
    }

    pub fn query_error(&mut self) -> Result<String, InstrumentError> {
        self.session.next_error()
    }

    pub fn capture_scope_png(&mut self) -> Result<Vec<u8>, InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "screen capture")?;
        let vendor = self.identity.manufacturer.to_uppercase();
        if vendor.contains("TEKTRONIX") {
            self.session.write("HARDCOPY:FORMAT PNG")?;
            self.session.write("HARDCOPY START")?;
        } else if vendor.contains("RIGOL") {
            self.session.write(":DISPLAY:DATA? ON,OFF,PNG")?;
        } else {
            return Err(InstrumentError::Unsupported(format!(
                "{} screen capture",
                self.profile.name
            )));
        }
        Ok(parse_ieee_block(&self.session.read_raw()?).to_vec())
    }

    pub fn read_scope_waveform(
        &mut self,
        channel: u8,
        max_points: usize,
    ) -> Result<WaveformTrace, InstrumentError> {
        self.require(InstrumentKind::Oscilloscope, "waveform")?;
        let vendor = self.identity.manufacturer.to_uppercase();
        let (xincr, xzero, ymult, yoff, yzero, signed) = if vendor.contains("RIGOL") {
            self.session.write(&format!(":WAV:SOUR CHAN{channel}"))?;
            self.session.write(":WAV:MODE NORM")?;
            self.session.write(":WAV:FORM BYTE")?;
            (
                self.session.query_f64(":WAV:XINC?")?,
                self.session.query_f64(":WAV:XOR?").unwrap_or(0.0),
                self.session.query_f64(":WAV:YINC?")?,
                self.session.query_f64(":WAV:YOR?")?,
                self.session.query_f64(":WAV:YREF?").unwrap_or(0.0),
                false,
            )
        } else {
            self.session.write(&format!("DATA:SOURCE CH{channel}"))?;
            self.session.write("DATA:ENCDG RIBINARY")?;
            self.session.write("DATA:WIDTH 1")?;
            (
                self.session.query_f64("WFMOUTPRE:XINCR?")?,
                self.session.query_f64("WFMOUTPRE:XZERO?")?,
                self.session.query_f64("WFMOUTPRE:YMULT?")?,
                self.session.query_f64("WFMOUTPRE:YOFF?")?,
                self.session.query_f64("WFMOUTPRE:YZERO?")?,
                true,
            )
        };
        self.session.write(if vendor.contains("RIGOL") {
            ":WAV:DATA?"
        } else {
            "CURVE?"
        })?;
        let raw = self.session.read_raw()?;
        let data = parse_ieee_block(&raw);
        let mut x = Vec::with_capacity(data.len());
        let mut y = Vec::with_capacity(data.len());
        for (index, byte) in data.iter().copied().enumerate() {
            let code = if signed {
                (byte as i8) as f64
            } else {
                byte as f64
            };
            x.push(xzero + index as f64 * xincr);
            y.push((code - yoff) * ymult + yzero);
        }
        let (x, y) = downsample_minmax(&x, &y, max_points.max(2));
        Ok(WaveformTrace {
            channel: format!("CH{channel}"),
            x,
            y,
            x_unit: "s".into(),
            y_unit: "V".into(),
        })
    }

    fn require(
        &mut self,
        kind: InstrumentKind,
        operation: &str,
    ) -> Result<&mut ScpiSession, InstrumentError> {
        if self.profile.kind != kind {
            return Err(InstrumentError::Unsupported(format!(
                "{} ({operation})",
                self.profile.name
            )));
        }
        Ok(&mut self.session)
    }
}

fn normalize_load_mode<'a>(
    profile: &InstrumentProfile,
    mode: &'a str,
) -> Result<&'a str, InstrumentError> {
    let mode = mode.trim();
    if profile
        .capabilities
        .load_modes
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(mode))
    {
        Ok(match mode.to_ascii_uppercase().as_str() {
            "CC" => "CC",
            "CV" => "CV",
            "CR" => "CR",
            "CP" => "CP",
            _ => mode,
        })
    } else {
        Err(InstrumentError::Unsupported(format!("load mode {mode}")))
    }
}

fn validate_range(
    value: f64,
    range: (f64, f64),
    unit: &'static str,
) -> Result<(), InstrumentError> {
    validate(value, range.0, range.1, unit)
}

fn validate(value: f64, min: f64, max: f64, unit: &'static str) -> Result<(), InstrumentError> {
    if value.is_finite() && value >= min && value <= max {
        Ok(())
    } else {
        Err(InstrumentError::OutOfRange {
            value,
            min,
            max,
            unit,
        })
    }
}

fn on_off(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument::MockTransport;

    #[test]
    fn detects_common_profiles() {
        let id = Identity::parse("TEKTRONIX,MDO3014,1,2");
        let profile = detect_profile(&id, None);
        assert_eq!(profile.kind, InstrumentKind::Oscilloscope);
        assert!(profile.capabilities.waveform);

        let id = Identity::parse("ITECH,IT8512A,1,2");
        assert_eq!(
            detect_profile(&id, None).kind,
            InstrumentKind::ElectronicLoad
        );
    }

    #[test]
    fn source_safety_check_prevents_write() {
        let mock = MockTransport::scripted([("*IDN?", "RIGOL,DP832,SN,1")]);
        let mut device = InstrumentDevice::from_transport(
            "MOCK",
            Box::new(mock),
            Some(InstrumentKind::DcSource),
        )
        .unwrap();
        assert!(matches!(
            device.execute(ControlCommand::SourceVoltage(1000.0)),
            Err(InstrumentError::OutOfRange { .. })
        ));
    }

    #[test]
    fn reads_source_measurements() {
        let mock = MockTransport::scripted([
            ("*IDN?", "RIGOL,DP832,SN,1"),
            ("MEAS:VOLT?", "5"),
            ("MEAS:CURR?", "2"),
        ]);
        let mut device = InstrumentDevice::from_transport(
            "MOCK",
            Box::new(mock),
            Some(InstrumentKind::DcSource),
        )
        .unwrap();
        let values = device.read_measurements().unwrap();
        assert_eq!(values[2].value, 10.0);
    }
}
